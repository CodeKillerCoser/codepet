//! Host-owned input ledger. Only explicit columns are persisted; no request hashes.
use codepet_gateway_sdk as g;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, g::ProtocolError>;
pub(crate) fn error(code: &str, message: &str) -> g::ProtocolError {
    g::ProtocolError {
        code: code.into(),
        message: message.into(),
        retryable: false,
        details: None,
    }
}
fn db_error(e: impl ToString) -> g::ProtocolError {
    error("input_queue_storage_error", &e.to_string())
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) struct InputQueueStore {
    path: PathBuf,
}
#[derive(Clone)]
pub(crate) struct PendingInput {
    pub item: g::InputQueueItem,
}

pub(crate) struct InputDispatchGuard<'a> {
    store: &'a InputQueueStore,
    route: g::RoutedResourceId,
    id: String,
    completed: bool,
}
impl InputDispatchGuard<'_> {
    pub fn complete(mut self) {
        self.completed = true;
    }
}
impl Drop for InputDispatchGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            let failure = error(
                "input_dispatch_cancelled",
                "Dispatch was interrupted before its outcome was confirmed",
            );
            let _ = self
                .store
                .finish(&self.route, &self.id, None, Some((&failure, true)));
        }
    }
}

impl InputQueueStore {
    pub fn dispatch_guard(&self, route: &g::RoutedResourceId, id: &str) -> InputDispatchGuard<'_> {
        InputDispatchGuard {
            store: self,
            route: route.clone(),
            id: id.into(),
            completed: false,
        }
    }
    pub fn open(path: &Path) -> crate::HostResult<Self> {
        let mut db = crate::storage::open(path)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(crate::storage::error)?;
        tx.execute_batch(include_str!("input_queue.sql"))
            .map_err(crate::storage::error)?;
        // A crash between intent and acknowledgement cannot safely be retried.
        tx.execute("UPDATE conversation_inputs SET state='outcomeUnknown',error_code='host_restarted',error_message='Host restarted before the send outcome was confirmed' WHERE state IN ('dispatching','steering')",[]).map_err(crate::storage::error)?;
        tx.execute("UPDATE conversations SET queue_paused=1,queue_revision=queue_revision+1,updated_at=?1 WHERE EXISTS(SELECT 1 FROM conversation_inputs i WHERE i.conversation_id=conversations.id AND i.state IN ('queued','blocked','outcomeUnknown'))",[now()]).map_err(crate::storage::error)?;
        tx.commit().map_err(crate::storage::error)?;
        Ok(Self { path: path.into() })
    }
    fn db(&self) -> Result<Connection> {
        crate::storage::open(&self.path).map_err(db_error)
    }

    pub fn snapshot(
        &self,
        route: &g::RoutedResourceId,
        caller: &str,
        cursor: String,
    ) -> Result<g::InputQueueSnapshot> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(db_error)?;
        let Some(cid) = find_conversation(&tx, route)? else {
            return Ok(g::InputQueueSnapshot {
                conversation: route.clone(),
                revision: 0,
                paused: false,
                items: vec![],
                recoverable_edits: vec![],
                snapshot_cursor: cursor,
            });
        };
        let (revision, paused) = queue_state(&tx, cid)?;
        let mut statement=tx.prepare(&format!("{ITEM_SELECT} WHERE conversation_id=?1 AND (state IN ('queued','dispatching','steering','blocked','outcomeUnknown') OR (state='withdrawn' AND withdraw_kind='edit' AND withdraw_caller=?2 AND edit_acknowledged=0)) ORDER BY sequence")).map_err(db_error)?;
        let rows = statement
            .query_map(params![cid, caller], |r| read_item(r, route))
            .map_err(db_error)?;
        let mut items = Vec::new();
        let mut edits = Vec::new();
        for row in rows {
            let item = row.map_err(db_error)?;
            if item.state == g::InputQueueItemState::Withdrawn {
                edits.push(item)
            } else {
                items.push(item)
            }
        }
        Ok(g::InputQueueSnapshot {
            conversation: route.clone(),
            revision,
            paused,
            items,
            recoverable_edits: edits,
            snapshot_cursor: cursor,
        })
    }

    pub fn revision(&self, route: &g::RoutedResourceId) -> Result<Option<u64>> {
        let db = self.db()?;
        db.query_row("SELECT queue_revision FROM conversations WHERE provider_instance_id=?1 AND native_conversation_id=?2 AND EXISTS(SELECT 1 FROM conversation_inputs WHERE conversation_id=conversations.id)", params![route.provider_id,route.native_resource_id], |r| r.get(0)).optional().map_err(db_error)
    }

    pub fn existing_enqueue(
        &self,
        caller: &str,
        request: &g::InputQueueEnqueueRequest,
    ) -> Result<Option<g::InputQueueMutationResponse>> {
        let db = self.db()?;
        let Some(cid) = find_conversation(&db, &request.conversation)? else {
            return Ok(None);
        };
        existing_enqueue(&db, cid, caller, request)
    }
    pub fn enqueue(
        &self,
        caller: &str,
        request: &g::InputQueueEnqueueRequest,
        conversation: &g::Conversation,
    ) -> Result<g::InputQueueMutationResponse> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute("INSERT INTO conversations(provider_instance_id,native_conversation_id,title,activity_scope_id,activity_version,created_at,updated_at) SELECT ?1,?2,?3,'host',last_version,?4,?4 FROM activity_scopes WHERE id='host' ON CONFLICT(provider_instance_id,native_conversation_id) DO NOTHING",params![request.conversation.provider_id,request.conversation.native_resource_id,conversation.title,now()]).map_err(db_error)?;
        let cid = conversation_id(&tx, &request.conversation)?;
        if let Some(existing) = existing_enqueue(&tx, cid, caller, request)? {
            return Ok(existing);
        }
        // Bound the whole visible snapshot, including recoverable edits. JSON
        // escaping can expand UTF-8 bodies sixfold; 1 MiB stays under wire limits.
        let (count, bytes): (i64, i64) = tx.query_row("SELECT count(*),coalesce(sum(length(CAST(body AS BLOB))),0) FROM conversation_inputs WHERE conversation_id=?1 AND (state IN ('queued','dispatching','steering','blocked','outcomeUnknown') OR (state='withdrawn' AND withdraw_kind='edit' AND edit_acknowledged=0))", [cid], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_error)?;
        if count >= 100 || bytes + request.input.text.len() as i64 > 1024 * 1024 {
            return Err(error("input_queue_full", "Pending inputs and recoverable edits are limited to 100 messages and 1 MiB per conversation"));
        }
        let seq: u64 = tx
            .query_row(
                "SELECT next_queue_sequence FROM conversations WHERE id=?1",
                [cid],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let id = uuid::Uuid::new_v4().to_string();
        let (kind, provider, model) = model_columns(&request.selection.model);
        tx.execute("INSERT INTO conversation_inputs(id,conversation_id,owner,request_id,sequence,capability_revision,body,access_mode,reasoning_effort,model_kind,model_provider,model_id,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'queued',?13,?13)",params![id,cid,caller,request.client_request_id,seq,request.capability_revision,request.input.text,request.selection.access_mode_id,request.selection.reasoning_effort_id,kind,provider,model,now()]).map_err(db_error)?;
        tx.execute(
            "UPDATE conversations SET next_queue_sequence=next_queue_sequence+1 WHERE id=?1",
            [cid],
        )
        .map_err(db_error)?;
        if let Some(turn) = &conversation.active_turn {
            remember_active(&tx, cid, &turn.resource.native_resource_id)?;
        }
        bump(&tx, cid)?;
        let response = mutation(&tx, cid, &id, &request.conversation)?;
        tx.commit().map_err(db_error)?;
        Ok(response)
    }

    pub fn withdraw(
        &self,
        caller: &str,
        request: &g::InputQueueWithdrawRequest,
    ) -> Result<g::InputQueueMutationResponse> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let cid = conversation_id(&tx, &request.conversation)?;
        let kind = match request.kind {
            g::InputQueueWithdrawKind::Edit => "edit",
            g::InputQueueWithdrawKind::Delete => "delete",
        };
        if operation(
            &tx,
            cid,
            caller,
            &request.client_request_id,
            kind,
            &request.item_id,
            None,
            None,
            request.expected_revision,
        )? {
            return mutation(&tx, cid, &request.item_id, &request.conversation);
        }
        check_revision(&tx, cid, request.expected_revision)?;
        let item = get_item(&tx, cid, &request.item_id, &request.conversation)?;
        let allowed = matches!(
            item.state,
            g::InputQueueItemState::Queued | g::InputQueueItemState::Blocked
        ) || (item.state == g::InputQueueItemState::OutcomeUnknown
            && kind == "delete");
        if !allowed {
            return Err(error(
                "input_already_dispatched",
                "This message has already started sending and cannot be withdrawn for editing",
            ));
        }
        tx.execute("UPDATE conversation_inputs SET state='withdrawn',withdraw_kind=?2,withdraw_caller=?3,updated_at=?4 WHERE id=?1",params![request.item_id,kind,caller,now()]).map_err(db_error)?;
        insert_operation(
            &tx,
            cid,
            caller,
            &request.client_request_id,
            kind,
            &request.item_id,
            None,
            None,
            request.expected_revision,
        )?;
        bump(&tx, cid)?;
        let response = mutation(&tx, cid, &request.item_id, &request.conversation)?;
        tx.commit().map_err(db_error)?;
        Ok(response)
    }
    pub fn acknowledge_edit(
        &self,
        caller: &str,
        request: &g::InputQueueAcknowledgeEditRequest,
    ) -> Result<()> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let cid = conversation_id(&tx, &request.conversation)?;
        let changed=tx.execute("UPDATE conversation_inputs SET edit_acknowledged=1 WHERE conversation_id=?1 AND id=?2 AND withdraw_caller=?3 AND withdraw_kind='edit' AND edit_acknowledged=0",params![cid,request.item_id,caller]).map_err(db_error)?;
        if changed > 0 {
            bump(&tx, cid)?;
        }
        tx.commit().map_err(db_error)
    }
    pub fn resume(
        &self,
        request: &g::InputQueueResumeRequest,
        active: Option<&g::RoutedResourceId>,
    ) -> Result<()> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let cid = conversation_id(&tx, &request.conversation)?;
        check_revision(&tx, cid, request.expected_revision)?;
        let uncertain:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM conversation_inputs WHERE conversation_id=?1 AND state IN ('dispatching','steering','outcomeUnknown'))",[cid],|r|r.get(0)).map_err(db_error)?;
        if uncertain {
            return Err(error("input_outcome_unknown","Verify the uncertain message in the conversation and remove its queue record before resuming"));
        }
        tx.execute("UPDATE conversation_inputs SET state='queued',error_code=NULL,error_message=NULL WHERE conversation_id=?1 AND state='blocked'",[cid]).map_err(db_error)?;
        tx.execute("UPDATE conversations SET queue_paused=0 WHERE id=?1", [cid])
            .map_err(db_error)?;
        tx.execute("INSERT INTO input_queue_runtime VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET active_turn_id=excluded.active_turn_id",params![cid,active.map(|r|&r.native_resource_id)]).map_err(db_error)?;
        bump(&tx, cid)?;
        tx.commit().map_err(db_error)
    }
    pub fn pause(&self, route: &g::RoutedResourceId) -> Result<()> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        if let Some(cid) = find_conversation(&tx, route)? {
            pause(&tx, cid)?;
        }
        tx.commit().map_err(db_error)
    }
    pub fn pause_provider(&self, provider: &str) -> Result<Vec<g::RoutedResourceId>> {
        let routes = self.routes(Some(provider))?;
        for route in &routes {
            self.pause(route)?;
        }
        Ok(routes)
    }
    pub fn routes(&self, provider: Option<&str>) -> Result<Vec<g::RoutedResourceId>> {
        let db = self.db()?;
        let mut st=db.prepare("SELECT DISTINCT c.provider_instance_id,c.native_conversation_id FROM conversations c JOIN conversation_inputs i ON i.conversation_id=c.id WHERE (?1 IS NULL OR c.provider_instance_id=?1) AND i.state IN ('queued','dispatching','steering','blocked','outcomeUnknown')").map_err(db_error)?;
        let rows = st
            .query_map([provider], |r| {
                Ok(g::RoutedResourceId {
                    provider_id: r.get(0)?,
                    native_resource_id: r.get(1)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(rows)
    }
    pub fn head(&self, route: &g::RoutedResourceId) -> Result<Option<PendingInput>> {
        let db = self.db()?;
        let Some(cid) = find_conversation(&db, route)? else {
            return Ok(None);
        };
        if queue_state(&db, cid)?.1 {
            return Ok(None);
        }
        let active: Option<String> = db
            .query_row(
                "SELECT active_turn_id FROM input_queue_runtime WHERE conversation_id=?1",
                [cid],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
            .flatten();
        if active.is_some() {
            return Ok(None);
        }
        let row=db.query_row(&format!("{ITEM_SELECT} WHERE conversation_id=?1 AND state IN ('queued','dispatching','steering','blocked','outcomeUnknown') ORDER BY sequence LIMIT 1"),[cid],|r|Ok(PendingInput{item:read_item(r,route)?})).optional().map_err(db_error)?;
        Ok(row.filter(|r| r.item.state == g::InputQueueItemState::Queued))
    }
    pub fn has_pending(&self, route: &g::RoutedResourceId) -> Result<bool> {
        let db = self.db()?;
        let Some(cid) = find_conversation(&db, route)? else {
            return Ok(false);
        };
        db.query_row("SELECT EXISTS(SELECT 1 FROM conversation_inputs WHERE conversation_id=?1 AND state IN ('queued','dispatching','steering','blocked','outcomeUnknown'))",[cid],|r|r.get(0)).map_err(db_error)
    }
    pub fn remember_active(
        &self,
        route: &g::RoutedResourceId,
        turn: &g::RoutedResourceId,
    ) -> Result<()> {
        let db = self.db()?;
        if let Some(cid) = find_conversation(&db, route)? {
            remember_active(&db, cid, &turn.native_resource_id)?;
        }
        Ok(())
    }
    pub fn claim_start(&self, route: &g::RoutedResourceId, id: &str) -> Result<()> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let cid = conversation_id(&tx, route)?;
        if queue_state(&tx, cid)?.1 {
            return Err(error("input_queue_paused", "Queue is paused"));
        }
        let changed=tx.execute("UPDATE conversation_inputs SET state='dispatching',updated_at=?3 WHERE id=?1 AND conversation_id=?2 AND state='queued'",params![id,cid,now()]).map_err(db_error)?;
        if changed != 1 {
            return Err(error(
                "input_already_dispatched",
                "Queue item is no longer pending",
            ));
        }
        bump(&tx, cid)?;
        tx.commit().map_err(db_error)
    }
    pub fn steer_operation(
        &self,
        caller: &str,
        request: &g::InputQueueSteerRequest,
    ) -> Result<Option<g::InputQueueMutationResponse>> {
        let db = self.db()?;
        let cid = conversation_id(&db, &request.conversation)?;
        if operation(
            &db,
            cid,
            caller,
            &request.client_request_id,
            "steer",
            &request.item_id,
            Some(&request.turn.native_resource_id),
            Some(&request.capability_revision),
            request.expected_revision,
        )? {
            Ok(Some(mutation(
                &db,
                cid,
                &request.item_id,
                &request.conversation,
            )?))
        } else {
            Ok(None)
        }
    }
    pub fn claim_steer(
        &self,
        caller: &str,
        request: &g::InputQueueSteerRequest,
    ) -> Result<g::InputQueueItem> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let cid = conversation_id(&tx, &request.conversation)?;
        check_revision(&tx, cid, request.expected_revision)?;
        let item = get_item(&tx, cid, &request.item_id, &request.conversation)?;
        if !matches!(
            item.state,
            g::InputQueueItemState::Queued | g::InputQueueItemState::Blocked
        ) {
            return Err(error(
                "input_already_dispatched",
                "Queue item is no longer pending",
            ));
        }
        tx.execute("UPDATE conversation_inputs SET state='steering',target_turn_id=?2,error_code=NULL,error_message=NULL,updated_at=?3 WHERE id=?1",params![request.item_id,request.turn.native_resource_id,now()]).map_err(db_error)?;
        insert_operation(
            &tx,
            cid,
            caller,
            &request.client_request_id,
            "steer",
            &request.item_id,
            Some(&request.turn.native_resource_id),
            Some(&request.capability_revision),
            request.expected_revision,
        )?;
        bump(&tx, cid)?;
        tx.commit().map_err(db_error)?;
        Ok(item)
    }
    pub fn finish(
        &self,
        route: &g::RoutedResourceId,
        id: &str,
        turn: Option<&g::TurnTask>,
        failure: Option<(&g::ProtocolError, bool)>,
    ) -> Result<g::InputQueueMutationResponse> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let cid = conversation_id(&tx, route)?;
        if let Some((e, unknown)) = failure {
            tx.execute("UPDATE conversation_inputs SET state=?2,error_code=?3,error_message=?4,updated_at=?5 WHERE id=?1 AND conversation_id=?6",params![id,if unknown {"outcomeUnknown"}else{"blocked"},e.code,e.message,now(),cid]).map_err(db_error)?;
            pause(&tx, cid)?;
        } else if let Some(turn) = turn {
            tx.execute("UPDATE conversation_inputs SET state='accepted',target_turn_id=?2,error_code=NULL,error_message=NULL,updated_at=?3 WHERE id=?1 AND conversation_id=?4",params![id,turn.resource.native_resource_id,now(),cid]).map_err(db_error)?;
            remember_active(&tx, cid, &turn.resource.native_resource_id)?;
            terminal(&tx, cid, &turn.resource.native_resource_id, turn.status)?;
        }
        bump(&tx, cid)?;
        let response = mutation(&tx, cid, id, route)?;
        tx.commit().map_err(db_error)?;
        Ok(response)
    }
    pub fn observe_turn(&self, turn: &g::TurnTask) -> Result<()> {
        if !matches!(
            turn.status,
            g::TurnStatus::Completed | g::TurnStatus::Failed | g::TurnStatus::Interrupted
        ) {
            return Ok(());
        }
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        if let Some(cid) = find_conversation(&tx, &turn.conversation)? {
            let tracked: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversation_inputs WHERE conversation_id=?1)",
                    [cid],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if tracked {
                terminal(&tx, cid, &turn.resource.native_resource_id, turn.status)?;
            }
        }
        tx.commit().map_err(db_error)
    }
}

