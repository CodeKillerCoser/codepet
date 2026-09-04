use crate::client::{
    CodexAppServerSession, CodexRequestOutcome, THREAD_TURNS_PAGE_LIMIT,
};
use crate::mapper::{parse_permission_level, CodexProtocolMapper};
use crate::protocol::{
    approval_generation, approval_resource_id, CodexAppServerError, CodexApprovalRequest,
    CodexConversationSnapshot, CodexIncoming, CodexNotification, CodexProjectCreateRequest,
    CodexProjectRoot, CodexProjectUpdateRequest, CodexThreadListRequest, CodexThreadStartRequest,
    CodexThreadItem, CodexTurn, CodexTurnItemsView, CodexTurnStartRequest, CodexTurnStatus,
    CodexTurnSteerRequest,
    CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalRequestedEvent, ApprovalResolveRequest, ApprovalResolveResponse,
    ConversationAcquireInteractionRequest, ConversationAcquireInteractionResponse,
    ConversationCreateRequest, ConversationProjectFilter, ConversationUpsertedEvent,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ConversationSearchRequest,
    ConversationSearchResponse, InstanceCapabilitiesRequest, InstanceCapabilitiesResponse,
    InstanceCreateRequest, InstanceCreateResponse,
    InstanceDestroyRequest, InstanceDestroyResponse, InstanceStartRequest,
    InstanceStartResponse, InstanceStatus, InstanceStatusChangedEvent, InstanceStopRequest,
    InstanceStopResponse, PageInfo, ProjectCreateRequest, ProjectCreateResponse,
    ProjectDeleteRequest, ProjectDeleteResponse, ProjectGetRequest, ProjectGetResponse,
    ProjectListRequest, ProjectListResponse, ProjectUpdateRequest, ProjectUpdateResponse,
    ProtocolError, ProtocolEvent, ProtocolFuture, Provider, ProviderCapabilities,
    ProviderAuthentication, ProviderAuthenticationStatus, ProviderCapability,
    ProviderDescribeRequest, ProviderDescribeResponse, ProviderEventSink, ProviderUsage,
    ProviderUsageDetail,
    FlatModelCatalogKind, FlatModelSelection, HarnessDescriptor, ModelCatalog, ModelSelection, ProviderApproval,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance,
    ProviderInstanceRoute, ProviderPluginDescriptor, ProviderShutdownRequest,
    ProviderShutdownResponse, RoutedResourceId, RuntimeCandidate, RuntimeGetInstalledRequest,
    RuntimeGetInstalledResponse, RuntimeInstallation, RuntimeSelectRequest, RuntimeSelectResponse,
    TurnInterruptRequest, TurnInterruptResponse,
    TurnSelection, TurnStartRequest, TurnStartResponse, TurnSteerRequest, TurnSteerResponse,
    VersionRange, PROTOCOL_VERSION, fit_single_turn_conversation_history,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_EXECUTION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
static NEXT_INSTANCE_SESSION: AtomicU64 = AtomicU64::new(1);
static NEXT_MANAGED_WORKTREE: AtomicU64 = AtomicU64::new(1);
const MAX_THREAD_TURN_PAGES: usize = 10_000;
const MAX_FILTERED_THREAD_PAGES: usize = 10_000;
const CODEX_LIST_PAGE_LIMIT: u32 = 100;
const FILTERED_CONVERSATION_CURSOR_PREFIX: &str = "codepet-codex-membership-v1:";
const DEFAULT_CONVERSATION_GET_TURN_LIMIT: u64 = 40;
const MAX_CONVERSATION_GET_TURN_LIMIT: u64 = 100;
const INTERACTION_LEASE_DURATION: Duration = Duration::from_secs(30);
const INTERACTION_REAPER_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct CodexInstanceSettings {
    app_server_executable: PathBuf,
    app_server_args: Vec<String>,
}

#[doc(hidden)]
pub trait ExecutionLifecycleHook: Send + Sync + 'static {
    fn after_handle_acquired(&self, _conversation_id: &str, _operation: &str) {}

    fn before_resume_linearization(&self, _conversation_id: &str) {}

    fn after_execution_cancelled(&self, _conversation_id: &str) {}

    fn interaction_lease_duration(&self) -> Duration {
        INTERACTION_LEASE_DURATION
    }
}

struct NoopExecutionLifecycleHook;

impl ExecutionLifecycleHook for NoopExecutionLifecycleHook {}

struct InstanceMutable {
    destroyed: bool,
    cleanup_in_progress: bool,
    status: InstanceStatus,
    capabilities: ProviderCapabilities,
    harness: HarnessDescriptor,
    authentication: Option<ProviderAuthentication>,
    usage: Option<ProviderUsage>,
    lifecycle_generation: u64,
    sessions: HashMap<u64, Arc<InstanceSessionSlot>>,
    observer_session_id: Option<u64>,
    observer_generation: Option<String>,
    executions: HashMap<String, Arc<ExecutionSlot>>,
    pending_approvals: HashMap<String, PendingApproval>,
    approval_history: Vec<ObservedApproval>,
    pending_materialization: HashMap<String, CodexConversationSnapshot>,
}

enum InstanceSessionState {
    Pending,
    Spawning,
    Spawned(CodexAppServerSession),
    Finished,
}

struct InstanceSessionSlot {
    id: u64,
    lifecycle_generation: u64,
    cancelled: AtomicBool,
    send_gate: Mutex<()>,
    state: Mutex<InstanceSessionState>,
    changed: Condvar,
}

impl InstanceSessionSlot {
    fn new(lifecycle_generation: u64) -> Self {
        Self {
            id: NEXT_INSTANCE_SESSION.fetch_add(1, Ordering::SeqCst),
            lifecycle_generation,
            cancelled: AtomicBool::new(false),
            send_gate: Mutex::new(()),
            state: Mutex::new(InstanceSessionState::Pending),
            changed: Condvar::new(),
        }
    }

    fn begin_spawn(&self) -> bool {
        let _send_gate = lock(&self.send_gate);
        if self.cancelled.load(Ordering::SeqCst) {
            return false;
        }
        let mut state = lock(&self.state);
        if !matches!(*state, InstanceSessionState::Pending) {
            return false;
        }
        *state = InstanceSessionState::Spawning;
        true
    }

    fn register_spawned(&self, session: CodexAppServerSession) -> bool {
        let mut state = lock(&self.state);
        if !matches!(*state, InstanceSessionState::Spawning) {
            return false;
        }
        *state = InstanceSessionState::Spawned(session);
        self.changed.notify_all();
        !self.cancelled.load(Ordering::SeqCst)
    }

    fn session(&self) -> Option<CodexAppServerSession> {
        match &*lock(&self.state) {
            InstanceSessionState::Spawned(session) => Some(session.clone()),
            InstanceSessionState::Pending
            | InstanceSessionState::Spawning
            | InstanceSessionState::Finished => None,
        }
    }

    fn cancel_and_take(&self) -> Option<CodexAppServerSession> {
        let _send_gate = lock(&self.send_gate);
        self.cancelled.store(true, Ordering::SeqCst);
        let mut state = lock(&self.state);
        while matches!(*state, InstanceSessionState::Spawning) {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let previous = std::mem::replace(&mut *state, InstanceSessionState::Finished);
        self.changed.notify_all();
        match previous {
            InstanceSessionState::Spawned(session) => Some(session),
            InstanceSessionState::Pending
            | InstanceSessionState::Spawning
            | InstanceSessionState::Finished => None,
        }
    }

    fn finish(&self) {
        let mut state = lock(&self.state);
        *state = InstanceSessionState::Finished;
        self.changed.notify_all();
    }
}

#[derive(Clone)]
struct ExecutionSession {
    session: CodexAppServerSession,
    generation: String,
    active_turn_id: Option<String>,
    interaction_expires_at: Option<Instant>,
}

enum ExecutionSlotState {
    Creating(Option<CodexAppServerSession>),
    Ready(ExecutionSession),
    Closing(ExecutionSession),
    Failed(ProtocolError),
    Closed,
}

struct ExecutionSlot {
    attempt_generation: u64,
    cancelled: AtomicBool,
    resume_send: Mutex<()>,
    state: Mutex<ExecutionSlotState>,
    changed: Condvar,
    operation: Mutex<()>,
}

#[derive(Clone)]
struct ExecutionHandle {
    slot: Arc<ExecutionSlot>,
    session: CodexAppServerSession,
    generation: String,
}

enum ExecutionStep<T> {
    Complete(T),
    Retry,
}

impl ExecutionSlot {
    fn new() -> Self {
        Self {
            attempt_generation: NEXT_EXECUTION_ATTEMPT.fetch_add(1, Ordering::SeqCst),
            cancelled: AtomicBool::new(false),
            resume_send: Mutex::new(()),
            state: Mutex::new(ExecutionSlotState::Creating(None)),
            changed: Condvar::new(),
            operation: Mutex::new(()),
        }
    }

    fn wait_ready(self: &Arc<Self>) -> Result<Option<ExecutionHandle>, ProtocolError> {
        let mut state = lock(&self.state);
        loop {
            match &*state {
                ExecutionSlotState::Creating(_) | ExecutionSlotState::Closing(_) => {
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                ExecutionSlotState::Ready(execution) => {
                    return Ok(Some(ExecutionHandle {
                        slot: self.clone(),
                        session: execution.session.clone(),
                        generation: execution.generation.clone(),
                    }));
                }
                ExecutionSlotState::Failed(error) => return Err(error.clone()),
                ExecutionSlotState::Closed => return Ok(None),
            }
        }
    }

    fn set_starting_session(&self, session: CodexAppServerSession) -> bool {
        if self.cancelled.load(Ordering::SeqCst) {
            return false;
        }
        let mut state = lock(&self.state);
        let ExecutionSlotState::Creating(starting) = &mut *state else {
            return false;
        };
        if starting.is_some() || self.cancelled.load(Ordering::SeqCst) {
            return false;
        }
        *starting = Some(session);
        true
    }

    fn set_ready(&self, execution: ExecutionSession) -> bool {
        if self.cancelled.load(Ordering::SeqCst) {
            return false;
        }
        let mut state = lock(&self.state);
        if !matches!(
            &*state,
            ExecutionSlotState::Creating(Some(session))
                if session.generation() == execution.generation
        ) || self.cancelled.load(Ordering::SeqCst)
        {
            return false;
        }
        *state = ExecutionSlotState::Ready(execution);
        self.changed.notify_all();
        true
    }

    fn fail(&self, error: ProtocolError) {
        let mut state = lock(&self.state);
        if matches!(*state, ExecutionSlotState::Creating(_)) {
            *state = ExecutionSlotState::Failed(error);
            self.changed.notify_all();
        }
    }

    fn force_close(&self) -> Option<CodexAppServerSession> {
        let _resume_send = lock(&self.resume_send);
        self.cancelled.store(true, Ordering::SeqCst);
        let mut state = lock(&self.state);
        let previous = std::mem::replace(&mut *state, ExecutionSlotState::Closed);
        self.changed.notify_all();
        match previous {
            ExecutionSlotState::Ready(execution) => Some(execution.session),
            ExecutionSlotState::Closing(execution) => Some(execution.session),
            ExecutionSlotState::Creating(session) => session,
            ExecutionSlotState::Failed(_) | ExecutionSlotState::Closed => None,
        }
    }

    fn creation_is_current(&self, attempt_generation: u64, session_generation: &str) -> bool {
        self.attempt_generation == attempt_generation
            && !self.cancelled.load(Ordering::SeqCst)
            && matches!(
                &*lock(&self.state),
                ExecutionSlotState::Creating(Some(session))
                    if session.generation() == session_generation
            )
    }

    fn begin_closing(&self, generation: &str) -> bool {
        let mut state = lock(&self.state);
        let previous = std::mem::replace(&mut *state, ExecutionSlotState::Closed);
        match previous {
            ExecutionSlotState::Ready(execution) if execution.generation == generation => {
                self.cancelled.store(true, Ordering::SeqCst);
                *state = ExecutionSlotState::Closing(execution);
                true
            }
            previous => {
                *state = previous;
                false
            }
        }
    }

    fn closing_session(&self, generation: &str) -> Option<CodexAppServerSession> {
        match &*lock(&self.state) {
            ExecutionSlotState::Closing(execution) if execution.generation == generation => {
                Some(execution.session.clone())
            }
            _ => None,
        }
    }

    fn finish_closing(&self, generation: &str) {
        let mut state = lock(&self.state);
        if matches!(
            &*state,
            ExecutionSlotState::Closing(execution) if execution.generation == generation
        ) {
            *state = ExecutionSlotState::Closed;
            self.changed.notify_all();
        }
    }

    fn matches_generation(&self, generation: &str) -> bool {
        matches!(
            &*lock(&self.state),
            ExecutionSlotState::Ready(execution) if execution.generation == generation
        )
    }

    fn mark_active_turn(&self, generation: &str, turn_id: &str) -> bool {
        let mut state = lock(&self.state);
        let ExecutionSlotState::Ready(execution) = &mut *state else {
            return false;
        };
        if execution.generation != generation {
            return false;
        }
        execution.active_turn_id = Some(turn_id.to_string());
        true
    }

    fn has_active_turn(&self, generation: &str) -> bool {
        matches!(
            &*lock(&self.state),
            ExecutionSlotState::Ready(execution)
                if execution.generation == generation && execution.active_turn_id.is_some()
        )
    }

    fn renew_interaction(&self, generation: &str, expires_at: Instant) -> bool {
        let mut state = lock(&self.state);
        let ExecutionSlotState::Ready(execution) = &mut *state else {
            return false;
        };
        if execution.generation != generation {
            return false;
        }
        execution.interaction_expires_at = Some(expires_at);
        true
    }

    fn finish_active_turn_and_should_close(
        &self,
        generation: &str,
        completed_turn_id: Option<&str>,
        now: Instant,
    ) -> Option<bool> {
        let mut state = lock(&self.state);
        let ExecutionSlotState::Ready(execution) = &mut *state else {
            return None;
        };
        if execution.generation != generation {
            return None;
        }
        if completed_turn_id.is_some_and(|completed_turn_id| {
            execution
                .active_turn_id
                .as_deref()
                .is_some_and(|active_turn_id| active_turn_id != completed_turn_id)
        }) {
            return Some(false);
        }
        execution.active_turn_id = None;
        Some(
            execution
                .interaction_expires_at
                .is_none_or(|expires_at| expires_at <= now),
        )
    }

    fn interaction_expired_while_idle(&self, generation: &str, now: Instant) -> bool {
        matches!(
            &*lock(&self.state),
            ExecutionSlotState::Ready(execution)
                if execution.generation == generation
                    && execution.active_turn_id.is_none()
                    && execution.interaction_expires_at.is_some_and(|expires_at| expires_at <= now)
        )
    }

    fn idle_without_valid_interaction(&self, generation: &str, now: Instant) -> bool {
        matches!(
            &*lock(&self.state),
            ExecutionSlotState::Ready(execution)
                if execution.generation == generation
                    && execution.active_turn_id.is_none()
                    && execution
                        .interaction_expires_at
                        .is_none_or(|expires_at| expires_at <= now)
        )
    }
}

#[derive(Clone)]
struct PendingApproval {
    request: CodexApprovalRequest,
    approval: ProviderApproval,
}

#[derive(Clone)]
struct ObservedApproval {
    item_id: String,
    approval: ProviderApproval,
}

struct CodexInstanceRuntime {
    route: ProviderInstanceRoute,
    instance_kind: String,
    display_name: String,
    settings: CodexInstanceSettings,
    lifecycle_transition: Mutex<()>,
    lifecycle_changed: Condvar,
    mutable: Mutex<InstanceMutable>,
    mapper: Mutex<CodexProtocolMapper>,
    events: Arc<dyn ProviderEventSink>,
    lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
}

impl CodexInstanceRuntime {
    fn new(
        request: InstanceCreateRequest,
        settings: CodexInstanceSettings,
        events: Arc<dyn ProviderEventSink>,
        lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
    ) -> Self {
        let executable_path = settings.app_server_executable.to_string_lossy().into_owned();
        Self {
            route: request.route.clone(),
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            lifecycle_transition: Mutex::new(()),
            lifecycle_changed: Condvar::new(),
            mutable: Mutex::new(InstanceMutable {
                destroyed: false,
                cleanup_in_progress: false,
                status: InstanceStatus::Created,
                capabilities: CodexProtocolMapper::unavailable_capabilities(
                    "codex-not-ready".to_string(),
                ),
                harness: HarnessDescriptor {
                    id: CODEX_INSTANCE_KIND.to_string(),
                    display_name: "Codex".to_string(),
                    version: None,
                    executable_path: Some(executable_path),
                },
                authentication: None,
                usage: None,
                lifecycle_generation: 0,
                sessions: HashMap::new(),
                observer_session_id: None,
                observer_generation: None,
                executions: HashMap::new(),
                pending_approvals: HashMap::new(),
                approval_history: Vec::new(),
                pending_materialization: HashMap::new(),
            }),
            mapper: Mutex::new(CodexProtocolMapper::new(request.route)),
            events,
            lifecycle_hook,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        let mutable = lock(&self.mutable);
        lock(&self.mapper).instance(
            CODEX_PLUGIN_ID.to_string(),
            self.instance_kind.clone(),
            self.display_name.clone(),
            mutable.harness.clone(),
            mutable.status,
            mutable.authentication.clone(),
            mutable.usage.clone(),
            mutable.capabilities.clone(),
        )
    }

    fn publish_status_change(
        &self,
        previous_status: InstanceStatus,
    ) -> Result<ProviderInstance, ProtocolError> {
        let instance = self.snapshot();
        self.events.publish(ProtocolEvent::EventInstanceStatusChanged {
            jsonrpc: "2.0".to_string(),
            params: InstanceStatusChangedEvent {
                instance: instance.clone(),
                previous_status: Some(previous_status),
            },
        })?;
        Ok(instance)
    }

    fn session_is_current(&self, slot: &Arc<InstanceSessionSlot>) -> bool {
        let mutable = lock(&self.mutable);
        mutable.lifecycle_generation == slot.lifecycle_generation
            && mutable
                .sessions
                .get(&slot.id)
                .is_some_and(|current| Arc::ptr_eq(current, slot))
    }

    fn unregister_session(&self, slot: &Arc<InstanceSessionSlot>) {
        slot.finish();
        let mut mutable = lock(&self.mutable);
        if mutable
            .sessions
            .get(&slot.id)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            mutable.sessions.remove(&slot.id);
        }
        if mutable.observer_session_id == Some(slot.id) {
            mutable.observer_session_id = None;
            mutable.observer_generation = None;
        }
    }

