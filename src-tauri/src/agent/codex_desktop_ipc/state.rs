use super::protocol::DesktopIpcError;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ThreadSnapshot {
    pub conversation_id: String,
    pub owner_client_id: String,
    pub revision: u64,
    pub state: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateChangeOutcome {
    Applied,
    Gap {
        conversation_id: String,
        owner_client_id: String,
        expected_base_revision: Option<u64>,
        received_base_revision: Option<u64>,
    },
    AwaitingSnapshot,
    Ignored,
}

#[derive(Default)]
pub struct FollowerStore {
    snapshots: HashMap<String, ThreadSnapshot>,
    expected_owners: HashMap<String, String>,
    awaiting_snapshot: HashSet<String>,
    bootstrapped: HashSet<String>,
    restore_after_snapshot: HashSet<String>,
    known_threads: HashSet<String>,
    invalidated_threads: HashSet<String>,
    epoch: u64,
}

impl FollowerStore {
    pub fn remember_thread(&mut self, conversation_id: &str) -> bool {
        self.known_threads.insert(conversation_id.to_string())
    }

    pub fn begin_bootstrap(&mut self, conversation_id: &str) {
        self.invalidated_threads.remove(conversation_id);
        self.awaiting_snapshot.insert(conversation_id.to_string());
        self.bootstrapped.remove(conversation_id);
        self.restore_after_snapshot.remove(conversation_id);
    }

    pub fn bind_owner(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
    ) -> Result<(), DesktopIpcError> {
        if owner_client_id.is_empty() {
            return Err(DesktopIpcError::Protocol(
                "thread owner identity cannot be empty".to_string(),
            ));
        }
        self.remember_thread(conversation_id);
        self.expected_owners
            .insert(conversation_id.to_string(), owner_client_id.to_string());
        Ok(())
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn was_invalidated(&self, conversation_id: &str) -> bool {
        self.invalidated_threads.contains(conversation_id)
    }

    pub fn known_threads(&self) -> Vec<String> {
        let mut threads = self.known_threads.iter().cloned().collect::<Vec<_>>();
        threads.sort();
        threads
    }

    pub fn snapshot(&self, conversation_id: &str) -> Option<ThreadSnapshot> {
        self.snapshots.get(conversation_id).cloned()
    }

    pub fn is_bootstrapped(&self, conversation_id: &str) -> bool {
        self.bootstrapped.contains(conversation_id)
    }

    pub fn expected_owner(&self, conversation_id: &str) -> Option<String> {
        self.expected_owners.get(conversation_id).cloned()
    }

    pub fn discard_unremembered_thread(&mut self, conversation_id: &str) {
        if self.known_threads.contains(conversation_id) {
            return;
        }
        self.snapshots.remove(conversation_id);
        self.expected_owners.remove(conversation_id);
        self.awaiting_snapshot.remove(conversation_id);
        self.bootstrapped.remove(conversation_id);
        self.restore_after_snapshot.remove(conversation_id);
        self.invalidated_threads.remove(conversation_id);
    }

    pub fn threads_needing_bootstrap(&self) -> Vec<String> {
        let mut threads = self
            .known_threads
            .iter()
            .filter(|conversation_id| !self.bootstrapped.contains(*conversation_id))
            .cloned()
            .collect::<Vec<_>>();
        threads.sort();
        threads
    }

    pub fn forget_owner(&mut self, owner_client_id: &str) -> Vec<String> {
        let mut affected = self
            .expected_owners
            .iter()
            .filter(|(_, expected_owner)| expected_owner.as_str() == owner_client_id)
            .map(|(conversation_id, _)| conversation_id.clone())
            .collect::<HashSet<_>>();
        affected.extend(
            self.snapshots
                .iter()
                .filter(|(_, snapshot)| snapshot.owner_client_id == owner_client_id)
                .map(|(conversation_id, _)| conversation_id.clone()),
        );
        let mut affected = affected.into_iter().collect::<Vec<_>>();
        affected.sort();
        for conversation_id in &affected {
            self.snapshots.remove(conversation_id);
            self.expected_owners.remove(conversation_id);
            self.awaiting_snapshot.remove(conversation_id);
            self.bootstrapped.remove(conversation_id);
            self.restore_after_snapshot.remove(conversation_id);
            self.invalidated_threads.insert(conversation_id.clone());
        }
        affected
    }

    pub fn complete_bootstrap(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        minimum_revision: u64,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        if self.awaiting_snapshot.contains(conversation_id)
            || self.invalidated_threads.contains(conversation_id)
        {
            return Err(DesktopIpcError::Disconnected(format!(
                "thread {conversation_id} cannot complete bootstrap without an authoritative snapshot"
            )));
        }
        let snapshot = self.snapshot(conversation_id).ok_or_else(|| {
            DesktopIpcError::Disconnected(format!(
                "thread {conversation_id} bootstrap has no local snapshot"
            ))
        })?;
        if self.expected_owners.get(conversation_id).map(String::as_str)
            != Some(owner_client_id)
            || snapshot.owner_client_id != owner_client_id
            || snapshot.revision < minimum_revision
        {
            return Err(DesktopIpcError::Disconnected(format!(
                "thread {conversation_id} bootstrap snapshot changed before commit"
            )));
        }
        self.bootstrapped.insert(conversation_id.to_string());
        Ok(snapshot)
    }

    pub fn reset_for_reconnect(&mut self) {
        self.snapshots.clear();
        self.expected_owners.clear();
        self.awaiting_snapshot.clear();
        self.bootstrapped.clear();
        self.restore_after_snapshot.clear();
        self.invalidated_threads = self.known_threads.clone();
        self.epoch = self.epoch.saturating_add(1);
    }

    pub fn apply_stream_change(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        change: &Value,
    ) -> Result<StateChangeOutcome, DesktopIpcError> {
        match self.expected_owners.get(conversation_id).map(String::as_str) {
            Some(expected_owner) if expected_owner == owner_client_id => {}
            Some(expected_owner) => {
                return Err(DesktopIpcError::Protocol(format!(
                    "thread state owner mismatch: expected {expected_owner}, received {owner_client_id}"
                )))
            }
            None => {
                return Err(DesktopIpcError::Protocol(format!(
                    "thread {conversation_id} received state before owner discovery"
                )))
            }
        }
        match change.get("type").and_then(Value::as_str) {
            Some("snapshot") => self.apply_snapshot(conversation_id, owner_client_id, change),
            Some("patches") => self.apply_patches(conversation_id, owner_client_id, change),
            Some(other) => Err(DesktopIpcError::Protocol(format!(
                "unknown thread stream change type {other}"
            ))),
            None => Err(DesktopIpcError::Protocol(
                "thread stream change is missing type".to_string(),
            )),
        }
    }

    fn apply_snapshot(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        change: &Value,
    ) -> Result<StateChangeOutcome, DesktopIpcError> {
        let revision = required_u64(change, "revision")?;
        let state = change
            .get("conversationState")
            .cloned()
            .ok_or_else(|| DesktopIpcError::Protocol("snapshot is missing conversationState".to_string()))?;
        validate_conversation_state(conversation_id, &state)?;
        if !self.awaiting_snapshot.contains(conversation_id) {
            if let Some(current) = self.snapshots.get(conversation_id) {
                if current.owner_client_id == owner_client_id && revision <= current.revision {
                    return Ok(StateChangeOutcome::Ignored);
                }
            }
        }
        self.snapshots.insert(
            conversation_id.to_string(),
            ThreadSnapshot {
                conversation_id: conversation_id.to_string(),
                owner_client_id: owner_client_id.to_string(),
                revision,
                state,
            },
        );
        self.awaiting_snapshot.remove(conversation_id);
        if self.restore_after_snapshot.remove(conversation_id) {
            self.bootstrapped.insert(conversation_id.to_string());
        }
        Ok(StateChangeOutcome::Applied)
    }

    fn apply_patches(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        change: &Value,
    ) -> Result<StateChangeOutcome, DesktopIpcError> {
        let base_revision = required_u64(change, "baseRevision")?;
        let revision = required_u64(change, "revision")?;
        if self.awaiting_snapshot.contains(conversation_id)
            || self.invalidated_threads.contains(conversation_id)
        {
            return Ok(StateChangeOutcome::AwaitingSnapshot);
        }
        let Some(current) = self.snapshots.get(conversation_id) else {
            self.mark_gap(conversation_id);
            return Ok(StateChangeOutcome::Gap {
                conversation_id: conversation_id.to_string(),
                owner_client_id: owner_client_id.to_string(),
                expected_base_revision: None,
                received_base_revision: Some(base_revision),
            });
        };
        let current_owner_client_id = current.owner_client_id.clone();
        let current_revision = current.revision;
        let mut next_state = current.state.clone();
        if current_owner_client_id == owner_client_id && revision <= current_revision {
            return Ok(StateChangeOutcome::Ignored);
        }
        if current_owner_client_id != owner_client_id
            || current_revision != base_revision
            || revision != base_revision.checked_add(1).unwrap_or(u64::MAX)
        {
            let expected = Some(current_revision);
            self.mark_gap(conversation_id);
            return Ok(StateChangeOutcome::Gap {
                conversation_id: conversation_id.to_string(),
                owner_client_id: owner_client_id.to_string(),
                expected_base_revision: expected,
                received_base_revision: Some(base_revision),
            });
        }
        let patches = change
            .get("patches")
            .and_then(Value::as_array)
            .ok_or_else(|| DesktopIpcError::Protocol("patch change is missing patches".to_string()))?;
        for patch in patches {
            apply_patch(&mut next_state, patch)?;
        }
        validate_conversation_state(conversation_id, &next_state)?;
        self.snapshots.insert(
            conversation_id.to_string(),
            ThreadSnapshot {
                conversation_id: conversation_id.to_string(),
                owner_client_id: owner_client_id.to_string(),
                revision,
                state: next_state,
            },
        );
        Ok(StateChangeOutcome::Applied)
    }

    fn mark_gap(&mut self, conversation_id: &str) {
        self.awaiting_snapshot.insert(conversation_id.to_string());
        if self.bootstrapped.remove(conversation_id) {
            self.restore_after_snapshot.insert(conversation_id.to_string());
        }
    }
}

fn validate_conversation_state(
    conversation_id: &str,
    state: &Value,
) -> Result<(), DesktopIpcError> {
    if !state.is_object() {
        return Err(DesktopIpcError::Protocol(
            "snapshot conversationState is not an object".to_string(),
        ));
    }
    let state_id = state.get("id").and_then(Value::as_str).ok_or_else(|| {
        DesktopIpcError::Protocol(
            "snapshot conversationState is missing string id".to_string(),
        )
    })?;
    if state_id != conversation_id {
        return Err(DesktopIpcError::Protocol(format!(
            "snapshot conversation id mismatch: envelope={conversation_id} state={state_id}"
        )));
    }
    Ok(())
}

fn apply_patch(target: &mut Value, patch: &Value) -> Result<(), DesktopIpcError> {
    let operation = patch
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| DesktopIpcError::Protocol("state patch is missing op".to_string()))?;
    let path = patch
        .get("path")
        .and_then(Value::as_array)
        .ok_or_else(|| DesktopIpcError::Protocol("state patch is missing path".to_string()))?;
    apply_at_path(target, path, operation, patch.get("value").cloned())
}

fn apply_at_path(
    target: &mut Value,
    path: &[Value],
    operation: &str,
    value: Option<Value>,
) -> Result<(), DesktopIpcError> {
    if path.is_empty() {
        return match operation {
            "add" | "replace" => {
                *target = value.ok_or_else(|| {
                    DesktopIpcError::Protocol(format!("{operation} patch is missing value"))
                })?;
                Ok(())
            }
            "remove" => {
                *target = Value::Null;
                Ok(())
            }
            other => Err(DesktopIpcError::Protocol(format!(
                "unsupported state patch operation {other}"
            ))),
        };
    }
    if path.len() > 1 {
        let child = child_mut(target, &path[0])?;
        return apply_at_path(child, &path[1..], operation, value);
    }
    match target {
        Value::Object(object) => {
            let key = path[0].as_str().ok_or_else(|| {
                DesktopIpcError::Protocol("object patch path is not a string".to_string())
            })?;
            match operation {
                "add" => {
                    object.insert(
                        key.to_string(),
                        value.ok_or_else(|| {
                            DesktopIpcError::Protocol("add patch is missing value".to_string())
                        })?,
                    );
                }
                "replace" => {
                    let replacement = value.ok_or_else(|| {
                        DesktopIpcError::Protocol("replace patch is missing value".to_string())
                    })?;
                    let existing = object.get_mut(key).ok_or_else(|| {
                        DesktopIpcError::Protocol(format!(
                            "replace patch path does not exist: {key}"
                        ))
                    })?;
                    *existing = replacement;
                }
                "remove" => {
                    object.remove(key).ok_or_else(|| {
                        DesktopIpcError::Protocol(format!("remove patch path does not exist: {key}"))
                    })?;
                }
                other => {
                    return Err(DesktopIpcError::Protocol(format!(
                        "unsupported state patch operation {other}"
                    )))
                }
            }
            Ok(())
        }
        Value::Array(array) => {
            let index = value_index(&path[0])?;
            match operation {
                "add" if index <= array.len() => array.insert(
                    index,
                    value.ok_or_else(|| {
                        DesktopIpcError::Protocol("add patch is missing value".to_string())
                    })?,
                ),
                "replace" if index < array.len() => {
                    array[index] = value.ok_or_else(|| {
                        DesktopIpcError::Protocol("replace patch is missing value".to_string())
                    })?;
                }
                "remove" if index < array.len() => {
                    array.remove(index);
                }
                "add" | "replace" | "remove" => {
                    return Err(DesktopIpcError::Protocol(format!(
                        "array patch index {index} is out of bounds"
                    )))
                }
                other => {
                    return Err(DesktopIpcError::Protocol(format!(
                        "unsupported state patch operation {other}"
                    )))
                }
            }
            Ok(())
        }
        _ => Err(DesktopIpcError::Protocol(
            "state patch parent is not an object or array".to_string(),
        )),
    }
}

fn child_mut<'a>(target: &'a mut Value, component: &Value) -> Result<&'a mut Value, DesktopIpcError> {
    match target {
        Value::Object(object) => {
            let key = component.as_str().ok_or_else(|| {
                DesktopIpcError::Protocol("object patch path is not a string".to_string())
            })?;
            object.get_mut(key).ok_or_else(|| {
                DesktopIpcError::Protocol(format!("state patch path does not exist: {key}"))
            })
        }
        Value::Array(array) => {
            let index = value_index(component)?;
            array.get_mut(index).ok_or_else(|| {
                DesktopIpcError::Protocol(format!("state patch index {index} is out of bounds"))
            })
        }
        _ => Err(DesktopIpcError::Protocol(
            "state patch path crosses a scalar value".to_string(),
        )),
    }
}

