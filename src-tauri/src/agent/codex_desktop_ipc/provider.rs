use super::client::{
    CodexDesktopClient, DesktopClientEvent, DesktopConnectionSnapshot, DesktopConnectionStatus,
};
use super::mapper::{map_thread, provider, MappedThread};
use super::protocol::DesktopIpcError;
use super::state::ThreadSnapshot;
use crate::runtime_gateway::generated::{
    ApprovalResolveRequest, ApprovalResolveResponse, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ConversationUpsertedEvent, ProtocolError,
    ProtocolEvent, Provider, ProviderStatus, ProviderStatusChangedEvent, TurnInterruptRequest,
    TurnInterruptResponse, TurnSendRequest, TurnSendResponse, TurnUpsertedEvent, PROTOCOL_VERSION,
};
use crate::runtime_gateway::{ProviderAdapter, ProviderEventSink, ProviderFuture};
use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct CodexProviderState {
    status: ProviderStatus,
    unavailable_reason: Option<String>,
    connection_generation: u64,
    follower_epoch: Option<u64>,
    threads: BTreeMap<String, MappedThread>,
    bootstrap_workers: HashSet<String>,
    retired: bool,
}

pub struct CodexProviderAdapter {
    client: CodexDesktopClient,
    state: Arc<Mutex<CodexProviderState>>,
    forwarder: Mutex<Option<JoinHandle<()>>>,
}

impl CodexProviderAdapter {
    pub fn spawn(events: ProviderEventSink) -> Self {
        let client = CodexDesktopClient::spawn();
        let connection = client.connection_snapshot();
        let (status, unavailable_reason) = provider_connection_state(&connection);
        let state = Arc::new(Mutex::new(CodexProviderState {
            status,
            unavailable_reason,
            connection_generation: connection.generation,
            follower_epoch: None,
            threads: BTreeMap::new(),
            bootstrap_workers: HashSet::new(),
            retired: false,
        }));
        let incoming = client.subscribe();
        let handle = start_event_forwarder(
            incoming,
            client.clone(),
            state.clone(),
            events.clone(),
        );
        Self {
            client,
            state,
            forwarder: Mutex::new(Some(handle)),
        }
    }

    pub(crate) fn retire(&self) {
        lock(&self.state).retired = true;
        if let Err(error) = self.client.shutdown() {
            crate::app_log::error(
                "codex_desktop_provider",
                &format!("failed to stop Desktop IPC client error={error}"),
            );
        }
        if let Some(handle) = lock(&self.forwarder).take() {
            let _ = handle.join();
        }
    }
}

impl Drop for CodexProviderAdapter {
    fn drop(&mut self) {
        self.retire();
    }
}

