//! Shared v1 activity/read authority. Host and local providers must use the same
//! path and this transaction implementation; never retain a separate writer.
use crate as gateway;
use crate as provider;
use crate::ProtocolError;
use rusqlite::{params, Connection};

use std::ops::{Deref, DerefMut};
type StateResult<T> = Result<T, ProtocolError>;
pub const CONVERSATION_STATE_PATH_ENV: &str = "CODEPET_CONVERSATION_STATE_DATABASE";
use ring::digest::{digest, SHA256};
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const ACTIVITY_VERSION_PREFIX: &str = "activity-";

#[derive(Clone, Debug, PartialEq)]
struct ConversationActivityRecord {
    resource: gateway::RoutedResourceId,
    latest_version: u64,
    title: Option<String>,
    summary_fingerprint: Option<String>,
    detail_fingerprint: Option<String>,
    last_activity_event_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ClientReadRecord {
    baseline_version: u64,
    reads: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq)]
struct ConversationStateSnapshot {
    latest_version: u64,
    conversations: BTreeMap<String, ConversationActivityRecord>,
    clients: BTreeMap<String, ClientReadRecord>,
}

impl Default for ConversationStateSnapshot {
    fn default() -> Self {
        Self {
            latest_version: 0,
            conversations: BTreeMap::new(),
            clients: BTreeMap::new(),
        }
    }
}

pub struct SharedConversationStateStore {
    path: Option<PathBuf>,
    document: Mutex<ConversationStateSnapshot>,
}

impl SharedConversationStateStore {
    pub fn memory() -> Self {
        Self {
            path: None,
            document: Mutex::new(ConversationStateSnapshot::default()),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> StateResult<Self> {
        let path = path.as_ref().to_path_buf();
        let store = Self {
            path: Some(path),
            document: Mutex::new(ConversationStateSnapshot::default()),
        };
        // Validate the fresh schema and read state under a SQLite transaction.
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
        document.conversations.entry(key.clone()).or_insert_with(|| ConversationActivityRecord {
            resource: conversation.clone(), latest_version: latest, title: None,
            summary_fingerprint: None, detail_fingerprint: None, last_activity_event_key: None,
        });
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
        let mut document = self.lock()?;
        let (version, mut dirty) = observe_in_document(&mut document, &conversation.resource, FingerprintKind::Summary, fingerprint.expect("summary always exists"))?;
        dirty |= update_title(&mut document, conversation);
        if dirty { self.persist(&document)?; }
        Ok(version)
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
        let (_, mut dirty) = observe_in_document(
            &mut document,
            &conversation.resource,
            FingerprintKind::Summary,
            value,
        )?;
        dirty |= update_title(&mut document, conversation);
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
            dirty |= update_title(&mut document, row);
            decorate_from_document(&document, scope, row);
        }
        if dirty {
            self.persist(&document)?;
        }
        Ok(dirty)
    }

    /// Page discovery establishes baselines without invalidating the page being
    /// built. Existing changed summaries or newly discovered unread members do
    /// change the attention ranking and must still invalidate recent views.
    pub fn observe_page_summaries(&self, scope: &str, rows: &mut [gateway::Conversation]) -> StateResult<bool> {
        let mut document = self.lock()?;
        let mut dirty = ensure_scope(&mut document, scope);
        let mut changed = false;
        for row in rows.iter_mut() {
            let existed = document.conversations.contains_key(&resource_key(&row.resource));
            let mut observed = false;
            if let Some(value) = summary_fingerprint(row) {
                observed = observe_in_document(&mut document, &row.resource, FingerprintKind::Summary, value)?.1;
                dirty |= observed;
            }
            dirty |= update_title(&mut document, row);
            decorate_from_document(&document, scope, row);
            changed |= observed && (existed || row.read_state.as_ref().is_some_and(|state| state.unread));
        }
        if dirty { self.persist(&document)?; }
        Ok(changed)
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
        let db = if let Some(path) = &self.path {
            let db = crate::app_state::open_app_state(path)?;
            db.execute_batch("BEGIN IMMEDIATE").map_err(crate::db_error)?;
            *document = read_document(&db)?;
            Some(db)
        } else { None };
        let original = document.clone();
        Ok(StateGuard { document, original, db })
    }

    fn persist(&self, document: &StateGuard<'_>) -> StateResult<()> {
        if let Some(db) = &document.db {
            write_state_document(db, document, &document.original)?;
            db.execute_batch("COMMIT").map_err(crate::db_error)?;
        }
        Ok(())
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
            Self::Event => &record.last_activity_event_key,
        }
    }

