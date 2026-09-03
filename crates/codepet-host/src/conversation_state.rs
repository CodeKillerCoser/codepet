use crate::persistence::{persistence_io, write_json_atomically};
use crate::{HostError, HostResult};
use codepet_gateway_sdk as gateway;
use codepet_provider_sdk as provider;
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

pub(crate) struct ConversationStateStore {
    path: Option<PathBuf>,
    document: Mutex<ConversationStateDocument>,
}

impl ConversationStateStore {
    pub(crate) fn memory() -> Self {
        Self {
            path: None,
            document: Mutex::new(ConversationStateDocument::default()),
        }
    }

    pub(crate) fn open(path: impl AsRef<Path>) -> HostResult<Self> {
        let path = path.as_ref().to_path_buf();
        let document = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ConversationStateDocument>(&bytes) {
                Ok(document) if document.version == DOCUMENT_VERSION => document,
                Ok(document) => {
                    eprintln!(
                        "Ignoring conversation state {} with unsupported version {}",
                        path.display(),
                        document.version
                    );
                    ConversationStateDocument::default()
                }
                Err(error) => {
                    eprintln!(
                        "Ignoring invalid conversation state {}: {error}",
                        path.display()
                    );
                    ConversationStateDocument::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ConversationStateDocument::default()
            }
            Err(error) => return Err(persistence_io("read conversation state", &path, error)),
        };
        Ok(Self {
            path: Some(path),
            document: Mutex::new(document),
        })
    }