    fn cancel_sessions(slots: Vec<Arc<InstanceSessionSlot>>) -> Result<(), CodexAppServerError> {
        let sessions = slots
            .into_iter()
            .filter_map(|slot| slot.cancel_and_take())
            .collect::<Vec<_>>();
        shutdown_sessions(sessions)
    }

    fn mark_start_failed(
        &self,
        slot: &Arc<InstanceSessionSlot>,
    ) -> Result<bool, ProtocolError> {
        let _transition = lock(&self.lifecycle_transition);
        let previous_status = {
            let mut mutable = lock(&self.mutable);
            let current = mutable.lifecycle_generation == slot.lifecycle_generation
                && mutable.status == InstanceStatus::Starting
                && mutable
                    .sessions
                    .get(&slot.id)
                    .is_some_and(|current| Arc::ptr_eq(current, slot));
            if !current {
                drop(mutable);
                slot.finish();
                return Ok(false);
            }
            mutable.sessions.remove(&slot.id);
            mutable.observer_session_id = None;
            mutable.observer_generation = None;
            let previous = mutable.status;
            mutable.status = InstanceStatus::Error;
            self.lifecycle_changed.notify_all();
            previous
        };
        slot.finish();
        self.publish_status_change(previous_status)?;
        Ok(true)
    }

    async fn stop(self: &Arc<Self>) -> Result<ProviderInstance, ProtocolError> {
        enum StopAction {
            Return(Box<ProviderInstance>),
            Wait,
            Stop {
                lifecycle_generation: u64,
                sessions: Vec<Arc<InstanceSessionSlot>>,
                executions: Vec<(String, Arc<ExecutionSlot>)>,
                status_event_error: Option<ProtocolError>,
            },
        }

        loop {
        let action = {
            let _transition = lock(&self.lifecycle_transition);
            let mut mutable = lock(&self.mutable);
            if mutable.cleanup_in_progress {
                StopAction::Wait
            } else if mutable.destroyed {
                drop(mutable);
                StopAction::Return(Box::new(self.snapshot()))
            } else {
            match mutable.status {
                InstanceStatus::Stopped => {
                    drop(mutable);
                    StopAction::Return(Box::new(self.snapshot()))
                }
                InstanceStatus::Stopping => StopAction::Wait,
                InstanceStatus::Created => {
                    let previous = mutable.status;
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
                    mutable.status = InstanceStatus::Stopped;
                    self.lifecycle_changed.notify_all();
                    drop(mutable);
                    StopAction::Return(Box::new(self.publish_status_change(previous)?))
                }
                _ => {
                    let previous = mutable.status;
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
                    let lifecycle_generation = mutable.lifecycle_generation;
                    mutable.status = InstanceStatus::Stopping;
                    mutable.observer_session_id = None;
                    mutable.observer_generation = None;
                    let sessions = mutable.sessions.drain().map(|(_, slot)| slot).collect();
                    let executions = mutable.executions.drain().collect();
                    mutable.pending_approvals.clear();
                    mutable.approval_history.clear();
                    mutable.pending_materialization.clear();
                    drop(mutable);
                    let status_event_error = self.publish_status_change(previous).err();
                    StopAction::Stop {
                        lifecycle_generation,
                        sessions,
                        executions,
                        status_event_error,
                    }
                }
            }
            }
        };

        match action {
            StopAction::Return(instance) => return Ok(*instance),
            StopAction::Wait => {
                let runtime = self.clone();
                tokio::task::spawn_blocking(move || {
                    let mut mutable = lock(&runtime.mutable);
                    while mutable.status == InstanceStatus::Stopping
                        || mutable.cleanup_in_progress
                    {
                        mutable = runtime
                            .lifecycle_changed
                            .wait(mutable)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                })
                .await
                .map_err(provider_task_error)?;
                continue;
            }
            StopAction::Stop {
                lifecycle_generation,
                sessions,
                executions,
                status_event_error,
            } => {
                let mut execution_sessions = Vec::new();
                for (conversation_id, slot) in executions {
                    let session = slot.force_close();
                    self.lifecycle_hook
                        .after_execution_cancelled(&conversation_id);
                    execution_sessions.extend(session);
                }
                let shutdown_result = tokio::task::spawn_blocking(move || {
                    let lifecycle_result = Self::cancel_sessions(sessions);
                    let execution_result = shutdown_sessions(execution_sessions);
                    lifecycle_result.and(execution_result)
                })
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error);

                let (instance, stopped_event_error) = {
                    let _transition = lock(&self.lifecycle_transition);
                    let previous = {
                        let mut mutable = lock(&self.mutable);
                        if mutable.status == InstanceStatus::Stopping
                            && mutable.lifecycle_generation == lifecycle_generation
                        {
                            let previous = mutable.status;
                            mutable.status = InstanceStatus::Stopped;
                            mutable.pending_approvals.clear();
                            mutable.approval_history.clear();
                            mutable.pending_materialization.clear();
                            self.lifecycle_changed.notify_all();
                            Some(previous)
                        } else {
                            None
                        }
                    };
                    match previous {
                        Some(previous) => match self.publish_status_change(previous) {
                            Ok(instance) => (instance, None),
                            Err(error) => (self.snapshot(), Some(error)),
                        },
                        None => (self.snapshot(), None),
                    }
                };
                if let Some(error) = status_event_error.or(stopped_event_error) {
                    return Err(error);
                }
                shutdown_result?;
                return Ok(instance);
            }
        }
        }
    }

    fn ready_observer(&self) -> Result<CodexAppServerSession, ProtocolError> {
        let mutable = lock(&self.mutable);
        if mutable.status != InstanceStatus::Ready {
            return Err(protocol_error(
                "provider_unavailable",
                format!(
                    "Codex Provider instance {} is not ready",
                    self.route.provider_instance_id
                ),
                true,
            ));
        }
        let session = mutable
            .observer_session_id
            .and_then(|id| mutable.sessions.get(&id))
            .and_then(|slot| slot.session());
        session.ok_or_else(|| {
            protocol_error(
                "provider_unavailable",
                "Codex observer App Server session is unavailable".to_string(),
                true,
            )
        })
    }

