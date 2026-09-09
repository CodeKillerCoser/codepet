mod conversation_observer;
mod hook_observation;
use codepet_provider_data::conversation_atoms::{self, ConversationAtoms};
use codepet_provider_sdk::local_runtime;
use crate::client::{
    CodexAppServerSession, CodexRequestOutcome,
};
use crate::mapper::{parse_permission_level, CodexProtocolMapper};
use crate::protocol::{
    approval_generation, approval_resource_id, CodexAppServerError, CodexApprovalRequest,
    CodexConversationSnapshot, CodexIncoming, CodexNotification, CodexProjectCreateRequest,
    CodexProjectRoot, CodexProjectUpdateRequest, CodexThreadListRequest, CodexThreadStartRequest,
    CodexTurn, CodexTurnItemsView, CodexTurnStartRequest, CodexTurnStatus,
    CodexTurnSteerRequest,
    CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID,
};
use codepet_provider_sdk::{
    Conversation,
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
    FlatModelCatalogKind, FlatModelSelection, HarnessDescriptor, ModelCatalog, ModelSelection, Approval,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance,
    ProviderInstanceRoute, ProviderPluginDescriptor, ProviderResourceId, ProviderShutdownRequest,
    ProviderShutdownResponse, RuntimeCandidate, RuntimeGetInstalledRequest,
    RuntimeGetInstalledResponse, RuntimeInstallation, RuntimeSelectRequest, RuntimeSelectResponse,
    TurnInterruptRequest, TurnInterruptResponse,
    TurnSelection, TurnStartRequest, TurnStartResponse, TurnSteerRequest, TurnSteerResponse,
    VersionRange, PROTOCOL_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod server_events;

static NEXT_EXECUTION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
static NEXT_INSTANCE_SESSION: AtomicU64 = AtomicU64::new(1);
static NEXT_MANAGED_WORKTREE: AtomicU64 = AtomicU64::new(1);
const DEFAULT_CONVERSATION_GET_TURN_LIMIT: u64 = 20;
const MAX_CONVERSATION_GET_TURN_LIMIT: u64 = 100;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct CodexInstanceSettings {
    app_server_executable: PathBuf,
    app_server_args: Vec<String>,
    #[serde(default, alias = "codexHome")]
    data_directory: Option<PathBuf>,
}

#[doc(hidden)]
pub trait ExecutionLifecycleHook: Send + Sync + 'static {
    fn after_handle_acquired(&self, _conversation_id: &str, _operation: &str) {}

    fn before_resume_linearization(&self, _conversation_id: &str) {}

    fn after_execution_cancelled(&self, _conversation_id: &str) {}

}

struct NoopExecutionLifecycleHook;

impl ExecutionLifecycleHook for NoopExecutionLifecycleHook {}

struct InstanceMutable {
    atomic_task: Option<tokio::task::JoinHandle<()>>,
    atomic_facts_ready: bool,
    atomic_readiness_pending: bool,
    atomic_facts_epoch: u64,
    metadata_epoch: u64,
    metadata_task: Option<tokio::task::JoinHandle<()>>,
    destroyed: bool,
    cleanup_in_progress: bool,
    status: InstanceStatus,
    capabilities: ProviderCapabilities,
    harness: HarnessDescriptor,
    authentication: Option<ProviderAuthentication>,
    usage: Option<ProviderUsage>,
    lifecycle_generation: u64,
    sessions: HashMap<u64, Arc<InstanceSessionSlot>>,
    server_session_id: Option<u64>,
    server_generation: Option<String>,
    executions: HashMap<String, Arc<ExecutionSlot>>,
    pending_approvals: HashMap<String, PendingApproval>,
    approval_history: Vec<ObservedApproval>,
    pending_materialization: HashMap<String, CodexConversationSnapshot>,
    observed_thread_ids: HashSet<String>,
    auto_title_attempted: HashSet<String>,
}

impl Drop for InstanceMutable {
    fn drop(&mut self) { if let Some(task) = self.metadata_task.take() { task.abort(); } if let Some(task) = self.atomic_task.take() { task.abort(); } }
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
}

enum ExecutionSlotState {
    Creating(Option<CodexAppServerSession>),
    Ready(ExecutionSession),
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
                ExecutionSlotState::Creating(_) => {
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

    fn finish_active_turn(&self, generation: &str, completed_turn_id: Option<&str>) {
        let mut state = lock(&self.state);
        if let ExecutionSlotState::Ready(execution) = &mut *state {
            if execution.generation == generation && completed_turn_id.is_none_or(|id|
                execution.active_turn_id.as_deref().is_none_or(|active| active == id)) {
                execution.active_turn_id = None;
            }
        }
    }

}

#[derive(Clone)]
struct PendingApproval {
    request: CodexApprovalRequest,
    approval: Approval,
}

#[derive(Clone)]
struct ObservedApproval {
    item_id: String,
    approval: Approval,
}

struct CodexInstanceRuntime {
    hook_activity: Arc<Mutex<hook_observation::ActivityProjection>>,
    hook_ready: AtomicBool,
    atoms: ConversationAtoms,
    route: ProviderInstanceRoute,
    instance_kind: String,
    display_name: String,
    settings: CodexInstanceSettings,
    lifecycle_transition: Mutex<()>,
    lifecycle_changed: Condvar,
    title_slots: Arc<tokio::sync::Semaphore>,
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
        let atoms = ConversationAtoms::default();
        let hook_activity = Arc::new(Mutex::new(hook_observation::ActivityProjection::default()));
        let events = Arc::new(hook_observation::SummaryEvents {
            projection: hook_activity.clone(),
            sink: atoms.explicit_activity_event_sink(request.route.clone(), events),
        });
        Self {
            hook_activity,
            hook_ready: AtomicBool::new(false),
            atoms,
            route: request.route.clone(),
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            lifecycle_transition: Mutex::new(()),
            lifecycle_changed: Condvar::new(),
            title_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            mutable: Mutex::new(InstanceMutable {
                atomic_task: None, atomic_facts_ready: false, atomic_readiness_pending: false, atomic_facts_epoch: 0,
                metadata_epoch: 0, metadata_task: None,
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
                server_session_id: None,
                server_generation: None,
                executions: HashMap::new(),
                pending_approvals: HashMap::new(),
                approval_history: Vec::new(),
                pending_materialization: HashMap::new(),
                observed_thread_ids: HashSet::new(),
                auto_title_attempted: HashSet::new(),
            }),
            mapper: Mutex::new(CodexProtocolMapper::new(request.route)),
            events,
            lifecycle_hook,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        let mutable = lock(&self.mutable);
        self.snapshot_locked(&mutable)
    }

