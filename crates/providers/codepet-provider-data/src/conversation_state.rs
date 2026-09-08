//! Shared v1 activity/read authority. Host and local providers must use the same
//! path and this transaction implementation; never retain a separate writer.
use crate as gateway;
use crate as provider;
use crate::ProtocolError;
use fs2::FileExt;

use std::ops::{Deref, DerefMut};
type StateResult<T> = Result<T, ProtocolError>;
pub const CONVERSATION_STATE_PATH_ENV: &str = "CODEPET_CONVERSATION_STATE_DATABASE";
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const DOCUMENT_VERSION: u32 = 1;
const ACTIVITY_VERSION_PREFIX: &str = "activity-";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationActivityRecord {
    resource: gateway::RoutedResourceId,
    latest_version: u64,
    summary_fingerprint: Option<String>,
    detail_fingerprint: Option<String>,
    event_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientReadRecord {
    baseline_version: u64,
    reads: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationStateDocument {
    version: u32,
    latest_version: u64,
    conversations: BTreeMap<String, ConversationActivityRecord>,
    clients: BTreeMap<String, ClientReadRecord>,
}

impl Default for ConversationStateDocument {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            latest_version: 0,
            conversations: BTreeMap::new(),
            clients: BTreeMap::new(),
        }
    }
}

pub struct SharedConversationStateStore {
    path: Option<PathBuf>,
    document: Mutex<ConversationStateDocument>,
}

