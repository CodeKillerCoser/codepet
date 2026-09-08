//! Local provider implementation of the optional v1 conversation atoms.
//! Harness adapters supply complete summaries; this module has no recent policy.
use crate::conversation_query::{query_error, EnumerationProgress, SnapshotPager};
use crate::conversation_state::SharedConversationStateStore;
use crate::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct ConversationAtoms {
    fact_epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    fact_gate: std::sync::Arc<std::sync::Mutex<()>>,
    tracked_sink: std::sync::OnceLock<std::sync::Arc<TrackedEvents>>,
    lists: SnapshotPager<Conversation>,
    active: SnapshotPager<ConversationActiveEntry>,
    unread: SnapshotPager<ConversationUnreadEntry>,
}

pub fn active(status: ConversationStatus) -> bool {
    matches!(
        status,
        ConversationStatus::Running
            | ConversationStatus::WaitingApproval
            | ConversationStatus::WaitingUserInput
    )
}

pub fn shared_state_configured() -> bool {
    std::env::var_os(crate::conversation_state::CONVERSATION_STATE_PATH_ENV)
        .is_some_and(|value| !value.is_empty())
}

pub fn advertise(capabilities: &mut ProviderCapabilities) {
    if !capabilities
        .methods
        .contains(&ProviderCapability::ConversationActiveList)
    {
        capabilities
            .methods
            .push(ProviderCapability::ConversationActiveList);
    }
    // Native global read markers do not provide Host readerScope semantics.
    capabilities.conversation_list_query = Some(ConversationListQueryCapabilities {
        updated_after: true,
        ids: true,
    });
    if shared_state_configured() {
        for method in [
            ProviderCapability::ConversationUnreadList,
            ProviderCapability::ConversationMarkRead,
        ] {
            if !capabilities.methods.contains(&method) {
                capabilities.methods.push(method);
            }
        }
    }
}

pub fn with_capabilities(mut capabilities: ProviderCapabilities) -> ProviderCapabilities {
    advertise(&mut capabilities);
    capabilities
}

/// Mirror existing status facts into the optional typed active notification.
/// The Host remains the only observer that advances shared activity versions.
struct TrackedEvents {
    route: ProviderInstanceRoute,
    sink: std::sync::Arc<dyn ProviderEventSink>,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    gate: std::sync::Arc<std::sync::Mutex<()>>,
    statuses: std::sync::Mutex<BTreeMap<String, ConversationStatus>>,
    revision: std::sync::atomic::AtomicU64,
}
impl ProviderEventSink for TrackedEvents {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        let _guard = self.gate.lock().map_err(|_| {
            query_error(
                "conversation_state_unavailable",
                "fact commit lock poisoned",
            )
        })?;
        self.publish_locked(event, None)
    }
}
impl TrackedEvents {
    fn publish_locked(
        &self,
        event: ProtocolEvent,
        versions: Option<&BTreeMap<String, String>>,
    ) -> Result<(), ProtocolError> {
        self.publish_batch_locked(vec![event], versions)
    }

