//! Durable Notice publication. Source interpretation stays in Providers.
use crate::{HostError, HostResult};
use codepet_provider_sdk as p;
use p::common::NoticeActionResultStatus;
use p::{Notice, NoticeAction, NoticeActionInput, NoticeActionResult};
use rusqlite::{params, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use tokio::sync::broadcast;

fn err(code: &str, message: impl Into<String>) -> p::ProtocolError {
    p::ProtocolError {
        code: code.into(),
        message: message.into(),
        retryable: false,
        details: None,
    }
}
fn db_err(error: impl std::fmt::Display) -> p::ProtocolError {
    err("notice_storage_error", error.to_string())
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone)]
pub struct NoticeStore {
    path: PathBuf,
    changed: broadcast::Sender<Notice>,
}
pub enum ActionClaim {
    Existing(NoticeActionResult),
    Execute {
        notice: Notice,
        action: NoticeAction,
    },
}
impl NoticeStore {
    pub fn open(path: &Path) -> HostResult<Arc<Self>> {
        let store = Arc::new(Self {
            path: path.into(),
            changed: broadcast::channel(256).0,
        });
        let db = crate::storage::open(path)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS notice_entries(id TEXT PRIMARY KEY,plugin_id TEXT NOT NULL,source_id TEXT NOT NULL,creation_key TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,body TEXT NOT NULL,UNIQUE(plugin_id,source_id,creation_key));
          CREATE TABLE IF NOT EXISTS notice_operations(plugin_id TEXT NOT NULL,id TEXT NOT NULL,request TEXT NOT NULL,response TEXT NOT NULL,PRIMARY KEY(plugin_id,id));
          CREATE TABLE IF NOT EXISTS notice_action_operations(id TEXT PRIMARY KEY,request TEXT NOT NULL,response TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS notice_source_preferences(plugin_id TEXT PRIMARY KEY,enabled INTEGER NOT NULL);")
            .map_err(|e|HostError::new("notice_storage_error",e.to_string()))?;
        Ok(store)
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Notice> {
        self.changed.subscribe()
    }
    fn connection(&self) -> Result<rusqlite::Connection, p::ProtocolError> {
        crate::storage::open(&self.path).map_err(db_err)
    }
    pub fn get(&self, id: &str) -> Result<Notice, p::ProtocolError> {
        let body: Option<String> = self
            .connection()?
            .query_row("SELECT body FROM notice_entries WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_err)?;
        serde_json::from_str(&body.ok_or_else(|| err("notice_not_found", "Notice does not exist"))?)
            .map_err(db_err)
    }
    pub fn list(&self, offset: u64, limit: u64) -> Result<Vec<Notice>, p::ProtocolError> {
        let db = self.connection()?;
        let mut query = db
            .prepare(
                "SELECT body FROM notice_entries ORDER BY updated_at DESC,id LIMIT ?1 OFFSET ?2",
            )
            .map_err(db_err)?;
        let rows = query
            .query_map(params![limit.min(201), offset], |r| r.get::<_, String>(0))
            .map_err(db_err)?;
        rows.map(|r| serde_json::from_str(&r.map_err(db_err)?).map_err(db_err))
            .collect()
    }
    pub fn enabled(&self, plugin: &str) -> Result<bool, p::ProtocolError> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT enabled FROM notice_source_preferences WHERE plugin_id=?1",
                [plugin],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(true))
    }
    pub fn set_enabled(&self, plugin: &str, enabled: bool) -> Result<(), p::ProtocolError> {
        self.connection()?.execute("INSERT INTO notice_source_preferences VALUES(?1,?2) ON CONFLICT(plugin_id) DO UPDATE SET enabled=excluded.enabled",params![plugin,enabled]).map_err(db_err)?;
        Ok(())
    }
    pub fn remove_plugin(&self, plugin: &str) -> Result<(), p::ProtocolError> {
        let mut db = self.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        tx.execute("DELETE FROM notice_entries WHERE plugin_id=?1", [plugin])
            .map_err(db_err)?;
        tx.execute("DELETE FROM notice_operations WHERE plugin_id=?1", [plugin])
            .map_err(db_err)?;
        tx.execute(
            "DELETE FROM notice_source_preferences WHERE plugin_id=?1",
            [plugin],
        )
        .map_err(db_err)?;
        // Keep execution receipts: a reused execution ID must never replay a side effect.
        tx.commit().map_err(db_err)
    }
    pub fn create(
        &self,
        plugin: &str,
        request: p::NoticeCreateRequest,
    ) -> Result<p::NoticePublishResponse, p::ProtocolError> {
        if request.creation_key.is_empty() || request.source_id.is_empty() {
            return Err(err(
                "invalid_params",
                "Notice requires stable creationKey and sourceId",
            ));
        }
        let mut db = self.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let old:Option<String>=tx.query_row("SELECT body FROM notice_entries WHERE plugin_id=?1 AND source_id=?2 AND creation_key=?3",params![plugin,request.source_id,request.creation_key],|r|r.get(0)).optional().map_err(db_err)?;
        if let Some(old) = old {
            return Ok(p::NoticePublishResponse {
                notice: serde_json::from_str(&old).map_err(db_err)?,
            });
        }
        let time = now();
        let notice = Notice {
            notice_id: format!("notice-{}", uuid::Uuid::new_v4()),
            plugin_id: plugin.into(),
            source_id: request.source_id,
            status: request.status,
            content: request.content,
            actions: request.actions.unwrap_or_default(),
            revision: 1,
            finished: false,
            created_at: time,
            updated_at: time,
        };
        validate_notice(&notice)?;
        tx.execute(
            "INSERT INTO notice_entries VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                notice.notice_id,
                plugin,
                notice.source_id,
                request.creation_key,
                notice.revision,
                notice.updated_at,
                serde_json::to_string(&notice).map_err(db_err)?
            ],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        let _ = self.changed.send(notice.clone());
        Ok(p::NoticePublishResponse { notice })
    }
    pub fn update(
        &self,
        plugin: &str,
        r: p::NoticeUpdateRequest,
    ) -> Result<p::NoticePublishResponse, p::ProtocolError> {
        if r.status.is_none() && r.content.is_none() && r.actions.is_none() {
            return Err(err(
                "invalid_params",
                "Notice update requires at least one change",
            ));
        }
        let fingerprint = format!("update:{}", serde_json::to_string(&r).map_err(db_err)?);
        self.mutate(
            plugin,
            &r.notice_id,
            &r.operation_id,
            r.expected_revision,
            fingerprint,
            r.status,
            r.content,
            r.actions,
            false,
        )
    }
    pub fn finish(
        &self,
        plugin: &str,
        r: p::NoticeFinishRequest,
    ) -> Result<p::NoticePublishResponse, p::ProtocolError> {
        let fingerprint = format!("finish:{}", serde_json::to_string(&r).map_err(db_err)?);
        self.mutate(
            plugin,
            &r.notice_id,
            &r.operation_id,
            r.expected_revision,
            fingerprint,
            Some(r.status),
            r.content,
            r.actions,
            true,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn mutate(
        &self,
        plugin: &str,
        id: &str,
        operation: &str,
        expected: u64,
        fingerprint: String,
        status: Option<p::NoticeStatus>,
        content: Option<p::NoticeContent>,
        actions: Option<Vec<NoticeAction>>,
        finish: bool,
    ) -> Result<p::NoticePublishResponse, p::ProtocolError> {
        if operation.is_empty() {
            return Err(err("invalid_params", "operationId is required"));
        }
        let mut db = self.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT request,response FROM notice_operations WHERE plugin_id=?1 AND id=?2",
                params![plugin, operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_err)?;
        if let Some((request, response)) = old {
            if request != fingerprint {
                return Err(err(
                    "notice_operation_conflict",
                    "Operation ID was used with different input",
                ));
            }
            return serde_json::from_str(&response).map_err(db_err);
        }
        let body: Option<String> = tx
            .query_row(
                "SELECT body FROM notice_entries WHERE id=?1 AND plugin_id=?2",
                params![id, plugin],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        let mut notice: Notice = serde_json::from_str(
            &body
                .ok_or_else(|| err("notice_not_found", "Notice does not belong to this plugin"))?,
        )
        .map_err(db_err)?;
        if notice.revision != expected {
            return Err(err(
                "notice_revision_conflict",
                "Notice revision changed; read current Notice before submitting a new operation",
            ));
        }
        if notice.finished && !finish {
            return Err(err(
                "notice_finished",
                "Use finish to correct a finished Notice",
            ));
        }
        if let Some(status) = status {
            notice.status = status;
        }
        if let Some(content) = content {
            notice.content = content;
        }
        if let Some(actions) = actions {
            notice.actions = actions;
        }
        notice.finished |= finish;
        notice.revision = notice
            .revision
            .checked_add(1)
            .ok_or_else(|| err("notice_revision_exhausted", "Notice revision exhausted"))?;
        notice.updated_at = now().max(notice.updated_at + 1);
        validate_notice(&notice)?;
        let response = p::NoticePublishResponse {
            notice: notice.clone(),
        };
        tx.execute(
            "UPDATE notice_entries SET revision=?1,updated_at=?2,body=?3 WHERE id=?4",
            params![
                notice.revision,
                notice.updated_at,
                serde_json::to_string(&notice).map_err(db_err)?,
                id
            ],
        )
        .map_err(db_err)?;
        tx.execute(
            "INSERT INTO notice_operations VALUES(?1,?2,?3,?4)",
            params![
                plugin,
                operation,
                fingerprint,
                serde_json::to_string(&response).map_err(db_err)?
            ],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        let _ = self.changed.send(notice);
        Ok(response)
    }
    pub fn claim_action(
        &self,
        r: &p::NoticeActionExecuteRequest,
    ) -> Result<ActionClaim, p::ProtocolError> {
        validate_schema(
            &ACTION_INPUT_SCHEMA,
            &serde_json::to_value(&r.input).map_err(db_err)?,
        )?;
        if r.execution_id.is_empty() {
            return Err(err("invalid_params", "executionId is required"));
        }
        let request = serde_json::to_string(r).map_err(db_err)?;
        let mut db = self.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT request,response FROM notice_action_operations WHERE id=?1",
                [&r.execution_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_err)?;
        if let Some((old, result)) = old {
            if old != request {
                return Err(err(
                    "notice_action_conflict",
                    "Execution ID was used with different input",
                ));
            }
            return Ok(ActionClaim::Existing(
                serde_json::from_str(&result).map_err(db_err)?,
            ));
        }
        let body: String = tx
            .query_row(
                "SELECT body FROM notice_entries WHERE id=?1",
                [&r.notice_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| err("notice_not_found", "Notice does not exist"))?;
        let notice: Notice = serde_json::from_str(&body).map_err(db_err)?;
        if notice.revision != r.expected_revision {
            return Err(err(
                "notice_revision_conflict",
                "Action belongs to an older Notice revision",
            ));
        }
        let input = serde_json::to_value(&r.input).map_err(db_err)?;
        let action = notice
            .actions
            .iter()
            .find(|a| {
                serde_json::to_value(a)
                    .ok()
                    .is_some_and(|v| v["id"] == r.action_id && v["enabled"] == true)
            })
            .cloned()
            .ok_or_else(|| err("notice_action_unavailable", "Action is missing or disabled"))?;
        let value = serde_json::to_value(&action).map_err(db_err)?;
        if !matches!(
            &action,
            NoticeAction::NoticeReplyAction(_)
                | NoticeAction::NoticeActivateAction(_)
                | NoticeAction::NoticePermissionAction(_)
        ) || value["kind"] != input["kind"]
        {
            return Err(err(
                "notice_action_unavailable",
                "Unsupported or mismatched action kind",
            ));
        }
        match &r.input {
            NoticeActionInput::NoticeReplyInput(v) if v.text.trim().is_empty() => {
                return Err(err("invalid_params", "Reply text is empty"))
            }
            NoticeActionInput::NoticePermissionInput(v) => {
                if !value["options"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|o| o["id"] == v.option_id))
                {
                    return Err(err(
                        "notice_action_unavailable",
                        "Permission option is no longer available",
                    ));
                }
            }
            _ => {}
        }
        let result = NoticeActionResult {
            execution_id: r.execution_id.clone(),
            status: NoticeActionResultStatus::Unknown,
            message: Some(
                "Execution has been claimed; its outcome must not be retried automatically".into(),
            ),
        };
        tx.execute(
            "INSERT INTO notice_action_operations VALUES(?1,?2,?3)",
            params![
                r.execution_id,
                request,
                serde_json::to_string(&result).map_err(db_err)?
            ],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(ActionClaim::Execute { notice, action })
    }
    pub fn complete_action(&self, result: &NoticeActionResult) -> Result<(), p::ProtocolError> {
        self.connection()?
            .execute(
                "UPDATE notice_action_operations SET response=?1 WHERE id=?2",
                params![
                    serde_json::to_string(result).map_err(db_err)?,
                    result.execution_id
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }
}
fn validate_notice(n: &Notice) -> Result<(), p::ProtocolError> {
    let mut known = serde_json::to_value(n).map_err(db_err)?;
    // Forward-compatible unknown actions remain stored but cannot be executed.
    // Apply canonical constraints to every known action; do not coerce malformed known kinds.
    known["actions"].as_array_mut().unwrap().retain(|a| {
        matches!(
            a["kind"].as_str(),
            Some("reply" | "activate" | "permission")
        )
    });
    validate_schema(&NOTICE_SCHEMA, &known)?;
    if n.status.value.trim().is_empty()
        || n.status.label.trim().is_empty()
        || n.content.title.trim().is_empty()
    {
        return Err(err(
            "invalid_params",
            "Status value/label and content title are required",
        ));
    }
    if n.content
        .progress
        .as_ref()
        .and_then(|v| v.percent)
        .is_some_and(|v| v > 100)
    {
        return Err(err("invalid_params", "Progress percent must be 0..100"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for action in &n.actions {
        let v = serde_json::to_value(action).map_err(db_err)?;
        let id = v["id"].as_str().unwrap_or("");
        if id.is_empty() || !ids.insert(id.to_owned()) {
            return Err(err(
                "invalid_params",
                "Action IDs must be nonempty and unique",
            ));
        }
        if v["enabled"] == true {
            match action {
                NoticeAction::NoticeActivateAction(a) if a.target.is_none() => {
                    return Err(err("invalid_params", "Enabled activate requires target"))
                }
                NoticeAction::NoticePermissionAction(a)
                    if a.options.as_ref().is_none_or(|a| a.is_empty()) =>
                {
                    return Err(err("invalid_params", "Enabled permission requires options"))
                }
                _ => {}
            }
        }
    }
    Ok(())
}
fn validator(name: &str) -> Result<jsonschema::Validator, String> {
    let mut schema: serde_json::Value =
        serde_json::from_str(include_str!("notice.schema.generated.json"))
            .map_err(|e| e.to_string())?;
    schema["$ref"] = format!("#/$defs/{name}").into();
    jsonschema::draft202012::options()
        .build(&schema)
        .map_err(|e| e.to_string())
}
static NOTICE_SCHEMA: LazyLock<Result<jsonschema::Validator, String>> =
    LazyLock::new(|| validator("Notice"));
static ACTION_INPUT_SCHEMA: LazyLock<Result<jsonschema::Validator, String>> =
    LazyLock::new(|| validator("NoticeActionInput"));
fn validate_schema(
    schema: &Result<jsonschema::Validator, String>,
    value: &serde_json::Value,
) -> Result<(), p::ProtocolError> {
    schema
        .as_ref()
        .map_err(|_| err("notice_schema_error", "Bundled Notice Schema is invalid"))?
        .validate(value)
        .map_err(|e| {
            err(
                "invalid_params",
                format!(
                    "Invalid Notice at {} ({})",
                    e.instance_path(),
                    e.kind().keyword()
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn decode<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }
    fn create(store: &NoticeStore) -> Notice {
        store.create("observer",decode(json!({"creationKey":"external-1","sourceId":"chat","status":{"value":"provider-custom","label":"处理中"},"content":{"title":"标题"},"actions":[{"id":"reply","kind":"reply","label":"回复","enabled":true}]}))).unwrap().notice
    }
    #[test]
    fn publication_is_durable_owned_revisioned_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.sqlite");
        let store = NoticeStore::open(&path).unwrap();
        let mut events = store.subscribe();
        let notice = create(&store);
        assert_eq!(create(&store).notice_id, notice.notice_id);
        assert!(events.try_recv().is_ok());
        assert!(events.try_recv().is_err());
        let update = decode::<p::NoticeUpdateRequest>(
            json!({"noticeId":notice.notice_id,"operationId":"u1","expectedRevision":1,"content":{"title":"新标题"}}),
        );
        let changed = store.update("observer", update.clone()).unwrap().notice;
        assert_eq!(changed.revision, 2);
        assert_eq!(
            store
                .update("observer", update.clone())
                .unwrap()
                .notice
                .revision,
            2
        );
        assert_eq!(
            store.update("other", update.clone()).unwrap_err().code,
            "notice_not_found"
        );
        let mut stale = update;
        stale.operation_id = "u2".into();
        assert_eq!(
            store.update("observer", stale).unwrap_err().code,
            "notice_revision_conflict"
        );
        let finish = decode::<p::NoticeFinishRequest>(
            json!({"noticeId":notice.notice_id,"operationId":"f1","expectedRevision":2,"status":{"value":"cancelled-by-user","label":"已撤回"}}),
        );
        let finished = store.finish("observer", finish.clone()).unwrap().notice;
        assert!(finished.finished);
        assert_eq!(finished.status.value, "cancelled-by-user");
        assert_eq!(finished.actions.len(), 1);
        assert_eq!(store.finish("observer", finish).unwrap().notice.revision, 3);
        let mut change = decode::<p::NoticeUpdateRequest>(
            json!({"noticeId":notice.notice_id,"operationId":"u3","expectedRevision":3,"status":{"value":"x","label":"x"}}),
        );
        assert_eq!(
            store.update("observer", change.clone()).unwrap_err().code,
            "notice_finished"
        );
        change.operation_id = "f1".into();
        change.expected_revision = 2;
        assert_eq!(
            store.update("observer", change).unwrap_err().code,
            "notice_operation_conflict"
        );
        let corrected=store.finish("observer",decode(json!({"noticeId":notice.notice_id,"operationId":"f2","expectedRevision":3,"status":{"value":"anything","label":"最终确认"},"actions":[]}))).unwrap().notice;
        assert!(corrected.finished);
        assert_eq!(corrected.revision, 4);
        drop(store);
        let reopened = NoticeStore::open(&path).unwrap();
        assert_eq!(reopened.get(&notice.notice_id).unwrap().revision, 4);
        assert!(create(&reopened).finished);
    }
    #[test]
    fn action_claim_survives_restart_and_never_replays_an_unknown_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.sqlite");
        let store = NoticeStore::open(&path).unwrap();
        let notice = create(&store);
        let request = decode::<p::NoticeActionExecuteRequest>(
            json!({"noticeId":notice.notice_id,"actionId":"reply","expectedRevision":1,"executionId":"exec-1","input":{"kind":"reply","text":"你好"}}),
        );
        assert!(matches!(
            store.claim_action(&request).unwrap(),
            ActionClaim::Execute { .. }
        ));
        drop(store);
        let store = NoticeStore::open(&path).unwrap();
        assert!(
            matches!(store.claim_action(&request).unwrap(),ActionClaim::Existing(r) if r.status==NoticeActionResultStatus::Unknown)
        );
        store
            .complete_action(&NoticeActionResult {
                execution_id: "exec-1".into(),
                status: NoticeActionResultStatus::Succeeded,
                message: None,
            })
            .unwrap();
        assert!(
            matches!(store.claim_action(&request).unwrap(),ActionClaim::Existing(r) if r.status==NoticeActionResultStatus::Succeeded)
        );
        let mut changed = request.clone();
        changed.expected_revision = 2;
        assert_eq!(
            store.claim_action(&changed).err().unwrap().code,
            "notice_action_conflict"
        );
        changed.execution_id = "exec-2".into();
        assert_eq!(
            store.claim_action(&changed).err().unwrap().code,
            "notice_revision_conflict"
        );
        changed.expected_revision = 1;
        changed.input = decode(json!({"kind":"reply","text":"  "}));
        assert_eq!(
            store.claim_action(&changed).err().unwrap().code,
            "invalid_params"
        );
    }
    #[test]
    fn canonical_schema_rejects_malformed_known_actions() {
        let dir = tempfile::tempdir().unwrap();
        let store = NoticeStore::open(&dir.path().join("app.sqlite")).unwrap();
        for action in [
            json!({"id":"activate","kind":"activate","label":"打开","enabled":true}),
            json!({"id":"p","kind":"permission","label":"授权","enabled":true,"options":[{"id":"yes","label":"允许"}]}),
            json!({"id":"r","kind":"reply","label":"","enabled":true}),
        ] {
            let r = decode(
                json!({"creationKey":"bad","sourceId":"chat","status":{"value":"x","label":"x"},"content":{"title":"x"},"actions":[action]}),
            );
            assert_eq!(
                store.create("observer", r).unwrap_err().code,
                "invalid_params"
            );
        }
    }
}