impl SharedConversationStateStore {
    pub fn memory() -> Self {
        Self {
            path: None,
            document: Mutex::new(ConversationStateDocument::default()),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> StateResult<Self> {
        let path = path.as_ref().to_path_buf();
        let store = Self {
            path: Some(path),
            document: Mutex::new(ConversationStateDocument::default()),
        };
        // Acquire the stable sidecar lock before validating or backing up v1.
        {
            let _guard = store.lock()?;
        }
        Ok(store)
    }

    /// No default home-directory store: it would silently discard Host scopes.
    /// Standalone providers retain ordinary methods and report unread unsupported.
    pub fn from_env() -> StateResult<Self> {
        let path = std::env::var_os(CONVERSATION_STATE_PATH_ENV)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| state_error("unsupported", "shared reader state is not configured"))?;
        Self::open(PathBuf::from(path))
    }

    /// Enumerates all known unread identities for exactly one instance and scope.
    /// Call ensure_client before observing a first snapshot, as the old Host did.
    pub fn unread(
        &self,
        caller_scope: &str,
        provider_id: &str,
    ) -> StateResult<Vec<(gateway::RoutedResourceId, gateway::ConversationReadState)>> {
        self.ensure_client(caller_scope)?;
        let document = self.lock()?;
        let client = document.clients.get(caller_scope).expect("client ensured");
        Ok(document
            .conversations
            .iter()
            .filter_map(|(key, record)| {
                let read = client
                    .reads
                    .get(key)
                    .copied()
                    .unwrap_or(client.baseline_version);
                (record.resource.provider_id == provider_id && record.latest_version > read).then(
                    || {
                        (
                            record.resource.clone(),
                            gateway::ConversationReadState {
                                unread: true,
                                activity_version: activity_version(record.latest_version),
                            },
                        )
                    },
                )
            })
            .collect())
    }

    pub fn ensure_client(&self, caller_scope: &str) -> StateResult<()> {
        let mut document = self.lock()?;
        if document.clients.contains_key(caller_scope) {
            return Ok(());
        }
        let baseline_version = document.latest_version;
        document.clients.insert(
            caller_scope.to_string(),
            ClientReadRecord {
                baseline_version,
                reads: BTreeMap::new(),
            },
        );
        self.persist(&document)
    }

    pub fn decorate(
        &self,
        caller_scope: &str,
        conversation: &mut gateway::Conversation,
    ) -> StateResult<()> {
        self.ensure_client(caller_scope)?;
        let document = self.lock()?;
        let key = resource_key(&conversation.resource);
        let latest = document
            .conversations
            .get(&key)
            .map(|record| record.latest_version)
            .unwrap_or(document.latest_version);
        let client = document.clients.get(caller_scope).ok_or_else(|| {
            state_error(
                "conversation_state_unavailable",
                "client read state is unavailable",
            )
        })?;
        let read = client
            .reads
            .get(&key)
            .copied()
            .unwrap_or(client.baseline_version);
        conversation.read_state = Some(gateway::ConversationReadState {
            unread: latest > read,
            activity_version: activity_version(latest),
        });
        Ok(())
    }

    pub fn mark_read(
        &self,
        caller_scope: &str,
        conversation: &gateway::RoutedResourceId,
        observed_activity_version: &str,
    ) -> StateResult<gateway::ConversationReadState> {
        let observed = parse_activity_version(observed_activity_version)?;
        let mut document = self.lock()?;
        let key = resource_key(conversation);
        let latest = document
            .conversations
            .get(&key)
            .map(|record| record.latest_version)
            .unwrap_or(document.latest_version);
        if observed > latest {
            return Err(state_error(
                "invalid_activity_version",
                "observed activity version is ahead of the state authority",
            ));
        }
        let baseline_version = document.latest_version;
        let client = document
            .clients
            .entry(caller_scope.to_string())
            .or_insert_with(|| ClientReadRecord {
                baseline_version,
                reads: BTreeMap::new(),
            });
        let read = client.reads.entry(key).or_insert(client.baseline_version);
        *read = (*read).max(observed);
        let unread = latest > *read;
        self.persist(&document)?;
        Ok(gateway::ConversationReadState {
            unread,
            activity_version: activity_version(latest),
        })
    }

    pub fn observe_summary(
        &self,
        conversation: &gateway::Conversation,
    ) -> StateResult<Option<u64>> {
        let fingerprint = summary_fingerprint(conversation);
        self.observe_fingerprint(
            &conversation.resource,
            FingerprintKind::Summary,
            fingerprint,
        )
    }

    /// Whether summary observation changed any stored fact (including a new
    /// baseline record), independent of whether its activity version advanced.
    pub fn observe_summary_changed(
        &self,
        conversation: &gateway::Conversation,
    ) -> StateResult<bool> {
        let Some(value) = summary_fingerprint(conversation) else {
            return Ok(false);
        };
        let mut document = self.lock()?;
        let (_, dirty) = observe_in_document(
            &mut document,
            &conversation.resource,
            FingerprintKind::Summary,
            value,
        )?;
        if dirty {
            self.persist(&document)?;
        }
        Ok(dirty)
    }

    pub fn observe_detail(
        &self,
        conversation: &gateway::RoutedResourceId,
        items: &[gateway::ConversationItem],
    ) -> StateResult<Option<u64>> {
        let fingerprint = detail_fingerprint(items);
        self.observe_fingerprint(conversation, FingerprintKind::Detail, fingerprint)
    }

    pub fn observe_provider_event(
        &self,
        event: &provider::ProtocolEvent,
    ) -> StateResult<Option<(gateway::RoutedResourceId, u64)>> {
        let observed = match event {
            provider::ProtocolEvent::EventConversationItemUpserted { params, .. }
                if params.item.is_none() && params.conversation.is_some() && params.update_id.is_some() => Some((
                    params.conversation.clone().expect("guarded conversation"),
                    format!("content-invalidation:{}", params.update_id.as_deref().expect("guarded update id")),
                )),
            provider::ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.kind == provider::ConversationContentKind::Text
                    && !params.delta.is_empty() =>
            {
                Some((
                    gateway_resource(params.conversation.clone()),
                    format!(
                        "output:{}:{}",
                        params.turn.native_resource_id, params.content_id
                    ),
                ))
            }
            provider::ProtocolEvent::EventApprovalRequested { params, .. } => Some((
                params.approval.conversation.clone(),
                format!("approval:{}", params.approval.resource.native_resource_id),
            )),
            provider::ProtocolEvent::EventConversationItemUpserted { params, .. }
                if params.item.as_ref().is_some_and(|item| matches!(
                    item_status(item),
                    provider::ConversationItemStatus::Completed
                        | provider::ConversationItemStatus::Failed
                        | provider::ConversationItemStatus::Interrupted
                )) => Some((
                    item_conversation(params.item.as_ref().expect("guarded item")).clone(),
                    format!(
                        "item-terminal:{}:{:?}",
                        item_resource(params.item.as_ref().expect("guarded item")).native_resource_id,
                        item_status(params.item.as_ref().expect("guarded item"))
                    ),
                )),
            provider::ProtocolEvent::EventTurnUpserted { params, .. }
                if matches!(
                    params.turn.status,
                    provider::TurnStatus::Completed | provider::TurnStatus::Failed
                ) =>
            {
                Some((
                    params.turn.conversation.clone(),
                    format!(
                        "turn-terminal:{}:{:?}",
                        params.turn.resource.native_resource_id, params.turn.status
                    ),
                ))
            }
            provider::ProtocolEvent::EventConversationUpserted { params, .. }
                if params.conversation.status == provider::ConversationStatus::WaitingUserInput =>
            {
                Some((
                    params.conversation.resource.clone(),
                    format!(
                        "waiting-input:{}",
                        params
                            .conversation
                            .active_turn
                            .as_ref()
                            .map(|turn| turn.resource.native_resource_id.as_str())
                            .unwrap_or("conversation")
                    ),
                ))
            }
            _ => None,
        };
        let Some((resource, fingerprint)) = observed else {
            return Ok(None);
        };
        let Some(version) =
            self.observe_fingerprint(&resource, FingerprintKind::Event, Some(fingerprint))?
        else {
            return Ok(None);
        };
        Ok(Some((resource, version)))
    }

    fn observe_fingerprint(
        &self,
        resource: &gateway::RoutedResourceId,
        kind: FingerprintKind,
        fingerprint: Option<String>,
    ) -> StateResult<Option<u64>> {
        let Some(fingerprint) = fingerprint else {
            return Ok(None);
        };
        let mut document = self.lock()?;
        let (version, dirty) = observe_in_document(&mut document, resource, kind, fingerprint)?;
        if dirty {
            self.persist(&document)?;
        }
        Ok(version)
    }

    /// One locked read and at most one write for an entire summary batch.
    /// Establish the reader baseline before observations, matching the old Host.
    pub fn observe_and_decorate_summaries(
        &self,
        scope: &str,
        rows: &mut [gateway::Conversation],
    ) -> StateResult<bool> {
        let mut document = self.lock()?;
        let mut dirty = ensure_scope(&mut document, scope);
        for row in rows.iter_mut() {
            if let Some(value) = summary_fingerprint(row) {
                dirty |= observe_in_document(
                    &mut document,
                    &row.resource,
                    FingerprintKind::Summary,
                    value,
                )?
                .1;
            }
            decorate_from_document(&document, scope, row);
        }
        if dirty {
            self.persist(&document)?;
        }
        Ok(dirty)
    }

    pub fn decorate_many(
        &self,
        scope: &str,
        rows: &mut [gateway::Conversation],
    ) -> StateResult<()> {
        let mut document = self.lock()?;
        let added = ensure_scope(&mut document, scope);
        for row in rows {
            decorate_from_document(&document, scope, row);
        }
        if added {
            self.persist(&document)?;
        }
        Ok(())
    }

    pub fn activity_versions(
        &self,
        resources: &[gateway::RoutedResourceId],
    ) -> StateResult<Vec<String>> {
        let document = self.lock()?;
        Ok(resources
            .iter()
            .map(|resource| {
                activity_version(
                    document
                        .conversations
                        .get(&resource_key(resource))
                        .map(|record| record.latest_version)
                        .unwrap_or(document.latest_version),
                )
            })
            .collect())
    }

    fn lock(&self) -> StateResult<StateGuard<'_>> {
        let mut document = self.document.lock().map_err(|_| {
            state_error(
                "conversation_state_unavailable",
                "conversation state lock is poisoned",
            )
        })?;
        let file = if let Some(path) = &self.path {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::create_dir_all(parent).map_err(|e| persistence_io("create parent", path, e))?;
            let mut lock_path = path.as_os_str().to_os_string();
            lock_path.push(".lock");
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(PathBuf::from(lock_path))
                .map_err(|e| persistence_io("open state lock", path, e))?;
            FileExt::lock_exclusive(&file).map_err(|e| persistence_io("lock state", path, e))?;
            *document = read_document(path)?;
            Some(file)
        } else {
            None
        };
        Ok(StateGuard {
            document,
            _file: file,
        })
    }