    fn publish_batch_locked(
        &self,
        events: Vec<ProtocolEvent>,
        versions: Option<&BTreeMap<String, String>>,
    ) -> Result<(), ProtocolError> {
        let mut states = self.statuses.lock().map_err(|_| {
            query_error(
                "conversation_state_unavailable",
                "active event lock poisoned",
            )
        })?;
        let mut next_states = states.clone();
        let mut batch = Vec::with_capacity(events.len());
        for event in events {
            if matches!(
                &event,
                ProtocolEvent::EventConversationUpserted { .. }
                    | ProtocolEvent::EventConversationDeleted { .. }
                    | ProtocolEvent::EventTurnUpserted { .. }
                    | ProtocolEvent::EventApprovalRequested { .. }
                    | ProtocolEvent::EventApprovalResolved { .. }
            ) {
                self.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let changed = if let ProtocolEvent::EventConversationUpserted { params, .. } = &event {
                let row = &params.conversation;
                let previous =
                    next_states.insert(row.resource.native_resource_id.clone(), row.status);
                (previous != Some(row.status)
                    && (active(row.status) || previous.is_some_and(active)))
                .then(|| row.clone())
            } else {
                None
            };
            batch.push(event);
            if let Some(row) = changed {
                let version = if let Some(version) =
                    versions.and_then(|values| values.get(&row.resource.native_resource_id))
                {
                    version.clone()
                } else if shared_state_configured() {
                    SharedConversationStateStore::from_env()?
                        .activity_versions(&[row.resource.clone()])?
                        .remove(0)
                } else {
                    format!(
                        "active:{:?}:{}",
                        row.status,
                        row.updated_at.unwrap_or_default()
                    )
                };
                let revision = self
                    .revision
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                batch.push(ProtocolEvent::EventConversationActiveChanged {
                    jsonrpc: "2.0".into(),
                    params: ConversationActiveChangedEvent {
                        conversation: resource(&self.route, row.resource.native_resource_id),
                        status: row.status,
                        activity_version: version,
                        active: active(row.status),
                        revision: format!("active-{revision}"),
                    },
                });
            }
        }
        self.sink.publish_batch(batch)?;
        // Failed admission must not suppress typed status changes on retry.
        *states = next_states;
        Ok(())
    }
}

pub fn routed(resource: &ProviderResourceId) -> RoutedResourceId {
    RoutedResourceId {
        provider_id: resource.provider_instance_id.clone(),
        native_resource_id: resource.native_resource_id.clone(),
    }
}

fn resource(route: &ProviderInstanceRoute, native_resource_id: String) -> ProviderResourceId {
    ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id,
    }
}

fn binding<T: serde::Serialize>(
    kind: &str,
    generation: &str,
    request: &T,
) -> Result<String, ProtocolError> {
    let mut value = serde_json::to_value(request)
        .map_err(|error| query_error("invalid_request", error.to_string()))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("cursor");
        object.remove("limit");
    }
    Ok(format!("{kind}:{generation}:{value}"))
}