    fn acquire_execution(
        self: &Arc<Self>,
        conversation_id: &str,
    ) -> Result<ExecutionHandle, ProtocolError> {
        loop {
            let (slot, creator) = {
                let mut mutable = lock(&self.mutable);
                let observer_ready = mutable
                    .observer_session_id
                    .and_then(|id| mutable.sessions.get(&id))
                    .and_then(|slot| slot.session())
                    .is_some();
                if mutable.status != InstanceStatus::Ready || !observer_ready {
                    return Err(protocol_error(
                        "provider_unavailable",
                        format!(
                            "Codex Provider instance {} is not ready",
                            self.route.provider_instance_id
                        ),
                        true,
                    ));
                }
                match mutable.executions.get(conversation_id) {
                    Some(slot) => (slot.clone(), false),
                    None => {
                        let slot = Arc::new(ExecutionSlot::new());
                        mutable
                            .executions
                            .insert(conversation_id.to_string(), slot.clone());
                        (slot, true)
                    }
                }
            };
            if !creator {
                if let Some(handle) = slot.wait_ready()? {
                    return Ok(handle);
                }
                continue;
            }

            let attempt_generation = slot.attempt_generation;
            let executable = self.settings.app_server_executable.clone();
            let args = self.settings.app_server_args.clone();
            let session = match CodexAppServerSession::spawn_uninitialized(&executable, &args) {
                Ok(session) => session,
                Err(error) => {
                    let error = CodexProtocolMapper::error(error);
                    self.fail_execution_creation(conversation_id, &slot, error.clone());
                    return Err(error);
                }
            };
            let registered = {
                let mutable = lock(&self.mutable);
                mutable.status == InstanceStatus::Ready
                    && mutable
                        .executions
                        .get(conversation_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &slot))
                    && slot.attempt_generation == attempt_generation
                    && slot.set_starting_session(session.clone())
            };
            if !registered {
                let _ = session.shutdown();
                let error = execution_start_cancelled_error();
                self.fail_execution_creation(conversation_id, &slot, error.clone());
                return Err(error);
            }
            if let Err(error) = session.initialize() {
                let error = CodexProtocolMapper::error(error);
                let _ = session.shutdown();
                self.fail_execution_creation(conversation_id, &slot, error.clone());
                return Err(error);
            }
            let generation = session.generation().to_string();
            if !self.execution_creation_is_current(
                conversation_id,
                &slot,
                attempt_generation,
                &generation,
            ) {
                let _ = session.shutdown();
                let error = execution_start_cancelled_error();
                self.fail_execution_creation(conversation_id, &slot, error.clone());
                return Err(error);
            }
            let incoming = match session.subscribe() {
                Ok(incoming) => incoming,
                Err(error) => {
                    let error = CodexProtocolMapper::error(error);
                    let _ = session.shutdown();
                    self.fail_execution_creation(conversation_id, &slot, error.clone());
                    return Err(error);
                }
            };
            if !self.execution_creation_is_current(
                conversation_id,
                &slot,
                attempt_generation,
                &generation,
            ) {
                let _ = session.shutdown();
                let error = execution_start_cancelled_error();
                self.fail_execution_creation(conversation_id, &slot, error.clone());
                return Err(error);
            }
            self.lifecycle_hook
                .before_resume_linearization(conversation_id);
            let resume_outcome = session.ensure_thread_loaded_outcome_with_sender(
                conversation_id,
                |message| {
                    let _resume_send = lock(&slot.resume_send);
                    if !self.execution_creation_is_current(
                        conversation_id,
                        &slot,
                        attempt_generation,
                        &generation,
                    ) {
                        return CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown);
                    }
                    session.write_prepared_request(message)
                },
            );
            if let outcome @ (CodexRequestOutcome::NotSent(_)
            | CodexRequestOutcome::ExplicitRpcReject(_)
            | CodexRequestOutcome::SentOutcomeUnknown(_)) =
                resume_outcome
            {
                let error = execution_outcome_error(
                    "thread/resume",
                    Some(conversation_id),
                    outcome,
                );
                let _ = session.shutdown();
                self.fail_execution_creation(conversation_id, &slot, error.clone());
                return Err(error);
            }
            let installed = {
                let mutable = lock(&self.mutable);
                mutable.status == InstanceStatus::Ready
                    && mutable
                        .executions
                        .get(conversation_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &slot))
                    && slot.attempt_generation == attempt_generation
                    && slot.set_ready(ExecutionSession {
                        session: session.clone(),
                        generation: generation.clone(),
                        active_turn_id: None,
                        interaction_expires_at: None,
                    })
            };
            if !installed {
                let _ = session.shutdown();
                let error = execution_start_cancelled_error();
                slot.fail(error.clone());
                return Err(error);
            }
            self.start_execution_event_forwarder(
                conversation_id.to_string(),
                generation,
                session,
                incoming,
            );
            if let Some(handle) = slot.wait_ready()? {
                return Ok(handle);
            }
        }
    }

    fn execution_creation_is_current(
        &self,
        conversation_id: &str,
        slot: &Arc<ExecutionSlot>,
        attempt_generation: u64,
        session_generation: &str,
    ) -> bool {
        let mutable = lock(&self.mutable);
        mutable.status == InstanceStatus::Ready
            && mutable
                .executions
                .get(conversation_id)
                .is_some_and(|current| Arc::ptr_eq(current, slot))
            && slot.creation_is_current(attempt_generation, session_generation)
    }

    fn with_current_execution<T>(
        self: &Arc<Self>,
        conversation_id: &str,
        operation_name: &str,
        mut operation: impl FnMut(&ExecutionHandle) -> Result<ExecutionStep<T>, ProtocolError>,
    ) -> Result<T, ProtocolError> {
        loop {
            let handle = self.acquire_execution(conversation_id)?;
            self.lifecycle_hook
                .after_handle_acquired(conversation_id, operation_name);
            let operation_guard = lock(&handle.slot.operation);
            if !self.execution_handle_is_current(conversation_id, &handle) {
                drop(operation_guard);
                continue;
            }
            match operation(&handle)? {
                ExecutionStep::Complete(result) => return Ok(result),
                ExecutionStep::Retry => {}
            }
        }
    }

    fn execution_handle_is_current(
        &self,
        conversation_id: &str,
        handle: &ExecutionHandle,
    ) -> bool {
        let mutable = lock(&self.mutable);
        mutable.status == InstanceStatus::Ready
            && mutable
                .executions
                .get(conversation_id)
                .is_some_and(|current| Arc::ptr_eq(current, &handle.slot))
            && handle.slot.matches_generation(&handle.generation)
    }

    fn fail_execution_creation(
        &self,
        conversation_id: &str,
        slot: &Arc<ExecutionSlot>,
        error: ProtocolError,
    ) {
        let mut mutable = lock(&self.mutable);
        slot.fail(error);
        if mutable
            .executions
            .get(conversation_id)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            mutable.executions.remove(conversation_id);
        }
    }

    fn execution_slot(
        &self,
        conversation_id: &str,
        generation: &str,
    ) -> Option<Arc<ExecutionSlot>> {
        lock(&self.mutable)
            .executions
            .get(conversation_id)
            .filter(|slot| slot.matches_generation(generation))
            .cloned()
    }

    fn has_execution_generation(&self, generation: &str) -> bool {
        lock(&self.mutable)
            .executions
            .values()
            .any(|slot| slot.matches_generation(generation))
    }

    fn release_execution(&self, conversation_id: &str, generation: &str) {
        let Some((slot, expired)) = self.begin_execution_close(conversation_id, generation) else {
            return;
        };
        self.finish_execution_close(conversation_id, generation, slot, expired);
    }

    fn begin_execution_close(
        &self,
        conversation_id: &str,
        generation: &str,
    ) -> Option<(Arc<ExecutionSlot>, Vec<PendingApproval>)> {
        let (slot, expired) = {
            let mut mutable = lock(&self.mutable);
            let slot = mutable.executions.get(conversation_id)?.clone();
            if !slot.matches_generation(generation) || !slot.begin_closing(generation) {
                return None;
            }
            let approval_ids = mutable
                .pending_approvals
                .iter()
                .filter(|(_, pending)| pending.request.session_generation == generation)
                .map(|(approval_id, _)| approval_id.clone())
                .collect::<Vec<_>>();
            let expired = approval_ids
                .into_iter()
                .filter_map(|approval_id| mutable.pending_approvals.remove(&approval_id))
                .collect::<Vec<_>>();
            (slot, expired)
        };
        Some((slot, expired))
    }

    fn finish_execution_close(
        &self,
        conversation_id: &str,
        generation: &str,
        slot: Arc<ExecutionSlot>,
        expired: Vec<PendingApproval>,
    ) {
        let Some(session) = slot.closing_session(generation) else {
            return;
        };
        if let Err(error) = session.shutdown() {
            eprintln!("Codex execution session shutdown failed: {error}");
        }
        {
            let mut mutable = lock(&self.mutable);
            if mutable
                .executions
                .get(conversation_id)
                .is_some_and(|current| Arc::ptr_eq(current, &slot))
            {
                mutable.executions.remove(conversation_id);
            }
        }
        slot.finish_closing(generation);
        for pending in expired {
            let (approval, event) =
                lock(&self.mapper).approval_expired(pending.approval, now_ms());
            {
                let mut mutable = lock(&self.mutable);
                update_recorded_approval(&mut mutable, &approval);
            }
            if let Err(error) = self.events.publish(event) {
                eprintln!("Codex approval expiration forwarding failed: {}", error.message);
            }
        }
    }

    fn release_idle_execution(&self, conversation_id: &str, handle: &ExecutionHandle) {
        if handle.slot.idle_without_valid_interaction(
            &handle.generation,
            Instant::now(),
        ) {
            self.release_execution(conversation_id, &handle.generation);
        }
    }

    fn finish_turn_execution(
        &self,
        conversation_id: &str,
        handle: &ExecutionHandle,
        completed_turn_id: Option<&str>,
    ) {
        if handle
            .slot
            .finish_active_turn_and_should_close(
                &handle.generation,
                completed_turn_id,
                Instant::now(),
            )
            .unwrap_or(false)
        {
            self.release_execution(conversation_id, &handle.generation);
        }
    }

    fn start_observer_forwarder(
        self: &Arc<Self>,
        observer_generation: String,
        session: CodexAppServerSession,
        incoming: Receiver<Result<CodexIncoming, CodexAppServerError>>,
    ) {
        let runtime = Arc::downgrade(self);
        thread::spawn(move || {
            while let Ok(message) = incoming.recv() {
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let is_current_observer = {
                    let mutable = lock(&runtime.mutable);
                    mutable.observer_generation.as_deref() == Some(observer_generation.as_str())
                        && matches!(
                            mutable.status,
                            InstanceStatus::Ready | InstanceStatus::Starting
                        )
                };
                if !is_current_observer {
                    return;
                }
                match message {
                    Ok(CodexIncoming::Notification(
                        CodexNotification::ThreadNameUpdated {
                            thread_id,
                            thread_name,
                        },
                    )) => {
                        if let Some(event) = runtime.conversation_upsert_event(
                            &session,
                            &thread_id,
                            thread_name,
                        ) {
                            if let Err(error) = runtime.events.publish(event) {
                                eprintln!(
                                    "Codex observer title event forwarding failed: {}",
                                    error.message
                                );
                            }
                        }
                    }
                    Ok(CodexIncoming::Notification(
                        CodexNotification::ThreadStatusChanged { thread_id, status },
                    )) => {
                        if let Some(event) = runtime.conversation_status_upsert_event(
                            &session,
                            &thread_id,
                            status,
                        ) {
                            if let Err(error) = runtime.events.publish(event) {
                                eprintln!(
                                    "Codex observer status event forwarding failed: {}",
                                    error.message
                                );
                            }
                        }
                    }
                    Ok(incoming @ CodexIncoming::Notification(
                        CodexNotification::ProjectChanged { .. },
                    )) => match lock(&runtime.mapper).events(incoming) {
                        Ok(events) => {
                            for event in events {
                                if let Err(error) = runtime.events.publish(event) {
                                    eprintln!(
                                        "Codex observer project event forwarding failed: {}",
                                        error.message
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            eprintln!("Codex observer project event mapping failed: {}", error.message);
                        }
                    },
                    Ok(_) => {}
                    Err(error) => {
                        runtime.fail_observer(
                            &observer_generation,
                            CodexProtocolMapper::error(error),
                        );
                        return;
                    }
                }
            }
        });
    }

    fn start_execution_event_forwarder(
        self: &Arc<Self>,
        conversation_id: String,
        session_generation: String,
        session: CodexAppServerSession,
        incoming: Receiver<Result<CodexIncoming, CodexAppServerError>>,
    ) {
        let runtime = Arc::downgrade(self);
        thread::spawn(move || {
            loop {
                let message = match incoming.recv_timeout(INTERACTION_REAPER_INTERVAL) {
                    Ok(message) => message,
                    Err(RecvTimeoutError::Timeout) => {
                        let Some(runtime) = runtime.upgrade() else {
                            return;
                        };
                        let Some(slot) =
                            runtime.execution_slot(&conversation_id, &session_generation)
                        else {
                            return;
                        };
                        let _operation = lock(&slot.operation);
                        if slot.interaction_expired_while_idle(
                            &session_generation,
                            Instant::now(),
                        ) {
                            runtime.release_execution(&conversation_id, &session_generation);
                            return;
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        if let Some(runtime) = runtime.upgrade() {
                            runtime.release_execution(&conversation_id, &session_generation);
                        }
                        return;
                    }
                };
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let Some(slot) = runtime.execution_slot(&conversation_id, &session_generation)
                else {
                    return;
                };
                let _operation = lock(&slot.operation);
                match message {
                    Ok(incoming) => {
                        let terminal_turn_id = match &incoming {
                            CodexIncoming::Notification(CodexNotification::TurnCompleted {
                                thread_id,
                                turn,
                            }) if turn.status != CodexTurnStatus::InProgress => {
                                if thread_id != &conversation_id {
                                    eprintln!(
                                        "Codex execution session emitted a terminal event for another conversation"
                                    );
                                    runtime.release_execution(
                                        &conversation_id,
                                        &session_generation,
                                    );
                                    return;
                                }
                                Some(turn.id.clone())
                            }
                            CodexIncoming::Notification(CodexNotification::TurnCompleted {
                                ..
                            }) => {
                                eprintln!(
                                    "Codex execution session emitted turn/completed with an in-progress turn"
                                );
                                runtime.release_execution(
                                    &conversation_id,
                                    &session_generation,
                                );
                                return;
                            }
                            _ => None,
                        };
                        if let Some(terminal_turn_id) = terminal_turn_id {
                            let Some(should_close) = slot.finish_active_turn_and_should_close(
                                &session_generation,
                                Some(&terminal_turn_id),
                                Instant::now(),
                            ) else {
                                return;
                            };
                            let closing = if should_close {
                                runtime.begin_execution_close(
                                    &conversation_id,
                                    &session_generation,
                                )
                            } else {
                                None
                            };
                            let forwarded = match runtime.map_execution_incoming(
                                &conversation_id,
                                &session_generation,
                                &session,
                                incoming,
                            ) {
                                Ok(events) => {
                                    let mut published = true;
                                    for event in events {
                                        if let Err(error) = runtime.events.publish(event) {
                                            eprintln!(
                                                "Codex execution event forwarding failed: {}",
                                                error.message
                                            );
                                            published = false;
                                            break;
                                        }
                                    }
                                    published
                                }
                                Err(error) => {
                                    eprintln!(
                                        "Codex execution event mapping failed: {}",
                                        error.message
                                    );
                                    false
                                }
                            };
                            if !forwarded {
                                if let Some((closing_slot, expired)) = closing {
                                    runtime.finish_execution_close(
                                        &conversation_id,
                                        &session_generation,
                                        closing_slot,
                                        expired,
                                    );
                                } else {
                                    runtime.release_execution(
                                        &conversation_id,
                                        &session_generation,
                                    );
                                }
                                return;
                            }
                            if let Some((closing_slot, expired)) = closing {
                                runtime.finish_execution_close(
                                    &conversation_id,
                                    &session_generation,
                                    closing_slot,
                                    expired,
                                );
                                return;
                            }
                            continue;
                        }
                        let events = runtime.map_execution_incoming(
                            &conversation_id,
                            &session_generation,
                            &session,
                            incoming,
                        );
                        let events = match events {
                            Ok(events) => events,
                            Err(error) => {
                                eprintln!(
                                    "Codex execution event mapping failed: {}",
                                    error.message
                                );
                                runtime.release_execution(
                                    &conversation_id,
                                    &session_generation,
                                );
                                return;
                            }
                        };
                        for event in events {
                            if let Err(error) = runtime.events.publish(event) {
                                eprintln!(
                                    "Codex execution event forwarding failed: {}",
                                    error.message
                                );
                                runtime.release_execution(
                                    &conversation_id,
                                    &session_generation,
                                );
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("Codex execution App Server failed: {error}");
                        runtime.release_execution(&conversation_id, &session_generation);
                        return;
                    }
                }
            }
        });
    }

    fn map_execution_incoming(
        &self,
        conversation_id: &str,
        session_generation: &str,
        session: &CodexAppServerSession,
        incoming: CodexIncoming,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        if incoming_conversation_id(&incoming)
            .is_some_and(|incoming_id| incoming_id != conversation_id)
        {
            return Err(protocol_error(
                "provider_protocol_error",
                "Codex execution session emitted an event for another conversation".to_string(),
                false,
            ));
        }
        if let Some(turn_id) = incoming_active_turn_id(&incoming, session_generation) {
            if let Some(slot) = self.execution_slot(conversation_id, session_generation) {
                slot.mark_active_turn(session_generation, turn_id);
            }
        }
        match incoming {
            CodexIncoming::Notification(CodexNotification::ProjectChanged { .. }) => {
                Ok(Vec::new())
            }
            CodexIncoming::Notification(CodexNotification::ThreadNameUpdated {
                thread_id,
                thread_name,
            }) => Ok(self
                .conversation_upsert_event(session, &thread_id, thread_name)
                .into_iter()
                .collect()),
            CodexIncoming::Notification(CodexNotification::ThreadStatusChanged {
                thread_id,
                status,
            }) => Ok(self
                .conversation_status_upsert_event(session, &thread_id, status)
                .into_iter()
                .collect()),
            CodexIncoming::ApprovalRequested(request) => {
                let approval = lock(&self.mapper).approval(&request);
                let approval_id = approval.resource.native_resource_id.clone();
                let item_id = request.item_id.clone();
                let mut mutable = lock(&self.mutable);
                if request.session_generation != session_generation
                    || !mutable
                        .executions
                        .get(conversation_id)
                        .is_some_and(|slot| slot.matches_generation(session_generation))
                {
                    return Ok(Vec::new());
                }
                mutable.pending_approvals.insert(
                    approval_id.clone(),
                    PendingApproval {
                        request,
                        approval: approval.clone(),
                    },
                );
                record_approval(&mut mutable, item_id, approval.clone());
                Ok(vec![ProtocolEvent::EventApprovalRequested {
                    jsonrpc: "2.0".to_string(),
                    params: ApprovalRequestedEvent { approval },
                }])
            }
            CodexIncoming::Notification(CodexNotification::ServerRequestResolved {
                request_id,
                session_generation: notification_generation,
                ..
            }) => {
                let approval_id = approval_resource_id(&notification_generation, &request_id);
                let pending = {
                    let mut mutable = lock(&self.mutable);
                    if notification_generation != session_generation
                        || !mutable
                            .executions
                            .get(conversation_id)
                            .is_some_and(|slot| slot.matches_generation(session_generation))
                    {
                        return Ok(Vec::new());
                    }
                    mutable.pending_approvals.remove(&approval_id)
                };
                let Some(pending) = pending else {
                    return Ok(Vec::new());
                };
                let (approval, event) = lock(&self.mapper).approval_expired(pending.approval, now_ms());
                {
                    let mut mutable = lock(&self.mutable);
                    update_recorded_approval(&mut mutable, &approval);
                }
                Ok(vec![event])
            }
            incoming => lock(&self.mapper).events(incoming),
        }
    }

    fn conversation_upsert_event(
        &self,
        session: &CodexAppServerSession,
        conversation_id: &str,
        thread_name: Option<String>,
    ) -> Option<ProtocolEvent> {
        let pending = {
            let mut mutable = lock(&self.mutable);
            mutable
                .pending_materialization
                .get_mut(conversation_id)
                .map(|snapshot| {
                    snapshot.thread.name = thread_name.clone();
                    snapshot.clone()
                })
        };
        let mut snapshot = match session.thread_read_metadata(conversation_id) {
            Ok(snapshot) => snapshot,
            Err(error) => match pending {
                Some(snapshot) => snapshot,
                None => {
                    eprintln!(
                        "Codex title update could not refresh conversation {conversation_id}: {error}"
                    );
                    return None;
                }
            },
        };
        snapshot.thread.name = thread_name;
        Some(ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".to_string(),
            params: ConversationUpsertedEvent {
                conversation: lock(&self.mapper).conversation(&snapshot),
            },
        })
    }

    fn conversation_status_upsert_event(
        &self,
        session: &CodexAppServerSession,
        conversation_id: &str,
        status: crate::protocol::CodexThreadStatus,
    ) -> Option<ProtocolEvent> {
        let pending = {
            let mut mutable = lock(&self.mutable);
            mutable
                .pending_materialization
                .get_mut(conversation_id)
                .map(|snapshot| {
                    snapshot.thread.status = status.clone();
                    snapshot.clone()
                })
        };
        let mut snapshot = match session.thread_read_metadata(conversation_id) {
            Ok(snapshot) => snapshot,
            Err(error) => match pending {
                Some(snapshot) => snapshot,
                None => {
                    eprintln!(
                        "Codex status update could not refresh conversation {conversation_id}: {error}"
                    );
                    return None;
                }
            },
        };
        snapshot.thread.status = status;
        Some(ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".to_string(),
            params: ConversationUpsertedEvent {
                conversation: lock(&self.mapper).conversation(&snapshot),
            },
        })
    }

    fn fail_observer(
        &self,
        observer_generation: &str,
        error: ProtocolError,
    ) {
        eprintln!("Codex observer App Server failed: {}", error.message);
        let (failure_generation, sessions, execution_slots) = {
            let _transition = lock(&self.lifecycle_transition);
            let mut mutable = lock(&self.mutable);
            if mutable.observer_generation.as_deref() != Some(observer_generation)
                || !matches!(mutable.status, InstanceStatus::Ready | InstanceStatus::Starting)
            {
                return;
            }
            let previous = mutable.status;
            mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            let failure_generation = mutable.lifecycle_generation;
            mutable.cleanup_in_progress = true;
            mutable.status = InstanceStatus::Error;
            mutable.observer_session_id = None;
            mutable.observer_generation = None;
            let sessions = mutable.sessions.drain().map(|(_, slot)| slot).collect();
            let execution_slots = mutable
                .executions
                .drain()
                .collect::<Vec<_>>();
            mutable.pending_approvals.clear();
            mutable.approval_history.clear();
            mutable.pending_materialization.clear();
            self.lifecycle_changed.notify_all();
            drop(mutable);
            let _ = self.publish_status_change(previous);
            (failure_generation, sessions, execution_slots)
        };
        let _ = Self::cancel_sessions(sessions);
        for (conversation_id, slot) in execution_slots {
            let session = slot.force_close();
            self.lifecycle_hook
                .after_execution_cancelled(&conversation_id);
            if let Some(session) = session {
                let _ = session.shutdown();
            }
        }
        let _transition = lock(&self.lifecycle_transition);
        let mut mutable = lock(&self.mutable);
        if mutable.lifecycle_generation == failure_generation {
            mutable.cleanup_in_progress = false;
            self.lifecycle_changed.notify_all();
        }
    }
}

struct ProviderState {
    host_device_id: Option<String>,
    initialized_client_id: Option<String>,
    instances: HashMap<String, Arc<CodexInstanceRuntime>>,
    selected_runtime: Option<RuntimeInstallation>,
}

pub struct CodexProvider {
    state: Mutex<ProviderState>,
    events: Arc<dyn ProviderEventSink>,
    lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
    shutdown: AtomicBool,
    shutdown_complete: AtomicBool,
    shutdown_changed: tokio::sync::Notify,
}

impl CodexProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self::new_with_execution_lifecycle_hook(
            events,
            Arc::new(NoopExecutionLifecycleHook),
        )
    }

    #[doc(hidden)]
    pub fn new_with_execution_lifecycle_hook(
        events: Arc<dyn ProviderEventSink>,
        lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
    ) -> Self {
        Self {
            state: Mutex::new(ProviderState {
                host_device_id: None,
                initialized_client_id: None,
                instances: HashMap::new(),
                selected_runtime: None,
            }),
            events,
            lifecycle_hook,
            shutdown: AtomicBool::new(false),
            shutdown_complete: AtomicBool::new(false),
            shutdown_changed: tokio::sync::Notify::new(),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn descriptor() -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            plugin_id: CODEX_PLUGIN_ID.to_string(),
            display_name: "Codex".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_workspace_root: codex_home()
                .map(|path| path.join("codepet-workspaces").to_string_lossy().into_owned()),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
            instance_kinds: vec![CODEX_INSTANCE_KIND.to_string()],
        }
    }

    fn instance(&self, route: &ProviderInstanceRoute) -> Result<Arc<CodexInstanceRuntime>, ProtocolError> {
        validate_route(route)?;
        let state = lock(&self.state);
        let expected_device = state.host_device_id.as_deref().ok_or_else(|| {
            protocol_error(
                "provider_not_initialized",
                "Provider must be initialized before using instances".to_string(),
                false,
            )
        })?;
        if route.device_id != expected_device {
            return Err(protocol_error(
                "wrong_device_route",
                "Provider route targets a different Host device".to_string(),
                false,
            ));
        }
        state
            .instances
            .get(&route.provider_instance_id)
            .cloned()
            .ok_or_else(|| {
                protocol_error(
                    "unknown_provider_instance",
                    format!(
                        "unknown Codex Provider instance: {}",
                        route.provider_instance_id
                    ),
                    false,
                )
            })
    }

    fn resource_instance(&self, resource: &RoutedResourceId) -> Result<Arc<CodexInstanceRuntime>, ProtocolError> {
        validate_resource(resource)?;
        self.instance(&ProviderInstanceRoute {
            device_id: resource.device_id.clone(),
            provider_plugin_id: resource.provider_plugin_id.clone(),
            provider_instance_id: resource.provider_instance_id.clone(),
        })
    }
}

impl Provider for CodexProvider {
    fn provider_initialize<'a>(
        &'a self,
        request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async move {
            if request.supported_versions.min_version > request.supported_versions.max_version {
                return Err(protocol_error(
                    "invalid_protocol_range",
                    "Host protocol range is invalid".to_string(),
                    false,
                ));
            }
            if PROTOCOL_VERSION < request.supported_versions.min_version
                || PROTOCOL_VERSION > request.supported_versions.max_version
            {
                return Err(protocol_error(
                    "unsupported_protocol_version",
                    "Provider protocol v1 is outside the Host-supported range".to_string(),
                    false,
                ));
            }
            if request.host_client_id.trim().is_empty()
                || request.host_device_id.trim().is_empty()
                || request.host_version.trim().is_empty()
            {
                return Err(protocol_error(
                    "invalid_host_identity",
                    "Host client, device, and version must not be empty".to_string(),
                    false,
                ));
            }
            let mut state = lock(&self.state);
            if let Some(device_id) = state.host_device_id.as_ref() {
                if device_id != &request.host_device_id
                    || state.initialized_client_id.as_ref() != Some(&request.host_client_id)
                {
                    return Err(protocol_error(
                        "provider_already_initialized",
                        "Provider process is already bound to another Host identity".to_string(),
                        false,
                    ));
                }
            } else {
                state.host_device_id = Some(request.host_device_id);
                state.initialized_client_id = Some(request.host_client_id);
            }
            Ok(ProviderInitializeResponse {
                selected_version: PROTOCOL_VERSION,
                plugin: Self::descriptor(),
            })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        _request: ProviderDescribeRequest,
    ) -> ProtocolFuture<'a, ProviderDescribeResponse> {
        Box::pin(async move {
            Ok(ProviderDescribeResponse {
                plugin: Self::descriptor(),
            })
        })
    }

    fn runtime_get_installed<'a>(
        &'a self,
        _request: RuntimeGetInstalledRequest,
    ) -> ProtocolFuture<'a, RuntimeGetInstalledResponse> {
        Box::pin(async move {
            let selected = lock(&self.state).selected_runtime.clone();
            Ok(runtime_inventory(discover_codex_candidates(), selected, "codex"))
        })
    }

    fn runtime_select<'a>(
        &'a self,
        request: RuntimeSelectRequest,
    ) -> ProtocolFuture<'a, RuntimeSelectResponse> {
        Box::pin(async move {
            let selected = inspect_runtime_candidate(request.candidate, "codex")?;
            lock(&self.state).selected_runtime = Some(selected.clone());
            Ok(RuntimeSelectResponse { selected })
        })
    }

    fn instance_create<'a>(
        &'a self,
        mut request: InstanceCreateRequest,
    ) -> ProtocolFuture<'a, InstanceCreateResponse> {
        Box::pin(async move {
            Self::descriptor().validate_instance_kind(&request.instance_kind)?;
            validate_route(&request.route)?;
            if request.display_name.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_provider_instance",
                    "Provider instance display name must not be empty".to_string(),
                    false,
                ));
            }
            let selected = if request.settings.contains_key("appServerExecutable") { None } else {
                Some(lock(&self.state).selected_runtime.clone().or_else(|| {
                    runtime_inventory(discover_codex_candidates(), None, "codex").installed.into_iter().next()
                }).ok_or_else(|| protocol_error("provider_unavailable", "Codex Provider did not find a compatible local runtime".to_string(), true))?)
            };
            if let Some(selected) = selected.as_ref() {
                request.settings.insert("appServerExecutable".to_string(), json!(selected.executable_path.clone()));
            }
            let settings = decode_settings(request.settings.clone())?;
            let mut state = lock(&self.state);
            if let Some(selected) = selected { state.selected_runtime = Some(selected); }
            let host_device = state.host_device_id.as_deref().ok_or_else(|| {
                protocol_error(
                    "provider_not_initialized",
                    "Provider must be initialized before creating instances".to_string(),
                    false,
                )
            })?;
            if request.route.device_id != host_device {
                return Err(protocol_error(
                    "wrong_device_route",
                    "Provider instance targets a different Host device".to_string(),
                    false,
                ));
            }
            if let Some(existing) = state.instances.get(&request.route.provider_instance_id) {
                if existing.route != request.route
                    || existing.instance_kind != request.instance_kind
                    || existing.display_name != request.display_name
                    || existing.settings != settings
                {
                    return Err(protocol_error(
                        "provider_instance_conflict",
                        "Provider instance id was reused with different configuration".to_string(),
                        false,
                    ));
                }
                return Ok(InstanceCreateResponse {
                    instance: existing.snapshot(),
                });
            }
            let runtime = Arc::new(CodexInstanceRuntime::new(
                request,
                settings,
                self.events.clone(),
                self.lifecycle_hook.clone(),
            ));
            let instance = runtime.snapshot();
            state.instances.insert(
                runtime.route.provider_instance_id.clone(),
                runtime,
            );
            Ok(InstanceCreateResponse { instance })
        })
    }

    fn instance_start<'a>(
        &'a self,
        request: InstanceStartRequest,
    ) -> ProtocolFuture<'a, InstanceStartResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let (slot, starting_event_error) = {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                if mutable.destroyed {
                    return Err(protocol_error(
                        "unknown_provider_instance",
                        format!(
                            "unknown Codex Provider instance: {}",
                            runtime.route.provider_instance_id
                        ),
                        false,
                    ));
                }
                if mutable.cleanup_in_progress {
                    return Err(protocol_error(
                        "provider_instance_stopping",
                        "Codex Provider instance is cleaning up a failed lifecycle".to_string(),
                        true,
                    ));
                }
                match mutable.status {
                    InstanceStatus::Ready => {
                        drop(mutable);
                        return Ok(InstanceStartResponse {
                            instance: runtime.snapshot(),
                        });
                    }
                    InstanceStatus::Starting => {
                        return Err(protocol_error(
                            "provider_instance_starting",
                            "Codex Provider instance is already starting".to_string(),
                            true,
                        ));
                    }
                    InstanceStatus::Stopping => {
                        return Err(protocol_error(
                            "provider_instance_stopping",
                            "Codex Provider instance is stopping".to_string(),
                            true,
                        ));
                    }
                    _ => {}
                }
                let previous = mutable.status;
                mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
                mutable.status = InstanceStatus::Starting;
                mutable.observer_session_id = None;
                mutable.observer_generation = None;
                mutable.pending_approvals.clear();
                mutable.approval_history.clear();
                mutable.pending_materialization.clear();
                let slot = Arc::new(InstanceSessionSlot::new(mutable.lifecycle_generation));
                mutable.sessions.insert(slot.id, slot.clone());
                drop(mutable);
                let event_error = runtime.publish_status_change(previous).err();
                (slot, event_error)
            };
            if let Some(error) = starting_event_error {
                let _ = runtime.mark_start_failed(&slot);
                return Err(error);
            }
            if !slot.begin_spawn() {
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("observer session"));
            }
            let executable = runtime.settings.app_server_executable.clone();
            let args = runtime.settings.app_server_args.clone();
            let observer = match tokio::task::spawn_blocking(move || {
                CodexAppServerSession::spawn_uninitialized(&executable, &args)
            })
            .await {
                Ok(Ok(observer)) => observer,
                Ok(Err(error)) => {
                    let mapped = CodexProtocolMapper::error(error);
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("observer session"));
                }
                Err(error) => {
                    let mapped = protocol_error(
                    "provider_task_failed",
                    format!("Codex App Server start task failed: {error}"),
                    true,
                    );
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("observer session"));
                }
            };
            if !slot.register_spawned(observer.clone()) || !runtime.session_is_current(&slot) {
                let _ = observer.shutdown();
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("observer session"));
            }
            let initialize_session = observer.clone();
            let initialize_result = tokio::task::spawn_blocking(move || initialize_session.initialize())
                .await
                .map_err(provider_task_error)?;
            if let Err(error) = initialize_result {
                let mapped = CodexProtocolMapper::error(error);
                let _ = observer.shutdown();
                if runtime.mark_start_failed(&slot)? {
                    return Err(mapped);
                }
                return Err(instance_session_cancelled_error("observer session"));
            }
            let discovery_session = observer.clone();
            let models = match tokio::task::spawn_blocking(move || discovery_session.model_list())
                .await
                .map_err(provider_task_error)?
            {
                Ok(models) => models,
                Err(error) => {
                    let mapped = CodexProtocolMapper::error(error);
                    let _ = observer.shutdown();
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("observer session"));
                }
            };
            let project_discovery_session = observer.clone();
            let project_api_supported = match tokio::task::spawn_blocking(move || {
                project_discovery_session.project_list(None, Some(1))
            })
            .await
            .map_err(provider_task_error)?
            {
                Ok(_) => true,
                Err(error) if error.is_method_not_found() => false,
                Err(error) => {
                    let mapped = CodexProtocolMapper::error(error);
                    let _ = observer.shutdown();
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("observer session"));
                }
            };
            let metadata_session = observer.clone();
            let (account, rate_limits, token_usage) = tokio::task::spawn_blocking(move || {
                (
                    metadata_session.account_read(),
                    metadata_session.account_rate_limits_read(),
                    metadata_session.account_usage_read(),
                )
            })
            .await
            .map_err(provider_task_error)?;
            for (method, result) in [
                ("account/read", account.as_ref().map(|_| ())),
                (
                    "account/rateLimits/read",
                    rate_limits.as_ref().map(|_| ()),
                ),
                ("account/usage/read", token_usage.as_ref().map(|_| ())),
            ] {
                if let Err(error) = result {
                    eprintln!("Codex {method} metadata probe failed: {error}");
                }
            }
            let authentication = account
                .as_ref()
                .ok()
                .and_then(codex_authentication);
            let usage = codex_usage(
                rate_limits.as_ref().ok(),
                token_usage.as_ref().ok(),
            );
            let capabilities = match CodexProtocolMapper::capabilities(
                observer.generation().to_string(),
                models,
                project_api_supported,
            ) {
                Ok(capabilities) => capabilities,
                Err(error) => {
                    let _ = observer.shutdown();
                    if runtime.mark_start_failed(&slot)? {
                        return Err(error);
                    }
                    return Err(instance_session_cancelled_error("observer session"));
                }
            };
            let harness = HarnessDescriptor {
                id: CODEX_INSTANCE_KIND.to_string(),
                display_name: "Codex".to_string(),
                version: observer.harness_version(),
                executable_path: Some(runtime.settings.app_server_executable.to_string_lossy().into_owned()),
            };
            let incoming = match observer.subscribe() {
                Ok(incoming) => incoming,
                Err(error) => {
                    let mapped = CodexProtocolMapper::error(error);
                    let _ = observer.shutdown();
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("observer session"));
                }
            };
            let observer_generation = observer.generation().to_string();
            let (installed, ready_event_error) = {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                let current = mutable.status == InstanceStatus::Starting
                    && mutable.lifecycle_generation == slot.lifecycle_generation
                    && mutable
                        .sessions
                        .get(&slot.id)
                        .is_some_and(|current| Arc::ptr_eq(current, &slot))
                    && !slot.cancelled.load(Ordering::SeqCst)
                    && mutable.observer_session_id.is_none()
                    && mutable.executions.is_empty();
                if !current {
                    (false, None)
                } else {
                    mutable.capabilities = capabilities;
                    mutable.harness = harness;
                    mutable.authentication = authentication;
                    mutable.usage = usage;
                    mutable.observer_session_id = Some(slot.id);
                    mutable.observer_generation = Some(observer_generation.clone());
                    mutable.pending_approvals.clear();
                    mutable.approval_history.clear();
                    mutable.pending_materialization.clear();
                    let previous = mutable.status;
                    mutable.status = InstanceStatus::Ready;
                    runtime.lifecycle_changed.notify_all();
                    drop(mutable);
                    (true, runtime.publish_status_change(previous).err())
                }
            };
            if !installed {
                let _ = observer.shutdown();
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("observer session"));
            }
            if let Some(error) = ready_event_error {
                runtime.fail_observer(&observer_generation, error.clone());
                return Err(error);
            }
            runtime.start_observer_forwarder(observer_generation, observer, incoming);
            let instance = runtime.snapshot();
            Ok(InstanceStartResponse { instance })
        })
    }

    fn instance_stop<'a>(
        &'a self,
        request: InstanceStopRequest,
    ) -> ProtocolFuture<'a, InstanceStopResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            Ok(InstanceStopResponse {
                instance: runtime.stop().await?,
            })
        })
    }

    fn instance_destroy<'a>(
        &'a self,
        request: InstanceDestroyRequest,
    ) -> ProtocolFuture<'a, InstanceDestroyResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                if mutable.destroyed {
                    return Ok(InstanceDestroyResponse { destroyed: true });
                }
                if mutable.cleanup_in_progress
                    || matches!(
                        mutable.status,
                        InstanceStatus::Ready
                            | InstanceStatus::Starting
                            | InstanceStatus::Stopping
                    )
                {
                    return Err(protocol_error(
                        "provider_instance_running",
                        "Stop the Codex Provider instance before destroying it".to_string(),
                        false,
                    ));
                }
                mutable.destroyed = true;
                mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
                mutable.observer_session_id = None;
                mutable.observer_generation = None;
                runtime.lifecycle_changed.notify_all();
            }
            let mut state = lock(&self.state);
            if state
                .instances
                .get(&request.route.provider_instance_id)
                .is_some_and(|current| Arc::ptr_eq(current, &runtime))
            {
                state.instances.remove(&request.route.provider_instance_id);
            }
            Ok(InstanceDestroyResponse { destroyed: true })
        })
    }

    fn instance_capabilities<'a>(
        &'a self,
        request: InstanceCapabilitiesRequest,
    ) -> ProtocolFuture<'a, InstanceCapabilitiesResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let capabilities = lock(&runtime.mutable).capabilities.clone();
            Ok(InstanceCapabilitiesResponse {
                capabilities,
            })
        })
    }

    fn project_list<'a>(
        &'a self,
        request: ProjectListRequest,
    ) -> ProtocolFuture<'a, ProjectListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            require_capability(&runtime, ProviderCapability::ProjectList)?;
            let limit = request.limit.map(u32::try_from).transpose().map_err(|_| {
                protocol_error(
                    "invalid_request",
                    "project list limit exceeds the Codex App Server range".to_string(),
                    false,
                )
            })?;
            let session = runtime.ready_observer()?;
            let page = tokio::task::spawn_blocking(move || {
                session.project_list(request.cursor, limit)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let projects = {
                let mapper = lock(&runtime.mapper);
                page.data
                    .into_iter()
                    .map(|project| mapper.project(project))
                    .collect::<Result<Vec<_>, _>>()?
            };
            Ok(ProjectListResponse {
                projects,
                page_info: PageInfo {
                    next_cursor: page.next_cursor,
                },
            })
        })
    }

    fn project_get<'a>(
        &'a self,
        request: ProjectGetRequest,
    ) -> ProtocolFuture<'a, ProjectGetResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.project)?;
            require_capability(&runtime, ProviderCapability::ProjectGet)?;
            let project_id = request.project.native_resource_id;
            let session = runtime.ready_observer()?;
            let project = tokio::task::spawn_blocking(move || session.project_read(&project_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
            let project = lock(&runtime.mapper).project(project)?;
            Ok(ProjectGetResponse { project })
        })
    }

    fn project_create<'a>(
        &'a self,
        request: ProjectCreateRequest,
    ) -> ProtocolFuture<'a, ProjectCreateResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            require_capability(&runtime, ProviderCapability::ProjectCreate)?;
            validate_project_fields(&request.name, &request.roots)?;
            let session = runtime.ready_observer()?;
            let native_request = CodexProjectCreateRequest {
                idempotency_key: request.idempotency_key,
                name: request.name,
                roots: request
                    .roots
                    .into_iter()
                    .map(|root| CodexProjectRoot { path: root.path })
                    .collect(),
                metadata: request.metadata,
            };
            let project = tokio::task::spawn_blocking(move || {
                session.project_create(native_request)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let project = lock(&runtime.mapper).project(project)?;
            Ok(ProjectCreateResponse { project })
        })
    }

    fn project_update<'a>(
        &'a self,
        request: ProjectUpdateRequest,
    ) -> ProtocolFuture<'a, ProjectUpdateResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.project)?;
            require_capability(&runtime, ProviderCapability::ProjectUpdate)?;
            if let Some(name) = request.name.as_deref() {
                if name.trim().is_empty() {
                    return Err(protocol_error(
                        "invalid_request",
                        "project name must not be empty".to_string(),
                        false,
                    ));
                }
            }
            if request
                .roots
                .as_ref()
                .is_some_and(|roots| roots.is_empty() || roots.iter().any(|root| root.path.trim().is_empty()))
            {
                return Err(protocol_error(
                    "invalid_request",
                    "project roots must contain non-empty paths".to_string(),
                    false,
                ));
            }
            let session = runtime.ready_observer()?;
            let native_request = CodexProjectUpdateRequest {
                project_id: request.project.native_resource_id,
                name: request.name,
                roots: request.roots.map(|roots| {
                    roots
                        .into_iter()
                        .map(|root| CodexProjectRoot { path: root.path })
                        .collect()
                }),
                metadata: request.metadata,
            };
            let project = tokio::task::spawn_blocking(move || {
                session.project_update(native_request)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let project = lock(&runtime.mapper).project(project)?;
            Ok(ProjectUpdateResponse { project })
        })
    }

    fn project_delete<'a>(
        &'a self,
        request: ProjectDeleteRequest,
    ) -> ProtocolFuture<'a, ProjectDeleteResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.project)?;
            require_capability(&runtime, ProviderCapability::ProjectDelete)?;
            let project_id = request.project.native_resource_id;
            let session = runtime.ready_observer()?;
            tokio::task::spawn_blocking(move || session.project_delete(&project_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
            Ok(ProjectDeleteResponse {})
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let membership_filter = match request.project_filter {
                ConversationProjectFilter::ConversationProjectFilterAll(_) => {
                    CodexConversationMembershipFilter::All
                }
                ConversationProjectFilter::ConversationProjectFilterStandalone(_) => {
                    require_capability(&runtime, ProviderCapability::ProjectList)?;
                    CodexConversationMembershipFilter::Standalone
                }
                ConversationProjectFilter::ConversationProjectFilterProject(filter) => {
                    validate_resource_route(&filter.project, &request.route)?;
                    require_capability(&runtime, ProviderCapability::ProjectList)?;
                    CodexConversationMembershipFilter::Project(
                        filter.project.native_resource_id,
                    )
                }
            };
            let limit = request.limit.map(u32::try_from).transpose().map_err(|_| {
                protocol_error(
                    "invalid_request",
                    "conversation list limit exceeds the Codex App Server range".to_string(),
                    false,
                )
            })?;
            let session = runtime.ready_observer()?;
            let assignments = load_codex_desktop_project_assignments()?;
            let page = tokio::task::spawn_blocking(move || {
                list_codex_conversations(
                    &session,
                    CodexThreadListRequest {
                        cursor: request.cursor,
                        limit,
                        project_id: None,
                        workspace_root: None,
                        search_term: None,
                    },
                    membership_filter,
                    &assignments,
                )
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let conversations = {
                let mapper = lock(&runtime.mapper);
                page.data
                    .iter()
                    .map(|snapshot| mapper.conversation(snapshot))
                    .collect::<Vec<_>>()
            };
            Ok(ConversationListResponse {
                conversations,
                page_info: PageInfo {
                    next_cursor: page.next_cursor,
                },
            })
        })
    }

    fn conversation_search<'a>(
        &'a self,
        request: ConversationSearchRequest,
    ) -> ProtocolFuture<'a, ConversationSearchResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if request.search_term.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_request",
                    "conversation.search requires a non-empty searchTerm".to_string(),
                    false,
                ));
            }
            let limit = request.limit.map(u32::try_from).transpose().map_err(|_| {
                protocol_error(
                    "invalid_request",
                    "conversation search limit exceeds the Codex App Server range".to_string(),
                    false,
                )
            })?;
            let session = runtime.ready_observer()?;
            let page = tokio::task::spawn_blocking(move || {
                session.thread_list(CodexThreadListRequest {
                    cursor: request.cursor,
                    limit,
                    project_id: None,
                    workspace_root: None,
                    search_term: Some(request.search_term),
                })
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let conversations = {
                let mapper = lock(&runtime.mapper);
                page.data
                    .iter()
                    .map(|snapshot| mapper.conversation(snapshot))
                    .collect::<Vec<_>>()
            };
            Ok(ConversationSearchResponse {
                conversations,
                page_info: PageInfo {
                    next_cursor: page.next_cursor,
                },
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProtocolFuture<'a, ConversationGetResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            let requested_limit = request
                .limit
                .unwrap_or(DEFAULT_CONVERSATION_GET_TURN_LIMIT);
            if !(1..=MAX_CONVERSATION_GET_TURN_LIMIT).contains(&requested_limit) {
                return Err(protocol_error(
                    "invalid_request",
                    format!(
                        "conversation.get limit must be between 1 and {MAX_CONVERSATION_GET_TURN_LIMIT}"
                    ),
                    false,
                ));
            }
            let initial_cursor = request.cursor;
            let session = runtime.ready_observer()?;
            tokio::task::spawn_blocking(move || {
                let (snapshot, used_pending_snapshot) =
                    match session.thread_read_metadata(&conversation_id) {
                        Ok(snapshot) => (snapshot, false),
                        Err(error)
                            if error.is_thread_not_loaded(&conversation_id)
                                || is_created_conversation_not_ready(
                                    &error,
                                    &conversation_id,
                                ) =>
                        {
                            let snapshot = lock(&runtime.mutable)
                                .pending_materialization
                                .get(&conversation_id)
                                .cloned();
                            match snapshot {
                                Some(snapshot) => (snapshot, true),
                                None => return Err(CodexProtocolMapper::error(error)),
                            }
                        }
                        Err(error) => return Err(CodexProtocolMapper::error(error)),
                    };
                let approvals = {
                    let mutable = lock(&runtime.mutable);
                    mutable
                        .approval_history
                        .iter()
                        .filter(|observed| {
                            observed.approval.conversation.native_resource_id == snapshot.thread.id
                        })
                        .map(|observed| (observed.item_id.clone(), observed.approval.clone()))
                        .collect::<Vec<_>>()
                };
                let mut emitted_approvals = vec![false; approvals.len()];
                let mut items = Vec::new();
                let mut active_turn = None;
                let loaded = load_conversation_turns(
                    &session,
                    &conversation_id,
                    initial_cursor,
                    requested_limit,
                    used_pending_snapshot,
                )?;
                let turns = loaded.turns;
                let response_next_cursor = loaded.next_cursor;
                let history_materialized = loaded.materialized;
                let mapper = lock(&runtime.mapper);
                for turn in turns.into_iter().rev() {
                    if turn.status == CodexTurnStatus::InProgress {
                        active_turn = Some(mapper.turn(&snapshot.thread.id, &turn));
                    }
                    mapper.append_conversation_turn_items(
                        &snapshot.thread.id,
                        &turn,
                        &approvals,
                        &mut emitted_approvals,
                        &mut items,
                    );
                }
                let conversation =
                    mapper.conversation_with_active_turn(&snapshot, active_turn);
                drop(mapper);
                if history_materialized {
                    lock(&runtime.mutable)
                        .pending_materialization
                        .remove(&conversation_id);
                }
                Ok(fit_single_turn_conversation_history(Some(requested_limit), ConversationGetResponse {
                    conversation,
                    items,
                    page_info: Some(PageInfo {
                        next_cursor: response_next_cursor,
                    }),
                }))
            })
                .await
                .map_err(provider_task_error)?
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: ConversationAcquireInteractionRequest,
    ) -> ProtocolFuture<'a, ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            tokio::task::spawn_blocking(move || {
                runtime.with_current_execution(
                    &conversation_id,
                    "conversation.acquireInteraction",
                    |handle| {
                        let configuration = handle
                            .session
                            .thread_configuration(&conversation_id)
                            .ok_or_else(|| {
                                protocol_error(
                                    "provider_protocol_error",
                                    format!(
                                        "Codex thread/resume did not return configuration for conversation {conversation_id}"
                                    ),
                                    false,
                                )
                            })?;
                        let lease_duration = runtime.lifecycle_hook.interaction_lease_duration();
                        if !handle.slot.renew_interaction(
                            &handle.generation,
                            Instant::now() + lease_duration,
                        ) {
                            return Ok(ExecutionStep::Retry);
                        }
                        Ok(ExecutionStep::Complete(
                            ConversationAcquireInteractionResponse {
                                selection: TurnSelection {
                                    access_mode_id: configuration
                                        .permission_level
                                        .map(permission_level_id)
                                        .map(str::to_string),
                                    reasoning_effort_id: configuration.reasoning_effort.clone(),
                                    model: configuration.model.clone().map(|model_id| {
                                        ModelSelection::FlatModelSelection(FlatModelSelection {
                                            kind: FlatModelCatalogKind::Flat,
                                            model_id,
                                        })
                                    }),
                                },
                                lease_expires_at: Some(now_ms().saturating_add(
                                    lease_duration
                                        .as_millis()
                                        .min(u128::from(u64::MAX))
                                        as u64,
                                )),
                            },
                        ))
                    },
                )
            })
            .await
            .map_err(provider_task_error)?
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
            if request.title.is_some() {
                return Err(protocol_error(
                    "capability_unsupported",
                    "Codex App Server thread/start does not support setting a title".to_string(),
                    false,
                ));
            }
            if request.extension.is_some() {
                return Err(protocol_error(
                    "capability_unsupported",
                    "Codex Provider does not define conversation.create extensions".to_string(),
                    false,
                ));
            }
            let runtime = self.instance(&request.route)?;
            let project_id = if let Some(project) = request.project.as_ref() {
                validate_resource_route(project, &request.route)?;
                require_capability(&runtime, ProviderCapability::ProjectGet)?;
                Some(project.native_resource_id.clone())
            } else {
                None
            };
            let permission_level = parse_permission_level(&request.permission_level)?;
            let slot = {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                let observer_ready = mutable
                    .observer_session_id
                    .and_then(|id| mutable.sessions.get(&id))
                    .and_then(|slot| slot.session())
                    .is_some();
                if mutable.status != InstanceStatus::Ready || !observer_ready {
                    return Err(protocol_error(
                        "provider_unavailable",
                        format!(
                            "Codex Provider instance {} is not ready",
                            runtime.route.provider_instance_id
                        ),
                        true,
                    ));
                }
                let slot = Arc::new(InstanceSessionSlot::new(mutable.lifecycle_generation));
                mutable.sessions.insert(slot.id, slot.clone());
                slot
            };
            let workspace_root = match prepare_conversation_workspace(
                request.workspace_root.as_deref(),
                request.workspace_mode.as_deref(),
            ) {
                Ok(workspace_root) => workspace_root,
                Err(error) => {
                    runtime.unregister_session(&slot);
                    return Err(error);
                }
            };
            if !slot.begin_spawn() {
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("conversation creation session"));
            }
            let executable = runtime.settings.app_server_executable.clone();
            let args = runtime.settings.app_server_args.clone();
            let operation_runtime = runtime.clone();
            let operation_slot = slot.clone();
            let (snapshot, execution_slot, execution_generation, execution_session, incoming) = tokio::task::spawn_blocking(move || {
                let session = match CodexAppServerSession::spawn_uninitialized(&executable, &args) {
                    Ok(session) => session,
                    Err(error) => {
                        operation_runtime.unregister_session(&operation_slot);
                        return Err(CodexProtocolMapper::error(error));
                    }
                };
                if !operation_slot.register_spawned(session.clone())
                    || !operation_runtime.session_is_current(&operation_slot)
                {
                    let _ = session.shutdown();
                    operation_runtime.unregister_session(&operation_slot);
                    return Err(instance_session_cancelled_error(
                        "conversation creation session",
                    ));
                }
                if let Err(error) = session.initialize() {
                    let cancelled = operation_slot.cancelled.load(Ordering::SeqCst)
                        || !operation_runtime.session_is_current(&operation_slot);
                    let _ = session.shutdown();
                    operation_runtime.unregister_session(&operation_slot);
                    return Err(if cancelled {
                        instance_session_cancelled_error("conversation creation session")
                    } else {
                        CodexProtocolMapper::error(error)
                    });
                }
                let incoming = match session.subscribe() {
                    Ok(incoming) => incoming,
                    Err(error) => {
                        let _ = session.shutdown();
                        operation_runtime.unregister_session(&operation_slot);
                        return Err(CodexProtocolMapper::error(error));
                    }
                };
                let outcome = session.thread_start_outcome_with_sender(CodexThreadStartRequest {
                    workspace_root,
                    project_id,
                    permission_level,
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                }, |message| {
                    let _send_gate = lock(&operation_slot.send_gate);
                    let current = !operation_slot.cancelled.load(Ordering::SeqCst)
                        && operation_runtime.session_is_current(&operation_slot)
                        && lock(&operation_runtime.mutable).status == InstanceStatus::Ready;
                    if !current {
                        return CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown);
                    }
                    session.write_prepared_request(message)
                });
                let snapshot = match outcome {
                    CodexRequestOutcome::Success(snapshot) => snapshot,
                    outcome => {
                        let error = execution_outcome_error("thread/start", None, outcome);
                        let _ = session.shutdown();
                        operation_runtime.unregister_session(&operation_slot);
                        return Err(error);
                    }
                };
                let execution_slot = Arc::new(ExecutionSlot::new());
                if !execution_slot.set_starting_session(session.clone()) {
                    let _ = session.shutdown();
                    operation_runtime.unregister_session(&operation_slot);
                    return Err(execution_start_cancelled_error());
                }
                let execution_generation = session.generation().to_string();
                if !execution_slot.set_ready(ExecutionSession {
                    session: session.clone(),
                    generation: execution_generation.clone(),
                    active_turn_id: None,
                    interaction_expires_at: Some(
                        Instant::now() + operation_runtime.lifecycle_hook.interaction_lease_duration(),
                    ),
                }) {
                    let _ = session.shutdown();
                    operation_runtime.unregister_session(&operation_slot);
                    return Err(execution_start_cancelled_error());
                }
                operation_runtime.unregister_session(&operation_slot);
                Ok((snapshot, execution_slot, execution_generation, session, incoming))
            })
            .await
            .map_err(provider_task_error)??;
            let conversation = {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                if mutable.status != InstanceStatus::Ready
                    || mutable.executions.contains_key(&snapshot.thread.id)
                {
                    drop(mutable);
                    let _ = execution_session.shutdown();
                    return Err(instance_session_cancelled_error(
                        "conversation creation session",
                    ));
                }
                mutable
                    .executions
                    .insert(snapshot.thread.id.clone(), execution_slot);
                mutable
                    .pending_materialization
                    .insert(snapshot.thread.id.clone(), snapshot.clone());
                lock(&runtime.mapper).conversation(&snapshot)
            };
            runtime.start_execution_event_forwarder(
                snapshot.thread.id.clone(),
                execution_generation,
                execution_session,
                incoming,
            );
            Ok(ConversationCreateResponse { conversation })
        })
    }

    fn turn_start<'a>(
        &'a self,
        request: TurnStartRequest,
    ) -> ProtocolFuture<'a, TurnStartResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            let capabilities = lock(&runtime.mutable).capabilities.clone();
            if request.capability_revision != capabilities.revision {
                return Err(protocol_error(
                    "stale_capability_revision",
                    "turn.start capabilityRevision no longer matches the Provider instance"
                        .to_string(),
                    true,
                ));
            }
            if request.input.text.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_turn_input",
                    "turn.start text input must not be empty".to_string(),
                    false,
                ));
            }
            let input_text = request.input.text;
            let client_request_id = request.client_request_id;
            let requested_selection = request.selection;
            let pending_snapshot = lock(&runtime.mutable)
                .pending_materialization
                .get(&conversation_id)
                .cloned();
            let operation_runtime = runtime.clone();
            let operation_conversation_id = conversation_id.clone();
            let (turn, effective_selection) = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.start",
                    move |handle| {
                        let snapshot = match pending_snapshot.clone() {
                            Some(snapshot) => snapshot,
                            None => match handle.session.thread_read(&execution_conversation_id) {
                                Ok(snapshot) => snapshot,
                                Err(error) => {
                                    execution_runtime.release_execution(
                                        &execution_conversation_id,
                                        &handle.generation,
                                    );
                                    return Err(CodexProtocolMapper::error(error));
                                }
                            },
                        };
                        if let Some(active_turn) = snapshot
                            .thread
                            .turns
                            .iter()
                            .find(|turn| turn.status == CodexTurnStatus::InProgress)
                        {
                            handle
                                .slot
                                .mark_active_turn(&handle.generation, &active_turn.id);
                            return Err(protocol_error(
                                "turn_already_active",
                                "Codex conversation already has an active turn".to_string(),
                                false,
                            ));
                        }
                        if handle.slot.has_active_turn(&handle.generation) {
                            execution_runtime.finish_turn_execution(
                                &execution_conversation_id,
                                handle,
                                None,
                            );
                            if !handle.slot.matches_generation(&handle.generation) {
                                return Ok(ExecutionStep::Retry);
                            }
                        }
                        let effective_selection = match resolve_turn_selection(
                            &capabilities,
                            requested_selection.clone(),
                            TurnSelection {
                                access_mode_id: snapshot
                                    .permission_level
                                    .map(permission_level_id)
                                    .map(str::to_string),
                                reasoning_effort_id: snapshot.reasoning_effort,
                                model: snapshot.model.map(|model_id| {
                                    ModelSelection::FlatModelSelection(FlatModelSelection {
                                        kind: FlatModelCatalogKind::Flat,
                                        model_id,
                                    })
                                }),
                            },
                        ) {
                            Ok(selection) => selection,
                            Err(error) => {
                                execution_runtime
                                    .release_idle_execution(&execution_conversation_id, handle);
                                return Err(error);
                            }
                        };
                        let permission_level = match effective_selection
                            .access_mode_id
                            .as_deref()
                            .map(parse_permission_level)
                            .transpose()
                        {
                            Ok(permission_level) => permission_level,
                            Err(error) => {
                                execution_runtime
                                    .release_idle_execution(&execution_conversation_id, handle);
                                return Err(error);
                            }
                        };
                        let model = match effective_selection.model.as_ref() {
                            Some(ModelSelection::FlatModelSelection(selection)) => {
                                Some(selection.model_id.clone())
                            }
                            Some(ModelSelection::GroupedModelSelection(_)) => {
                                execution_runtime
                                    .release_idle_execution(&execution_conversation_id, handle);
                                return Err(protocol_error(
                                    "invalid_model_selection",
                                    "Codex Provider requires a flat model selection".to_string(),
                                    false,
                                ));
                            }
                            None => None,
                        };
                        let outcome = handle.session.turn_start_outcome(CodexTurnStartRequest {
                            thread_id: execution_conversation_id.clone(),
                            message: input_text.clone(),
                            client_message_id: Some(client_request_id.clone()),
                            permission_level,
                            model,
                            reasoning_effort: effective_selection.reasoning_effort_id.clone(),
                        });
                        match outcome {
                            CodexRequestOutcome::Success(turn) => {
                                if turn.status == CodexTurnStatus::InProgress {
                                    handle.slot.mark_active_turn(&handle.generation, &turn.id);
                                } else {
                                    execution_runtime.finish_turn_execution(
                                        &execution_conversation_id,
                                        handle,
                                        Some(&turn.id),
                                    );
                                }
                                Ok(ExecutionStep::Complete((turn, effective_selection)))
                            }
                            outcome => {
                                execution_runtime.release_execution(
                                    &execution_conversation_id,
                                    &handle.generation,
                                );
                                Err(execution_outcome_error(
                                    "turn/start",
                                    Some(&execution_conversation_id),
                                    outcome,
                                ))
                            }
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            let mapper = lock(&runtime.mapper);
            let user_item = mapper.turn_user_item(&conversation_id, &turn);
            let mapped_turn = mapper.turn(&conversation_id, &turn);
            Ok(TurnStartResponse {
                accepted: true,
                turn: mapped_turn,
                user_item,
                effective_selection,
            })
        })
    }

    fn turn_steer<'a>(
        &'a self,
        request: TurnSteerRequest,
    ) -> ProtocolFuture<'a, TurnSteerResponse> {
        Box::pin(async move {
            validate_same_resource_route(&request.conversation, &request.turn)?;
            let runtime = self.resource_instance(&request.conversation)?;
            let turn_id = request.turn.native_resource_id;
            let conversation_id = request.conversation.native_resource_id;
            let native_conversation_id = conversation_id.clone();
            let operation_conversation_id = conversation_id.clone();
            let expected_turn_id = turn_id.clone();
            let message = request.message;
            let client_message_id = request.client_message_id;
            let operation_runtime = runtime.clone();
            let turn = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.steer",
                    move |handle| {
                        let outcome = handle.session.turn_steer_outcome(CodexTurnSteerRequest {
                            thread_id: native_conversation_id.clone(),
                            expected_turn_id: expected_turn_id.clone(),
                            message: message.clone(),
                            client_message_id: Some(client_message_id.clone()),
                        });
                        match outcome {
                            CodexRequestOutcome::Success(turn) => {
                                if turn.status == CodexTurnStatus::InProgress {
                                    handle.slot.mark_active_turn(&handle.generation, &turn.id);
                                } else {
                                    execution_runtime.finish_turn_execution(
                                        &execution_conversation_id,
                                        handle,
                                        Some(&turn.id),
                                    );
                                }
                                Ok(ExecutionStep::Complete(turn))
                            }
                            outcome => {
                                execution_runtime.release_execution(
                                    &execution_conversation_id,
                                    &handle.generation,
                                );
                                Err(execution_outcome_error(
                                    "turn/steer",
                                    Some(&execution_conversation_id),
                                    outcome,
                                ))
                            }
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn);
            Ok(TurnSteerResponse {
                turn: mapped_turn,
            })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProtocolFuture<'a, TurnInterruptResponse> {
        Box::pin(async move {
            validate_same_resource_route(&request.conversation, &request.turn)?;
            let runtime = self.resource_instance(&request.conversation)?;
            let turn_id = request.turn.native_resource_id;
            let conversation_id = request.conversation.native_resource_id;
            let native_conversation_id = conversation_id.clone();
            let operation_conversation_id = conversation_id.clone();
            let native_turn_id = turn_id.clone();
            let operation_runtime = runtime.clone();
            let turn = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.interrupt",
                    move |handle| {
                        let outcome = handle.session.turn_interrupt_outcome(
                            &native_conversation_id,
                            &native_turn_id,
                        );
                        match outcome {
                            CodexRequestOutcome::Success(turn) => {
                                if turn.status == CodexTurnStatus::InProgress {
                                    handle.slot.mark_active_turn(&handle.generation, &turn.id);
                                } else {
                                    execution_runtime.finish_turn_execution(
                                        &execution_conversation_id,
                                        handle,
                                        Some(&turn.id),
                                    );
                                }
                                Ok(ExecutionStep::Complete(turn))
                            }
                            outcome => {
                                execution_runtime.release_execution(
                                    &execution_conversation_id,
                                    &handle.generation,
                                );
                                Err(execution_outcome_error(
                                    "turn/interrupt",
                                    Some(&execution_conversation_id),
                                    outcome,
                                ))
                            }
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn);
            Ok(TurnInterruptResponse {
                turn: mapped_turn,
            })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.approval)?;
            let approval_id = request.approval.native_resource_id.clone();
            let resource_generation = approval_generation(&approval_id).ok_or_else(|| {
                protocol_error(
                    "invalid_approval_resource",
                    "approval nativeResourceId does not contain a session generation".to_string(),
                    false,
                )
            })?;
            if !runtime.has_execution_generation(resource_generation) {
                return Err(protocol_error(
                    "stale_approval_session",
                    "approval belongs to a closed Codex execution session".to_string(),
                    false,
                ));
            }
            let pending = {
                let mutable = lock(&runtime.mutable);
                mutable
                    .pending_approvals
                    .get(&approval_id)
                    .cloned()
            }
                .ok_or_else(|| {
                    protocol_error(
                        "approval_not_found",
                        format!("approval {approval_id} is not pending in this Provider instance"),
                        false,
                    )
                })?;
            if pending.request.session_generation != resource_generation
                || pending.approval.resource != request.approval
            {
                return Err(protocol_error(
                    "stale_approval_session",
                    "approval route does not match the owning Codex App Server session".to_string(),
                    false,
                ));
            }
            let conversation_id = pending.request.thread_id.clone();
            let operation_runtime = runtime.clone();
            let operation_conversation_id = conversation_id.clone();
            let pending_request = pending.request.clone();
            let pending_approval = pending.approval;
            let operation_approval_id = approval_id.clone();
            let operation_generation = resource_generation.to_string();
            let decision = request.decision;
            let approval = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "approval.resolve",
                    move |handle| {
                        if handle.generation != operation_generation {
                            execution_runtime.release_idle_execution(
                                &execution_conversation_id,
                                handle,
                            );
                            return Err(protocol_error(
                                "stale_approval_session",
                                "approval belongs to a closed Codex execution session".to_string(),
                                false,
                            ));
                        }
                        match handle
                            .session
                            .respond_to_approval_outcome(&pending_request, decision)
                        {
                            CodexRequestOutcome::Success(()) => {
                                lock(&execution_runtime.mutable)
                                    .pending_approvals
                                    .remove(&operation_approval_id);
                                let (approval, event) = lock(&execution_runtime.mapper)
                                    .approval_resolved(
                                        pending_approval.clone(),
                                        decision,
                                        now_ms(),
                                    )?;
                                {
                                    let mut mutable = lock(&execution_runtime.mutable);
                                    update_recorded_approval(&mut mutable, &approval);
                                }
                                if let Err(error) = execution_runtime.events.publish(event) {
                                    execution_runtime.release_execution(
                                        &execution_conversation_id,
                                        &handle.generation,
                                    );
                                    return Err(error);
                                }
                                Ok(ExecutionStep::Complete(approval))
                            }
                            outcome => {
                                execution_runtime.release_execution(
                                    &execution_conversation_id,
                                    &handle.generation,
                                );
                                Err(execution_outcome_error(
                                    "approval/resolve",
                                    Some(&execution_conversation_id),
                                    outcome,
                                ))
                            }
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            Ok(ApprovalResolveResponse { approval })
        })
    }

    fn provider_shutdown<'a>(
        &'a self,
        _request: ProviderShutdownRequest,
    ) -> ProtocolFuture<'a, ProviderShutdownResponse> {
        Box::pin(async move {
            if self.shutdown.swap(true, Ordering::SeqCst) {
                loop {
                    let notified = self.shutdown_changed.notified();
                    if self.shutdown_complete.load(Ordering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
                return Ok(ProviderShutdownResponse { accepted: true });
            }
            let instances = lock(&self.state)
                .instances
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let mut first_error = None;
            for runtime in instances {
                if let Err(error) = runtime.stop().await {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            self.shutdown_complete.store(true, Ordering::SeqCst);
            self.shutdown_changed.notify_waiters();
            if let Some(error) = first_error {
                return Err(error);
            }
            Ok(ProviderShutdownResponse { accepted: true })
        })
    }
}

fn prepare_conversation_workspace(
    workspace_root: Option<&str>,
    workspace_mode: Option<&str>,
) -> Result<Option<String>, ProtocolError> {
    let mode = workspace_mode.unwrap_or("main");
    if mode == "main" {
        return workspace_root.map(ensure_conversation_workspace).transpose();
    }
    if mode != "worktree" {
        return Err(protocol_error(
            "unsupported_workspace_mode",
            format!("Codex Provider does not support workspace mode {mode}"),
            false,
        ));
    }
    let requested = workspace_root.ok_or_else(|| {
        protocol_error(
            "workspace_required",
            "Codex worktree mode requires workspaceRoot".to_string(),
            false,
        )
    })?;
    let requested = Path::new(requested);
    if !requested.is_absolute() || !requested.is_dir() {
        return Err(protocol_error(
            "invalid_workspace_root",
            "Codex worktree workspaceRoot must be an existing absolute directory".to_string(),
            false,
        ));
    }
    let codex_home = codex_home().ok_or_else(|| {
        protocol_error(
            "worktree_create_failed",
            "cannot resolve Codex home for managed worktrees".to_string(),
            false,
        )
    })?;
    create_managed_worktree(requested, &codex_home).map(Some)
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".codex"))
        })
}

fn ensure_conversation_workspace(path: &str) -> Result<String, ProtocolError> {
    let workspace = Path::new(path);
    if !workspace.is_absolute() {
        return Err(protocol_error(
            "invalid_workspace_root",
            "Codex workspaceRoot must be an absolute path".to_string(),
            false,
        ));
    }
    std::fs::create_dir_all(workspace).map_err(|error| {
        protocol_error(
            "workspace_create_failed",
            format!("failed to create Codex workspaceRoot: {error}"),
            false,
        )
    })?;
    Ok(path.to_string())
}

fn create_managed_worktree(
    requested: &Path,
    codex_home: &Path,
) -> Result<String, ProtocolError> {
    let requested = requested.canonicalize().map_err(|error| {
        protocol_error(
            "invalid_workspace_root",
            format!("failed to resolve workspaceRoot: {error}"),
            false,
        )
    })?;
    let output = Command::new("git")
        .arg("-C")
        .arg(&requested)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to inspect Git workspace: {error}"),
                true,
            )
        })?;
    if !output.status.success() {
        return Err(protocol_error(
            "workspace_not_git_repository",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
            false,
        ));
    }
    let repository_root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        .canonicalize()
        .map_err(|error| {
            protocol_error(
                "invalid_workspace_root",
                format!("failed to resolve Git repository root: {error}"),
                false,
            )
        })?;
    let relative_cwd = requested.strip_prefix(&repository_root).map_err(|_| {
        protocol_error(
            "invalid_workspace_root",
            "workspaceRoot is outside the discovered Git repository".to_string(),
            false,
        )
    })?;
    let project_name = repository_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("workspace");
    let unique = format!(
        "remote-{}-{}-{}",
        now_ms(),
        std::process::id(),
        NEXT_MANAGED_WORKTREE.fetch_add(1, Ordering::SeqCst),
    );
    let worktree_root = codex_home.join("worktrees").join(unique).join(project_name);
    if let Some(parent) = worktree_root.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to prepare managed worktree directory: {error}"),
                true,
            )
        })?;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(&repository_root)
        .args(["worktree", "add", "--detach"])
        .arg(&worktree_root)
        .arg("HEAD")
        .output()
        .map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to start git worktree: {error}"),
                true,
            )
        })?;
    if !output.status.success() {
        return Err(protocol_error(
            "worktree_create_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
            false,
        ));
    }
    let worktree_cwd = worktree_root.join(relative_cwd);
    std::fs::create_dir_all(&worktree_cwd).map_err(|error| {
        protocol_error(
            "worktree_create_failed",
            format!("failed to prepare worktree working directory: {error}"),
            true,
        )
    })?;
    Ok(worktree_cwd.to_string_lossy().into_owned())
}