    fn persist(&self, document: &ConversationStateDocument) -> StateResult<()> {
        match self.path.as_deref() {
            Some(path) => write_state_document(path, document),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FingerprintKind {
    Summary,
    Detail,
    Event,
}

impl FingerprintKind {
    fn value(self, record: &ConversationActivityRecord) -> &Option<String> {
        match self {
            Self::Summary => &record.summary_fingerprint,
            Self::Detail => &record.detail_fingerprint,
            Self::Event => &record.event_fingerprint,
        }
    }

    fn slot(self, record: &mut ConversationActivityRecord) -> &mut Option<String> {
        match self {
            Self::Summary => &mut record.summary_fingerprint,
            Self::Detail => &mut record.detail_fingerprint,
            Self::Event => &mut record.event_fingerprint,
        }
    }
}

pub fn summary_fingerprint(conversation: &gateway::Conversation) -> Option<String> {
    let status = match conversation.status {
        gateway::ConversationStatus::Idle => "idle",
        gateway::ConversationStatus::Running => "running",
        gateway::ConversationStatus::WaitingApproval => "waiting-approval",
        gateway::ConversationStatus::WaitingUserInput => "waiting-user-input",
        gateway::ConversationStatus::Error => "error",
        gateway::ConversationStatus::Archived => "archived",
    };
    Some(fingerprint([
        "summary",
        conversation.preview.as_deref().unwrap_or("").trim(),
        status,
    ]))
}

pub fn detail_fingerprint(items: &[gateway::ConversationItem]) -> Option<String> {
    let item = items.iter().rev().find(|item| {
        item_role(item) == Some(gateway::ConversationItemRole::Assistant)
            || item_approval(item)
                .is_some_and(|approval| approval.status == gateway::ApprovalStatus::Pending)
    })?;
    Some(fingerprint([
        "detail",
        item_resource(item).native_resource_id.as_str(),
        item_turn(item).native_resource_id.as_str(),
    ]))
}

fn item_status(item: &gateway::ConversationItem) -> gateway::ConversationItemStatus {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => item.status,
        gateway::ConversationItem::ReasoningConversationItem(item) => item.status,
        gateway::ConversationItem::CommandConversationItem(item) => item.status,
        gateway::ConversationItem::FileChangeConversationItem(item) => item.status,
        gateway::ConversationItem::ToolConversationItem(item) => item.status,
        gateway::ConversationItem::ApprovalConversationItem(item) => item.status,
        gateway::ConversationItem::UnknownConversationItem(item) => item.status,
    }
}

fn item_resource(item: &gateway::ConversationItem) -> &gateway::RoutedResourceId {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => &item.resource,
        gateway::ConversationItem::ReasoningConversationItem(item) => &item.resource,
        gateway::ConversationItem::CommandConversationItem(item) => &item.resource,
        gateway::ConversationItem::FileChangeConversationItem(item) => &item.resource,
        gateway::ConversationItem::ToolConversationItem(item) => &item.resource,
        gateway::ConversationItem::ApprovalConversationItem(item) => &item.resource,
        gateway::ConversationItem::UnknownConversationItem(item) => &item.resource,
    }
}

pub fn item_conversation(item: &gateway::ConversationItem) -> &gateway::RoutedResourceId {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => &item.conversation,
        gateway::ConversationItem::ReasoningConversationItem(item) => &item.conversation,
        gateway::ConversationItem::CommandConversationItem(item) => &item.conversation,
        gateway::ConversationItem::FileChangeConversationItem(item) => &item.conversation,
        gateway::ConversationItem::ToolConversationItem(item) => &item.conversation,
        gateway::ConversationItem::ApprovalConversationItem(item) => &item.conversation,
        gateway::ConversationItem::UnknownConversationItem(item) => &item.conversation,
    }
}