impl ConversationAtoms {
    pub fn event_epoch(&self) -> u64 {
        self.fact_epoch.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn event_sink(
        &self,
        route: ProviderInstanceRoute,
        sink: std::sync::Arc<dyn ProviderEventSink>,
    ) -> std::sync::Arc<dyn ProviderEventSink> {
        self.tracked_sink
            .get_or_init(|| {
                std::sync::Arc::new(TrackedEvents {
                    route,
                    sink,
                    epoch: self.fact_epoch.clone(),
                    gate: self.fact_gate.clone(),
                    statuses: std::sync::Mutex::new(BTreeMap::new()),
                    revision: std::sync::atomic::AtomicU64::new(0),
                })
            })
            .clone()
    }

    /// Conditional application and native fact publication share this lock.
    /// The callback must not recursively publish through the event sink.
    pub fn with_fact_fence<T>(
        &self,
        expected: u64,
        apply: impl FnOnce() -> T,
    ) -> Result<Option<T>, ProtocolError> {
        let _guard = self.fact_gate.lock().map_err(|_| {
            query_error(
                "conversation_state_unavailable",
                "fact commit lock poisoned",
            )
        })?;
        if self.event_epoch() != expected {
            return Ok(None);
        }
        Ok(Some(apply()))
    }
    pub fn commit_events(
        &self,
        expected: u64,
        events: Vec<ProtocolEvent>,
    ) -> Result<bool, ProtocolError> {
        let sink = self.tracked_sink.get().ok_or_else(|| {
            query_error(
                "conversation_state_unavailable",
                "event sink is not configured",
            )
        })?;
        self.with_fact_fence(expected, || {
            let resources = events
                .iter()
                .filter_map(|event| match event {
                    ProtocolEvent::EventConversationUpserted { params, .. } => {
                        Some(params.conversation.resource.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            let versions = if shared_state_configured() && !resources.is_empty() {
                let versions =
                    SharedConversationStateStore::from_env()?.activity_versions(&resources)?;
                resources
                    .into_iter()
                    .zip(versions)
                    .map(|(id, version)| (id.native_resource_id, version))
                    .collect()
            } else {
                BTreeMap::new()
            };
            sink.publish_batch_locked(events, Some(&versions))
        })?
        .transpose()
        .map(|result| result.is_some())
    }

    pub fn list_cached(
        &self,
        generation: &str,
        request: &ConversationListRequest,
    ) -> Result<Option<ConversationListResponse>, ProtocolError> {
        validate_list(request)?;
        request
            .cursor
            .as_deref()
            .map(|cursor| {
                let page = self.lists.page(
                    &binding("list", generation, request)?,
                    cursor,
                    request.limit,
                )?;
                Ok(ConversationListResponse {
                    conversations: page.rows,
                    page_info: PageInfo {
                        next_cursor: page.next_cursor,
                    },
                })
            })
            .transpose()
    }

    pub fn list(
        &self,
        generation: &str,
        request: &ConversationListRequest,
        mut rows: Vec<Conversation>,
    ) -> Result<ConversationListResponse, ProtocolError> {
        validate_list(request)?;
        rows.retain(|row| match &request.project_filter {
            ConversationProjectFilter::ConversationProjectFilterAll(_) => true,
            ConversationProjectFilter::ConversationProjectFilterStandalone(_) => {
                row.project.is_none()
            }
            ConversationProjectFilter::ConversationProjectFilterProject(filter) => {
                row.project.as_ref().is_some_and(|project| {
                    project.native_resource_id == filter.project.native_resource_id
                })
            }
        });
        match request.query.as_ref() {
            Some(ConversationListQuery::ConversationUpdatedAfterQuery(query)) => {
                rows.retain(|row| {
                    row.updated_at
                        .is_some_and(|updated| updated >= query.updated_after)
                })
            }
            Some(ConversationListQuery::ConversationIdsQuery(query)) => {
                let ids: BTreeSet<_> = query.ids.iter().collect();
                rows.retain(|row| ids.contains(&row.resource.native_resource_id));
            }
            None => {}
        }
        sort_summaries(&mut rows);
        if matches!(
            &request.query,
            Some(ConversationListQuery::ConversationIdsQuery(_))
        ) {
            rows.sort_by(|a, b| {
                a.resource
                    .native_resource_id
                    .cmp(&b.resource.native_resource_id)
            });
        }
        if let Some(scope) = &request.reader_scope {
            SharedConversationStateStore::from_env()?.decorate_many(scope, &mut rows)?;
        }
        let page = self
            .lists
            .start(binding("list", generation, request)?, rows, request.limit)?;
        Ok(ConversationListResponse {
            conversations: page.rows,
            page_info: PageInfo {
                next_cursor: page.next_cursor,
            },
        })
    }

    pub fn active_cached(
        &self,
        generation: &str,
        request: &ConversationActiveListRequest,
    ) -> Result<Option<ConversationActiveListResponse>, ProtocolError> {
        request
            .cursor
            .as_deref()
            .map(|cursor| {
                let page = self.active.page(
                    &binding("active", generation, request)?,
                    cursor,
                    request.limit,
                )?;
                Ok(ConversationActiveListResponse {
                    conversations: page.rows,
                    page_info: PageInfo {
                        next_cursor: page.next_cursor,
                    },
                    revision: page.revision,
                })
            })
            .transpose()
    }

    pub fn active(
        &self,
        generation: &str,
        request: &ConversationActiveListRequest,
        rows: Vec<Conversation>,
    ) -> Result<ConversationActiveListResponse, ProtocolError> {
        let mut rows = rows
            .into_iter()
            .filter(|row| active(row.status))
            .collect::<Vec<_>>();
        sort_summaries(&mut rows);
        rows.sort_by(|a, b| {
            a.resource
                .native_resource_id
                .cmp(&b.resource.native_resource_id)
        });
        let versions = if shared_state_configured() {
            SharedConversationStateStore::from_env()?.activity_versions(
                &rows
                    .iter()
                    .map(|row| row.resource.clone())
                    .collect::<Vec<_>>(),
            )?
        } else {
            rows.iter()
                .map(|row| {
                    format!(
                        "active:{generation}:{:?}:{}",
                        row.status,
                        row.updated_at.unwrap_or_default()
                    )
                })
                .collect()
        };
        let entries = rows
            .into_iter()
            .zip(versions)
            .map(|(row, version)| ConversationActiveEntry {
                conversation: resource(&request.route, row.resource.native_resource_id),
                status: row.status,
                activity_version: version,
            })
            .collect();
        let page = self.active.start(
            binding("active", generation, request)?,
            entries,
            request.limit,
        )?;
        Ok(ConversationActiveListResponse {
            conversations: page.rows,
            page_info: PageInfo {
                next_cursor: page.next_cursor,
            },
            revision: page.revision,
        })
    }

    pub fn unread(
        &self,
        generation: &str,
        request: &ConversationUnreadListRequest,
    ) -> Result<ConversationUnreadListResponse, ProtocolError> {
        validate_scope(&request.reader_scope)?;
        let state = SharedConversationStateStore::from_env()?;
        let binding = binding("unread", generation, request)?;
        let page = if let Some(cursor) = &request.cursor {
            self.unread.page(&binding, cursor, request.limit)?
        } else {
            let rows = state
                .unread(&request.reader_scope, &request.route.provider_instance_id)?
                .into_iter()
                .map(|(id, read_state)| ConversationUnreadEntry {
                    conversation: resource(&request.route, id.native_resource_id),
                    read_state,
                })
                .collect();
            self.unread.start(binding, rows, request.limit)?
        };
        Ok(ConversationUnreadListResponse {
            conversations: page.rows,
            page_info: PageInfo {
                next_cursor: page.next_cursor,
            },
            revision: page.revision,
        })
    }

    pub fn mark_read(
        &self,
        request: &ConversationMarkReadRequest,
        events: &dyn ProviderEventSink,
    ) -> Result<ConversationMarkReadResponse, ProtocolError> {
        validate_scope(&request.reader_scope)?;
        let state = SharedConversationStateStore::from_env()?;
        let read_state = state.mark_read(
            &request.reader_scope,
            &routed(&request.conversation),
            &request.observed_activity_version,
        )?;
        events.publish(ProtocolEvent::EventConversationUnreadChanged {
            jsonrpc: "2.0".into(),
            params: ConversationUnreadChangedEvent {
                conversation: request.conversation.clone(),
                reader_scope: request.reader_scope.clone(),
                read_state: read_state.clone(),
                revision: format!("read:{}:{}", read_state.activity_version, read_state.unread),
            },
        })?;
        Ok(ConversationMarkReadResponse { read_state })
    }
}

pub fn validate_list(request: &ConversationListRequest) -> Result<(), ProtocolError> {
    if request.limit.is_some_and(|limit| limit == 0 || limit > 100) {
        return Err(query_error(
            "invalid_request",
            "query limit must be between 1 and 100",
        ));
    }
    if let Some(ConversationListQuery::ConversationIdsQuery(query)) = &request.query {
        if !matches!(
            request.project_filter,
            ConversationProjectFilter::ConversationProjectFilterAll(_)
        ) {
            return Err(query_error(
                "invalid_request",
                "IDs queries require projectFilter all",
            ));
        }
        if query.ids.is_empty()
            || query.ids.len() > 100
            || query.ids.iter().any(|id| id.trim().is_empty())
            || query.ids.iter().collect::<BTreeSet<_>>().len() != query.ids.len()
        {
            return Err(query_error(
                "invalid_request",
                "IDs queries require 1 to 100 distinct nonempty IDs",
            ));
        }
    }
    if request
        .reader_scope
        .as_ref()
        .is_some_and(|scope| scope.trim().is_empty())
    {
        return Err(query_error(
            "invalid_request",
            "readerScope must not be empty",
        ));
    }
    if let ConversationProjectFilter::ConversationProjectFilterProject(filter) =
        &request.project_filter
    {
        if filter.project.device_id != request.route.device_id
            || filter.project.provider_plugin_id != request.route.provider_plugin_id
            || filter.project.provider_instance_id != request.route.provider_instance_id
        {
            return Err(query_error(
                "resource_route_mismatch",
                "project belongs to another route",
            ));
        }
    }
    Ok(())
}

fn validate_scope(scope: &str) -> Result<(), ProtocolError> {
    if scope.trim().is_empty() {
        return Err(query_error(
            "invalid_request",
            "readerScope must not be empty",
        ));
    }
    Ok(())
}

pub fn sort_summaries(rows: &mut Vec<Conversation>) {
    let mut unique = BTreeMap::new();
    for row in rows.drain(..) {
        unique.insert(
            (
                row.resource.provider_id.clone(),
                row.resource.native_resource_id.clone(),
            ),
            row,
        );
    }
    rows.extend(unique.into_values());
    rows.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.resource.provider_id.cmp(&right.resource.provider_id))
            .then_with(|| {
                left.resource
                    .native_resource_id
                    .cmp(&right.resource.native_resource_id)
            })
    });
}

/// Every native page is awaited separately: dropping the caller stops discovery
/// before the next request and never abandons an unbounded blocking scan.
pub async fn collect_summaries<P: ProtocolServer + ?Sized>(
    provider: &P,
    route: &ProviderInstanceRoute,
) -> Result<Vec<Conversation>, ProtocolError> {
    let mut rows = Vec::new();
    let mut cursor = None;
    let mut progress = EnumerationProgress::default();
    loop {
        let page = provider
            .conversation_list(ConversationListRequest {
                route: route.clone(),
                project_filter: ConversationProjectFilter::ConversationProjectFilterAll(
                    ConversationProjectFilterAll {
                        kind: ConversationProjectFilterAllKind::All,
                    },
                ),
                cursor,
                limit: Some(100),
                query: None,
                reader_scope: None,
            })
            .await?;
        rows.extend(page.conversations);
        cursor = progress.advance(page.page_info.next_cursor)?;
        if cursor.is_none() {
            break;
        }
        tokio::task::yield_now().await;
    }
    Ok(rows)
}

pub fn generation_changed() -> ProtocolError {
    query_error(
        "conversation_snapshot_changed",
        "provider generation changed while collecting summaries",
    )
}
pub fn resource_route(resource: &ProviderResourceId) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
        provider_plugin_id: resource.provider_plugin_id.clone(),
        provider_instance_id: resource.provider_instance_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str, updated: Option<u64>) -> Conversation {
        serde_json::from_value(serde_json::json!({"resource":{"providerId":"instance","nativeResourceId":id}, "title":id,"status":"idle","updatedAt":updated})).unwrap()
    }
    fn request(query: serde_json::Value) -> ConversationListRequest {
        serde_json::from_value(serde_json::json!({"route":{"deviceId":"device","providerPluginId":"plugin","providerInstanceId":"instance"},"projectFilter":{"kind":"all"},"query":query,"limit":20})).unwrap()
    }
    #[test]
    fn date_filter_precedes_paging_includes_boundary_and_snapshot_is_immutable() {
        let service = ConversationAtoms::default();
        let mut request = request(serde_json::json!({"kind":"updatedAfter","updatedAfter":100}));
        let mut rows = (0..237)
            .map(|i| row(&format!("id-{i:03}"), Some(100)))
            .collect::<Vec<_>>();
        rows.extend([row("old", Some(99)), row("missing-date", None)]);
        let first = service.list("generation-1", &request, rows).unwrap();
        assert_eq!(first.conversations.len(), 20);
        assert_eq!(first.conversations[0].resource.native_resource_id, "id-000");
        request.cursor = first.page_info.next_cursor;
        let mut seen = first.conversations;
        while request.cursor.is_some() {
            let page = service
                .list_cached("generation-1", &request)
                .unwrap()
                .unwrap();
            seen.extend(page.conversations);
            request.cursor = page.page_info.next_cursor;
        }
        assert_eq!(seen.len(), 237);
        assert_eq!(seen.last().unwrap().resource.native_resource_id, "id-236");
    }
    #[test]
    fn ids_are_sorted_by_identity_and_reject_duplicates_wrong_project_and_generation() {
        let service = ConversationAtoms::default();
        let mut request = request(serde_json::json!({"kind":"ids","ids":["z","a","deleted"]}));
        request.limit = Some(1);
        let page = service
            .list(
                "generation-1",
                &request,
                vec![row("z", Some(500)), row("a", Some(1))],
            )
            .unwrap();
        assert_eq!(page.conversations[0].resource.native_resource_id, "a");
        request.cursor = page.page_info.next_cursor;
        assert_eq!(
            service
                .list_cached("generation-2", &request)
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
        request.query = Some(ConversationListQuery::ConversationIdsQuery(
            ConversationIdsQuery {
                kind: ConversationIdsQueryKind::Ids,
                ids: vec!["a".into(), "a".into()],
            },
        ));
        assert_eq!(validate_list(&request).unwrap_err().code, "invalid_request");
    }
    #[test]
    fn a_native_fact_rejects_an_older_scan_at_the_shared_commit_gate() {
        let atoms = ConversationAtoms::default();
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let output = emitted.clone();
        let route = request(serde_json::json!({"kind":"ids","ids":["a"]})).route;
        let sink = atoms.event_sink(
            route,
            std::sync::Arc::new(move |event| {
                output.lock().unwrap().push(event);
                Ok(())
            }),
        );
        let stale = atoms.event_epoch();
        sink.publish(ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".into(),
            params: ConversationUpsertedEvent {
                conversation: row("new", Some(200)),
            },
        })
        .unwrap();
        let stale_event = ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".into(),
            params: ConversationUpsertedEvent {
                conversation: row("old", Some(100)),
            },
        };
        assert!(!atoms.commit_events(stale, vec![stale_event]).unwrap());
        assert!(atoms
            .with_fact_fence(stale, || panic!("stale scan must not install"))
            .unwrap()
            .is_none());
        assert_eq!(emitted.lock().unwrap().len(), 1);
    }

    #[test]
    fn native_facts_cannot_interleave_a_committed_scan_batch() {
        let atoms = std::sync::Arc::new(ConversationAtoms::default());
        let records = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let output = records.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let route = request(serde_json::json!({"kind":"ids","ids":["a"]})).route;
        let sink = atoms.event_sink(
            route,
            std::sync::Arc::new(move |event| {
                if let ProtocolEvent::EventConversationUpserted { params, .. } = event {
                    let id = params.conversation.resource.native_resource_id;
                    if id == "batch-1" {
                        entered_tx.send(()).unwrap();
                        release_rx.lock().unwrap().recv().unwrap();
                    }
                    output.lock().unwrap().push(id);
                }
                Ok(())
            }),
        );
        let event = |id| ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".into(),
            params: ConversationUpsertedEvent {
                conversation: row(id, Some(100)),
            },
        };
        let owner = atoms.clone();
        let batch = std::thread::spawn(move || {
            owner
                .commit_events(0, vec![event("batch-1"), event("batch-2")])
                .unwrap()
        });
        entered_rx.recv().unwrap();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let native = std::thread::spawn(move || {
            attempt_tx.send(()).unwrap();
            sink.publish(event("native")).unwrap();
        });
        attempt_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        assert!(batch.join().unwrap());
        native.join().unwrap();
        assert_eq!(*records.lock().unwrap(), ["batch-1", "batch-2", "native"]);
    }

    #[test]
    fn waiting_statuses_remain_active_and_terminal_statuses_do_not() {
        for status in [
            ConversationStatus::Running,
            ConversationStatus::WaitingApproval,
            ConversationStatus::WaitingUserInput,
        ] {
            assert!(active(status));
        }
        for status in [
            ConversationStatus::Idle,
            ConversationStatus::Error,
            ConversationStatus::Archived,
        ] {
            assert!(!active(status));
        }
    }
}

