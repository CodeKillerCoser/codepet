use super::client::{
    CodexDesktopClient, DesktopApprovalDecision, DesktopClientEvent,
    DesktopConnectionSnapshot, DesktopConnectionStatus, NativeApprovalTarget,
};
use super::mapper::{
    map_thread, provider, MappedApproval, MappedThread, CODEX_PROVIDER_ID,
    CONTINUE_QUICK_REPLY_ID,
};
use super::protocol::DesktopIpcError;
use super::state::ThreadSnapshot;
use crate::runtime_gateway::generated::{
    Approval, ApprovalDecision, ApprovalRequestedEvent, ApprovalResolveRequest,
    ApprovalResolveResponse, ApprovalResolvedEvent, ApprovalStatus, ConversationCreateRequest,
    ConversationCreateResponse, Conversation, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ConversationUpsertedEvent, ProtocolError,
    ProtocolEvent, Provider, ProviderStatus, ProviderStatusChangedEvent, TurnInterruptRequest,
    TurnInterruptResponse, TurnSendRequest, TurnSendResponse, TurnTask, TurnTaskStatus,
    TurnUpsertedEvent, PROTOCOL_VERSION,
};
use crate::runtime_gateway::{ProviderAdapter, ProviderEventSink, ProviderFuture};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
struct ApprovalIntent {
    approval: Approval,
    decision: ApprovalDecision,
    acknowledged: bool,
    outcome_unknown: bool,
    authoritative_removed_at: Option<u64>,
}