impl ProviderAdapter for CodexProviderAdapter {
    fn provider(&self) -> Provider {
        let state = lock(&self.state);
        provider(state.status, state.unavailable_reason.as_deref())
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProviderFuture<'a, ConversationListResponse> {
        let state = self.state.clone();
        Box::pin(async move {
            let offset = parse_cursor(request.cursor.as_deref())?;
            let limit = request.limit.unwrap_or(50);
            if !(1..=100).contains(&limit) {
                return Err(invalid_limit_error());
            }
            let limit = usize::try_from(limit).map_err(|_| invalid_limit_error())?;
            let mut conversations = lock(&state)
                .threads
                .values()
                .map(|thread| thread.conversation.clone())
                .collect::<Vec<_>>();
            conversations.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            let total = conversations.len();
            let conversations = conversations
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            let next_offset = offset.saturating_add(conversations.len());
            Ok(ConversationListResponse {
                conversations,
                next_cursor: (next_offset < total).then(|| format!("desktop:{next_offset}")),
                event_sequence: 0,
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProviderFuture<'a, ConversationGetResponse> {
        let cached = lock(&self.state)
            .threads
            .get(&request.conversation_id)
            .map(|thread| thread.conversation.clone());
        if let Some(conversation) = cached {
            return Box::pin(async move { Ok(ConversationGetResponse { conversation }) });
        }
        let client = self.client.clone();
        Box::pin(async move {
            let conversation_id = request.conversation_id;
            let bootstrap_id = conversation_id.clone();
            let snapshot = tokio::task::spawn_blocking(move || {
                client.bootstrap_thread(&bootstrap_id)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(desktop_error)?;
            let mapped = map_thread(&snapshot);
            let conversation = mapped.conversation.clone();
            Ok(ConversationGetResponse { conversation })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        _request: ConversationCreateRequest,
    ) -> ProviderFuture<'a, ConversationCreateResponse> {
        unsupported("conversation.create")
    }

    fn turn_send<'a>(
        &'a self,
        _request: TurnSendRequest,
    ) -> ProviderFuture<'a, TurnSendResponse> {
        unsupported("turn.send")
    }

    fn turn_interrupt<'a>(
        &'a self,
        _request: TurnInterruptRequest,
    ) -> ProviderFuture<'a, TurnInterruptResponse> {
        unsupported("turn.interrupt")
    }

    fn approval_resolve<'a>(
        &'a self,
        _request: ApprovalResolveRequest,
    ) -> ProviderFuture<'a, ApprovalResolveResponse> {
        unsupported("approval.resolve")
    }
}

fn accept_follower_event(
    state: &Arc<Mutex<CodexProviderState>>,
    generation: u64,
    follower_epoch: u64,
) -> bool {
    let mut state = lock(state);
    if state.retired
        || state.status != ProviderStatus::Ready
        || state.connection_generation != generation
    {
        return false;
    }
    match state.follower_epoch {
        Some(current_epoch) => current_epoch == follower_epoch,
        None => {
            state.follower_epoch = Some(follower_epoch);
            true
        }
    }
}

fn accept_follower_reset(
    state: &Arc<Mutex<CodexProviderState>>,
    generation: u64,
    follower_epoch: u64,
) -> bool {
    let mut state = lock(state);
    if state.retired
        || state.status != ProviderStatus::Ready
        || state.connection_generation != generation
        || state
            .follower_epoch
            .is_some_and(|current_epoch| current_epoch > follower_epoch)
    {
        return false;
    }
    state.follower_epoch = Some(follower_epoch);
    true
}

fn start_event_forwarder(
    incoming: Receiver<DesktopClientEvent>,
    client: CodexDesktopClient,
    state: Arc<Mutex<CodexProviderState>>,
    events: ProviderEventSink,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(event) = incoming.recv() {
            if lock(&state).retired {
                break;
            }
            match event {
                DesktopClientEvent::ConnectionChanged(connection) => {
                    let ready = connection.status == DesktopConnectionStatus::Ready;
                    transition_connection(&state, &events, &connection);
                    if ready {
                        for conversation_id in client.known_threads() {
                            spawn_bootstrap(client.clone(), state.clone(), conversation_id);
                        }
                    }
                    if connection.status == DesktopConnectionStatus::Shutdown {
                        break;
                    }
                }
                DesktopClientEvent::ThreadDiscovered { conversation_id } => {
                    spawn_bootstrap(client.clone(), state.clone(), conversation_id);
                }
                DesktopClientEvent::ThreadStateChanged {
                    snapshot,
                    bootstrapped,
                    generation,
                    follower_epoch,
                } => {
                    if bootstrapped
                        && client.is_current_follower_epoch(generation, follower_epoch)
                        && accept_follower_event(
                            &state,
                            generation,
                            follower_epoch,
                        )
                    {
                        publish_snapshot(
                            &state,
                            &events,
                            snapshot,
                            PublicationKind::Live,
                        );
                    }
                }
                DesktopClientEvent::ThreadBootstrapped {
                    snapshot,
                    generation,
                    follower_epoch,
                } => {
                    if client.is_current_follower_epoch(generation, follower_epoch)
                        && accept_follower_event(&state, generation, follower_epoch)
                    {
                        publish_snapshot(
                            &state,
                            &events,
                            snapshot,
                            PublicationKind::Baseline,
                        );
                    }
                }
                DesktopClientEvent::RevisionGap {
                    conversation_id,
                    expected_base_revision,
                    received_base_revision,
                } => {
                    crate::app_log::warn(
                        "codex_desktop_provider",
                        &format!(
                            "thread revision gap; waiting for snapshot conversation_id={conversation_id} expected_base={expected_base_revision:?} received_base={received_base_revision:?}"
                        ),
                    );
                }
                DesktopClientEvent::FollowerStateReset {
                    reason,
                    generation,
                    follower_epoch,
                } => {
                    if accept_follower_reset(&state, generation, follower_epoch) {
                        reset_provider_projection(&state, &events, &reason);
                    }
                }
                DesktopClientEvent::Diagnostic(message) => {
                    crate::app_log::warn("codex_desktop_provider", &message);
                }
            }
        }
    })
}