#[cfg(test)]
mod workspace_mode_tests {
    use super::{create_managed_worktree, ensure_conversation_workspace};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn creates_a_missing_standalone_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("task-1");

        let prepared = ensure_conversation_workspace(workspace.to_str().unwrap()).unwrap();

        assert_eq!(Path::new(&prepared), workspace);
        assert!(workspace.is_dir());
    }

    #[test]
    fn creates_a_detached_managed_worktree_and_preserves_relative_cwd() {
        let fixture = tempfile::tempdir().unwrap();
        let repository = fixture.path().join("project");
        let nested = repository.join("packages/app");
        fs::create_dir_all(&nested).unwrap();
        git(&repository, &["init"]);
        git(&repository, &["config", "user.email", "test@codepet.dev"]);
        git(&repository, &["config", "user.name", "CodePet Test"]);
        fs::write(repository.join("README.md"), "fixture").unwrap();
        git(&repository, &["add", "README.md"]);
        git(&repository, &["commit", "-m", "fixture"]);

        let codex_home = fixture.path().join("codex-home");
        let cwd = create_managed_worktree(&nested, &codex_home).unwrap();
        let cwd = Path::new(&cwd);

        assert!(cwd.is_dir());
        assert!(cwd.starts_with(codex_home.join("worktrees")));
        assert_eq!(cwd.file_name().and_then(|value| value.to_str()), Some("app"));
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    }

    fn git(repository: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
}