pub struct SummaryDelta {
    pub event_epoch: u64,
    pub upserted: Vec<Conversation>,
    pub deleted: Vec<String>,
}

/// A single cancellable discovery loop. The adapter must confirm deletion in
/// `load`, and serialize `publish` with its native generation/lifecycle fence.
#[derive(Clone, Copy)]
pub enum SummaryPublication {
    Applied,
    Retry,
    Stop,
}

pub fn spawn_summary_poll<L, P>(
    load: L,
    publish: P,
    interval: std::time::Duration,
) -> tokio::task::JoinHandle<()>
where
    L: Fn(Vec<String>) -> ProtocolFuture<'static, (Vec<Conversation>, u64)> + Send + Sync + 'static,
    P: Fn(Result<SummaryDelta, ProtocolError>) -> SummaryPublication + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut previous = BTreeMap::<String, Conversation>::new();
        loop {
            match load(previous.keys().cloned().collect()).await {
                Ok((rows, event_epoch)) => {
                    let current = rows
                        .into_iter()
                        .map(|row| (row.resource.native_resource_id.clone(), row))
                        .collect::<BTreeMap<_, _>>();
                    let delta = SummaryDelta {
                        event_epoch,
                        upserted: current
                            .iter()
                            .filter(|(id, row)| previous.get(*id) != Some(*row))
                            .map(|(_, row)| row.clone())
                            .collect(),
                        deleted: previous
                            .keys()
                            .filter(|id| !current.contains_key(*id))
                            .cloned()
                            .collect(),
                    };
                    match publish(Ok(delta)) {
                        SummaryPublication::Applied => previous = current,
                        SummaryPublication::Retry => {}
                        SummaryPublication::Stop => return,
                    }
                }
                Err(error) => {
                    if matches!(publish(Err(error)), SummaryPublication::Stop) {
                        return;
                    }
                }
            }
            tokio::time::sleep(interval).await;
        }
    })
}

pub fn observed_capabilities(
    mut capabilities: ProviderCapabilities,
    ready: bool,
    epoch: u64,
) -> ProviderCapabilities {
    if !ready {
        capabilities
            .methods
            .retain(|method| *method != ProviderCapability::ConversationActiveList);
    }
    if epoch != 0 {
        capabilities.revision = format!("{}:observations-{epoch}", capabilities.revision);
    }
    capabilities
}

pub fn summary_delta_events(
    route: &ProviderInstanceRoute,
    delta: SummaryDelta,
) -> Vec<ProtocolEvent> {
    let mut events = delta
        .upserted
        .into_iter()
        .map(|conversation| ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".into(),
            params: ConversationUpsertedEvent { conversation },
        })
        .collect::<Vec<_>>();
    events.extend(
        delta
            .deleted
            .into_iter()
            .map(|id| ProtocolEvent::EventConversationDeleted {
                jsonrpc: "2.0".into(),
                params: ConversationDeletedEvent {
                    conversation: resource(route, id),
                },
            }),
    );
    events
}