fn spawn_bootstrap(
    client: CodexDesktopClient,
    state: Arc<Mutex<CodexProviderState>>,
    conversation_id: String,
) {
    {
        let mut state = lock(&state);
        if state.retired || !state.bootstrap_workers.insert(conversation_id.clone()) {
            return;
        }
    }
    thread::spawn(move || {
        let mut backoff = Duration::from_millis(250);
        let mut attempts = 0_u64;
        loop {
            if lock(&state).retired {
                break;
            }
            match client.bootstrap_followed_thread(&conversation_id) {
                Ok(_) => break,
                Err(DesktopIpcError::Shutdown) => break,
                Err(error) => {
                    attempts = attempts.saturating_add(1);
                    if attempts == 1 || attempts % 12 == 0 {
                        crate::app_log::warn(
                            "codex_desktop_provider",
                            &format!(
                                "could not bootstrap followed Desktop thread; retrying conversation_id={conversation_id} attempts={attempts} error={error}"
                            ),
                        );
                    }
                    thread::sleep(backoff);
                    backoff = backoff.saturating_mul(2).min(Duration::from_secs(5));
                }
            }
        }
        lock(&state).bootstrap_workers.remove(&conversation_id);
    });
}

fn publish_snapshot(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    snapshot: ThreadSnapshot,
    publication_kind: PublicationKind,
) {
    let mapped = map_thread(&snapshot);
    for diagnostic in &mapped.diagnostics {
        crate::app_log::warn(
            "codex_desktop_provider",
            &format!(
                "Desktop state mapping diagnostic conversation_id={} diagnostic={diagnostic}",
                snapshot.conversation_id
            ),
        );
    }
    publish_mapped_thread(state, events, mapped, publication_kind);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationKind {
    Baseline,
    Live,
}

fn publish_mapped_thread(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    mapped: MappedThread,
    publication_kind: PublicationKind,
) {
    let (conversation_changed, turn_changed) = {
        let mut state = lock(state);
        if state.retired || state.status != ProviderStatus::Ready {
            return;
        }
        let previous = state.threads.get(&mapped.conversation.id);
        if previous.is_some_and(|previous| mapped.revision < previous.revision) {
            crate::app_log::warn(
                "codex_desktop_provider",
                &format!(
                    "ignored stale Desktop projection conversation_id={} revision={} current_revision={}",
                    mapped.conversation.id,
                    mapped.revision,
                    previous.map(|thread| thread.revision).unwrap_or(0)
                ),
            );
            return;
        }
        let conversation_changed = previous
            .map(|previous| previous.conversation != mapped.conversation)
            .unwrap_or(true);
        let turn_changed = publication_kind == PublicationKind::Live
            && previous
                .map(|previous| previous.latest_turn != mapped.latest_turn)
                .unwrap_or(false);
        state
            .threads
            .insert(mapped.conversation.id.clone(), mapped.clone());
        (conversation_changed, turn_changed)
    };
    if conversation_changed {
        publish_event(
            events,
            ProtocolEvent::ConversationUpserted {
                protocol_version: PROTOCOL_VERSION,
                event_sequence: 0,
                payload: ConversationUpsertedEvent {
                    conversation: mapped.conversation.clone(),
                },
            },
        );
    }
    if turn_changed {
        if let Some(turn) = mapped.latest_turn {
            publish_event(
                events,
                ProtocolEvent::TurnUpserted {
                    protocol_version: PROTOCOL_VERSION,
                    event_sequence: 0,
                    payload: TurnUpsertedEvent { turn },
                },
            );
        }
    }
}

fn transition_connection(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    connection: &DesktopConnectionSnapshot,
) {
    let (next_status, next_reason) = provider_connection_state(connection);
    let event = {
        let mut state = lock(state);
        if state.retired {
            return;
        }
        let previous_status = state.status;
        let changed = previous_status != next_status || state.unavailable_reason != next_reason;
        if state.connection_generation != connection.generation {
            state.connection_generation = connection.generation;
            state.follower_epoch = None;
        }
        state.status = next_status;
        state.unavailable_reason = next_reason.clone();
        if next_status != ProviderStatus::Ready {
            state.threads.clear();
        }
        changed.then(|| ProtocolEvent::ProviderStatusChanged {
            protocol_version: PROTOCOL_VERSION,
            event_sequence: 0,
            payload: ProviderStatusChangedEvent {
                provider: provider(next_status, next_reason.as_deref()),
                previous_status: Some(previous_status),
            },
        })
    };
    if let Some(event) = event {
        publish_event(events, event);
    }
}

fn reset_provider_projection(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    reason: &str,
) {
    let events_to_publish = {
        let mut state = lock(state);
        if state.retired {
            return;
        }
        state.threads.clear();
        if state.status != ProviderStatus::Ready {
            Vec::new()
        } else {
            state.status = ProviderStatus::Connecting;
            state.unavailable_reason = Some(reason.to_string());
            let connecting = ProtocolEvent::ProviderStatusChanged {
                protocol_version: PROTOCOL_VERSION,
                event_sequence: 0,
                payload: ProviderStatusChangedEvent {
                    provider: provider(ProviderStatus::Connecting, Some(reason)),
                    previous_status: Some(ProviderStatus::Ready),
                },
            };
            state.status = ProviderStatus::Ready;
            state.unavailable_reason = None;
            let ready = ProtocolEvent::ProviderStatusChanged {
                protocol_version: PROTOCOL_VERSION,
                event_sequence: 0,
                payload: ProviderStatusChangedEvent {
                    provider: provider(ProviderStatus::Ready, None),
                    previous_status: Some(ProviderStatus::Connecting),
                },
            };
            vec![connecting, ready]
        }
    };
    for event in events_to_publish {
        publish_event(events, event);
    }
}

fn provider_connection_state(
    connection: &DesktopConnectionSnapshot,
) -> (ProviderStatus, Option<String>) {
    match connection.status {
        DesktopConnectionStatus::Connecting => (
            ProviderStatus::Connecting,
            Some("正在连接 Codex Desktop 私有 IPC".to_string()),
        ),
        DesktopConnectionStatus::Ready => (ProviderStatus::Ready, None),
        DesktopConnectionStatus::Unavailable => (
            ProviderStatus::Unavailable,
            Some(
                connection
                    .error
                    .as_ref()
                    .map(public_desktop_error)
                    .unwrap_or_else(|| "Codex Desktop 私有 IPC 不可用".to_string()),
            ),
        ),
        DesktopConnectionStatus::Shutdown => (
            ProviderStatus::Disconnected,
            Some("Codex Desktop 私有 IPC 已关闭".to_string()),
        ),
    }
}

fn parse_cursor(cursor: Option<&str>) -> Result<usize, ProtocolError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    cursor
        .strip_prefix("desktop:")
        .and_then(|offset| offset.parse::<usize>().ok())
        .ok_or_else(|| ProtocolError {
            code: "invalid_cursor".to_string(),
            message: "Codex Desktop conversation cursor is invalid".to_string(),
            retryable: false,
            details: None,
        })
}