fn execution_outcome_error<T>(
    operation: &str,
    conversation_id: Option<&str>,
    outcome: CodexRequestOutcome<T>,
) -> ProtocolError {
    match outcome {
        CodexRequestOutcome::ExplicitRpcReject(error @ CodexAppServerError::Rpc { .. })
            if operation == "thread/resume"
                && conversation_id.is_some_and(|conversation_id| {
                    is_active_writer_conflict(&error, conversation_id)
                }) =>
        {
            ProtocolError {
                code: "conversation_write_conflict".to_string(),
                message: "Conversation is being written by another runtime".to_string(),
                retryable: true,
                details: Some(
                    [
                        ("operation".to_string(), json!(operation)),
                        ("reason".to_string(), json!("owned-by-other-runtime")),
                    ]
                    .into_iter()
                    .collect(),
                ),
            }
        }
        CodexRequestOutcome::NotSent(error)
        | CodexRequestOutcome::ExplicitRpcReject(error)
        | CodexRequestOutcome::SentOutcomeUnknown(error) => CodexProtocolMapper::error(error),
        CodexRequestOutcome::Success(_) => unreachable!("successful outcome is not an error"),
    }
}

struct LoadedConversationTurns {
    turns: Vec<CodexTurn>,
    next_cursor: Option<String>,
    materialized: bool,
}