    fn snapshot_locked(&self, mutable: &InstanceMutable) -> ProviderInstance {
        let mut capabilities = conversation_atoms::observed_capabilities(mutable.capabilities.clone(), mutable.atomic_facts_ready, mutable.atomic_facts_epoch);
        if !self.hook_ready.load(Ordering::SeqCst) {
            capabilities.methods.retain(|method| *method != ProviderCapability::ConversationActiveList);
        }
        lock(&self.mapper).instance(
            CODEX_PLUGIN_ID.to_string(),
            self.instance_kind.clone(),
            self.display_name.clone(),
            mutable.harness.clone(),
            mutable.status,
            mutable.authentication.clone(),
            mutable.usage.clone(),
            capabilities,
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

    fn cancel_metadata(mutable: &mut InstanceMutable) {
        mutable.metadata_epoch = mutable.metadata_epoch.wrapping_add(1);
        if let Some(task) = mutable.metadata_task.take() {
            task.abort();
        }
    }

    fn refresh_metadata(self: &Arc<Self>) {
        let _transition = lock(&self.lifecycle_transition);
        let mut mutable = lock(&self.mutable);
        if !matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready)
            || (mutable.status == InstanceStatus::Starting && mutable.metadata_task.is_some()) {
            return;
        }
        let Some(server) = mutable.server_session_id
            .and_then(|id| mutable.sessions.get(&id)).and_then(|slot| slot.session()) else { return; };
        Self::cancel_metadata(&mut mutable);
        let epoch = mutable.metadata_epoch;
        let owner = Arc::downgrade(self);
        mutable.metadata_task = Some(tokio::spawn(async move {
            // The client multiplexes independent request IDs. Never hold a lifecycle lock
            // while waiting; each RPC is bounded and closing the session wakes all waiters.
            let model_server = server.clone();
            let project_server = server.clone();
            let account_server = server.clone();
            let limits_server = server.clone();
            let usage_server = server.clone();
            let models = tokio::task::spawn_blocking(move || model_server.model_list());
            let projects =
                tokio::task::spawn_blocking(move || project_server.project_list(None, Some(1)));
            let account = tokio::task::spawn_blocking(move || account_server.account_read());
            let limits =
                tokio::task::spawn_blocking(move || limits_server.account_rate_limits_read());
            let usage = tokio::task::spawn_blocking(move || usage_server.account_usage_read());
            let catalog = async {
                let (models, projects) = tokio::join!(models, projects);
                let Ok(Ok(models)) = models else { return None; };
                let supported = match projects {
                    Ok(Ok(_)) => true,
                    Ok(Err(error)) if error.is_method_not_found("project/list") => false,
                    _ => return None,
                };
                CodexProtocolMapper::capabilities(server.generation().to_string(), models, supported).ok()
            };
            let authentication = async {
                account.await.ok().and_then(Result::ok).map(|account| codex_authentication(&account))
            };
            let consumption = async {
                let (limits, usage) = tokio::join!(limits, usage);
                codex_usage(limits.ok().and_then(Result::ok).as_ref(), usage.ok().and_then(Result::ok).as_ref())
            };
            let (capabilities, authentication, usage) = tokio::join!(catalog, authentication, consumption);
            if let Some(owner) = owner.upgrade() {
                owner.apply_metadata(epoch, |state| {
                    if let Some(capabilities) = capabilities { state.capabilities = capabilities; }
                    state.authentication = authentication.flatten();
                    state.usage = usage;
                });
            }
        }));
    }

    fn apply_metadata(&self, epoch: u64, update: impl FnOnce(&mut InstanceMutable)) {
        let _transition = lock(&self.lifecycle_transition);
        let mut mutable = lock(&self.mutable);
        if !matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready) || mutable.metadata_epoch != epoch {
            return;
        }
        let previous = mutable.status;
        update(&mut mutable);
        mutable.status = InstanceStatus::Ready;
        self.lifecycle_changed.notify_all();
        drop(mutable);
        let _ = self.publish_status_change(previous);
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
        if mutable.server_session_id == Some(slot.id) {
            mutable.server_session_id = None;
            mutable.server_generation = None;
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
            mutable.server_session_id = None;
            mutable.server_generation = None;
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
                    Self::cancel_metadata(&mut mutable);
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.atomic_facts_ready = false;
            if let Some(task) = mutable.atomic_task.take() { task.abort(); }
                    mutable.status = InstanceStatus::Stopped;
                    self.lifecycle_changed.notify_all();
                    drop(mutable);
                    StopAction::Return(Box::new(self.publish_status_change(previous)?))
                }
                _ => {
                    let previous = mutable.status;
                    Self::cancel_metadata(&mut mutable);
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.atomic_facts_ready = false;
            if let Some(task) = mutable.atomic_task.take() { task.abort(); }
                    let lifecycle_generation = mutable.lifecycle_generation;
                    mutable.status = InstanceStatus::Stopping;
                    mutable.server_session_id = None;
                    mutable.server_generation = None;
                    let sessions = mutable.sessions.drain().map(|(_, slot)| slot).collect();
                    let executions = mutable.executions.drain().collect();
                    mutable.pending_approvals.clear();
                    mutable.approval_history.clear();
                    mutable.pending_materialization.clear();
                    mutable.observed_thread_ids.clear();
                    mutable.auto_title_attempted.clear();
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
                for (conversation_id, slot) in executions {
                    slot.force_close();
                    self.lifecycle_hook
                        .after_execution_cancelled(&conversation_id);
                }
                let shutdown_result = tokio::task::spawn_blocking(move || {
                    let lifecycle_result = Self::cancel_sessions(sessions);
                    lifecycle_result
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
                            mutable.observed_thread_ids.clear();
                            mutable.auto_title_attempted.clear();
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

    fn ready_server(&self) -> Result<CodexAppServerSession, ProtocolError> {
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
            .server_session_id
            .and_then(|id| mutable.sessions.get(&id))
            .and_then(|slot| slot.session());
        session.ok_or_else(|| {
            protocol_error(
                "provider_unavailable",
                "Codex server App Server session is unavailable".to_string(),
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
                let server_ready = mutable
                    .server_session_id
                    .and_then(|id| mutable.sessions.get(&id))
                    .and_then(|slot| slot.session())
                    .is_some();
                if mutable.status != InstanceStatus::Ready || !server_ready {
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
            let session = match self.ready_server() {
                Ok(session) => session,
                Err(error) => { self.fail_execution_creation(conversation_id, &slot, error.clone()); return Err(error); }
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
                let error = execution_start_cancelled_error();
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
                    })
            };
            if !installed {
                let error = execution_start_cancelled_error();
                slot.fail(error.clone());
                return Err(error);
            }
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

    fn finish_turn_execution(&self, _conversation_id: &str, handle: &ExecutionHandle, completed_turn_id: Option<&str>) {
        handle.slot.finish_active_turn(&handle.generation, completed_turn_id);
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

    fn fail_server(
        &self,
        server_generation: &str,
        error: ProtocolError,
    ) {
        eprintln!("Codex server App Server failed: {}", error.message);
        let (failure_generation, sessions, execution_slots) = {
            let _transition = lock(&self.lifecycle_transition);
            let mut mutable = lock(&self.mutable);
            if mutable.server_generation.as_deref() != Some(server_generation)
                || !matches!(mutable.status, InstanceStatus::Ready | InstanceStatus::Starting)
            {
                return;
            }
            let previous = mutable.status;
            Self::cancel_metadata(&mut mutable);
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.atomic_facts_ready = false;
            if let Some(task) = mutable.atomic_task.take() { task.abort(); }
            let failure_generation = mutable.lifecycle_generation;
            mutable.cleanup_in_progress = true;
            mutable.status = InstanceStatus::Error;
            mutable.server_session_id = None;
            mutable.server_generation = None;
            let sessions = mutable.sessions.drain().map(|(_, slot)| slot).collect();
            let execution_slots = mutable
                .executions
                .drain()
                .collect::<Vec<_>>();
            mutable.pending_approvals.clear();
            mutable.approval_history.clear();
            mutable.pending_materialization.clear();
            mutable.observed_thread_ids.clear();
            mutable.auto_title_attempted.clear();
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
    data: Arc<codepet_provider_data::ProviderData>,
    observation: codepet_observation::Observation,
    scanner: local_runtime::RuntimeScanner,
    state: Arc<Mutex<ProviderState>>,
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
        let data=Arc::new(codepet_provider_data::ProviderData::default());
        let events: Arc<dyn ProviderEventSink>=Arc::new(codepet_provider_data::UsageSink { data:data.clone(), downstream:events, provider:"codex" });
        let state = Arc::new(Mutex::new(ProviderState {
            host_device_id: None, initialized_client_id: None,
            instances: HashMap::new(), selected_runtime: None,
        }));
        let hook_events = Arc::new(hook_observation::HookEvents { state: Arc::downgrade(&state), sink: events.clone() });
        Self {
            data,
            scanner: local_runtime::RuntimeScanner::new(events.clone()),
            observation: codepet_observation::Observation::new(codepet_observation::Definition {
                windows_command_override: true,
                name: "codex", config: codepet_observation::config_home("CODEX_HOME", codepet_observation::home().join(".codex")).join("hooks.json"), events: &["SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop", "SubagentStart", "SubagentStop", "Interrupt"], plugin: None,
            }, hook_events),
            state,
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
            default_workspace_root: default_remote_workspace_root("codex"),
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

    fn resource_instance(&self, resource: &ProviderResourceId) -> Result<Arc<CodexInstanceRuntime>, ProtocolError> {
        validate_resource(resource)?;
        self.instance(&ProviderInstanceRoute {
            device_id: resource.device_id.clone(),
            provider_plugin_id: resource.provider_plugin_id.clone(),
            provider_instance_id: resource.provider_instance_id.clone(),
        })
    }
}

impl Provider for CodexProvider {
    fn event_subscribe<'a>(&'a self, request: codepet_provider_sdk::EventSubscribeRequest) -> codepet_provider_sdk::ProtocolFuture<'a, codepet_provider_sdk::EventSubscribeResponse> {
        Box::pin(async move {
            if self.is_shutdown() { return Err(codepet_provider_sdk::ProtocolError { code: "provider_shutdown".into(), message: "Provider stopped".into(), retryable: false, details: None }); }
            if lock(&self.state).host_device_id.is_none() { return Err(protocol_error("provider_not_initialized", "Initialize Provider before subscribing".into(), false)); }
            self.observation.subscribe(request.subscription_id).await
        })
    }
    fn event_unsubscribe<'a>(&'a self, request: codepet_provider_sdk::EventUnsubscribeRequest) -> codepet_provider_sdk::ProtocolFuture<'a, codepet_provider_sdk::EventUnsubscribeResponse> {
        Box::pin(async move { self.observation.unsubscribe(request.subscription_id).await })
    }

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
            drop(state);
            self.data.initialize(request.directories.as_ref())?;
            eprintln!("provider.initialize completed; business storage configured={}", self.data.configured());
            self.scanner.start_cancellable("codex", "@openai/codex", discover_codex_candidates, |candidate, timeout, control| inspect_runtime_candidate(candidate, "codex", timeout, control));
            Ok(ProviderInitializeResponse {
                selected_version: PROTOCOL_VERSION,
                plugin: Self::descriptor(),
            })
        })
    }

    fn usage_query<'a>(&'a self, request: codepet_provider_sdk::UsageQueryRequest) -> ProtocolFuture<'a, codepet_provider_sdk::UsageQueryResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let data=self.data.clone();
            let result=tokio::task::spawn_blocking(move || {
                let instance=request.route.provider_instance_id;
                if request.query.page.as_ref().and_then(|p|p.cursor.as_ref()).is_none() {
                    let response=runtime.ready_server()?.account_usage_read().map_err(|e|codepet_provider_data::error("usage_unavailable",e))?;
                    data.import_codex_daily(&instance,&response)?;
                }
                data.query(&instance, request.query, true)
            }).await.map_err(|e|codepet_provider_data::error("usage_query_failed",e))??;
            Ok(codepet_provider_sdk::UsageQueryResponse { result })
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

    fn runtime_get_installed<'a>(&'a self, request: RuntimeGetInstalledRequest) -> ProtocolFuture<'a, RuntimeGetInstalledResponse> {
        Box::pin(async move {
            if request.refresh == Some(true) && self.scanner.snapshot().scanning != Some(true) {
                self.scanner.stop();
                self.scanner.start_cancellable("codex", "@openai/codex", discover_codex_candidates, |candidate, timeout, control| inspect_runtime_candidate(candidate, "codex", timeout, control));
            }
            if request.refresh == Some(true) {
                let instances = lock(&self.state).instances.values().cloned().collect::<Vec<_>>();
                for instance in instances { instance.refresh_metadata(); }
            }
            Ok(self.scanner.snapshot())
        })
    }
    fn runtime_select<'a>(&'a self, request: RuntimeSelectRequest) -> ProtocolFuture<'a, RuntimeSelectResponse> {
        Box::pin(async move {
            let mut candidate = request.candidate;
            candidate.executable_path = local_runtime::resolve_executable(std::path::Path::new(&candidate.executable_path), "codex", "@openai/codex")
                .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?.to_string_lossy().into_owned();
            let selected=self.scanner.select(&candidate)?;
            lock(&self.state).selected_runtime=Some(selected.clone());
            Ok(RuntimeSelectResponse {selected})
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
            if std::env::var("CODEPET_RUNTIME_MIN_VERSION").is_ok_and(|value| !value.is_empty()) {
                if let Some(path) = request.settings.get("appServerExecutable").and_then(Value::as_str) {
                    let runtime = self.scanner.select(&RuntimeCandidate {
                        executable_path: path.to_string(),
                        source: codepet_provider_sdk::RuntimeCandidateSource::Configured,
                    })?;
                    if runtime.version.is_empty() {
                        return Err(protocol_error("runtime_scanning", "Runtime version validation is still in progress".into(), true));
                    }
                    local_runtime::require_compatible_runtime(&local_runtime::apply_runtime_requirement(runtime))?;
                }
            }
            let selected = if request.settings.contains_key("appServerExecutable") { None } else {
                let current = { lock(&self.state).selected_runtime.clone() };
                let installation = match current {
                    Some(selected) => Some(selected),
                    None => {
                        let inventory=self.scanner.snapshot();
                        if inventory.scanning==Some(true) {return Err(protocol_error("runtime_scanning", "Runtime discovery is still in progress".into(), true));}
                        inventory.installed.into_iter().find(|runtime| runtime.incompatibility_reason.is_none())
                    },
                };
                Some(installation.ok_or_else(|| protocol_error("provider_unavailable", "Codex Provider did not find a local runtime".to_string(), true))?)
            };
            if let Some(selected) = selected.as_ref() {
                if selected.version.is_empty() {
                    return Err(protocol_error("runtime_scanning", "Runtime version validation is still in progress".into(), true));
                }
                local_runtime::require_compatible_runtime(&local_runtime::apply_runtime_requirement(selected.clone()))?;
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
                        drop(mutable);
                        return Ok(InstanceStartResponse { instance: runtime.snapshot() });
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
                CodexInstanceRuntime::cancel_metadata(&mut mutable);
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.atomic_facts_ready = false;
            if let Some(task) = mutable.atomic_task.take() { task.abort(); }
                mutable.status = InstanceStatus::Starting;
                mutable.server_session_id = None;
                mutable.server_generation = None;
                mutable.pending_approvals.clear();
                mutable.approval_history.clear();
                mutable.pending_materialization.clear();
                mutable.observed_thread_ids.clear();
                mutable.auto_title_attempted.clear();
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
            // Reserve the cancellable instance slot before installing the
            // independent Hook source; stop must win while installation awaits.
            let hook_result = self.observation.subscribe(hook_observation::INTERNAL_SUBSCRIPTION.into()).await;
            let hooks_ready = hook_result.is_ok();
            if let Err(error) = hook_result { eprintln!("Codex activity observation unavailable: {}", error.message); }
            if runtime.hook_ready.swap(hooks_ready, Ordering::SeqCst) != hooks_ready {
                let status = {
                    let mut state = lock(&runtime.mutable);
                    state.atomic_facts_epoch = state.atomic_facts_epoch.saturating_add(1);
                    state.status
                };
                if status == InstanceStatus::Ready { let _ = runtime.publish_status_change(status); }
            }
            if !slot.begin_spawn() {
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("server session"));
            }
            let executable = runtime.settings.app_server_executable.clone();
            let args = runtime.settings.app_server_args.clone();
            let data_directory = runtime.settings.data_directory.clone();
            let server = match tokio::task::spawn_blocking(move || {
                CodexAppServerSession::spawn_uninitialized(&executable, &args, data_directory.as_deref())
            })
            .await {
                Ok(Ok(server)) => server,
                Ok(Err(error)) => {
                    let mapped = CodexProtocolMapper::error(error);
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("server session"));
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
                    return Err(instance_session_cancelled_error("server session"));
                }
            };
            if !slot.register_spawned(server.clone()) || !runtime.session_is_current(&slot) {
                let _ = server.shutdown();
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("server session"));
            }
            let initialize_session = server.clone();
            let initialize_result = tokio::task::spawn_blocking(move || initialize_session.initialize())
                .await
                .map_err(provider_task_error)?;
            if let Err(error) = initialize_result {
                let mapped = CodexProtocolMapper::error(error);
                let _ = server.shutdown();
                if runtime.mark_start_failed(&slot)? {
                    return Err(mapped);
                }
                return Err(instance_session_cancelled_error("server session"));
            }
            let harness = HarnessDescriptor {
                id: CODEX_INSTANCE_KIND.to_string(),
                display_name: "Codex".to_string(),
                version: server.harness_version(),
                executable_path: Some(runtime.settings.app_server_executable.to_string_lossy().into_owned()),
            };
            let incoming = match server.subscribe() {
                Ok(incoming) => incoming,
                Err(error) => {
                    let mapped = CodexProtocolMapper::error(error);
                    let _ = server.shutdown();
                    if runtime.mark_start_failed(&slot)? {
                        return Err(mapped);
                    }
                    return Err(instance_session_cancelled_error("server session"));
                }
            };
            let server_generation = server.generation().to_string();
            let installed = {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                let current = mutable.status == InstanceStatus::Starting
                    && mutable.lifecycle_generation == slot.lifecycle_generation
                    && mutable
                        .sessions
                        .get(&slot.id)
                        .is_some_and(|current| Arc::ptr_eq(current, &slot))
                    && !slot.cancelled.load(Ordering::SeqCst)
                    && mutable.server_session_id.is_none()
                    && mutable.executions.is_empty();
                if !current {
                    false
                } else {
                    mutable.capabilities = CodexProtocolMapper::unavailable_capabilities(server_generation.clone());
                    mutable.harness = harness;
                    mutable.authentication = None;
                    mutable.usage = None;
                    mutable.server_session_id = Some(slot.id);
                    mutable.server_generation = Some(server_generation.clone());
                    mutable.pending_approvals.clear();
                    mutable.approval_history.clear();
                    mutable.pending_materialization.clear();
                    mutable.observed_thread_ids.clear();
                    mutable.auto_title_attempted.clear();
                    runtime.lifecycle_changed.notify_all();
                    true
                }
            };
            if !installed {
                let _ = server.shutdown();
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("server session"));
            }
            runtime.start_server_forwarder(server_generation, server, incoming);
            let instance = runtime.snapshot();
            runtime.refresh_metadata();
            runtime.start_atomic_poll();
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
                CodexInstanceRuntime::cancel_metadata(&mut mutable);
                    mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.atomic_facts_ready = false;
            if let Some(task) = mutable.atomic_task.take() { task.abort(); }
                mutable.server_session_id = None;
                mutable.server_generation = None;
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
            let capabilities = runtime.snapshot().capabilities;
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
            let session = runtime.ready_server()?;
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
            let session = runtime.ready_server()?;
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
            let session = runtime.ready_server()?;
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
            let session = runtime.ready_server()?;
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
            let session = runtime.ready_server()?;
            tokio::task::spawn_blocking(move || session.project_delete(&project_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
            Ok(ProjectDeleteResponse {})
        })
    }

    fn conversation_active_list<'a>(&'a self, request: codepet_provider_sdk::ConversationActiveListRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationActiveListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if !runtime.hook_ready.load(Ordering::SeqCst) { return Err(protocol_error("unsupported", "Hook activity observation is not ready".into(), true)); }
            let generation = runtime.query_generation()?;
            if let Some(page) = runtime.atoms.active_cached(&generation, &request)? { return Ok(page); }
            let epoch = runtime.atoms.event_epoch();
            let rows = lock(&runtime.hook_activity).active_rows(&runtime.route);
            if generation != runtime.query_generation()? || epoch != runtime.atoms.event_epoch() { return Err(conversation_atoms::generation_changed()); }
            runtime.atoms.active(&generation, &request, rows)
        })
    }

    fn conversation_unread_list<'a>(&'a self, request: codepet_provider_sdk::ConversationUnreadListRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationUnreadListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            runtime.atoms.unread(&runtime.query_generation()?, &request)
        })
    }

    fn conversation_mark_read<'a>(&'a self, request: codepet_provider_sdk::ConversationMarkReadRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationMarkReadResponse> {
        Box::pin(async move {
            let runtime = self.instance(&conversation_atoms::resource_route(&request.conversation))?;
            runtime.query_generation()?;
            runtime.atoms.mark_read(&request, runtime.events.as_ref())
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let generation = runtime.query_generation()?;
            // This is continuation-only: no cursor always performs fresh discovery.
            if let Some(page) = runtime.atoms.list_cached(&generation, &request)? { return Ok(page); }
            if !matches!(request.project_filter, ConversationProjectFilter::ConversationProjectFilterAll(_)) {
                require_capability(&runtime, ProviderCapability::ProjectList)?;
            }
            let mut rows = self.complete_conversation_summaries(&request.route).await?;
            if let Some(codepet_provider_sdk::ConversationListQuery::ConversationIdsQuery(query)) = &request.query {
                self.complete_requested_summaries(&request.route, &query.ids, &mut rows).await?;
            }
            if matches!(request.project_filter, ConversationProjectFilter::ConversationProjectFilterStandalone(_)) {
                let assignments = load_codex_desktop_project_assignments(runtime.settings.data_directory.as_deref())?;
                rows.retain(|row| assignments.membership_for(&row.resource.native_resource_id,
                    row.project.as_ref().map(|project| project.native_resource_id.as_str())) == CodexConversationMembership::Standalone);
            }
            if generation != runtime.query_generation()? { return Err(conversation_atoms::generation_changed()); }
            runtime.atoms.list(&generation, &request, rows)
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
            let session = runtime.ready_server()?;
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
                    .map(|snapshot| {
                        let mut row = mapper.conversation(snapshot);
                        runtime.project_conversation(&mut row);
                        row
                    })
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
            let session = runtime.ready_server()?;
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
                let mut conversation =
                    mapper.conversation_with_active_turn(&snapshot, active_turn);
                drop(mapper);
                runtime.project_conversation(&mut conversation);
                if history_materialized {
                    lock(&runtime.mutable)
                        .pending_materialization
                        .remove(&conversation_id);
                }
                Ok(ConversationGetResponse {
                    conversation,
                    items,
                    page_info: Some(PageInfo {
                        next_cursor: response_next_cursor,
                    }),
                })
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
                                lease_expires_at: None,
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
            let workspace_root = prepare_conversation_workspace(
                request.workspace_root.as_deref(), request.workspace_mode.as_deref(),
            )?;
            let session = runtime.ready_server()?;
            let operation_runtime = runtime.clone();
            let snapshot = tokio::task::spawn_blocking(move || {
                let server_slot = {
                    let mutable = lock(&operation_runtime.mutable);
                    mutable.server_session_id.and_then(|id| mutable.sessions.get(&id)).cloned()
                        .ok_or_else(|| instance_session_cancelled_error("server"))?
                };
                let outcome = session.thread_start_outcome_with_sender(CodexThreadStartRequest {
                    workspace_root, project_id, permission_level,
                    model: request.model, reasoning_effort: request.reasoning_effort,
                }, |message| {
                    let _send_gate = lock(&server_slot.send_gate);
                    if !operation_runtime.session_is_current(&server_slot) || server_slot.cancelled.load(Ordering::SeqCst) {
                        return CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown);
                    }
                    session.write_prepared_request(message)
                });
                match outcome {
                    CodexRequestOutcome::Success(snapshot) => {
                        let mut mutable = lock(&operation_runtime.mutable);
                        if mutable.server_generation.as_deref() != Some(session.generation())
                            || mutable.status != InstanceStatus::Ready {
                            return Err(instance_session_cancelled_error("server"));
                        }
                        let slot = Arc::new(ExecutionSlot::new());
                        slot.set_starting_session(session.clone());
                        slot.set_ready(ExecutionSession { session: session.clone(), generation: session.generation().to_string(), active_turn_id: None });
                        mutable.executions.insert(snapshot.thread.id.clone(), slot);
                        mutable.pending_materialization.insert(snapshot.thread.id.clone(), snapshot.clone());
                        Ok(snapshot)
                    }
                    outcome => Err(execution_outcome_error("thread/start", None, outcome)),
                }
            }).await.map_err(provider_task_error)??;
            let mut conversation = lock(&runtime.mapper).conversation(&snapshot);
            runtime.project_conversation(&mut conversation);
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
            if request.capability_revision != capabilities.revision && request.capability_revision != runtime.snapshot().capabilities.revision {
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
            let title_input: String = request.input.text.chars().take(6000).collect();
            let input_text = request.input.text;
            let client_request_id = request.client_request_id;
            let requested_selection = request.selection;
            let pending_snapshot = lock(&runtime.mutable)
                .pending_materialization
                .get(&conversation_id)
                .cloned();
            let operation_runtime = runtime.clone();
            let operation_conversation_id = conversation_id.clone();
            let (turn, effective_selection, needs_title) = tokio::task::spawn_blocking(move || {
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
                        let needs_title = snapshot.thread.name.as_deref().is_none_or(|name| name.trim().is_empty());
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
                                return Err(error);
                            }
                        };
                        let model = match effective_selection.model.as_ref() {
                            Some(ModelSelection::FlatModelSelection(selection)) => {
                                Some(selection.model_id.clone())
                            }
                            Some(ModelSelection::GroupedModelSelection(_)) => {
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
                                Ok(ExecutionStep::Complete((turn, effective_selection, needs_title)))
                            }
                            outcome => {
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
            let schedule_title = needs_title && lock(&runtime.mutable).auto_title_attempted.insert(conversation_id.clone());
            if let (true, Ok(title_session)) = (schedule_title, runtime.ready_server()) {
                let title_runtime = runtime.clone();
                let title_conversation = conversation_id.clone();
                let title_model = match &effective_selection.model {
                    Some(ModelSelection::FlatModelSelection(model)) => Some(model.model_id.clone()),
                    _ => None,
                };
                tokio::spawn(async move {
                    let Ok(_permit) = title_runtime.title_slots.clone().acquire_owned().await else { return; };
                    if lock(&title_runtime.mutable).server_generation.as_deref() != Some(title_session.generation()) { return; }
                    let diagnostic_id = title_conversation.clone();
                    let outcome = tokio::task::spawn_blocking(move || {
                        title_session.generate_thread_title(&title_conversation, &title_input, title_model.as_deref())
                    }).await;
                    match outcome {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => eprintln!("Codex automatic title generation failed conversation_id={diagnostic_id}: {error}"),
                        Err(error) => eprintln!("Codex automatic title task failed conversation_id={diagnostic_id}: {error}"),
                    }
                });
            }
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
                || pending.approval.resource.native_resource_id
                    != request.approval.native_resource_id
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
                                    return Err(error);
                                }
                                Ok(ExecutionStep::Complete(approval))
                            }
                            outcome => {
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
            self.scanner.stop();
            self.observation.shutdown().await;
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
    let workspace_root = default_remote_workspace_root("codex").ok_or_else(|| {
        protocol_error(
            "worktree_create_failed",
            "cannot resolve CodePet workspace for managed worktrees".to_string(),
            false,
        )
    })?;
    create_managed_worktree(requested, Path::new(&workspace_root)).map(Some)
}

fn codex_home() -> Option<PathBuf> {
    local_runtime::data_dir("CODEX_HOME", ".codex")
}

fn default_remote_workspace_root(harness_name: &str) -> Option<String> {
    local_runtime::home_dir()
        .filter(|path| path.is_absolute())
        .map(|path| {
            path.join(".codepet")
                .join("remote_workspace")
                .join(harness_name)
                .to_string_lossy()
                .into_owned()
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
    workspace_root: &Path,
) -> Result<String, ProtocolError> {
    let requested = requested.canonicalize().map_err(|error| {
        protocol_error(
            "invalid_workspace_root",
            format!("failed to resolve workspaceRoot: {error}"),
            false,
        )
    })?;
    let output = codepet_provider_sdk::local_runtime::command("git")
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
    let unique = format!(
        "remote-{}-{}-{}",
        now_ms(),
        std::process::id(),
        NEXT_MANAGED_WORKTREE.fetch_add(1, Ordering::SeqCst),
    );
    let worktree_root = workspace_root.join("worktree").join(unique);
    if let Some(parent) = worktree_root.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to prepare managed worktree directory: {error}"),
                true,
            )
        })?;
    }
    let output = codepet_provider_sdk::local_runtime::command("git")
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
    use super::{create_managed_worktree, ensure_conversation_workspace, prepare_conversation_workspace};
    use std::fs;
    use std::path::Path;

    #[test]
    fn creates_a_missing_standalone_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("codex/task/task-1");

        let prepared = ensure_conversation_workspace(workspace.to_str().unwrap()).unwrap();

        assert_eq!(Path::new(&prepared), workspace);
        assert!(workspace.is_dir());
    }

    #[test]
    fn main_workspace_keeps_the_requested_project_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().to_str().unwrap();
        assert_eq!(
            prepare_conversation_workspace(Some(root), Some("main")).unwrap(),
            Some(root.to_string())
        );
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

        let workspace_root = fixture.path().join(".codepet/remote_workspace/codex");
        let cwd = create_managed_worktree(&nested, &workspace_root).unwrap();
        let cwd = Path::new(&cwd);

        assert!(cwd.is_dir());
        assert!(cwd.starts_with(workspace_root.join("worktree")));
        let relative = cwd.strip_prefix(workspace_root.join("worktree")).unwrap();
        assert_eq!(relative.components().count(), 3);
        assert!(cwd.join("../../README.md").is_file());
        assert_eq!(cwd.file_name().and_then(|value| value.to_str()), Some("app"));
        let output = codepet_provider_sdk::local_runtime::command("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
        let detached = codepet_provider_sdk::local_runtime::command("git")
            .arg("-C")
            .arg(cwd)
            .args(["symbolic-ref", "-q", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(detached.status.code(), Some(1));
        let second = create_managed_worktree(&nested, &workspace_root).unwrap();
        assert_ne!(Path::new(&second), cwd);
    }

    fn git(repository: &Path, args: &[&str]) {
        let status = codepet_provider_sdk::local_runtime::command("git")
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
    cursor: Option<String>,
    requested_limit: u64,
    used_pending_snapshot: bool,
) -> Result<LoadedConversationTurns, ProtocolError> {
    let page = match session.thread_turns_list_with_view(
        conversation_id, cursor.clone(), requested_limit as u32, CodexTurnItemsView::Full,
    ) {
        Ok(page) => page,
        Err(error) if cursor.is_none()
            && (error.is_thread_turns_unavailable_before_first_user_message(conversation_id)
                || used_pending_snapshot && (error.is_thread_not_loaded(conversation_id)
                    || is_created_conversation_not_ready(&error, conversation_id))) =>
        {
            return Ok(LoadedConversationTurns { turns: Vec::new(), next_cursor: None, materialized: false });
        }
        Err(error) => return Err(CodexProtocolMapper::error(error)),
    };
    if page.data.len() as u64 > requested_limit {
        return Err(protocol_error("provider_protocol_error", format!(
            "thread/turns/list returned {} turns for limit {requested_limit}", page.data.len(),
        ), false));
    }
    Ok(LoadedConversationTurns { turns: page.data, next_cursor: page.next_cursor, materialized: true })
}

fn discover_codex_candidates() -> Vec<RuntimeCandidate> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("CODE_PET_CODEX_BIN").filter(|value| !value.is_empty()) {
        candidates.push(local_runtime::candidate(PathBuf::from(path), codepet_provider_sdk::RuntimeCandidateSource::Environment));
    }
    candidates.extend(local_runtime::discover("codex", "@openai/codex"));
    #[cfg(target_os = "macos")]
    for app in ["Codex.app", "ChatGPT.app"] {
        candidates.push(local_runtime::candidate(Path::new("/Applications").join(app).join("Contents").join("Resources").join("codex"), codepet_provider_sdk::RuntimeCandidateSource::MacosApplication));
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let mut roots = vec![local.join("OpenAI").join("Codex").join("bin")];
        if let Ok(packages) = std::fs::read_dir(local.join("Packages")) {
            for package in packages.flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with("OpenAI.Codex_")) {
                roots.push(package.path().join("LocalCache").join("Local").join("OpenAI").join("Codex").join("bin"));
            }
        }
        for root in roots {
            candidates.push(local_runtime::candidate(root.join("codex.exe"), codepet_provider_sdk::RuntimeCandidateSource::WindowsApplication));
            if let Ok(versions) = std::fs::read_dir(root) {
                let mut versions: Vec<_> = versions.flatten().collect();
                versions.sort_by_key(|entry| std::cmp::Reverse(entry.metadata().and_then(|m| m.modified()).ok()));
                for version in versions {
                    candidates.push(local_runtime::candidate(version.path().join("codex.exe"), codepet_provider_sdk::RuntimeCandidateSource::WindowsApplication));
                }
            }
        }
    }
    candidates
}

fn inspect_runtime_candidate(
    candidate: RuntimeCandidate,
    product: &str,
    timeout: Duration,
    control: local_runtime::RuntimeProbeControl,
) -> Result<RuntimeInstallation, ProtocolError> {
    let canonical = local_runtime::resolve_executable(Path::new(&candidate.executable_path), "codex", "@openai/codex")
        .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?;
    let line = bounded_runtime_version(&canonical, timeout, control)?;
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
    Ok(RuntimeInstallation { minimum_version: None, incompatibility_reason: None,
        executable_path: canonical.to_string_lossy().into_owned(),
        version,
        source: candidate.source,
    })
}

fn bounded_runtime_version(executable: &Path, timeout: Duration, control: local_runtime::RuntimeProbeControl) -> Result<String, ProtocolError> {
    let mut child = codepet_provider_sdk::local_runtime::command(executable).arg("--version")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|error| {
        protocol_error(
            "invalid_runtime_selection",
            format!("Run runtime executable {}: {error}", executable.display()),
            false,
        )
    })?;
    control.track(child.control()).map_err(|error| protocol_error("runtime_scan_cancelled", error.to_string(), true))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().map_err(|error| protocol_error(
                    "invalid_runtime_selection", format!("Read runtime version: {error}"), false))?;
                if !status.success() {
                    return Err(protocol_error("invalid_runtime_selection", format!("Runtime executable {} rejected --version: {status}; {}", executable.display(), String::from_utf8_lossy(&output.stderr).chars().take(1024).collect::<String>().trim()), false));
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
    approval: Approval,
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

fn update_recorded_approval(mutable: &mut InstanceMutable, approval: &Approval) {
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
    if settings.data_directory.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(protocol_error("invalid_instance_settings", "dataDirectory must be absolute".into(), false));
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

fn validate_resource(resource: &ProviderResourceId) -> Result<(), ProtocolError> {
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
    left: &ProviderResourceId,
    right: &ProviderResourceId,
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
    resource: &ProviderResourceId,
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

fn load_codex_desktop_project_assignments(configured: Option<&Path>
) -> Result<CodexDesktopProjectAssignments, ProtocolError> {
    let Some(codex_home) = configured.map(Path::to_path_buf).or_else(codex_home) else {
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

#[cfg(test)]
mod storage_path_tests {
    use super::*;
    #[test]
    fn executable_and_data_directory_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("custom data 中文");
        let executable = std::env::current_exe().unwrap();
        let settings = decode_settings(serde_json::from_value(json!({
            "appServerExecutable": executable, "appServerArgs": [],
            "dataDirectory": data,
        })).unwrap()).unwrap();
        assert_eq!(settings.data_directory.as_deref(), Some(data.as_path()));
        let invalid = decode_settings(serde_json::from_value(json!({
            "appServerExecutable": executable, "appServerArgs": [],
            "dataDirectory": "relative-storage",
        })).unwrap());
        assert!(invalid.is_err());
    }
}

impl CodexInstanceRuntime {
    fn query_generation(&self) -> Result<String, ProtocolError> {
        self.ready_server()?;
        Ok(lock(&self.mutable).lifecycle_generation.to_string())
    }
}

impl CodexProvider {
    async fn complete_conversation_summaries(&self, route: &ProviderInstanceRoute) -> Result<Vec<Conversation>, ProtocolError> {
        self.instance(route)?.collect_directory_summaries(false).await
    }
}

impl CodexInstanceRuntime {
    async fn collect_atomic_summaries(&self) -> Result<Vec<Conversation>, ProtocolError> {
        // Loaded threads supplement discovery only; Hook activity does not
        // depend on this optional native namespace being available.
        self.collect_directory_summaries(false).await
    }

    async fn collect_directory_summaries(&self, require_loaded_namespace: bool) -> Result<Vec<Conversation>, ProtocolError> {
        let registry = self.atomic_server_registry()?;
        let assignments = Arc::new(load_codex_desktop_project_assignments(self.settings.data_directory.as_deref())?);
        let mut rows = Vec::new();
        let mut cursor = None;
        let mut progress = codepet_provider_sdk::conversation_query::EnumerationProgress::default();
        loop {
            let server = self.ready_server()?;
            let page = tokio::task::spawn_blocking(move || server.thread_list(CodexThreadListRequest { cursor, limit: Some(100), project_id: None, workspace_root: None, search_term: None }))
                .await.map_err(provider_task_error)?.map_err(CodexProtocolMapper::error)?;
            for mut snapshot in page.data {
                if snapshot.thread.ephemeral { continue; }
                assignments.decorate(&mut snapshot);
                rows.push(lock(&self.mapper).conversation(&snapshot));
            }
            cursor = progress.advance(page.next_cursor)?;
            if cursor.is_none() { break; }
            tokio::task::yield_now().await;
        }
        // Query a candidate superset: filtering by native project/time here can
        // discard legacy reads and desktop project assignments before correction.
        let home = self.settings.data_directory.clone().or_else(codex_home)
            .ok_or_else(|| protocol_error("conversation_query_incomplete", "cannot resolve Codex data directory".into(), true))?;
        let evidence = tokio::task::spawn_blocking(move || crate::directory::read_evidence(&home))
            .await.map_err(provider_task_error)??;
        let pending = lock(&self.mutable).pending_materialization.values().cloned().collect::<Vec<_>>();
        rows.extend(pending.iter().map(|snapshot| {
            let mut snapshot = snapshot.clone();
            assignments.decorate(&mut snapshot);
            lock(&self.mapper).conversation(&snapshot)
        }));
        let known = rows.iter().map(|row| row.resource.native_resource_id.clone()).collect::<HashSet<_>>();
        let mut candidates = evidence.keys().cloned().collect::<HashSet<_>>();
        candidates.extend(lock(&self.mutable).observed_thread_ids.iter().cloned());
        let mut missing = candidates.into_iter().filter(|id| !known.contains(id)
            && !evidence.get(id).is_some_and(|fact| fact.archived)).collect::<Vec<_>>();
        missing.sort();
        // Bounded concurrent single-ID reads on the same App Server connection.
        for batch in missing.chunks(4) {
            let mut reads = tokio::task::JoinSet::new();
            for id in batch {
                let server = self.ready_server()?;
                let id = id.clone();
                reads.spawn_blocking(move || server.thread_read_metadata(&id));
            }
            while let Some(result) = reads.join_next().await {
                let mut snapshot = result.map_err(provider_task_error)?.map_err(CodexProtocolMapper::error)?;
                if snapshot.thread.ephemeral { continue; }
                assignments.decorate(&mut snapshot);
                rows.push(lock(&self.mapper).conversation(&snapshot));
            }
        }
        for (_, server) in &registry {
            let mut cursor = None;
            let mut progress = codepet_provider_sdk::conversation_query::EnumerationProgress::default();
            loop {
                let source = server.clone();
                let (ids, next) = match tokio::task::spawn_blocking(move || source.thread_loaded_list(cursor, 100)).await.map_err(provider_task_error)? {
                    Ok(page) => page,
                    // Persistent listing works on older servers without the loaded
                    // namespace API. Never use this fallback to advertise active facts.
                    Err(error) if !require_loaded_namespace && error.is_method_not_found("thread/loaded/list") => break,
                    Err(error) => return Err(CodexProtocolMapper::error(error)),
                };
                for id in ids {
                    if server.is_ephemeral_thread(&id) { continue; }
                    let source = server.clone();
                    // A loaded/read race is an incomplete snapshot, never deletion.
                    let mut snapshot = tokio::task::spawn_blocking(move || source.thread_read_metadata(&id)).await.map_err(provider_task_error)?.map_err(CodexProtocolMapper::error)?;
                    assignments.decorate(&mut snapshot);
                    let row = lock(&self.mapper).conversation(&snapshot);
                    rows.push(row);
                }
                cursor = progress.advance(next)?;
                if cursor.is_none() { break; }
                tokio::task::yield_now().await;
            }
        }
        let after = self.atomic_server_registry()?;
        if registry.iter().map(|(id, server)| (*id, server.generation())).collect::<Vec<_>>() != after.iter().map(|(id, server)| (*id, server.generation())).collect::<Vec<_>>() {
            return Err(conversation_atoms::generation_changed());
        }
        // Native list metadata and live reads are field observations, not
        // last-response-wins replacements (legacy reads can regress timestamps).
        let mut directory = std::collections::BTreeMap::<String, Conversation>::new();
        for mut row in rows {
            let id = row.resource.native_resource_id.clone();
            if evidence.get(&id).is_some_and(|fact| fact.archived) { continue; }
            if let Some(previous) = directory.get(&id) {
                if row.updated_at == row.created_at && previous.updated_at > row.updated_at {
                    row.updated_at = previous.updated_at;
                }
                if row.title == id && previous.title != id { row.title = previous.title.clone(); }
                if row.preview.is_none() { row.preview = previous.preview.clone(); }
                if row.project.is_none() { row.project = previous.project.clone(); }
            }
            if let Some(fact) = evidence.get(&id) { crate::directory::repair_time(&mut row, fact); }
            directory.insert(id, row);
        }
        let mut rows = directory.into_values().collect::<Vec<_>>();
        conversation_atoms::sort_summaries(&mut rows);
        for row in &mut rows { self.project_conversation(row); }
        Ok(rows)
    }
}

impl CodexProvider {
    async fn complete_requested_summaries(&self, route: &ProviderInstanceRoute, ids: &[String], rows: &mut Vec<Conversation>) -> Result<(), ProtocolError> {
        let runtime = self.instance(route)?;
        for id in ids {
            if rows.iter().any(|row| &row.resource.native_resource_id == id) { continue; }
            let session = runtime.ready_server()?;
            let requested = id.clone();
            // Missing list membership is not deletion (e.g. archived sessions).
            // Unknown native failures propagate; never infer deletion from them.
            match tokio::task::spawn_blocking(move || session.thread_read_metadata(&requested)).await.map_err(provider_task_error)? {
                Ok(mut snapshot) => {
                    load_codex_desktop_project_assignments(runtime.settings.data_directory.as_deref())?.decorate(&mut snapshot);
                    let mut row = lock(&runtime.mapper).conversation(&snapshot);
                    runtime.project_conversation(&mut row);
                    rows.push(row);
                }
                Err(error) if error.is_thread_not_loaded(id) => {},
                Err(error) => return Err(CodexProtocolMapper::error(error)),
            }
        }
        Ok(())
    }
}

impl CodexInstanceRuntime {
    fn atomic_server_registry(&self) -> Result<Vec<(u64, CodexAppServerSession)>, ProtocolError> {
        let mutable = lock(&self.mutable);
        let mut registry = Vec::new();
        for (id, slot) in &mutable.sessions {
            match &*lock(&slot.state) {
                InstanceSessionState::Spawned(server) => registry.push((*id, server.clone())),
                InstanceSessionState::Pending | InstanceSessionState::Spawning => return Err(conversation_atoms::generation_changed()),
                InstanceSessionState::Finished => {}
            }
        }
        registry.sort_by_key(|(id, _)| *id);
        Ok(registry)
    }
}