fn invalid_limit_error() -> ProtocolError {
    ProtocolError {
        code: "invalid_request".to_string(),
        message: "conversation list limit exceeds the provider range".to_string(),
        retryable: false,
        details: None,
    }
}

fn desktop_error(error: DesktopIpcError) -> ProtocolError {
    let retryable = !matches!(
        error,
        DesktopIpcError::Protocol(_)
            | DesktopIpcError::UnsafeSocket(_)
            | DesktopIpcError::Unsupported(_)
            | DesktopIpcError::Shutdown
    );
    ProtocolError {
        code: "desktop_ipc_unavailable".to_string(),
        message: public_desktop_error(&error),
        retryable,
        details: None,
    }
}

fn public_desktop_error(error: &DesktopIpcError) -> String {
    match error {
        DesktopIpcError::SocketPath(message) => {
            format!("Codex Desktop IPC socket is unavailable: {message}")
        }
        DesktopIpcError::UnsafeSocket(message) => {
            format!("Codex Desktop IPC socket failed safety checks: {message}")
        }
        DesktopIpcError::Io(message) => {
            format!("Codex Desktop IPC connection failed: {message}")
        }
        DesktopIpcError::Protocol(_) => {
            "Codex Desktop IPC protocol is incompatible or malformed".to_string()
        }
        DesktopIpcError::Remote(_) => {
            "Codex Desktop refused the follower synchronization request".to_string()
        }
        DesktopIpcError::Timeout(_) => {
            "Timed out while synchronizing with Codex Desktop".to_string()
        }
        DesktopIpcError::Disconnected(_) => {
            "Codex Desktop IPC connection was lost".to_string()
        }
        DesktopIpcError::Shutdown => "Codex Desktop IPC client is shut down".to_string(),
        DesktopIpcError::Unsupported(_) => {
            "Codex Desktop IPC is unsupported on this platform".to_string()
        }
    }
}