fn load_conversation_turns(
    session: &CodexAppServerSession,
    conversation_id: &str,
    initial_cursor: Option<String>,
    requested_limit: u64,
    used_pending_snapshot: bool,
) -> Result<LoadedConversationTurns, ProtocolError> {
    let harness_version = session.harness_version();
    if harness_uses_item_pagination(harness_version.as_deref()) {
        let mut loaded = load_conversation_turn_pages(
            session,
            conversation_id,
            initial_cursor.clone(),
            requested_limit,
            used_pending_snapshot,
            CodexTurnItemsView::NotLoaded,
        )?;
        if !loaded.materialized || loaded.turns.is_empty() {
            return Ok(loaded);
        }
        match hydrate_turn_items(session, conversation_id, &mut loaded.turns) {
            Ok(ItemPaginationProbe::Supported) => return Ok(loaded),
            Ok(ItemPaginationProbe::Unsupported) => {
                if harness_allows_full_turn_fallback(harness_version.as_deref()) {
                    return load_conversation_turn_pages(
                        session,
                        conversation_id,
                        initial_cursor,
                        requested_limit,
                        used_pending_snapshot,
                        CodexTurnItemsView::Full,
                    );
                }
                for turn in &mut loaded.turns {
                    set_history_placeholder(turn);
                }
                return Ok(loaded);
            }
            Err(()) => return Ok(loaded),
        }
    }
    load_conversation_turn_pages(
        session,
        conversation_id,
        initial_cursor,
        requested_limit,
        used_pending_snapshot,
        CodexTurnItemsView::Full,
    )
}

fn load_conversation_turn_pages(
    session: &CodexAppServerSession,
    conversation_id: &str,
    initial_cursor: Option<String>,
    requested_limit: u64,
    used_pending_snapshot: bool,
    items_view: CodexTurnItemsView,
) -> Result<LoadedConversationTurns, ProtocolError> {
    let mut turns = Vec::new();
    let mut cursor = initial_cursor;
    let mut remaining_turns = requested_limit;
    let mut response_next_cursor = None;
    let mut seen_cursors = HashSet::new();
    let mut page_count = 0;
    loop {
        if page_count == MAX_THREAD_TURN_PAGES {
            return Err(protocol_error(
                "provider_protocol_error",
                format!("thread/turns/list exceeded the {MAX_THREAD_TURN_PAGES}-page limit"),
                false,
            ));
        }
        page_count += 1;
        let upstream_limit = remaining_turns.min(u64::from(THREAD_TURNS_PAGE_LIMIT));
        let page = match session.thread_turns_list_with_view(
            conversation_id,
            cursor.clone(),
            upstream_limit as u32,
            items_view,
        ) {
            Ok(page) => page,
            Err(error)
                if cursor.is_none()
                    && (error.is_thread_turns_unavailable_before_first_user_message(
                        conversation_id,
                    ) || used_pending_snapshot
                        && (error.is_thread_not_loaded(conversation_id)
                            || is_created_conversation_not_ready(&error, conversation_id))) =>
            {
                return Ok(LoadedConversationTurns {
                    turns,
                    next_cursor: None,
                    materialized: false,
                });
            }
            Err(error) => return Err(CodexProtocolMapper::error(error)),
        };
        if page.data.len() as u64 > upstream_limit {
            return Err(protocol_error(
                "provider_protocol_error",
                format!(
                    "thread/turns/list returned {} turns for limit {upstream_limit}",
                    page.data.len()
                ),
                false,
            ));
        }
        let returned_turns = page.data.len() as u64;
        let next_cursor = page.next_cursor;
        turns.extend(page.data);
        remaining_turns = remaining_turns.saturating_sub(returned_turns);
        if remaining_turns == 0 {
            response_next_cursor = next_cursor;
            break;
        }
        match next_cursor {
            Some(next_cursor) if seen_cursors.insert(next_cursor.clone()) => {
                cursor = Some(next_cursor);
            }
            Some(_) => {
                return Err(protocol_error(
                    "provider_protocol_error",
                    "thread/turns/list returned a repeated cursor".to_string(),
                    false,
                ));
            }
            None => break,
        }
    }
    Ok(LoadedConversationTurns {
        turns,
        next_cursor: response_next_cursor,
        materialized: true,
    })
}