const ITEM_SELECT:&str="SELECT id,sequence,body,access_mode,reasoning_effort,model_kind,model_provider,model_id,capability_revision,state,created_at,target_turn_id,error_code,error_message,conversation_id FROM conversation_inputs";
fn read_item(
    r: &rusqlite::Row<'_>,
    route: &g::RoutedResourceId,
) -> rusqlite::Result<g::InputQueueItem> {
    let state: String = r.get(9)?;
    let state = match state.as_str() {
        "queued" => g::InputQueueItemState::Queued,
        "dispatching" => g::InputQueueItemState::Dispatching,
        "steering" => g::InputQueueItemState::Steering,
        "accepted" => g::InputQueueItemState::Accepted,
        "blocked" => g::InputQueueItemState::Blocked,
        "outcomeUnknown" => g::InputQueueItemState::OutcomeUnknown,
        "withdrawn" => g::InputQueueItemState::Withdrawn,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let model = match r.get::<_, Option<String>>(5)?.as_deref() {
        None => None,
        Some("flat") => Some(g::ModelSelection::FlatModelSelection(
            g::FlatModelSelection {
                kind: g::FlatModelCatalogKind::Flat,
                model_id: r.get(7)?,
            },
        )),
        Some("grouped") => Some(g::ModelSelection::GroupedModelSelection(
            g::GroupedModelSelection {
                kind: g::GroupedModelCatalogKind::Grouped,
                provider_id: r.get(6)?,
                model_id: r.get(7)?,
            },
        )),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(g::InputQueueItem {
        id: r.get(0)?,
        sequence: r.get(1)?,
        input: g::TurnInput {
            kind: g::TurnInputKind::Text,
            text: r.get(2)?,
        },
        selection: g::TurnSelection {
            access_mode_id: r.get(3)?,
            reasoning_effort_id: r.get(4)?,
            model,
        },
        capability_revision: r.get(8)?,
        state,
        created_at: r.get(10)?,
        target_turn: r
            .get::<_, Option<String>>(11)?
            .map(|id| g::RoutedResourceId {
                provider_id: route.provider_id.clone(),
                native_resource_id: id,
            }),
        error_code: r.get(12)?,
        error_message: r.get(13)?,
    })
}
fn model_columns(model: &Option<g::ModelSelection>) -> (Option<&str>, Option<&str>, Option<&str>) {
    match model {
        None => (None, None, None),
        Some(g::ModelSelection::FlatModelSelection(m)) => (Some("flat"), None, Some(&m.model_id)),
        Some(g::ModelSelection::GroupedModelSelection(m)) => {
            (Some("grouped"), Some(&m.provider_id), Some(&m.model_id))
        }
    }
}
fn find_conversation(db: &Connection, r: &g::RoutedResourceId) -> Result<Option<i64>> {
    db.query_row(
        "SELECT id FROM conversations WHERE provider_instance_id=?1 AND native_conversation_id=?2",
        params![r.provider_id, r.native_resource_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(db_error)
}
fn conversation_id(db: &Connection, r: &g::RoutedResourceId) -> Result<i64> {
    find_conversation(db, r)?
        .ok_or_else(|| error("input_queue_not_found", "Conversation has no input queue"))
}
fn queue_state(db: &Connection, cid: i64) -> Result<(u64, bool)> {
    db.query_row(
        "SELECT queue_revision,queue_paused FROM conversations WHERE id=?1",
        [cid],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(db_error)
}
fn bump(db: &Connection, cid: i64) -> Result<()> {
    db.execute(
        "UPDATE conversations SET queue_revision=queue_revision+1,updated_at=?2 WHERE id=?1",
        params![cid, now()],
    )
    .map_err(db_error)?;
    Ok(())
}
fn pause(db: &Connection, cid: i64) -> Result<()> {
    if db
        .execute(
            "UPDATE conversations SET queue_paused=1 WHERE id=?1 AND queue_paused=0",
            [cid],
        )
        .map_err(db_error)?
        > 0
    {
        bump(db, cid)?;
    }
    Ok(())
}
fn check_revision(db: &Connection, cid: i64, expected: u64) -> Result<()> {
    if queue_state(db, cid)?.0 != expected {
        Err(error(
            "stale_queue_revision",
            "The queue changed; refresh it before retrying",
        ))
    } else {
        Ok(())
    }
}
fn get_item(
    db: &Connection,
    cid: i64,
    id: &str,
    route: &g::RoutedResourceId,
) -> Result<g::InputQueueItem> {
    db.query_row(
        &format!("{ITEM_SELECT} WHERE conversation_id=?1 AND id=?2"),
        params![cid, id],
        |r| read_item(r, route),
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| {
        error(
            "input_queue_item_not_found",
            "Queue item was not found in this conversation",
        )
    })
}
fn mutation(
    db: &Connection,
    cid: i64,
    id: &str,
    route: &g::RoutedResourceId,
) -> Result<g::InputQueueMutationResponse> {
    Ok(g::InputQueueMutationResponse {
        item: get_item(db, cid, id, route)?,
        revision: queue_state(db, cid)?.0,
    })
}
fn existing_enqueue(
    db: &Connection,
    cid: i64,
    caller: &str,
    r: &g::InputQueueEnqueueRequest,
) -> Result<Option<g::InputQueueMutationResponse>> {
    let id:Option<String>=db.query_row("SELECT id FROM conversation_inputs WHERE conversation_id=?1 AND owner=?2 AND request_id=?3",params![cid,caller,r.client_request_id],|r|r.get(0)).optional().map_err(db_error)?;
    let Some(id) = id else { return Ok(None) };
    let response = mutation(db, cid, &id, &r.conversation)?;
    if response.item.input != r.input
        || response.item.selection != r.selection
        || response.item.capability_revision != r.capability_revision
    {
        return Err(error(
            "client_request_conflict",
            "clientRequestId was already used for a different queue input",
        ));
    }
    Ok(Some(response))
}
#[allow(clippy::too_many_arguments)]
fn operation(
    db: &Connection,
    cid: i64,
    caller: &str,
    id: &str,
    kind: &str,
    item: &str,
    turn: Option<&str>,
    cap: Option<&str>,
    revision: u64,
) -> Result<bool> {
    let saved=db.query_row("SELECT kind,item_id,target_turn_id,capability_revision,expected_revision FROM input_queue_operations WHERE conversation_id=?1 AND caller=?2 AND request_id=?3",params![cid,caller,id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,u64>(4)?))).optional().map_err(db_error)?;
    match saved {
        None => Ok(false),
        Some(s)
            if s == (
                kind.into(),
                item.into(),
                turn.map(str::to_owned),
                cap.map(str::to_owned),
                revision,
            ) =>
        {
            Ok(true)
        }
        _ => Err(error(
            "client_request_conflict",
            "Operation ID was already used for a different queue action",
        )),
    }
}
#[allow(clippy::too_many_arguments)]
fn insert_operation(
    db: &Connection,
    cid: i64,
    caller: &str,
    id: &str,
    kind: &str,
    item: &str,
    turn: Option<&str>,
    cap: Option<&str>,
    revision: u64,
) -> Result<()> {
    db.execute(
        "INSERT INTO input_queue_operations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![cid, caller, id, kind, item, turn, cap, revision],
    )
    .map_err(db_error)?;
    Ok(())
}
fn remember_active(db: &Connection, cid: i64, turn: &str) -> Result<()> {
    let terminal: Option<String> = db
        .query_row(
            "SELECT status FROM input_queue_terminal_turns WHERE conversation_id=?1 AND turn_id=?2",
            params![cid, turn],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if terminal.as_deref().is_some_and(|s| s != "completed") {
        pause(db, cid)?;
    }
    db.execute("INSERT INTO input_queue_runtime VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET active_turn_id=excluded.active_turn_id",params![cid,if terminal.is_some(){None}else{Some(turn)}]).map_err(db_error)?;
    Ok(())
}
fn terminal(db: &Connection, cid: i64, turn: &str, status: g::TurnStatus) -> Result<()> {
    let status = match status {
        g::TurnStatus::Completed => "completed",
        g::TurnStatus::Failed => "failed",
        g::TurnStatus::Interrupted => "interrupted",
        _ => return Ok(()),
    };
    db.execute(
        "INSERT OR IGNORE INTO input_queue_terminal_turns VALUES(?1,?2,?3)",
        params![cid, turn, status],
    )
    .map_err(db_error)?;
    let matched=db.execute("UPDATE input_queue_runtime SET active_turn_id=NULL WHERE conversation_id=?1 AND active_turn_id=?2",params![cid,turn]).map_err(db_error)?>0;
    if matched && status != "completed" {
        pause(db, cid)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, InputQueueStore, g::Conversation) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("app-state.sqlite");
        let db = crate::storage::open(&path).unwrap();
        db.execute("INSERT INTO providers(id,display_name,created_at,updated_at) VALUES('p','Provider',0,0)",[]).unwrap();
        db.execute(
            "INSERT INTO provider_instances VALUES('i','p','default','Local')",
            [],
        )
        .unwrap();
        let c=serde_json::from_value(json!({"resource":{"providerId":"i","nativeResourceId":"c"},"title":"Queue","status":"idle","project":null})).unwrap();
        (temp, InputQueueStore::open(&path).unwrap(), c)
    }
    fn request(c: &g::Conversation, id: &str) -> g::InputQueueEnqueueRequest {
        serde_json::from_value(json!({"conversation":c.resource,"clientRequestId":id,"capabilityRevision":"r1","input":{"kind":"text","text":format!("Input {id}")},"selection":{"model":{"kind":"grouped","providerId":"models","modelId":"model"},"accessModeId":"write","reasoningEffortId":"high"}})).unwrap()
    }
    fn turn(c: &g::Conversation, status: &str) -> g::TurnTask {
        serde_json::from_value(json!({"conversation":c.resource,"resource":{"providerId":"i","nativeResourceId":"turn"},"status":status})).unwrap()
    }
    #[test]
    fn cancelled_dispatch_is_not_retried_and_snapshot_budget_includes_edits() {
        let (_temp, store, c) = fixture();
        let first = store.enqueue("phone", &request(&c, "first"), &c).unwrap();
        store.claim_start(&c.resource, &first.item.id).unwrap();
        drop(store.dispatch_guard(&c.resource, &first.item.id));
        let snapshot = store
            .snapshot(&c.resource, "phone", "cursor".into())
            .unwrap();
        assert!(snapshot.paused);
        assert_eq!(
            snapshot.items[0].state,
            g::InputQueueItemState::OutcomeUnknown
        );
        assert!(store.head(&c.resource).unwrap().is_none());
        let mut large = request(&c, "large");
        large.input.text = "x".repeat(1024 * 1024);
        assert_eq!(
            store.enqueue("phone", &large, &c).unwrap_err().code,
            "input_queue_full"
        );
    }
    #[test]
    fn input_ledger_deduplicates_exact_payload_without_hashing_and_preserves_fifo() {
        let (_temp, store, c) = fixture();
        let a = request(&c, "a");
        let first = store.enqueue("phone", &a, &c).unwrap();
        assert_eq!(store.enqueue("phone", &a, &c).unwrap(), first);
        let mut conflict = a.clone();
        conflict.input.text.push('!');
        assert_eq!(
            store.enqueue("phone", &conflict, &c).unwrap_err().code,
            "client_request_conflict"
        );
        let second = store.enqueue("desktop", &a, &c).unwrap();
        assert_ne!(first.item.id, second.item.id);
        assert_eq!(store.head(&c.resource).unwrap().unwrap().item, first.item);
        let db = store.db().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM conversation_inputs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert!(!db
            .prepare("PRAGMA table_info(conversation_inputs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .any(|name| name.unwrap().contains("json")));
    }
    #[test]
    fn editing_is_recoverable_after_reopen_and_isolated_to_the_withdrawing_client() {
        let (temp, store, c) = fixture();
        let item = store.enqueue("phone", &request(&c, "a"), &c).unwrap();
        let withdraw = g::InputQueueWithdrawRequest {
            conversation: c.resource.clone(),
            item_id: item.item.id.clone(),
            client_request_id: "edit".into(),
            expected_revision: item.revision,
            kind: g::InputQueueWithdrawKind::Edit,
        };
        let response = store.withdraw("phone", &withdraw).unwrap();
        assert_eq!(store.withdraw("phone", &withdraw).unwrap(), response);
        drop(store);
        let store = InputQueueStore::open(&temp.path().join("app-state.sqlite")).unwrap();
        let snapshot = store
            .snapshot(&c.resource, "phone", "event-0".into())
            .unwrap();
        assert!(snapshot.items.is_empty());
        assert_eq!(snapshot.recoverable_edits[0].input, item.item.input);
        assert_eq!(snapshot.recoverable_edits[0].selection, item.item.selection);
        assert!(store
            .snapshot(&c.resource, "desktop", "event-0".into())
            .unwrap()
            .recoverable_edits
            .is_empty());
        store
            .acknowledge_edit(
                "desktop",
                &g::InputQueueAcknowledgeEditRequest {
                    conversation: c.resource.clone(),
                    item_id: item.item.id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            store
                .snapshot(&c.resource, "phone", "event-0".into())
                .unwrap()
                .recoverable_edits
                .len(),
            1
        );
        store
            .acknowledge_edit(
                "phone",
                &g::InputQueueAcknowledgeEditRequest {
                    conversation: c.resource.clone(),
                    item_id: item.item.id,
                },
            )
            .unwrap();
        assert!(store
            .snapshot(&c.resource, "phone", "event-0".into())
            .unwrap()
            .recoverable_edits
            .is_empty());
    }
    #[test]
    fn dispatch_claim_blocks_withdrawal_and_restart_never_retries_uncertain_input() {
        let (temp, store, c) = fixture();
        let first = store.enqueue("phone", &request(&c, "a"), &c).unwrap();
        store.claim_start(&c.resource, &first.item.id).unwrap();
        let snapshot = store
            .snapshot(&c.resource, "phone", "event-0".into())
            .unwrap();
        assert_eq!(
            store
                .withdraw(
                    "phone",
                    &g::InputQueueWithdrawRequest {
                        conversation: c.resource.clone(),
                        item_id: first.item.id.clone(),
                        client_request_id: "edit".into(),
                        expected_revision: snapshot.revision,
                        kind: g::InputQueueWithdrawKind::Edit
                    }
                )
                .unwrap_err()
                .code,
            "input_already_dispatched"
        );
        drop(store);
        let store = InputQueueStore::open(&temp.path().join("app-state.sqlite")).unwrap();
        let snapshot = store
            .snapshot(&c.resource, "phone", "event-0".into())
            .unwrap();
        assert!(snapshot.paused);
        assert_eq!(
            snapshot.items[0].state,
            g::InputQueueItemState::OutcomeUnknown
        );
        assert!(store.head(&c.resource).unwrap().is_none());
        assert_eq!(
            store
                .resume(
                    &g::InputQueueResumeRequest {
                        conversation: c.resource.clone(),
                        expected_revision: snapshot.revision
                    },
                    None
                )
                .unwrap_err()
                .code,
            "input_outcome_unknown"
        );
    }
    #[test]
    fn terminal_before_send_ack_advances_once_and_failure_pauses() {
        let (_temp, store, c) = fixture();
        let first = store.enqueue("phone", &request(&c, "a"), &c).unwrap();
        let second = store.enqueue("phone", &request(&c, "b"), &c).unwrap();
        store.claim_start(&c.resource, &first.item.id).unwrap();
        store.observe_turn(&turn(&c, "completed")).unwrap();
        store
            .finish(
                &c.resource,
                &first.item.id,
                Some(&turn(&c, "running")),
                None,
            )
            .unwrap();
        assert_eq!(
            store.head(&c.resource).unwrap().unwrap().item.id,
            second.item.id
        );
        store.claim_start(&c.resource, &second.item.id).unwrap();
        let mut running = turn(&c, "running");
        running.resource.native_resource_id = "second".into();
        store
            .finish(&c.resource, &second.item.id, Some(&running), None)
            .unwrap();
        let third = store.enqueue("phone", &request(&c, "third"), &c).unwrap();
        store.observe_turn(&turn(&c, "completed")).unwrap();
        assert!(store.head(&c.resource).unwrap().is_none());
        running.status = g::TurnStatus::Failed;
        store.observe_turn(&running).unwrap();
        let snapshot = store
            .snapshot(&c.resource, "phone", "event-0".into())
            .unwrap();
        assert!(snapshot.paused);
        assert_eq!(snapshot.items[0].id, third.item.id);
    }
    #[test]
    fn steer_is_exclusive_and_stale_revisions_do_not_remove_input() {
        let (_temp, store, c) = fixture();
        let first = store.enqueue("phone", &request(&c, "a"), &c).unwrap();
        let second = store.enqueue("phone", &request(&c, "b"), &c).unwrap();
        let mut steer = g::InputQueueSteerRequest {
            conversation: c.resource.clone(),
            item_id: first.item.id.clone(),
            client_request_id: "steer".into(),
            expected_revision: first.revision,
            turn: turn(&c, "running").resource,
            capability_revision: "r1".into(),
        };
        assert_eq!(
            store.claim_steer("phone", &steer).unwrap_err().code,
            "stale_queue_revision"
        );
        steer.expected_revision = second.revision;
        store.claim_steer("phone", &steer).unwrap();
        assert!(store.head(&c.resource).unwrap().is_none());
        assert!(store.steer_operation("phone", &steer).unwrap().is_some());
        assert_eq!(
            store
                .claim_start(&c.resource, &first.item.id)
                .unwrap_err()
                .code,
            "input_already_dispatched"
        );
    }
}