fn item_turn(item: &gateway::ConversationItem) -> &gateway::RoutedResourceId {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => &item.turn,
        gateway::ConversationItem::ReasoningConversationItem(item) => &item.turn,
        gateway::ConversationItem::CommandConversationItem(item) => &item.turn,
        gateway::ConversationItem::FileChangeConversationItem(item) => &item.turn,
        gateway::ConversationItem::ToolConversationItem(item) => &item.turn,
        gateway::ConversationItem::ApprovalConversationItem(item) => &item.turn,
        gateway::ConversationItem::UnknownConversationItem(item) => &item.turn,
    }
}

fn item_role(item: &gateway::ConversationItem) -> Option<gateway::ConversationItemRole> {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => Some(item.role),
        _ => None,
    }
}

fn item_approval(item: &gateway::ConversationItem) -> Option<&gateway::Approval> {
    match item {
        gateway::ConversationItem::ApprovalConversationItem(item) => Some(&item.approval),
        _ => None,
    }
}

fn fingerprint<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut input = Vec::new();
    for part in parts {
        input.extend_from_slice(part.as_bytes());
        input.push(0);
    }
    let value = digest(&SHA256, &input);
    value
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn resource_key(resource: &gateway::RoutedResourceId) -> String {
    format!(
        "{}\u{1f}{}",
        resource.provider_id, resource.native_resource_id
    )
}