enum ItemPaginationProbe {
    Supported,
    Unsupported,
}

fn hydrate_turn_items(
    session: &CodexAppServerSession,
    conversation_id: &str,
    turns: &mut [CodexTurn],
) -> Result<ItemPaginationProbe, ()> {
    let mut method_was_observed = false;
    let mut history_failed = false;
    for turn in turns {
        if history_failed {
            set_history_placeholder(turn);
            continue;
        }
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        let mut items = Vec::new();
        for page_count in 0..MAX_THREAD_TURN_PAGES {
            let page = match session.thread_items_list(
                conversation_id,
                &turn.id,
                cursor.clone(),
                1,
            ) {
                Ok(page) => {
                    method_was_observed = true;
                    page
                }
                Err(error) if !method_was_observed && error.is_method_not_found() => {
                    return Ok(ItemPaginationProbe::Unsupported);
                }
                Err(_) => {
                    set_history_placeholder(turn);
                    history_failed = true;
                    break;
                }
            };
            items.extend(page.data.into_iter().map(|entry| entry.item));
            match page.next_cursor {
                Some(next_cursor) if seen_cursors.insert(next_cursor.clone()) => {
                    cursor = Some(next_cursor);
                }
                Some(_) => {
                    set_history_placeholder(turn);
                    history_failed = true;
                    break;
                }
                None => {
                    turn.items = items;
                    break;
                }
            }
            if page_count + 1 == MAX_THREAD_TURN_PAGES {
                set_history_placeholder(turn);
                history_failed = true;
            }
        }
    }
    if history_failed {
        Err(())
    } else {
        Ok(ItemPaginationProbe::Supported)
    }
}

fn set_history_placeholder(turn: &mut CodexTurn) {
    turn.items = vec![CodexThreadItem::Unknown {
        id: format!("{}:history-not-loaded", turn.id),
    }];
}

fn harness_uses_item_pagination(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return false;
    };
    let mut components = version.split(['.', '-']);
    let major = components.next().and_then(|value| value.parse::<u64>().ok());
    let minor = components.next().and_then(|value| value.parse::<u64>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if major > 0 || minor >= 151)
}

fn harness_allows_full_turn_fallback(version: Option<&str>) -> bool {
    !harness_uses_item_pagination(version)
        || !matches!(
            version.map(|version| {
                let mut components = version.split(['.', '-']);
                (
                    components.next().and_then(|value| value.parse::<u64>().ok()),
                    components.next().and_then(|value| value.parse::<u64>().ok()),
                )
            }),
            Some((Some(major), Some(minor))) if major > 0 || minor >= 152
        )
}

fn runtime_inventory(candidates: Vec<RuntimeCandidate>, selected: Option<RuntimeInstallation>, product: &str) -> RuntimeGetInstalledResponse {
    let mut seen = HashSet::new();
    let installed = candidates
        .into_iter()
        .filter_map(|candidate| inspect_runtime_candidate(candidate, product).ok())
        .filter(|installation| seen.insert(installation.executable_path.clone()))
        .collect::<Vec<_>>();
    let selected = selected.and_then(|selected| installed.iter()
        .find(|installation| installation.executable_path == selected.executable_path).cloned());
    RuntimeGetInstalledResponse { installed, selected }
}

fn discover_codex_candidates() -> Vec<RuntimeCandidate> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("CODE_PET_CODEX_BIN").filter(|value| !value.is_empty()) {
        candidates.push(RuntimeCandidate { executable_path: PathBuf::from(path).to_string_lossy().into_owned(), source: codepet_provider_sdk::RuntimeCandidateSource::Environment });
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            candidates.push(RuntimeCandidate { executable_path: directory.join("codex").to_string_lossy().into_owned(), source: codepet_provider_sdk::RuntimeCandidateSource::CurrentPath });
        }
    }
    if let Some(path) = discover_login_shell_command("codex") {
        candidates.push(RuntimeCandidate { executable_path: path, source: codepet_provider_sdk::RuntimeCandidateSource::LoginShell });
    }
    for path in [
        "/Applications/Codex.app/Contents/Resources/codex",
        "/Applications/ChatGPT.app/Contents/Resources/codex",
    ] {
        candidates.push(RuntimeCandidate { executable_path: path.to_string(), source: codepet_provider_sdk::RuntimeCandidateSource::MacosApplication });
    }
    candidates
}

fn discover_login_shell_command(command_name: &str) -> Option<String> {
    let shell = std::env::var_os("SHELL").map(PathBuf::from).filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
    let output = Command::new(shell).args(["-lc", &format!("command -v {command_name}")]).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|path| !path.is_empty())
}

fn inspect_runtime_candidate(
    candidate: RuntimeCandidate,
    product: &str,
) -> Result<RuntimeInstallation, ProtocolError> {
    let path = PathBuf::from(&candidate.executable_path);
    if !path.is_absolute() || !path.is_file() {
        return Err(protocol_error(
            "invalid_runtime_selection",
            format!("Runtime executable is unavailable: {}", path.display()),
            false,
        ));
    }
    let canonical = std::fs::canonicalize(&path).map_err(|error| {
        protocol_error(
            "invalid_runtime_selection",
            format!("Resolve runtime executable {}: {error}", path.display()),
            false,
        )
    })?;
    let line = bounded_runtime_version(&canonical)?;
    if line.is_empty() || !line.to_ascii_lowercase().contains(product) {
        return Err(protocol_error(
            "invalid_runtime_selection",
            format!("Runtime executable does not identify itself as {product}"),
            false,
        ));
    }
    let version = line
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|character| character.is_ascii_digit()))
        .unwrap_or(&line)
        .trim_start_matches('v')
        .to_string();
    Ok(RuntimeInstallation {
        executable_path: canonical.to_string_lossy().into_owned(),
        version,
        source: candidate.source,
    })
}

fn bounded_runtime_version(executable: &Path) -> Result<String, ProtocolError> {
    let mut child = Command::new(executable).arg("--version")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|error| {
        protocol_error(
            "invalid_runtime_selection",
            format!("Run runtime executable {}: {error}", executable.display()),
            false,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().map_err(|error| protocol_error(
                    "invalid_runtime_selection", format!("Read runtime version: {error}"), false))?;
                if !status.success() {
                    return Err(protocol_error("invalid_runtime_selection", format!("Runtime executable rejected --version: {status}"), false));
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Ok(stdout.lines().chain(stderr.lines()).find(|line| !line.trim().is_empty())
                    .unwrap_or_default().trim().to_string());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(protocol_error("invalid_runtime_selection", "Runtime version probe timed out".to_string(), true));
            }
            Err(error) => return Err(protocol_error("invalid_runtime_selection", format!("Inspect runtime version probe: {error}"), false)),
        }
    }
}

fn is_created_conversation_not_ready(
    error: &CodexAppServerError,
    conversation_id: &str,
) -> bool {
    if error.is_thread_not_loaded(conversation_id) {
        return true;
    }
    let CodexAppServerError::Rpc { message, .. } = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    (message.contains("failed to read session metadata")
        && (message.contains("is empty")
            || message.contains("not found")
            || message.contains("no such file")))
        || message.contains("thread not found")
}

fn is_active_writer_conflict(error: &CodexAppServerError, conversation_id: &str) -> bool {
    let CodexAppServerError::Rpc {
        code,
        message,
        data,
    } = error
    else {
        return false;
    };
    *code == -32600
        && data.is_none()
        && message == &format!("thread {conversation_id} already has an active writer")
}

fn execution_start_cancelled_error() -> ProtocolError {
    protocol_error(
        "provider_unavailable",
        "Codex Provider stopped while the execution session was starting".to_string(),
        true,
    )
}

fn instance_session_cancelled_error(session: &str) -> ProtocolError {
    protocol_error(
        "provider_unavailable",
        format!("Codex Provider stopped while the {session} was starting"),
        true,
    )
}

fn incoming_conversation_id(incoming: &CodexIncoming) -> Option<&str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::ThreadStarted { snapshot }) => {
            Some(&snapshot.thread.id)
        }
        CodexIncoming::Notification(
            CodexNotification::ThreadNameUpdated { thread_id, .. }
            | CodexNotification::ThreadStatusChanged { thread_id, .. }
            | CodexNotification::TurnStarted { thread_id, .. }
            | CodexNotification::TurnCompleted { thread_id, .. }
            | CodexNotification::ItemUpserted { thread_id, .. }
            | CodexNotification::OutputDelta { thread_id, .. }
            | CodexNotification::ServerRequestResolved { thread_id, .. },
        ) => Some(thread_id),
        CodexIncoming::ApprovalRequested(request) => Some(&request.thread_id),
        CodexIncoming::Notification(CodexNotification::ProjectChanged { .. })
        | CodexIncoming::Notification(CodexNotification::Unknown { .. })
        | CodexIncoming::UnsupportedServerRequest { .. } => None,
    }
}

fn incoming_active_turn_id<'a>(
    incoming: &'a CodexIncoming,
    session_generation: &str,
) -> Option<&'a str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::TurnStarted { turn, .. }) => Some(&turn.id),
        CodexIncoming::Notification(
            CodexNotification::ItemUpserted { turn_id, .. }
            | CodexNotification::OutputDelta { turn_id, .. },
        ) => Some(turn_id),
        CodexIncoming::ApprovalRequested(request)
            if request.session_generation == session_generation => Some(&request.turn_id),
        _ => None,
    }
}

fn shutdown_sessions(
    sessions: Vec<CodexAppServerSession>,
) -> Result<(), CodexAppServerError> {
    let mut first_error = None;
    for session in sessions {
        if let Err(error) = session.shutdown() {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn record_approval(
    mutable: &mut InstanceMutable,
    item_id: String,
    approval: ProviderApproval,
) {
    if let Some(observed) = mutable.approval_history.iter_mut().find(|observed| {
        observed.approval.resource.native_resource_id == approval.resource.native_resource_id
    }) {
        observed.item_id = item_id;
        observed.approval = approval;
    } else {
        mutable.approval_history.push(ObservedApproval { item_id, approval });
    }
}

fn update_recorded_approval(mutable: &mut InstanceMutable, approval: &ProviderApproval) {
    if let Some(observed) = mutable.approval_history.iter_mut().find(|observed| {
        observed.approval.resource.native_resource_id == approval.resource.native_resource_id
    }) {
        observed.approval = approval.clone();
    }
}

fn permission_level_id(permission: crate::protocol::CodexPermissionLevel) -> &'static str {
    match permission {
        crate::protocol::CodexPermissionLevel::ReadOnly => "read-only",
        crate::protocol::CodexPermissionLevel::WorkspaceWrite => "workspace-write",
        crate::protocol::CodexPermissionLevel::FullAccess => "full-access",
    }
}

fn resolve_turn_selection(
    capabilities: &ProviderCapabilities,
    requested: TurnSelection,
    current: TurnSelection,
) -> Result<TurnSelection, ProtocolError> {
    let turn_send = capabilities.turn_send.as_ref().ok_or_else(|| {
        protocol_error(
            "capability_unsupported",
            "Provider does not advertise turn.start controls".to_string(),
            false,
        )
    })?;
    let access_mode_id = resolve_choice(
        turn_send.access_mode.as_ref(),
        requested.access_mode_id,
        current.access_mode_id,
        "accessModeId",
    )?;
    let reasoning_effort_id = resolve_choice(
        turn_send.reasoning_effort.as_ref(),
        requested.reasoning_effort_id,
        current.reasoning_effort_id,
        "reasoningEffortId",
    )?;
    let model = match turn_send.model_catalog.as_ref() {
        None => {
            if requested.model.is_some() {
                return Err(protocol_error(
                    "unsupported_turn_control",
                    "model must be omitted when modelCatalog is unavailable".to_string(),
                    false,
                ));
            }
            None
        }
        Some(ModelCatalog::FlatModelCatalog(catalog)) => {
            let requested_model = match requested.model {
                Some(ModelSelection::FlatModelSelection(selection)) => Some(selection.model_id),
                Some(ModelSelection::GroupedModelSelection(_)) => {
                    return Err(protocol_error(
                        "invalid_model_selection",
                        "flat modelCatalog requires modelId without providerId".to_string(),
                        false,
                    ));
                }
                None => None,
            };
            let current_model = match current.model {
                Some(ModelSelection::FlatModelSelection(selection)) => Some(selection.model_id),
                _ => None,
            };
            let model_id = requested_model
                .or_else(|| {
                    current_model.filter(|id| catalog.models.iter().any(|option| option.id == *id))
                })
                .or_else(|| {
                    catalog
                        .default_selection
                        .as_ref()
                        .map(|selection| selection.model_id.clone())
                });
            match model_id {
                Some(model_id) => {
                    validate_choice(&catalog.models, &model_id, "model")?;
                    Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                        kind: FlatModelCatalogKind::Flat,
                        model_id,
                    }))
                }
                None => None,
            }
        }
        Some(ModelCatalog::GroupedModelCatalog(_)) => {
            return Err(protocol_error(
                "provider_capability_invalid",
                "Codex Provider cannot execute a grouped modelCatalog".to_string(),
                false,
            ));
        }
    };
    Ok(TurnSelection {
        access_mode_id,
        reasoning_effort_id,
        model,
    })
}

fn resolve_choice(
    control: Option<&codepet_provider_sdk::ChoiceSet>,
    requested: Option<String>,
    current: Option<String>,
    field: &str,
) -> Result<Option<String>, ProtocolError> {
    let Some(control) = control else {
        if requested.is_some() {
            return Err(protocol_error(
                "unsupported_turn_control",
                format!("{field} must be omitted when its control is unavailable"),
                false,
            ));
        }
        return Ok(None);
    };
    let selected = requested
        .or_else(|| {
            current.filter(|id| control.options.iter().any(|option| option.id == *id))
        })
        .or_else(|| control.default_id.clone());
    if let Some(selected) = selected.as_deref() {
        validate_choice(&control.options, selected, field)?;
    }
    Ok(selected)
}

fn validate_choice(
    options: &[codepet_provider_sdk::ChoiceOption],
    selected: &str,
    field: &str,
) -> Result<(), ProtocolError> {
    let option = options.iter().find(|option| option.id == selected).ok_or_else(|| {
        protocol_error(
            "invalid_turn_selection",
            format!("unknown {field} selection: {selected}"),
            false,
        )
    })?;
    if option.enabled == Some(false) {
        return Err(protocol_error(
            "disabled_turn_selection",
            option
                .disabled_reason
                .clone()
                .unwrap_or_else(|| format!("{field} selection {selected} is disabled")),
            false,
        ));
    }
    Ok(())
}

fn decode_settings(settings: codepet_provider_sdk::JsonObject) -> Result<CodexInstanceSettings, ProtocolError> {
    let value = Value::Object(settings.into_iter().collect());
    let settings: CodexInstanceSettings = serde_json::from_value(value).map_err(|error| {
        protocol_error(
            "invalid_instance_settings",
            format!("invalid Codex instance settings: {error}"),
            false,
        )
    })?;
    if !settings.app_server_executable.is_absolute() {
        return Err(protocol_error(
            "invalid_instance_settings",
            "appServerExecutable must be an absolute path resolved by the Host".to_string(),
            false,
        ));
    }
    Ok(settings)
}

fn validate_route(route: &ProviderInstanceRoute) -> Result<(), ProtocolError> {
    if route.device_id.trim().is_empty()
        || route.provider_plugin_id.trim().is_empty()
        || route.provider_instance_id.trim().is_empty()
    {
        return Err(protocol_error(
            "invalid_provider_route",
            "deviceId, providerPluginId, and providerInstanceId must not be empty".to_string(),
            false,
        ));
    }
    if route.provider_plugin_id != CODEX_PLUGIN_ID {
        return Err(protocol_error(
            "wrong_provider_plugin_route",
            format!(
                "Codex Provider cannot serve plugin {}",
                route.provider_plugin_id
            ),
            false,
        ));
    }
    Ok(())
}