fn provider_task_error(error: tokio::task::JoinError) -> ProtocolError {
    ProtocolError {
        code: "provider_error".to_string(),
        message: format!("Codex Desktop provider task failed: {error}"),
        retryable: false,
        details: None,
    }
}

fn unsupported<'a, T>(method: &'static str) -> ProviderFuture<'a, T> {
    Box::pin(async move {
        Err(ProtocolError {
            code: "capability_unsupported".to_string(),
            message: format!(
                "{method} is unavailable: the Codex Desktop phase-one provider is read-only"
            ),
            retryable: false,
            details: None,
        })
    })
}

fn publish_event(events: &ProviderEventSink, event: ProtocolEvent) {
    if let Err(error) = events.publish(event) {
        crate::app_log::error(
            "codex_desktop_provider",
            &format!("failed to publish Runtime Gateway event error={error:?}"),
        );
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_gateway::Gateway;
    use serde_json::json;

    fn mapped_thread(revision: u64) -> MappedThread {
        map_thread(&ThreadSnapshot {
            conversation_id: "thread-one".to_string(),
            owner_client_id: "owner-one".to_string(),
            revision,
            state: json!({
                "id": "thread-one",
                "title": "Desktop task",
                "createdAt": 1_700_000_000_000_u64,
                "updatedAt": 1_700_000_001_000_u64,
                "threadRuntimeStatus": { "type": "idle" },
                "currentPermissions": {
                    "sandboxPolicy": { "type": "workspaceWrite" }
                },
                "turns": [{
                    "turnId": "turn-one",
                    "status": "completed",
                    "turnStartedAtMs": 1_700_000_000_000_u64,
                    "durationMs": 1_000
                }]
            }),
        })
    }

    #[test]
    fn standard_protocol_errors_do_not_expose_private_method_names() {
        let error = DesktopIpcError::Timeout(
            "thread-follower-load-complete-history".to_string(),
        );
        let protocol = desktop_error(error);
        assert_eq!(
            protocol.message,
            "Timed out while synchronizing with Codex Desktop"
        );
        assert!(!protocol.message.contains("thread-follower"));
    }

    #[test]
    fn bootstrap_is_a_non_ringing_baseline_and_cannot_overwrite_newer_revision() {
        let gateway = Gateway::default();
        let events = gateway.event_sink();
        let state = Arc::new(Mutex::new(CodexProviderState {
            status: ProviderStatus::Ready,
            unavailable_reason: None,
            connection_generation: 1,
            follower_epoch: Some(1),
            threads: BTreeMap::new(),
            bootstrap_workers: HashSet::new(),
            retired: false,
        }));

        publish_mapped_thread(
            &state,
            &events,
            mapped_thread(2),
            PublicationKind::Baseline,
        );
        assert!(matches!(
            gateway.replay_events(None).unwrap().as_slice(),
            [ProtocolEvent::ConversationUpserted { .. }]
        ));

        publish_mapped_thread(
            &state,
            &events,
            mapped_thread(1),
            PublicationKind::Live,
        );
        assert_eq!(lock(&state).threads["thread-one"].revision, 2);
        assert_eq!(gateway.replay_events(None).unwrap().len(), 1);
    }
}