fn value_index(value: &Value) -> Result<usize, DesktopIpcError> {
    value
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| DesktopIpcError::Protocol("array patch path is not an index".to_string()))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, DesktopIpcError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| DesktopIpcError::Protocol(format!("stream change is missing {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retry_bootstrap_keeps_the_owner_while_waiting_for_a_new_snapshot() {
        let mut store = FollowerStore::default();
        store.remember_thread("thread-one");
        store.bind_owner("thread-one", "owner-one").unwrap();
        store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "snapshot",
                    "revision": 4,
                    "conversationState": { "id": "thread-one", "status": "running" }
                }),
            )
            .unwrap();
        store
            .complete_bootstrap("thread-one", "owner-one", 4)
            .unwrap();

        store.begin_bootstrap("thread-one");

        assert_eq!(
            store.expected_owner("thread-one").as_deref(),
            Some("owner-one")
        );
        assert_eq!(
            store
                .apply_stream_change(
                    "thread-one",
                    "owner-one",
                    &json!({
                        "type": "patches",
                        "baseRevision": 4,
                        "revision": 5,
                        "patches": []
                    }),
                )
                .unwrap(),
            StateChangeOutcome::AwaitingSnapshot
        );
        assert_eq!(store.snapshot("thread-one").unwrap().revision, 4);
    }

    #[test]
    fn revision_gap_waits_for_snapshot_without_applying_later_patches() {
        let mut store = FollowerStore::default();
        store.remember_thread("thread-one");
        store.bind_owner("thread-one", "owner-one").unwrap();
        assert_eq!(
            store.expected_owner("thread-one").as_deref(),
            Some("owner-one")
        );
        store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "snapshot",
                    "revision": 4,
                    "conversationState": { "id": "thread-one", "status": "running" }
                }),
            )
            .unwrap();
        store
            .complete_bootstrap("thread-one", "owner-one", 4)
            .unwrap();
        let gap = store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "patches",
                    "baseRevision": 5,
                    "revision": 6,
                    "patches": [{ "op": "replace", "path": ["status"], "value": "done" }]
                }),
            )
            .unwrap();
        assert!(matches!(
            gap,
            StateChangeOutcome::Gap {
                expected_base_revision: Some(4),
                received_base_revision: Some(5),
                ..
            }
        ));
        assert!(!store.is_bootstrapped("thread-one"));
        assert_eq!(
            store.snapshot("thread-one").unwrap().state["status"],
            "running"
        );
        let ignored = store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "patches",
                    "baseRevision": 6,
                    "revision": 7,
                    "patches": [{ "op": "replace", "path": ["status"], "value": "failed" }]
                }),
            )
            .unwrap();
        assert_eq!(ignored, StateChangeOutcome::AwaitingSnapshot);
        assert_eq!(
            store
                .apply_stream_change(
                    "thread-one",
                    "owner-one",
                    &json!({
                        "type": "snapshot",
                        "revision": 7,
                        "conversationState": { "id": "thread-one", "status": "authoritative" }
                    }),
                )
                .unwrap(),
            StateChangeOutcome::Applied
        );
        assert!(store.is_bootstrapped("thread-one"));
        assert_eq!(
            store.snapshot("thread-one").unwrap().state["status"],
            "authoritative"
        );
    }

    #[test]
    fn ordered_patches_apply_and_reconnect_resets_state_but_preserves_known_ids() {
        let mut store = FollowerStore::default();
        store.remember_thread("thread-one");
        store.bind_owner("thread-one", "owner-one").unwrap();
        store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "snapshot",
                    "revision": 1,
                    "conversationState": {
                        "id": "thread-one",
                        "items": [{ "status": "running" }]
                    }
                }),
            )
            .unwrap();
        assert_eq!(
            store
                .apply_stream_change(
                    "thread-one",
                    "owner-one",
                    &json!({
                        "type": "patches",
                        "baseRevision": 1,
                        "revision": 2,
                        "patches": [
                            { "op": "replace", "path": ["items", 0, "status"], "value": "completed" },
                            { "op": "add", "path": ["items", 1], "value": { "status": "new" } }
                        ]
                    }),
                )
                .unwrap(),
            StateChangeOutcome::Applied
        );
        let snapshot = store.snapshot("thread-one").unwrap();
        assert_eq!(snapshot.revision, 2);
        assert_eq!(snapshot.state["items"][0]["status"], "completed");
        assert_eq!(snapshot.state["items"][1]["status"], "new");
        assert_eq!(
            store
                .apply_stream_change(
                    "thread-one",
                    "owner-one",
                    &json!({
                        "type": "patches",
                        "baseRevision": 1,
                        "revision": 2,
                        "patches": [
                            { "op": "replace", "path": ["items", 0, "status"], "value": "stale" }
                        ]
                    }),
                )
                .unwrap(),
            StateChangeOutcome::Ignored
        );
        assert_eq!(
            store.snapshot("thread-one").unwrap().state["items"][0]["status"],
            "completed"
        );

        store.remember_thread("thread-two");
        store.reset_for_reconnect();
        assert!(store.snapshot("thread-one").is_none());
        assert_eq!(
            store.known_threads(),
            vec!["thread-one".to_string(), "thread-two".to_string()]
        );
        assert_eq!(store.threads_needing_bootstrap(), store.known_threads());
        assert_eq!(store.epoch(), 1);
    }

    #[test]
    fn state_from_a_non_discovered_owner_is_rejected() {
        let mut store = FollowerStore::default();
        store.remember_thread("thread-one");
        store.bind_owner("thread-one", "owner-one").unwrap();

        let error = store
            .apply_stream_change(
                "thread-one",
                "bystander",
                &json!({
                    "type": "snapshot",
                    "revision": 1,
                    "conversationState": { "id": "thread-one" }
                }),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            DesktopIpcError::Protocol(message) if message.contains("owner mismatch")
        ));
        assert!(store.snapshot("thread-one").is_none());

        let mut unbound = FollowerStore::default();
        let _ = unbound.apply_stream_change(
            "bogus-thread",
            "bystander",
            &json!({
                "type": "snapshot",
                "revision": 1,
                "conversationState": { "id": "bogus-thread" }
            }),
        );
        assert!(unbound.known_threads().is_empty());
    }

    #[test]
    fn patches_cannot_replace_the_conversation_root_or_identity() {
        let mut store = FollowerStore::default();
        store.bind_owner("thread-one", "owner-one").unwrap();
        store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "snapshot",
                    "revision": 1,
                    "conversationState": { "id": "thread-one", "status": "running" }
                }),
            )
            .unwrap();

        let wrong_id = store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "patches",
                    "baseRevision": 1,
                    "revision": 2,
                    "patches": [{
                        "op": "replace",
                        "path": ["id"],
                        "value": "other-thread"
                    }]
                }),
            )
            .unwrap_err();
        assert!(matches!(
            wrong_id,
            DesktopIpcError::Protocol(message) if message.contains("id mismatch")
        ));

        let scalar_root = store
            .apply_stream_change(
                "thread-one",
                "owner-one",
                &json!({
                    "type": "patches",
                    "baseRevision": 1,
                    "revision": 2,
                    "patches": [{ "op": "replace", "path": [], "value": 7 }]
                }),
            )
            .unwrap_err();
        assert!(matches!(
            scalar_root,
            DesktopIpcError::Protocol(message) if message.contains("not an object")
        ));
        assert_eq!(store.snapshot("thread-one").unwrap().revision, 1);
    }
}