    pub(crate) fn ensure_client(&self, caller_scope: &str) -> HostResult<()> {
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

    pub(crate) fn decorate(
        &self,
        caller_scope: &str,
        conversation: &mut gateway::Conversation,
    ) -> HostResult<()> {
        self.ensure_client(caller_scope)?;
        let document = self.lock()?;
        let key = resource_key(&conversation.resource);
        let latest = document
            .conversations
            .get(&key)
            .map(|record| record.latest_version)
            .unwrap_or(document.latest_version);
        let client = document.clients.get(caller_scope).ok_or_else(|| {
            HostError::new("conversation_state_unavailable", "client read state is unavailable")
        })?;
        let read = client.reads.get(&key).copied().unwrap_or(client.baseline_version);
        conversation.read_state = Some(gateway::ConversationReadState {
            unread: latest > read,
            activity_version: activity_version(latest),
        });
        Ok(())
    }

    pub(crate) fn mark_read(
        &self,
        caller_scope: &str,
        conversation: &gateway::RoutedResourceId,
        observed_activity_version: &str,
    ) -> HostResult<gateway::ConversationReadState> {
        let observed = parse_activity_version(observed_activity_version)?;
        let mut document = self.lock()?;
        let key = resource_key(conversation);
        let latest = document
            .conversations
            .get(&key)
            .map(|record| record.latest_version)
            .unwrap_or(document.latest_version);
        if observed > latest {
            return Err(HostError::new(
                "invalid_activity_version",
                "observed activity version is ahead of the Host",
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

    pub(crate) fn observe_summary(
        &self,
        conversation: &gateway::Conversation,
    ) -> HostResult<Option<u64>> {
        let fingerprint = summary_fingerprint(conversation);
        self.observe_fingerprint(&conversation.resource, FingerprintKind::Summary, fingerprint)
    }

    pub(crate) fn observe_detail(
        &self,
        conversation: &gateway::RoutedResourceId,
        items: &[gateway::ConversationItem],
    ) -> HostResult<Option<u64>> {
        let fingerprint = detail_fingerprint(items);
        self.observe_fingerprint(conversation, FingerprintKind::Detail, fingerprint)
    }

    pub(crate) fn observe_provider_event(
        &self,
        event: &provider::ProtocolEvent,
    ) -> HostResult<Option<(gateway::RoutedResourceId, u64)>> {
        let observed = match event {
            provider::ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.kind == provider::ConversationContentKind::Text
                    && !params.delta.is_empty() => Some((
                        params.conversation.clone(),
                        format!(
                            "output:{}:{}",
                            params.turn.native_resource_id, params.content_id
                        ),
                    )),
            provider::ProtocolEvent::EventApprovalRequested { params, .. } => Some((
                params.approval.conversation.clone(),
                format!("approval:{}", params.approval.resource.native_resource_id),
            )),
            provider::ProtocolEvent::EventConversationItemUpserted { params, .. }
                if matches!(
                    params.item.status,
                    provider::ConversationItemStatus::Completed
                        | provider::ConversationItemStatus::Failed
                        | provider::ConversationItemStatus::Interrupted
                ) => Some((
                    params.item.conversation.clone(),
                    format!(
                        "item-terminal:{}:{:?}",
                        params.item.resource.native_resource_id, params.item.status
                    ),
                )),
            provider::ProtocolEvent::EventTurnUpserted { params, .. }
                if matches!(
                    params.turn.status,
                    provider::TurnStatus::Completed | provider::TurnStatus::Failed
                ) => Some((
                    params.turn.conversation.clone(),
                    format!(
                        "turn-terminal:{}:{:?}",
                        params.turn.resource.native_resource_id, params.turn.status
                    ),
                )),
            provider::ProtocolEvent::EventConversationUpserted { params, .. }
                if params.conversation.status == provider::ConversationStatus::WaitingUserInput => {
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
        let Some(version) = self.observe_fingerprint(
            &resource,
            FingerprintKind::Event,
            Some(fingerprint),
        )? else {
            return Ok(None);
        };
        Ok(Some((resource, version)))
    }

    fn observe_fingerprint(
        &self,
        resource: &gateway::RoutedResourceId,
        kind: FingerprintKind,
        fingerprint: Option<String>,
    ) -> HostResult<Option<u64>> {
        let Some(fingerprint) = fingerprint else {
            return Ok(None);
        };
        let mut document = self.lock()?;
        let key = resource_key(resource);
        if let Some(record) = document.conversations.get(&key) {
            if kind.value(record).as_ref() == Some(&fingerprint) {
                return Ok(None);
            }
            if kind != FingerprintKind::Event && kind.value(record).is_none() {
                let record = document
                    .conversations
                    .get_mut(&key)
                    .expect("conversation record checked");
                *kind.slot(record) = Some(fingerprint);
                self.persist(&document)?;
                return Ok(None);
            }
            document.latest_version = document.latest_version.saturating_add(1);
            let version = document.latest_version;
            let record = document
                .conversations
                .get_mut(&key)
                .expect("conversation record checked");
            *kind.slot(record) = Some(fingerprint);
            record.latest_version = version;
            self.persist(&document)?;
            return Ok(Some(version));
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
        self.persist(&document)?;
        Ok((kind == FingerprintKind::Event).then_some(version))
    }

    fn lock(&self) -> HostResult<std::sync::MutexGuard<'_, ConversationStateDocument>> {
        self.document.lock().map_err(|_| {
            HostError::new(
                "conversation_state_unavailable",
                "conversation state lock is poisoned",
            )
        })
    }

    fn persist(&self, document: &ConversationStateDocument) -> HostResult<()> {
        match self.path.as_deref() {
            Some(path) => write_json_atomically(path, document),
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

fn summary_fingerprint(conversation: &gateway::Conversation) -> Option<String> {
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

fn detail_fingerprint(items: &[gateway::ConversationItem]) -> Option<String> {
    let item = items.iter().rev().find(|item| {
        item.role == Some(gateway::ConversationItemRole::Assistant)
            || item.approval.as_ref().is_some_and(|approval| {
                approval.status == gateway::ApprovalStatus::Pending
            })
    })?;
    Some(fingerprint([
        "detail",
        item.resource.native_resource_id.as_str(),
        item.turn.native_resource_id.as_str(),
    ]))
}

fn fingerprint<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut input = Vec::new();
    for part in parts {
        input.extend_from_slice(part.as_bytes());
        input.push(0);
    }
    let value = digest(&SHA256, &input);
    value.as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn resource_key(resource: &gateway::RoutedResourceId) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        resource.device_id,
        resource.provider_plugin_id,
        resource.provider_instance_id,
        resource.native_resource_id
    )
}

fn activity_version(version: u64) -> String {
    format!("{ACTIVITY_VERSION_PREFIX}{version}")
}

fn parse_activity_version(value: &str) -> HostResult<u64> {
    value
        .strip_prefix(ACTIVITY_VERSION_PREFIX)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|version| activity_version(*version) == value)
        .ok_or_else(|| HostError::new("invalid_activity_version", "invalid activity version"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_read_only_advances_to_the_observed_activity() {
        let store = ConversationStateStore::memory();
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
        let snapshots = ConversationStateStore::memory();
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

        let events = ConversationStateStore::memory();
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

    fn conversation(preview: &str) -> gateway::Conversation {
        gateway::Conversation {
            resource: gateway::RoutedResourceId {
                device_id: "host".to_string(),
                provider_plugin_id: "dev.codepet.codex".to_string(),
                provider_instance_id: "codex-work".to_string(),
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