    fn slot(self, record: &mut ConversationActivityRecord) -> &mut Option<String> {
        match self {
            Self::Summary => &mut record.summary_fingerprint,
            Self::Detail => &mut record.detail_fingerprint,
            Self::Event => &mut record.last_activity_event_key,
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
    Some(format!("s1:{}", fingerprint([
        "summary",
        bounded_preview(conversation.preview.as_deref().unwrap_or("")),
        status,
    ])))
}

pub fn detail_fingerprint(items: &[gateway::ConversationItem]) -> Option<String> {
    let item = items.iter().rev().find(|item| {
        item_role(item) == Some(gateway::ConversationItemRole::Assistant)
            || item_approval(item)
                .is_some_and(|approval| approval.status == gateway::ApprovalStatus::Pending)
    })?;
    if item_resource(item).native_resource_id.len()>512 || item_turn(item).native_resource_id.len()>512 { return None; }
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
    fn open_store(path: impl AsRef<Path>) -> StateResult<SharedConversationStateStore> {
        let db = crate::app_state::open_app_state(path.as_ref())?;
        db.execute("INSERT OR IGNORE INTO providers(id,display_name,created_at,updated_at) VALUES('codex','Codex',0,0)",[]).unwrap();
        db.execute("INSERT OR IGNORE INTO provider_instances VALUES('codex-work','codex','local','work')",[]).unwrap();
        SharedConversationStateStore::open(path)
    }
    #[test]
    fn summary_hash_input_is_bounded_and_utf8_safe() {
        let mut row = conversation(&format!("{}tail-a", "界".repeat(200)));
        let before = summary_fingerprint(&row);
        row.preview = Some(format!("{}tail-b", "界".repeat(200)));
        assert_eq!(before, summary_fingerprint(&row));
        assert_eq!(bounded_preview(row.preview.as_deref().unwrap()).len(), 510);
        row.status = gateway::ConversationStatus::Running;
        assert_ne!(before, summary_fingerprint(&row));
    }

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
        let first = open_store(&path).unwrap();
        first.ensure_client("mobile").unwrap();
        first.ensure_client("desktop").unwrap();
        let mut row = conversation("baseline");
        first.observe_summary(&row).unwrap();
        let second = open_store(&path).unwrap();
        row.preview = Some("new response".into());
        assert_eq!(second.observe_summary(&row).unwrap(), Some(1));
        assert_eq!(first.observe_summary(&row).unwrap(), None);
        first
            .mark_read("mobile", &row.resource, "activity-1")
            .unwrap();
        assert!(second.unread("mobile", "codex-work").unwrap().is_empty());
        assert_eq!(second.unread("desktop", "codex-work").unwrap().len(), 1);
        drop(first);
        let reopened = open_store(&path).unwrap();
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
            assert!(open_store(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn identical_fingerprints_and_batches_do_not_replace_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let store = open_store(&path).unwrap();
        let mut rows = vec![conversation("baseline")];
        store
            .observe_and_decorate_summaries("mobile", &mut rows)
            .unwrap();
        let before: String = crate::app_state::open_app_state(&path)
            .unwrap()
            .query_row(
                "SELECT group_concat(coalesce(summary_fingerprint,''),'|') FROM conversations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(store.observe_summary(&rows[0]).unwrap(), None);
        assert!(!store
            .observe_and_decorate_summaries("mobile", &mut rows)
            .unwrap());
        let after: String = crate::app_state::open_app_state(&path)
            .unwrap()
            .query_row(
                "SELECT group_concat(coalesce(summary_fingerprint,''),'|') FROM conversations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn page_baselines_do_not_invalidate_but_existing_activity_does() {
        let store = SharedConversationStateStore::memory();
        let mut rows = vec![conversation("first-page")];
        assert!(!store.observe_page_summaries("mobile", &mut rows).unwrap());
        assert!(!rows[0].read_state.as_ref().unwrap().unread);
        rows[0].preview = Some("new content".into());
        assert!(store.observe_page_summaries("mobile", &mut rows).unwrap());
        assert!(rows[0].read_state.as_ref().unwrap().unread);
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

// The SQLite write transaction coordinates Host and Provider processes. A read or
// failed mutation rolls back on drop; only persist commits a complete state change.
struct StateGuard<'a> {
    document: std::sync::MutexGuard<'a, ConversationStateSnapshot>,
    original: ConversationStateSnapshot,
    db: Option<Connection>,
}
impl Deref for StateGuard<'_> {
    type Target = ConversationStateSnapshot;
    fn deref(&self) -> &Self::Target { &self.document }
}
impl DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.document }
}
impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        if let Some(db) = &self.db {
            if !db.is_autocommit() { let _ = db.execute_batch("ROLLBACK"); }
        }
    }
}
fn state_error(code: &str, message: impl Into<String>) -> ProtocolError {
    crate::error(code, message.into())
}
fn bounded_preview(preview: &str) -> &str {
    let mut end = preview.len().min(512);
    while !preview.is_char_boundary(end) { end -= 1; }
    preview[..end].trim()
}
fn update_title(document: &mut ConversationStateSnapshot, row: &gateway::Conversation) -> bool {
    if let Some(record) = document.conversations.get_mut(&resource_key(&row.resource)) {
        if record.title.as_ref() != Some(&row.title) {
            record.title = Some(row.title.clone());
            return true;
        }
    }
    false
}
fn read_document(db: &Connection) -> StateResult<ConversationStateSnapshot> {
    let mut document = ConversationStateSnapshot::default();
    document.latest_version = db.query_row("SELECT last_version FROM activity_scopes WHERE id='host'", [], |r|r.get(0)).map_err(crate::db_error)?;
    let mut statement = db.prepare("SELECT provider_instance_id,native_conversation_id,activity_version,title,summary_fingerprint,detail_fingerprint,last_activity_event_key FROM conversations WHERE activity_scope_id='host'").map_err(crate::db_error)?;
    let rows = statement.query_map([], |r| Ok(ConversationActivityRecord {
        resource: gateway::RoutedResourceId { provider_id:r.get(0)?, native_resource_id:r.get(1)? },
        latest_version:r.get(2)?, title:r.get(3)?, summary_fingerprint:r.get(4)?, detail_fingerprint:r.get(5)?, last_activity_event_key:r.get(6)?,
    })).map_err(crate::db_error)?;
    for row in rows { let row = row.map_err(crate::db_error)?; document.conversations.insert(resource_key(&row.resource),row); }
    let mut statement = db.prepare("SELECT reader_id,baseline_version FROM reader_baselines WHERE scope_id='host'").map_err(crate::db_error)?;
    for row in statement.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get(1)?))).map_err(crate::db_error)? {
        let (reader, baseline_version) = row.map_err(crate::db_error)?;
        document.clients.insert(reader,ClientReadRecord { baseline_version, reads:BTreeMap::new() });
    }
    let mut statement = db.prepare("SELECT c.provider_instance_id,c.native_conversation_id,r.reader_id,r.read_version FROM conversation_reads r JOIN conversations c ON c.id=r.conversation_id WHERE c.activity_scope_id='host'").map_err(crate::db_error)?;
    for row in statement.query_map([], |r| Ok((gateway::RoutedResourceId { provider_id:r.get(0)?,native_resource_id:r.get(1)? },r.get::<_,String>(2)?,r.get(3)?))).map_err(crate::db_error)? {
        let (resource,reader,version) = row.map_err(crate::db_error)?;
        if let Some(client) = document.clients.get_mut(&reader) { client.reads.insert(resource_key(&resource),version); }
    }
    Ok(document)
}
fn write_state_document(db: &Connection, document: &ConversationStateSnapshot, original: &ConversationStateSnapshot) -> StateResult<()> {
    let now = chrono::Utc::now().timestamp_millis();
    db.execute("UPDATE activity_scopes SET last_version=?1 WHERE id='host' AND last_version!=?1", [document.latest_version]).map_err(crate::db_error)?;
    for (key,record) in &document.conversations {
        if original.conversations.get(key)==Some(record) { continue; }
        db.execute("INSERT INTO conversations(provider_instance_id,native_conversation_id,title,activity_scope_id,activity_version,summary_fingerprint,detail_fingerprint,last_activity_event_key,created_at,updated_at)
            VALUES(?1,?2,?3,'host',?4,?5,?6,?7,?8,?8)
            ON CONFLICT(provider_instance_id,native_conversation_id) DO UPDATE SET title=excluded.title,activity_version=excluded.activity_version,summary_fingerprint=excluded.summary_fingerprint,detail_fingerprint=excluded.detail_fingerprint,last_activity_event_key=excluded.last_activity_event_key,updated_at=excluded.updated_at
            WHERE title IS NOT excluded.title OR activity_version!=excluded.activity_version OR summary_fingerprint IS NOT excluded.summary_fingerprint OR detail_fingerprint IS NOT excluded.detail_fingerprint OR last_activity_event_key IS NOT excluded.last_activity_event_key",
            params![record.resource.provider_id,record.resource.native_resource_id,record.title,record.latest_version,record.summary_fingerprint,record.detail_fingerprint,record.last_activity_event_key,now]).map_err(crate::db_error)?;
    }
    for (reader, client) in &document.clients {
        if original.clients.get(reader)==Some(client) { continue; }
        db.execute("INSERT OR IGNORE INTO reader_baselines(scope_id,reader_id,baseline_version) VALUES('host',?1,?2)",params![reader,client.baseline_version]).map_err(crate::db_error)?;
        for (key, version) in &client.reads {
            if original.clients.get(reader).and_then(|c|c.reads.get(key))==Some(version) { continue; }
            let Some(record) = document.conversations.get(key) else { continue };
            db.execute("INSERT INTO conversation_reads(conversation_id,reader_id,read_version,updated_at) SELECT id,?3,?4,?5 FROM conversations WHERE provider_instance_id=?1 AND native_conversation_id=?2
                ON CONFLICT(conversation_id,reader_id) DO UPDATE SET read_version=excluded.read_version,updated_at=excluded.updated_at WHERE read_version<excluded.read_version",
                params![record.resource.provider_id,record.resource.native_resource_id,reader,version,now]).map_err(crate::db_error)?;
        }
    }
    Ok(())
}

fn ensure_scope(document: &mut ConversationStateSnapshot, scope: &str) -> bool {
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
    document: &ConversationStateSnapshot,
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
    document: &mut ConversationStateSnapshot,
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
        title: None,
        summary_fingerprint: None,
        detail_fingerprint: None,
        last_activity_event_key: None,
    };
    *kind.slot(&mut record) = Some(fingerprint);
    document.conversations.insert(key, record);
    Ok(((kind == FingerprintKind::Event).then_some(version), true))
}