fn gateway_resource(resource: provider::ProviderResourceId) -> gateway::RoutedResourceId {
    gateway::RoutedResourceId {
        provider_id: resource.provider_instance_id,
        native_resource_id: resource.native_resource_id,
    }
}

fn activity_version(version: u64) -> String {
    format!("{ACTIVITY_VERSION_PREFIX}{version}")
}

fn parse_activity_version(value: &str) -> StateResult<u64> {
    value
        .strip_prefix(ACTIVITY_VERSION_PREFIX)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|version| activity_version(*version) == value)
        .ok_or_else(|| state_error("invalid_activity_version", "invalid activity version"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_invalidations_advance_unread_once_per_source_update() {
        let store = SharedConversationStateStore::memory();
        store.ensure_client("mobile").unwrap();
        let mut summary = conversation("existing output");
        store.observe_summary(&summary).unwrap();
        let event = |id: &str| provider::ProtocolEvent::EventConversationItemUpserted {
            jsonrpc: "2.0".into(),
            params: provider::ConversationItemUpsertedEvent {
                conversation: Some(summary.resource.clone()), item: None, update_id: Some(id.into()),
            },
        };
        assert!(store.observe_provider_event(&event("hook-1")).unwrap().is_some());
        assert!(store.observe_provider_event(&event("hook-1")).unwrap().is_none());
        assert!(store.observe_provider_event(&event("hook-2")).unwrap().is_some());
        store.decorate("mobile", &mut summary).unwrap();
        assert!(summary.read_state.unwrap().unread);
    }

    #[test]
    fn mark_read_only_advances_to_the_observed_activity() {
        let store = SharedConversationStateStore::memory();
        let mut conversation = conversation("first response");
        store.ensure_client("mobile").unwrap();
        assert_eq!(store.observe_summary(&conversation).unwrap(), None);

        conversation.preview = Some("second response".to_string());
        assert_eq!(store.observe_summary(&conversation).unwrap(), Some(1));
        store.decorate("mobile", &mut conversation).unwrap();
        assert_eq!(
            conversation.read_state,
            Some(gateway::ConversationReadState {
                unread: true,
                activity_version: "activity-1".to_string(),
            })
        );

        conversation.preview = Some("third response".to_string());
        assert_eq!(store.observe_summary(&conversation).unwrap(), Some(2));
        let read_state = store
            .mark_read("mobile", &conversation.resource, "activity-1")
            .unwrap();
        assert_eq!(
            read_state,
            gateway::ConversationReadState {
                unread: true,
                activity_version: "activity-2".to_string(),
            }
        );

        let read_state = store
            .mark_read("mobile", &conversation.resource, "activity-2")
            .unwrap();
        assert_eq!(
            read_state,
            gateway::ConversationReadState {
                unread: false,
                activity_version: "activity-2".to_string(),
            }
        );
    }

    #[test]
    fn first_snapshot_channel_is_baseline_but_first_event_is_activity() {
        let snapshots = SharedConversationStateStore::memory();
        let mut conversation = conversation("first response");
        snapshots.ensure_client("mobile").unwrap();
        snapshots.observe_summary(&conversation).unwrap();
        assert_eq!(
            snapshots
                .observe_fingerprint(
                    &conversation.resource,
                    FingerprintKind::Detail,
                    Some("first-detail".to_string()),
                )
                .unwrap(),
            None
        );
        snapshots.decorate("mobile", &mut conversation).unwrap();
        assert_eq!(conversation.read_state.as_ref().unwrap().unread, false);

        let events = SharedConversationStateStore::memory();
        events.ensure_client("mobile").unwrap();
        assert_eq!(
            events
                .observe_fingerprint(
                    &conversation.resource,
                    FingerprintKind::Event,
                    Some("first-event".to_string()),
                )
                .unwrap(),
            Some(1)
        );
        events.decorate("mobile", &mut conversation).unwrap();
        assert_eq!(conversation.read_state.as_ref().unwrap().unread, true);
    }

    #[test]
    fn shared_database_handles_preserve_scopes_versions_and_fingerprints() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("conversation-state.sqlite");
        let first = SharedConversationStateStore::open(&path).unwrap();
        first.ensure_client("mobile").unwrap();
        first.ensure_client("desktop").unwrap();
        let mut row = conversation("baseline");
        first.observe_summary(&row).unwrap();
        let second = SharedConversationStateStore::open(&path).unwrap();
        row.preview = Some("new response".into());
        assert_eq!(second.observe_summary(&row).unwrap(), Some(1));
        assert_eq!(first.observe_summary(&row).unwrap(), None);
        first
            .mark_read("mobile", &row.resource, "activity-1")
            .unwrap();
        assert!(second.unread("mobile", "codex-work").unwrap().is_empty());
        assert_eq!(second.unread("desktop", "codex-work").unwrap().len(), 1);
        drop(first);
        let reopened = SharedConversationStateStore::open(&path).unwrap();
        assert!(reopened.unread("mobile", "codex-work").unwrap().is_empty());
        assert_eq!(
            reopened.unread("desktop", "codex-work").unwrap()[0]
                .1
                .activity_version,
            "activity-1"
        );
        assert!(fs::read(&path).unwrap().starts_with(b"SQLite format 3"));
    }

    #[test]
    fn rejects_unknown_or_corrupt_storage_without_resetting_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("conversation-state.sqlite");
        for bytes in [
            b"{broken".as_slice(),
            br#"{"version":2,"latestVersion":0,"conversations":{},"clients":{}}"#.as_slice(),
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(SharedConversationStateStore::open(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn identical_fingerprints_and_batches_do_not_replace_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let store = SharedConversationStateStore::open(&path).unwrap();
        let mut rows = vec![conversation("baseline")];
        store
            .observe_and_decorate_summaries("mobile", &mut rows)
            .unwrap();
        let before: String = state_database(&path)
            .unwrap()
            .query_row(
                "SELECT document FROM conversation_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(store.observe_summary(&rows[0]).unwrap(), None);
        assert!(!store
            .observe_and_decorate_summaries("mobile", &mut rows)
            .unwrap());
        let after: String = state_database(&path)
            .unwrap()
            .query_row(
                "SELECT document FROM conversation_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn new_baseline_records_invalidate_views_without_inventing_activity() {
        let store = SharedConversationStateStore::memory();
        let row = conversation("baseline");
        assert!(store.observe_summary_changed(&row).unwrap());
        assert!(!store.observe_summary_changed(&row).unwrap());
        assert_eq!(
            store.activity_versions(&[row.resource.clone()]).unwrap(),
            ["activity-0"]
        );
        let mut rows = vec![row];
        assert!(store
            .observe_and_decorate_summaries("new-scope", &mut rows)
            .unwrap());
        assert!(!store
            .observe_and_decorate_summaries("new-scope", &mut rows)
            .unwrap());
    }

    #[test]
    fn new_summary_can_add_unread_at_current_clock_without_advancing_it() {
        let store = SharedConversationStateStore::memory();
        let seed = conversation("seed");
        for index in 0..2 {
            store
                .observe_fingerprint(
                    &seed.resource,
                    FingerprintKind::Event,
                    Some(format!("event-{index}")),
                )
                .unwrap();
        }
        store.ensure_client("old-scope").unwrap();
        for index in 2..5 {
            store
                .observe_fingerprint(
                    &seed.resource,
                    FingerprintKind::Event,
                    Some(format!("event-{index}")),
                )
                .unwrap();
        }
        let mut newly_seen = conversation("discovered after baseline");
        newly_seen.resource.native_resource_id = "newly-seen".into();
        assert!(store.observe_summary_changed(&newly_seen).unwrap());
        assert_eq!(
            store
                .activity_versions(&[newly_seen.resource.clone()])
                .unwrap(),
            ["activity-5"]
        );
        assert!(store.unread("old-scope", "codex-work").unwrap().iter().any(
            |(resource, state)| resource == &newly_seen.resource
                && state.unread
                && state.activity_version == "activity-5"
        ));
        assert!(!store.observe_summary_changed(&newly_seen).unwrap());
    }

    fn conversation(preview: &str) -> gateway::Conversation {
        gateway::Conversation {
            resource: gateway::RoutedResourceId {
                provider_id: "codex-work".to_string(),
                native_resource_id: "thread-1".to_string(),
            },
            project: None,
            title: "Conversation".to_string(),
            preview: Some(preview.to_string()),
            status: gateway::ConversationStatus::Idle,
            permission_level: None,
            model: None,
            reasoning_effort: None,
            selection: None,
            workspace_root: None,
            created_at: None,
            updated_at: None,
            active_turn: None,
            read_state: None,
        }
    }
}

// The lock file is never renamed or removed. Dropping its handle releases the OS
// lock, including after process failure; atomic data-file replacement is separate.
struct StateGuard<'a> {
    document: std::sync::MutexGuard<'a, ConversationStateDocument>,
    _file: Option<fs::File>,
}
impl Deref for StateGuard<'_> {
    type Target = ConversationStateDocument;
    fn deref(&self) -> &Self::Target {
        &self.document
    }
}
impl DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.document
    }
}
fn state_error(code: &str, message: impl Into<String>) -> ProtocolError {
    ProtocolError {
        code: code.into(),
        message: message.into(),
        retryable: false,
        details: None,
    }
}
fn persistence_io(action: &str, path: &Path, error: std::io::Error) -> ProtocolError {
    let mut error = state_error(
        "conversation_state_unavailable",
        format!("{action} {}: {error}", path.display()),
    );
    error.retryable = true;
    error
}
fn state_database(path: &Path) -> StateResult<rusqlite::Connection> {
    let db = crate::open(path)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS conversation_state (id INTEGER PRIMARY KEY CHECK(id=1), document TEXT NOT NULL)").map_err(crate::db_error)?;
    Ok(db)
}
fn read_document(path: &Path) -> StateResult<ConversationStateDocument> {
    use rusqlite::OptionalExtension;
    let db = state_database(path)?;
    let value: Option<String> = db
        .query_row(
            "SELECT document FROM conversation_state WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(crate::db_error)?;
    let document: ConversationStateDocument = match value {
        Some(value) => serde_json::from_str(&value).map_err(crate::db_error)?,
        None => ConversationStateDocument::default(),
    };
    if document.version != DOCUMENT_VERSION {
        return Err(state_error(
            "conversation_state_invalid",
            "Unsupported database state version",
        ));
    }
    Ok(document)
}
fn write_state_document(path: &Path, document: &ConversationStateDocument) -> StateResult<()> {
    let db = state_database(path)?;
    db.execute("INSERT INTO conversation_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET document=excluded.document",[serde_json::to_string(document).map_err(crate::db_error)?]).map_err(crate::db_error)?;
    Ok(())
}

fn ensure_scope(document: &mut ConversationStateDocument, scope: &str) -> bool {
    if document.clients.contains_key(scope) {
        return false;
    }
    document.clients.insert(
        scope.into(),
        ClientReadRecord {
            baseline_version: document.latest_version,
            reads: BTreeMap::new(),
        },
    );
    true
}
fn decorate_from_document(
    document: &ConversationStateDocument,
    scope: &str,
    row: &mut gateway::Conversation,
) {
    let key = resource_key(&row.resource);
    let latest = document
        .conversations
        .get(&key)
        .map(|record| record.latest_version)
        .unwrap_or(document.latest_version);
    let client = document.clients.get(scope).expect("scope ensured");
    let read = client
        .reads
        .get(&key)
        .copied()
        .unwrap_or(client.baseline_version);
    row.read_state = Some(gateway::ConversationReadState {
        unread: latest > read,
        activity_version: activity_version(latest),
    });
}
fn observe_in_document(
    document: &mut ConversationStateDocument,
    resource: &gateway::RoutedResourceId,
    kind: FingerprintKind,
    fingerprint: String,
) -> StateResult<(Option<u64>, bool)> {
    let key = resource_key(resource);
    if let Some(record) = document.conversations.get(&key) {
        if kind.value(record).as_ref() == Some(&fingerprint) {
            return Ok((None, false));
        }
        if kind != FingerprintKind::Event && kind.value(record).is_none() {
            let record = document
                .conversations
                .get_mut(&key)
                .expect("conversation record checked");
            *kind.slot(record) = Some(fingerprint);
            return Ok((None, true));
        }
        document.latest_version = document.latest_version.saturating_add(1);
        let version = document.latest_version;
        let record = document
            .conversations
            .get_mut(&key)
            .expect("conversation record checked");
        *kind.slot(record) = Some(fingerprint);
        record.latest_version = version;
        return Ok((Some(version), true));
    }
    let version = if kind == FingerprintKind::Event {
        document.latest_version = document.latest_version.saturating_add(1);
        document.latest_version
    } else {
        document.latest_version
    };
    let mut record = ConversationActivityRecord {
        resource: resource.clone(),
        latest_version: version,
        summary_fingerprint: None,
        detail_fingerprint: None,
        event_fingerprint: None,
    };
    *kind.slot(&mut record) = Some(fingerprint);
    document.conversations.insert(key, record);
    Ok(((kind == FingerprintKind::Event).then_some(version), true))
}