fn validate_resource(resource: &RoutedResourceId) -> Result<(), ProtocolError> {
    validate_route(&ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
        provider_plugin_id: resource.provider_plugin_id.clone(),
        provider_instance_id: resource.provider_instance_id.clone(),
    })?;
    if resource.native_resource_id.trim().is_empty() {
        return Err(protocol_error(
            "invalid_provider_resource",
            "nativeResourceId must not be empty".to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_same_resource_route(
    left: &RoutedResourceId,
    right: &RoutedResourceId,
) -> Result<(), ProtocolError> {
    validate_resource(left)?;
    validate_resource(right)?;
    if left.device_id != right.device_id
        || left.provider_plugin_id != right.provider_plugin_id
        || left.provider_instance_id != right.provider_instance_id
    {
        return Err(protocol_error(
            "mismatched_provider_route",
            "resources must target the same device, Provider plugin, and Provider instance"
                .to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_resource_route(
    resource: &RoutedResourceId,
    route: &ProviderInstanceRoute,
) -> Result<(), ProtocolError> {
    validate_resource(resource)?;
    validate_route(route)?;
    if resource.device_id != route.device_id
        || resource.provider_plugin_id != route.provider_plugin_id
        || resource.provider_instance_id != route.provider_instance_id
    {
        return Err(protocol_error(
            "mismatched_provider_route",
            "resource must target the requested device, Provider plugin, and Provider instance"
                .to_string(),
            false,
        ));
    }
    Ok(())
}

fn require_capability(
    runtime: &CodexInstanceRuntime,
    capability: ProviderCapability,
) -> Result<(), ProtocolError> {
    if lock(&runtime.mutable)
        .capabilities
        .methods
        .contains(&capability)
    {
        return Ok(());
    }
    Err(protocol_error(
        "capability_unsupported",
        format!("Codex Provider does not advertise {capability:?}"),
        false,
    ))
}

fn validate_project_fields(
    name: &str,
    roots: &[codepet_provider_sdk::ProjectRoot],
) -> Result<(), ProtocolError> {
    if name.trim().is_empty() {
        return Err(protocol_error(
            "invalid_request",
            "project name must not be empty".to_string(),
            false,
        ));
    }
    if roots.is_empty() || roots.iter().any(|root| root.path.trim().is_empty()) {
        return Err(protocol_error(
            "invalid_request",
            "project roots must contain non-empty paths".to_string(),
            false,
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CodexConversationMembershipFilter {
    All,
    Standalone,
    Project(String),
}

impl CodexConversationMembershipFilter {
    fn cursor_filter(&self) -> Option<String> {
        match self {
            Self::All => None,
            Self::Standalone => Some("standalone".to_string()),
            Self::Project(project_id) => Some(format!("project:{project_id}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CodexConversationMembership {
    Project(String),
    UnmappedProject,
    Standalone,
}

#[derive(Clone, Debug, Default)]
struct CodexDesktopProjectAssignments {
    by_thread: HashMap<String, Option<String>>,
}

impl CodexDesktopProjectAssignments {
    fn membership(&self, snapshot: &CodexConversationSnapshot) -> CodexConversationMembership {
        self.membership_for(
            &snapshot.thread.id,
            snapshot.thread.project_id.as_deref(),
        )
    }

    fn membership_for(
        &self,
        thread_id: &str,
        native_project_id: Option<&str>,
    ) -> CodexConversationMembership {
        if let Some(project_id) = native_project_id.filter(|project_id| !project_id.is_empty()) {
            return CodexConversationMembership::Project(project_id.to_string());
        }
        match self.by_thread.get(thread_id) {
            Some(Some(project_id)) => CodexConversationMembership::Project(project_id.clone()),
            Some(None) => CodexConversationMembership::UnmappedProject,
            None => CodexConversationMembership::Standalone,
        }
    }

    fn decorate(&self, snapshot: &mut CodexConversationSnapshot) -> CodexConversationMembership {
        let membership = self.membership(snapshot);
        if let CodexConversationMembership::Project(project_id) = &membership {
            snapshot.thread.project_id = Some(project_id.clone());
        }
        membership
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FilteredConversationCursor {
    version: u8,
    filter: String,
    upstream_cursor: String,
}

fn list_codex_conversations(
    session: &CodexAppServerSession,
    mut request: CodexThreadListRequest,
    membership_filter: CodexConversationMembershipFilter,
    assignments: &CodexDesktopProjectAssignments,
) -> Result<crate::protocol::CodexThreadPage, CodexAppServerError> {
    if membership_filter == CodexConversationMembershipFilter::All {
        let mut page = session.thread_list(request)?;
        for snapshot in &mut page.data {
            assignments.decorate(snapshot);
        }
        return Ok(page);
    }

    let cursor_filter = membership_filter
        .cursor_filter()
        .expect("filtered conversation queries always have a cursor filter");
    request.cursor = decode_filtered_conversation_cursor(request.cursor, &cursor_filter)?;
    let target_limit = request.limit.unwrap_or(CODEX_LIST_PAGE_LIMIT);
    let mut conversations = Vec::new();
    let mut next_cursor = request.cursor.clone();
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_FILTERED_THREAD_PAGES {
        if target_limit == 0 {
            break;
        }
        let cursor_key = next_cursor.clone().unwrap_or_default();
        if !seen_cursors.insert(cursor_key) {
            return Err(CodexAppServerError::Protocol(
                "thread/list repeated a cursor while merging project membership".to_string(),
            ));
        }
        request.cursor = next_cursor;
        request.limit = Some(target_limit.saturating_sub(conversations.len() as u32));
        let mut page = session.thread_list(request.clone())?;
        next_cursor = page.next_cursor;
        for mut snapshot in page.data.drain(..) {
            let membership = assignments.decorate(&mut snapshot);
            let matches = match (&membership_filter, membership) {
                (
                    CodexConversationMembershipFilter::Project(expected),
                    CodexConversationMembership::Project(actual),
                ) => expected == &actual,
                (
                    CodexConversationMembershipFilter::Standalone,
                    CodexConversationMembership::Standalone,
                ) => true,
                _ => false,
            };
            if matches {
                conversations.push(snapshot);
            }
        }
        if conversations.len() >= target_limit as usize {
            if let Some(cursor) = next_cursor.take() {
                if filtered_conversation_exists_after(
                    session,
                    &request,
                    cursor.clone(),
                    &membership_filter,
                    assignments,
                )? {
                    next_cursor = Some(cursor);
                }
            }
            break;
        }
        if next_cursor.is_none() {
            break;
        }
    }
    if conversations.len() < target_limit as usize && next_cursor.is_some() {
        return Err(CodexAppServerError::Protocol(
            "thread/list exceeded the project membership pagination limit".to_string(),
        ));
    }
    Ok(crate::protocol::CodexThreadPage {
        data: conversations,
        next_cursor: next_cursor
            .map(|cursor| encode_filtered_conversation_cursor(&cursor_filter, cursor))
            .transpose()?,
    })
}

fn filtered_conversation_exists_after(
    session: &CodexAppServerSession,
    request: &CodexThreadListRequest,
    mut cursor: String,
    membership_filter: &CodexConversationMembershipFilter,
    assignments: &CodexDesktopProjectAssignments,
) -> Result<bool, CodexAppServerError> {
    let mut probe = request.clone();
    probe.limit = Some(1);
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_FILTERED_THREAD_PAGES {
        if !seen_cursors.insert(cursor.clone()) {
            return Err(CodexAppServerError::Protocol(
                "thread/list repeated a cursor while probing project membership".to_string(),
            ));
        }
        probe.cursor = Some(cursor);
        let page = session.thread_list(probe.clone())?;
        for snapshot in &page.data {
            let membership = assignments.membership(snapshot);
            let matches = match (membership_filter, membership) {
                (
                    CodexConversationMembershipFilter::Project(expected),
                    CodexConversationMembership::Project(actual),
                ) => expected == &actual,
                (
                    CodexConversationMembershipFilter::Standalone,
                    CodexConversationMembership::Standalone,
                ) => true,
                _ => false,
            };
            if matches {
                return Ok(true);
            }
        }
        let Some(next_cursor) = page.next_cursor else {
            return Ok(false);
        };
        cursor = next_cursor;
    }
    Err(CodexAppServerError::Protocol(
        "thread/list exceeded the project membership look-ahead limit".to_string(),
    ))
}

fn encode_filtered_conversation_cursor(
    filter: &str,
    upstream_cursor: String,
) -> Result<String, CodexAppServerError> {
    serde_json::to_string(&FilteredConversationCursor {
        version: 1,
        filter: filter.to_string(),
        upstream_cursor,
    })
    .map(|cursor| format!("{FILTERED_CONVERSATION_CURSOR_PREFIX}{cursor}"))
    .map_err(|error| {
        CodexAppServerError::Protocol(format!(
            "failed to encode project membership cursor: {error}"
        ))
    })
}

fn decode_filtered_conversation_cursor(
    cursor: Option<String>,
    expected_filter: &str,
) -> Result<Option<String>, CodexAppServerError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let encoded = cursor
        .strip_prefix(FILTERED_CONVERSATION_CURSOR_PREFIX)
        .ok_or_else(|| {
            CodexAppServerError::Protocol(
                "conversation cursor was not issued for project membership pagination"
                    .to_string(),
            )
        })?;
    let decoded: FilteredConversationCursor = serde_json::from_str(encoded).map_err(|error| {
        CodexAppServerError::Protocol(format!(
            "invalid project membership conversation cursor: {error}"
        ))
    })?;
    if decoded.version != 1 || decoded.filter != expected_filter {
        return Err(CodexAppServerError::Protocol(
            "conversation cursor does not match the requested project filter".to_string(),
        ));
    }
    Ok(Some(decoded.upstream_cursor))
}

fn load_codex_desktop_project_assignments(
) -> Result<CodexDesktopProjectAssignments, ProtocolError> {
    let Some(codex_home) = codex_home() else {
        return Ok(CodexDesktopProjectAssignments::default());
    };
    let path = codex_home.join(".codex-global-state.json");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CodexDesktopProjectAssignments::default());
        }
        Err(error) => {
            return Err(protocol_error(
                "project_assignment_state_unavailable",
                format!("failed to read Codex Desktop project assignments: {error}"),
                true,
            ));
        }
    };
    parse_codex_desktop_project_assignments(&contents, &codex_home).map_err(|error| {
        protocol_error(
            "project_assignment_state_invalid",
            format!("failed to parse Codex Desktop project assignments: {error}"),
            false,
        )
    })
}

fn parse_codex_desktop_project_assignments(
    contents: &str,
    codex_home: &Path,
) -> Result<CodexDesktopProjectAssignments, serde_json::Error> {
    let state: Value = serde_json::from_str(contents)?;
    let host_key = format!("local:{}", codex_home.to_string_lossy());
    let native_projects = state
        .get("app-server-project-id-by-legacy-project-id-by-host")
        .and_then(Value::as_object)
        .and_then(|hosts| hosts.get(&host_key))
        .and_then(Value::as_object);
    let mut by_thread = HashMap::new();
    if let Some(assignments) = state
        .get("thread-project-assignments")
        .and_then(Value::as_object)
    {
        for (thread_id, assignment) in assignments {
            if assignment.get("projectKind").and_then(Value::as_str) != Some("local") {
                continue;
            }
            let Some(legacy_project_id) = assignment.get("projectId").and_then(Value::as_str)
            else {
                continue;
            };
            let native_project_id = native_projects
                .and_then(|projects| projects.get(legacy_project_id))
                .and_then(Value::as_str)
                .map(str::to_string);
            by_thread.insert(thread_id.clone(), native_project_id);
        }
    }
    Ok(CodexDesktopProjectAssignments { by_thread })
}

#[cfg(test)]
mod project_assignment_compat_tests {
    use super::*;

    #[test]
    fn parses_only_local_assignments_and_maps_legacy_projects_for_the_current_host() {
        let assignments = parse_codex_desktop_project_assignments(
            &json!({
                "app-server-project-id-by-legacy-project-id-by-host": {
                    "local:/fixture/.codex": {
                        "legacy-project": "native-project"
                    },
                    "remote-host": {
                        "legacy-project": "wrong-native-project"
                    }
                },
                "thread-project-assignments": {
                    "mapped-thread": {
                        "projectKind": "local",
                        "projectId": "legacy-project"
                    },
                    "unmapped-thread": {
                        "projectKind": "local",
                        "projectId": "missing-project"
                    },
                    "remote-thread": {
                        "projectKind": "remote",
                        "projectId": "legacy-project"
                    }
                }
            })
            .to_string(),
            Path::new("/fixture/.codex"),
        )
        .unwrap();

        assert_eq!(
            assignments.by_thread.get("mapped-thread"),
            Some(&Some("native-project".to_string()))
        );
        assert_eq!(assignments.by_thread.get("unmapped-thread"), Some(&None));
        assert!(!assignments.by_thread.contains_key("remote-thread"));
    }

    #[test]
    fn filtered_cursor_round_trips_and_rejects_a_different_project_filter() {
        let cursor = encode_filtered_conversation_cursor(
            "project:native-project",
            "upstream-cursor".to_string(),
        )
        .unwrap();

        assert_eq!(
            decode_filtered_conversation_cursor(
                Some(cursor.clone()),
                "project:native-project"
            )
            .unwrap()
            .as_deref(),
            Some("upstream-cursor")
        );
        assert!(decode_filtered_conversation_cursor(Some(cursor), "standalone").is_err());
    }

    #[test]
    fn native_membership_wins_and_unmapped_legacy_projects_are_not_standalone() {
        let assignments = CodexDesktopProjectAssignments {
            by_thread: HashMap::from([
                (
                    "mapped-thread".to_string(),
                    Some("legacy-native-project".to_string()),
                ),
                ("unmapped-thread".to_string(), None),
            ]),
        };

        assert_eq!(
            assignments.membership_for("mapped-thread", Some("app-server-project")),
            CodexConversationMembership::Project("app-server-project".to_string())
        );
        assert_eq!(
            assignments.membership_for("mapped-thread", None),
            CodexConversationMembership::Project("legacy-native-project".to_string())
        );
        assert_eq!(
            assignments.membership_for("unmapped-thread", None),
            CodexConversationMembership::UnmappedProject
        );
        assert_eq!(
            assignments.membership_for("standalone-thread", None),
            CodexConversationMembership::Standalone
        );
    }
}

fn codex_authentication(response: &Value) -> Option<ProviderAuthentication> {
    let requires_auth = response
        .get("requiresOpenaiAuth")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let Some(account) = response.get("account").filter(|value| !value.is_null()) else {
        return Some(ProviderAuthentication {
            status: if requires_auth {
                ProviderAuthenticationStatus::SignedOut
            } else {
                ProviderAuthenticationStatus::Unsupported
            },
            display_text: Some(if requires_auth {
                "Not signed in".to_string()
            } else {
                "Authentication not required".to_string()
            }),
        });
    };
    let account_type = account
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let display_text = match account_type {
        "chatgpt" => account
            .get("planType")
            .and_then(Value::as_str)
            .map(|plan| format!("Signed in · ChatGPT {}", display_plan_name(plan)))
            .unwrap_or_else(|| "Signed in · ChatGPT".to_string()),
        "apiKey" => "Signed in · API key".to_string(),
        "amazonBedrock" => "Signed in · Amazon Bedrock".to_string(),
        _ => "Signed in".to_string(),
    };
    Some(ProviderAuthentication {
        status: ProviderAuthenticationStatus::SignedIn,
        display_text: Some(display_text),
    })
}

fn display_plan_name(plan: &str) -> String {
    plan.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn codex_usage(
    rate_limits: Option<&Value>,
    token_usage: Option<&Value>,
) -> Option<ProviderUsage> {
    let mut details = Vec::new();
    let mut display_parts = Vec::new();
    if let Some(rate_limits) = rate_limits {
        let mut data = codepet_provider_sdk::JsonObject::new();
        for key in ["rateLimits", "rateLimitsByLimitId", "rateLimitResetCredits"] {
            if let Some(value) = rate_limits.get(key).filter(|value| !value.is_null()) {
                data.insert(key.to_string(), value.clone());
            }
        }
        if !data.is_empty() {
            details.push(ProviderUsageDetail {
                namespace: "openai.codex.rate-limits".to_string(),
                schema_version: "1".to_string(),
                data,
            });
        }
        if let Some(snapshot) = preferred_rate_limit_snapshot(rate_limits) {
            for key in ["primary", "secondary"] {
                if let Some(window) = snapshot.get(key).filter(|value| !value.is_null()) {
                    if let Some(label) = rate_limit_window_label(window) {
                        display_parts.push(label);
                    }
                }
            }
        }
    }
    if let Some(summary) = token_usage
        .and_then(|usage| usage.get("summary"))
        .filter(|value| value.is_object())
    {
        details.push(ProviderUsageDetail {
            namespace: "openai.codex.token-usage".to_string(),
            schema_version: "1".to_string(),
            data: [("summary".to_string(), summary.clone())]
                .into_iter()
                .collect(),
        });
        if display_parts.is_empty() {
            if let Some(tokens) = summary.get("lifetimeTokens").and_then(Value::as_u64) {
                display_parts.push(format!("{} lifetime tokens", compact_number(tokens)));
            }
        }
    }
    if details.is_empty() {
        return None;
    }
    Some(ProviderUsage {
        display_text: if display_parts.is_empty() {
            "Usage data available".to_string()
        } else {
            display_parts.join(" · ")
        },
        observed_at: Some(now_ms()),
        details: Some(details),
    })
}

fn preferred_rate_limit_snapshot(response: &Value) -> Option<&Value> {
    response
        .pointer("/rateLimitsByLimitId/codex")
        .or_else(|| response.get("rateLimits"))
}

fn rate_limit_window_label(window: &Value) -> Option<String> {
    let used = window.get("usedPercent")?.as_u64()?.min(100);
    let remaining = 100_u64.saturating_sub(used);
    let duration = window
        .get("windowDurationMins")
        .and_then(Value::as_u64)
        .map(format_window_duration)
        .unwrap_or_else(|| "Quota".to_string());
    Some(format!("{duration} {remaining}% remaining"))
}

fn format_window_duration(minutes: u64) -> String {
    if minutes > 0 && minutes % (24 * 60) == 0 {
        format!("{}d", minutes / (24 * 60))
    } else if minutes > 0 && minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn provider_task_error(error: tokio::task::JoinError) -> ProtocolError {
    protocol_error(
        "provider_task_failed",
        format!("Codex Provider task failed: {error}"),
        true,
    )
}

fn protocol_error(code: &str, message: String, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.to_string(),
        message,
        retryable,
        details: None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