struct CodexProviderState {
    status: ProviderStatus,
    unavailable_reason: Option<String>,
    connection_generation: u64,
    follower_epoch: Option<u64>,
    threads: BTreeMap<String, MappedThread>,
    approval_intents: BTreeMap<String, ApprovalIntent>,
    bootstrap_workers: HashSet<String>,
    retired: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDesktopCompanionSnapshot {
    pub provider: Provider,
    pub conversations: Vec<Conversation>,
}

pub struct CodexProviderAdapter {
    client: CodexDesktopClient,
    state: Arc<Mutex<CodexProviderState>>,
    events: ProviderEventSink,
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
            approval_intents: BTreeMap::new(),
            bootstrap_workers: HashSet::new(),
            retired: false,
        }));
        let incoming = client.subscribe();
        let handle = start_event_forwarder(incoming, client.clone(), state.clone(), events.clone());
        Self {
            client,
            state,
            events,
            forwarder: Mutex::new(Some(handle)),
        }
    }

    pub fn snapshot(&self) -> CodexDesktopCompanionSnapshot {
        let provider = self.provider();
        let mut conversations = if provider.status == ProviderStatus::Ready {
            lock(&self.state)
                .threads
                .values()
                .map(|thread| thread.conversation.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        conversations.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        let conversation_ids = conversations
            .iter()
            .map(|conversation| conversation.id.clone())
            .collect::<HashSet<_>>();
        publish_pending_approval_hydration(&self.state, &self.events, &conversation_ids);
        CodexDesktopCompanionSnapshot {
            provider,
            conversations,
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
        _request: ConversationListRequest,
    ) -> ProviderFuture<'a, ConversationListResponse> {
        unsupported("conversation.list")
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
                client.bootstrap_followed_thread(&bootstrap_id)
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
        request: TurnSendRequest,
    ) -> ProviderFuture<'a, TurnSendResponse> {
        let client = self.client.clone();
        Box::pin(async move {
            validate_provider_id(&request.provider_id)?;
            if request
                .quick_reply_id
                .as_deref()
                .is_some_and(|id| id != CONTINUE_QUICK_REPLY_ID)
            {
                return Err(ProtocolError {
                    code: "invalid_request".to_string(),
                    message: "quick reply id is not advertised by Codex Desktop".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let acknowledgement = tokio::task::spawn_blocking(move || {
                client
                    .send_turn(
                        &request.conversation_id,
                        &request.client_message_id,
                        &request.message,
                        request.steer_turn_id.as_deref(),
                    )
                    .map_err(desktop_error)
            })
            .await
            .map_err(provider_task_error)??;
            let mapped = map_thread(&acknowledgement.frozen_snapshot);
            let turn = if acknowledgement.started {
                TurnTask {
                    id: acknowledgement.turn_id,
                    provider_id: CODEX_PROVIDER_ID.to_string(),
                    conversation_id: acknowledgement.frozen_snapshot.conversation_id,
                    status: TurnTaskStatus::Queued,
                    display_summary: None,
                    started_at: None,
                    updated_at: now_ms(),
                    completed_at: None,
                    extension: None,
                }
            } else {
                mapped
                    .latest_turn
                    .filter(|turn| turn.id == acknowledgement.turn_id)
                    .ok_or_else(|| stale_action_error(
                        "the acknowledged steer turn is absent from the frozen snapshot",
                    ))?
            };
            Ok(TurnSendResponse { turn })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProviderFuture<'a, TurnInterruptResponse> {
        let client = self.client.clone();
        Box::pin(async move {
            validate_provider_id(&request.provider_id)?;
            let requested_turn_id = request.turn_id.clone();
            let snapshot = tokio::task::spawn_blocking(move || {
                client
                    .interrupt_turn(&request.conversation_id, &request.turn_id)
                    .map_err(desktop_error)
            })
            .await
            .map_err(provider_task_error)??;
            let turn = map_thread(&snapshot)
                .latest_turn
                .filter(|turn| turn.id == requested_turn_id)
                .ok_or_else(|| stale_action_error(
                    "the interrupt target is absent from the frozen snapshot",
                ))?;
            Ok(TurnInterruptResponse { turn })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProviderFuture<'a, ApprovalResolveResponse> {
        let client = self.client.clone();
        let state = self.state.clone();
        let events = self.events.clone();
        Box::pin(async move {
            validate_provider_id(&request.provider_id)?;
            let decision = request.decision;
            let mapped = begin_approval_resolution(&state, &request.approval_id, decision)?;
            let pending_approval = mapped.approval.clone();
            let target = NativeApprovalTarget {
                conversation_id: mapped.thread_id,
                owner_client_id: mapped.owner_client_id,
                revision: mapped.revision,
                request_id: mapped.raw_request_id,
                request_method: mapped.native_method,
                turn_id: mapped.turn_id,
                item_id: mapped.item_id,
            };
            let desktop_decision = match decision {
                ApprovalDecision::Approve => DesktopApprovalDecision::Approve,
                ApprovalDecision::Deny => DesktopApprovalDecision::Deny,
            };
            let result = tokio::task::spawn_blocking(move || {
                client
                    .resolve_approval(&target, desktop_decision)
                    .map_err(desktop_error)
            })
            .await
            .map_err(provider_task_error)?;
            match result {
                Ok(_) => {
                    if let Some(resolved) = acknowledge_approval_resolution(
                        &state,
                        &pending_approval.id,
                    ) {
                        publish_approval_resolved(&events, resolved);
                    }
                    Ok(ApprovalResolveResponse {
                        approval: pending_approval,
                    })
                }
                Err(error) => {
                    if error.code == "desktop_ipc_outcome_unknown" {
                        if let Some(expired) =
                            mark_approval_outcome_unknown(&state, &pending_approval.id)
                        {
                            publish_approval_resolved(&events, expired);
                        }
                    } else {
                        if let Some(expired) = fail_approval_resolution(
                            &state,
                            &pending_approval.id,
                        ) {
                            publish_approval_resolved(&events, expired);
                        }
                    }
                    Err(error)
                }
            }
        })
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
                        publish_local_snapshot(
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
                        publish_local_snapshot(
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

fn publish_local_snapshot(
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

fn publish_pending_approval_hydration(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    conversation_ids: &HashSet<String>,
) {
    let state = lock(state);
    for approval in state
        .threads
        .values()
        .filter(|thread| conversation_ids.contains(&thread.conversation.id))
        .flat_map(|thread| thread.approvals.iter())
        .map(|mapped| mapped.approval.clone())
    {
        publish_event(
            events,
            ProtocolEvent::ApprovalRequested {
                protocol_version: PROTOCOL_VERSION,
                event_sequence: 0,
                payload: ApprovalRequestedEvent { approval },
            },
        );
    }
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
    let (conversation_changed, turn_changed, requested, resolved) = {
        let mut state = lock(state);
        if state.retired || state.status != ProviderStatus::Ready {
            return;
        }
        let previous = state.threads.get(&mapped.conversation.id).cloned();
        if previous.as_ref().is_some_and(|previous| mapped.revision < previous.revision) {
            crate::app_log::warn(
                "codex_desktop_provider",
                &format!(
                    "ignored stale Desktop projection conversation_id={} revision={} current_revision={}",
                    mapped.conversation.id,
                    mapped.revision,
                    previous.as_ref().map(|thread| thread.revision).unwrap_or(0)
                ),
            );
            return;
        }
        let conversation_changed = previous
            .as_ref()
            .map(|previous| previous.conversation != mapped.conversation)
            .unwrap_or(true);
        let turn_changed = publication_kind == PublicationKind::Live
            && previous
                .as_ref()
                .map(|previous| previous.latest_turn != mapped.latest_turn)
                .unwrap_or(false);
        let previous_approvals = previous
            .as_ref()
            .map(|thread| {
                thread
                    .approvals
                    .iter()
                    .map(|mapped| (mapped.approval.id.clone(), mapped.approval.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let current_approval_ids = mapped
            .approvals
            .iter()
            .map(|mapped| mapped.approval.id.clone())
            .collect::<HashSet<_>>();
        let requested = mapped
            .approvals
            .iter()
            .filter(|mapped| !previous_approvals.contains_key(&mapped.approval.id))
            .map(|mapped| mapped.approval.clone())
            .collect::<Vec<_>>();
        let mut resolved = Vec::new();
        let intents_without_previous = state
            .approval_intents
            .iter()
            .filter(|(approval_id, intent)| {
                intent.approval.conversation_id == mapped.conversation.id
                    && !current_approval_ids.contains(*approval_id)
                    && !previous_approvals.contains_key(*approval_id)
            })
            .map(|(approval_id, _)| approval_id.clone())
            .collect::<Vec<_>>();
        for approval_id in intents_without_previous {
            let removal_time = mapped.conversation.updated_at;
            let still_native_pending = mapped
                .native_pending_approval_ids
                .contains(&approval_id);
            let outcome_unknown = state
                .approval_intents
                .get(&approval_id)
                .is_some_and(|intent| intent.outcome_unknown);
            if still_native_pending || outcome_unknown {
                if let Some(intent) = state.approval_intents.remove(&approval_id) {
                    resolved.push(resolve_approval_dto(
                        intent.approval,
                        None,
                        removal_time,
                    ));
                }
            } else if let Some(intent) = state.approval_intents.get_mut(&approval_id) {
                intent.authoritative_removed_at = Some(removal_time);
            }
        }
        for (approval_id, approval) in previous_approvals {
            if current_approval_ids.contains(&approval_id) {
                continue;
            }
            let removal_time = mapped.conversation.updated_at;
            if mapped.native_pending_approval_ids.contains(&approval_id) {
                let approval = state
                    .approval_intents
                    .remove(&approval_id)
                    .map(|intent| intent.approval)
                    .unwrap_or(approval);
                resolved.push(resolve_approval_dto(approval, None, removal_time));
                continue;
            }
            let acknowledged = state
                .approval_intents
                .get(&approval_id)
                .is_some_and(|intent| intent.acknowledged);
            let outcome_unknown = state
                .approval_intents
                .get(&approval_id)
                .is_some_and(|intent| intent.outcome_unknown);
            if acknowledged {
                if let Some(intent) = state.approval_intents.remove(&approval_id) {
                    resolved.push(resolve_approval_dto(
                        intent.approval,
                        Some(intent.decision),
                        removal_time,
                    ));
                }
            } else if outcome_unknown {
                if let Some(intent) = state.approval_intents.remove(&approval_id) {
                    resolved.push(resolve_approval_dto(
                        intent.approval,
                        None,
                        removal_time,
                    ));
                }
            } else if let Some(intent) = state.approval_intents.get_mut(&approval_id) {
                intent.authoritative_removed_at = Some(removal_time);
            } else {
                resolved.push(resolve_approval_dto(approval, None, removal_time));
            }
        }
        state
            .threads
            .insert(mapped.conversation.id.clone(), mapped.clone());
        (conversation_changed, turn_changed, requested, resolved)
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
        if let Some(turn) = mapped.latest_turn.clone() {
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
    for approval in requested {
        publish_event(
            events,
            ProtocolEvent::ApprovalRequested {
                protocol_version: PROTOCOL_VERSION,
                event_sequence: 0,
                payload: ApprovalRequestedEvent { approval },
            },
        );
    }
    for approval in resolved {
        publish_approval_resolved(events, approval);
    }
}

fn begin_approval_resolution(
    state: &Arc<Mutex<CodexProviderState>>,
    approval_id: &str,
    decision: ApprovalDecision,
) -> Result<MappedApproval, ProtocolError> {
    let mut state = lock(state);
    if state.retired || state.status != ProviderStatus::Ready {
        return Err(ProtocolError {
            code: "provider_unavailable".to_string(),
            message: "Codex Desktop provider is not ready".to_string(),
            retryable: false,
            details: None,
        });
    }
    if let Some(intent) = state.approval_intents.get(approval_id) {
        return Err(ProtocolError {
            code: if intent.outcome_unknown {
                "action_outcome_unknown"
            } else {
                "action_in_flight"
            }
            .to_string(),
            message: if intent.outcome_unknown {
                "A previous decision has an unknown outcome; wait for authoritative Desktop state"
                    .to_string()
            } else {
                "This approval already has a decision in flight".to_string()
            },
            retryable: false,
            details: None,
        });
    }
    let mut matches = state
        .threads
        .values()
        .flat_map(|thread| thread.approvals.iter())
        .filter(|mapped| mapped.approval.id == approval_id)
        .cloned()
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(stale_action_error(
            "the approval is no longer pending in authoritative Desktop state",
        ));
    }
    let mapped = matches.remove(0);
    if mapped.approval.status != ApprovalStatus::Pending
        || !mapped.approval.decisions.contains(&decision)
    {
        return Err(stale_action_error(
            "the approval does not accept this decision",
        ));
    }
    state.approval_intents.insert(
        approval_id.to_string(),
        ApprovalIntent {
            approval: mapped.approval.clone(),
            decision,
            acknowledged: false,
            outcome_unknown: false,
            authoritative_removed_at: None,
        },
    );
    Ok(mapped)
}

fn acknowledge_approval_resolution(
    state: &Arc<Mutex<CodexProviderState>>,
    approval_id: &str,
) -> Option<Approval> {
    let mut state = lock(state);
    let removal_time = state
        .approval_intents
        .get(approval_id)
        .and_then(|intent| intent.authoritative_removed_at);
    if let Some(removal_time) = removal_time {
        let intent = state.approval_intents.remove(approval_id)?;
        return Some(resolve_approval_dto(
            intent.approval,
            Some(intent.decision),
            removal_time,
        ));
    }
    if let Some(intent) = state.approval_intents.get_mut(approval_id) {
        intent.acknowledged = true;
    }
    None
}

fn fail_approval_resolution(
    state: &Arc<Mutex<CodexProviderState>>,
    approval_id: &str,
) -> Option<Approval> {
    let intent = lock(state).approval_intents.remove(approval_id)?;
    intent.authoritative_removed_at.map(|removed_at| {
        resolve_approval_dto(intent.approval, None, removed_at)
    })
}

fn mark_approval_outcome_unknown(
    state: &Arc<Mutex<CodexProviderState>>,
    approval_id: &str,
) -> Option<Approval> {
    let mut state = lock(state);
    let removed_at = state
        .approval_intents
        .get_mut(approval_id)
        .and_then(|intent| {
            intent.outcome_unknown = true;
            intent.authoritative_removed_at
        });
    removed_at.and_then(|removed_at| {
        let intent = state.approval_intents.remove(approval_id)?;
        Some(resolve_approval_dto(intent.approval, None, removed_at))
    })
}

fn resolve_approval_dto(
    mut approval: Approval,
    decision: Option<ApprovalDecision>,
    resolved_at: u64,
) -> Approval {
    approval.status = match decision {
        Some(ApprovalDecision::Approve) => ApprovalStatus::Approved,
        Some(ApprovalDecision::Deny) => ApprovalStatus::Denied,
        None => ApprovalStatus::Expired,
    };
    approval.resolved_at = Some(resolved_at);
    approval.decision = decision;
    approval
}

fn publish_approval_resolved(events: &ProviderEventSink, approval: Approval) {
    publish_event(
        events,
        ProtocolEvent::ApprovalResolved {
            protocol_version: PROTOCOL_VERSION,
            event_sequence: 0,
            payload: ApprovalResolvedEvent { approval },
        },
    );
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

fn validate_provider_id(provider_id: &str) -> Result<(), ProtocolError> {
    if provider_id == CODEX_PROVIDER_ID {
        return Ok(());
    }
    Err(ProtocolError {
        code: "invalid_request".to_string(),
        message: "request provider does not match Codex Desktop".to_string(),
        retryable: false,
        details: None,
    })
}

fn stale_action_error(message: &str) -> ProtocolError {
    ProtocolError {
        code: "action_stale".to_string(),
        message: message.to_string(),
        retryable: false,
        details: None,
    }
}

fn desktop_error(error: DesktopIpcError) -> ProtocolError {
    let code = match &error {
        DesktopIpcError::Stale(_) => "action_stale",
        DesktopIpcError::OutcomeUnknown(_) => "desktop_ipc_outcome_unknown",
        DesktopIpcError::PartialFailure(_) => "desktop_ipc_partial_failure",
        DesktopIpcError::Remote(_) => "desktop_ipc_rejected",
        DesktopIpcError::Unsupported(_) => "capability_unsupported",
        _ => "desktop_ipc_unavailable",
    };
    let retryable = !matches!(
        error,
        DesktopIpcError::Protocol(_)
            | DesktopIpcError::UnsafeSocket(_)
            | DesktopIpcError::Unsupported(_)
            | DesktopIpcError::Stale(_)
            | DesktopIpcError::OutcomeUnknown(_)
            | DesktopIpcError::PartialFailure(_)
            | DesktopIpcError::Remote(_)
            | DesktopIpcError::Shutdown
    );
    ProtocolError {
        code: code.to_string(),
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
        DesktopIpcError::Stale(_) => {
            "Codex Desktop state changed before this action could be dispatched".to_string()
        }
        DesktopIpcError::OutcomeUnknown(_) => {
            "Codex Desktop did not confirm the action outcome; it will not be replayed"
                .to_string()
        }
        DesktopIpcError::PartialFailure(_) => {
            "Codex Desktop accepted the interruption, but could not pause the related goal"
                .to_string()
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
                "{method} is unavailable through the current Codex Desktop follower protocol"
            ),
            retryable: false,
            details: None,
        })
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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

    fn thread_snapshot(conversation_id: &str, revision: u64) -> ThreadSnapshot {
        ThreadSnapshot {
            conversation_id: conversation_id.to_string(),
            owner_client_id: "owner-one".to_string(),
            revision,
            state: json!({
                "id": conversation_id,
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
        }
    }

    fn mapped_thread(revision: u64) -> MappedThread {
        map_thread(&thread_snapshot("thread-one", revision))
    }

    fn mapped_thread_with_approval(revision: u64, pending: bool) -> MappedThread {
        let requests = if pending {
            json!([{
                "id": 41,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-approval",
                    "turnId": "turn-active",
                    "itemId": "command-item",
                    "reason": "Run a controlled check",
                },
            }])
        } else {
            json!([])
        };
        map_thread(&ThreadSnapshot {
            conversation_id: "thread-approval".to_string(),
            owner_client_id: "owner-one".to_string(),
            revision,
            state: json!({
                "id": "thread-approval",
                "title": "Approval task",
                "createdAt": 1_700_000_000_000_u64,
                "updatedAt": 1_700_000_000_000_u64 + revision,
                "threadRuntimeStatus": { "type": "active" },
                "currentPermissions": {
                    "sandboxPolicy": { "type": "workspaceWrite" }
                },
                "turns": [{
                    "turnId": "turn-active",
                    "status": "inProgress",
                    "turnStartedAtMs": 1_700_000_000_000_u64,
                }],
                "requests": requests,
            }),
        })
    }

    fn ready_provider_state() -> Arc<Mutex<CodexProviderState>> {
        Arc::new(Mutex::new(CodexProviderState {
            status: ProviderStatus::Ready,
            unavailable_reason: None,
            connection_generation: 1,
            follower_epoch: Some(1),
            threads: BTreeMap::new(),
            approval_intents: BTreeMap::new(),
            bootstrap_workers: HashSet::new(),
            retired: false,
        }))
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
        let state = ready_provider_state();

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

    #[test]
    fn approval_ack_is_deduplicated_and_resolves_only_after_authoritative_removal() {
        let gateway = Gateway::default();
        let events = gateway.event_sink();
        let state = ready_provider_state();
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(1, true),
            PublicationKind::Baseline,
        );
        let baseline = gateway.replay_events(None).unwrap();
        assert_eq!(
            baseline
                .iter()
                .filter(|event| matches!(event, ProtocolEvent::ApprovalRequested { .. }))
                .count(),
            1
        );
        assert!(!baseline
            .iter()
            .any(|event| matches!(event, ProtocolEvent::ApprovalResolved { .. })));
        let approval_id = lock(&state).threads["thread-approval"].approvals[0]
            .approval
            .id
            .clone();
        begin_approval_resolution(&state, &approval_id, ApprovalDecision::Approve).unwrap();
        assert_eq!(
            begin_approval_resolution(&state, &approval_id, ApprovalDecision::Approve)
                .unwrap_err()
                .code,
            "action_in_flight"
        );
        assert!(acknowledge_approval_resolution(&state, &approval_id).is_none());
        assert!(!gateway
            .replay_events(None)
            .unwrap()
            .iter()
            .any(|event| matches!(event, ProtocolEvent::ApprovalResolved { .. })));

        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(2, false),
            PublicationKind::Live,
        );
        let resolved = gateway
            .replay_events(None)
            .unwrap()
            .into_iter()
            .find_map(|event| match event {
                ProtocolEvent::ApprovalResolved { payload, .. } => Some(payload.approval),
                _ => None,
            })
            .unwrap();
        assert_eq!(resolved.id, approval_id);
        assert_eq!(resolved.status, ApprovalStatus::Approved);
        assert_eq!(resolved.decision, Some(ApprovalDecision::Approve));
    }

    #[test]
    fn authoritative_removal_during_in_flight_waits_for_ack_and_external_removal_expires() {
        let gateway = Gateway::default();
        let events = gateway.event_sink();
        let state = ready_provider_state();
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(1, true),
            PublicationKind::Baseline,
        );
        let approval_id = lock(&state).threads["thread-approval"].approvals[0]
            .approval
            .id
            .clone();
        begin_approval_resolution(&state, &approval_id, ApprovalDecision::Deny).unwrap();
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(2, false),
            PublicationKind::Live,
        );
        assert!(!gateway
            .replay_events(None)
            .unwrap()
            .iter()
            .any(|event| matches!(event, ProtocolEvent::ApprovalResolved { .. })));
        let resolved = acknowledge_approval_resolution(&state, &approval_id).unwrap();
        assert_eq!(resolved.status, ApprovalStatus::Denied);
        assert_eq!(resolved.decision, Some(ApprovalDecision::Deny));

        let external_gateway = Gateway::default();
        let external_events = external_gateway.event_sink();
        let external_state = ready_provider_state();
        publish_mapped_thread(
            &external_state,
            &external_events,
            mapped_thread_with_approval(1, true),
            PublicationKind::Baseline,
        );
        publish_mapped_thread(
            &external_state,
            &external_events,
            mapped_thread_with_approval(2, false),
            PublicationKind::Live,
        );
        let expired = external_gateway
            .replay_events(None)
            .unwrap()
            .into_iter()
            .find_map(|event| match event {
                ProtocolEvent::ApprovalResolved { payload, .. } => Some(payload.approval),
                _ => None,
            })
            .unwrap();
        assert_eq!(expired.status, ApprovalStatus::Expired);
        assert_eq!(expired.decision, None);
    }

    #[test]
    fn conversation_snapshot_hydrates_already_pending_approvals() {
        let initial_gateway = Gateway::default();
        let state = ready_provider_state();
        publish_mapped_thread(
            &state,
            &initial_gateway.event_sink(),
            mapped_thread_with_approval(1, true),
            PublicationKind::Baseline,
        );
        let hydration_gateway = Gateway::default();
        publish_pending_approval_hydration(
            &state,
            &hydration_gateway.event_sink(),
            &HashSet::from(["thread-approval".to_string()]),
        );
        assert!(matches!(
            hydration_gateway.replay_events(None).unwrap().as_slice(),
            [ProtocolEvent::ApprovalRequested { .. }]
        ));
    }

    #[test]
    fn unknown_approval_outcome_survives_rebootstrap_and_blocks_duplicate_resolution() {
        let gateway = Gateway::default();
        let events = gateway.event_sink();
        let state = ready_provider_state();
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(1, true),
            PublicationKind::Baseline,
        );
        let approval_id = lock(&state).threads["thread-approval"].approvals[0]
            .approval
            .id
            .clone();
        begin_approval_resolution(&state, &approval_id, ApprovalDecision::Approve).unwrap();
        transition_connection(
            &state,
            &events,
            &DesktopConnectionSnapshot {
                status: DesktopConnectionStatus::Unavailable,
                client_id: None,
                generation: 2,
                error: Some(DesktopIpcError::Timeout("approval".to_string())),
            },
        );
        transition_connection(
            &state,
            &events,
            &DesktopConnectionSnapshot {
                status: DesktopConnectionStatus::Ready,
                client_id: Some("codepet-new".to_string()),
                generation: 3,
                error: None,
            },
        );
        assert!(lock(&state).approval_intents.contains_key(&approval_id));
        assert!(mark_approval_outcome_unknown(&state, &approval_id).is_none());
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(2, true),
            PublicationKind::Baseline,
        );
        assert_eq!(
            begin_approval_resolution(&state, &approval_id, ApprovalDecision::Approve)
                .unwrap_err()
                .code,
            "action_outcome_unknown"
        );
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(3, false),
            PublicationKind::Live,
        );
        let resolved = gateway
            .replay_events(None)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                ProtocolEvent::ApprovalResolved { payload, .. }
                    if payload.approval.id == approval_id => Some(payload.approval),
                _ => None,
            })
            .last()
            .unwrap();
        assert_eq!(resolved.status, ApprovalStatus::Expired);
        assert_eq!(resolved.decision, None);
    }

    #[test]
    fn approval_that_remains_native_pending_but_becomes_unsafe_expires() {
        let gateway = Gateway::default();
        let events = gateway.event_sink();
        let state = ready_provider_state();
        publish_mapped_thread(
            &state,
            &events,
            mapped_thread_with_approval(1, true),
            PublicationKind::Baseline,
        );
        let approval_id = lock(&state).threads["thread-approval"].approvals[0]
            .approval
            .id
            .clone();
        begin_approval_resolution(&state, &approval_id, ApprovalDecision::Approve).unwrap();
        let mut no_longer_actionable = mapped_thread_with_approval(2, true);
        no_longer_actionable.approvals.clear();
        publish_mapped_thread(
            &state,
            &events,
            no_longer_actionable,
            PublicationKind::Live,
        );

        let resolved = gateway
            .replay_events(None)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                ProtocolEvent::ApprovalResolved { payload, .. }
                    if payload.approval.id == approval_id => Some(payload.approval),
                _ => None,
            })
            .last()
            .unwrap();
        assert_eq!(resolved.status, ApprovalStatus::Expired);
        assert_eq!(resolved.decision, None);
        assert!(!lock(&state).approval_intents.contains_key(&approval_id));
    }
}
