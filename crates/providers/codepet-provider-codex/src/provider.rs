use crate::client::{CodexAppServerSession, CodexRequestOutcome};
use crate::mapper::{parse_permission_level, CodexProtocolMapper};
use crate::protocol::{
    approval_generation, approval_resource_id, CodexAppServerError, CodexApprovalRequest,
    CodexIncoming, CodexNotification, CodexThreadListRequest, CodexThreadStartRequest,
    CodexTurnStartRequest, CodexTurnStatus, CodexTurnSteerRequest,
    CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalRequestedEvent, ApprovalResolveRequest, ApprovalResolveResponse,
    ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ConversationSearchRequest,
    ConversationSearchResponse, InstanceCapabilitiesRequest, InstanceCapabilitiesResponse,
    InstanceCreateRequest, InstanceCreateResponse,
    InstanceDestroyRequest, InstanceDestroyResponse, InstanceStartRequest,
    InstanceStartResponse, InstanceStatus, InstanceStatusChangedEvent, InstanceStopRequest,
    InstanceStopResponse, PageInfo, ProtocolError, ProtocolEvent, ProtocolFuture,
    ProtocolServer, ProviderCapabilities, ProviderDescribeRequest, ProviderDescribeResponse,
    FlatModelCatalogKind, FlatModelSelection, HarnessDescriptor, ModelCatalog, ModelSelection, ProviderApproval,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance,
    ProviderInstanceRoute, ProviderPluginDescriptor, ProviderShutdownRequest,
    ProviderShutdownResponse, RoutedResourceId, TurnInterruptRequest, TurnInterruptResponse,
    TurnSelection, TurnStartRequest, TurnStartResponse, TurnSteerRequest, TurnSteerResponse,
    VersionRange, PROTOCOL_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_EXECUTION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
static NEXT_INSTANCE_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct CodexInstanceSettings {
    app_server_executable: PathBuf,
    app_server_args: Vec<String>,
}

pub trait ProviderEventSink: Send + Sync + 'static {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError>;
}

impl<F> ProviderEventSink for F
where
    F: Fn(ProtocolEvent) -> Result<(), ProtocolError> + Send + Sync + 'static,
{
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        self(event)
    }
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
    destroyed: bool,
    cleanup_in_progress: bool,
    status: InstanceStatus,
    capabilities: ProviderCapabilities,
    harness: HarnessDescriptor,
    lifecycle_generation: u64,
    sessions: HashMap<u64, Arc<InstanceSessionSlot>>,
    observer_session_id: Option<u64>,
    observer_generation: Option<String>,
    executions: HashMap<String, Arc<ExecutionSlot>>,
    pending_approvals: HashMap<String, PendingApproval>,
    approval_history: Vec<ObservedApproval>,
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
                },
                lifecycle_generation: 0,
                sessions: HashMap::new(),
                observer_session_id: None,
                observer_generation: None,
                executions: HashMap::new(),
                pending_approvals: HashMap::new(),
                approval_history: Vec::new(),
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
        if !handle.slot.has_active_turn(&handle.generation) {
            self.release_execution(conversation_id, &handle.generation);
        }
    }

    fn start_observer_forwarder(
        self: &Arc<Self>,
        observer_generation: String,
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
        incoming: Receiver<Result<CodexIncoming, CodexAppServerError>>,
    ) {
        let runtime = Arc::downgrade(self);
        thread::spawn(move || {
            while let Ok(message) = incoming.recv() {
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
                        let terminal_turn = match &incoming {
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
                                true
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
                            _ => false,
                        };
                        if terminal_turn {
                            let Some((closing_slot, expired)) = runtime.begin_execution_close(
                                &conversation_id,
                                &session_generation,
                            ) else {
                                return;
                            };
                            match runtime.map_execution_incoming(
                                &conversation_id,
                                &session_generation,
                                incoming,
                            ) {
                                Ok(events) => {
                                    for event in events {
                                        if let Err(error) = runtime.events.publish(event) {
                                            eprintln!(
                                                "Codex execution event forwarding failed: {}",
                                                error.message
                                            );
                                            break;
                                        }
                                    }
                                }
                                Err(error) => {
                                    eprintln!(
                                        "Codex execution event mapping failed: {}",
                                        error.message
                                    );
                                }
                            }
                            runtime.finish_execution_close(
                                &conversation_id,
                                &session_generation,
                                closing_slot,
                                expired,
                            );
                            return;
                        }
                        let events = runtime.map_execution_incoming(
                            &conversation_id,
                            &session_generation,
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
        if let CodexIncoming::Notification(CodexNotification::TurnStarted { turn, .. }) = &incoming
        {
            if let Some(slot) = self.execution_slot(conversation_id, session_generation) {
                slot.mark_active_turn(session_generation, &turn.id);
            }
        }
        match incoming {
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

impl ProtocolServer for CodexProvider {
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

    fn instance_create<'a>(
        &'a self,
        request: InstanceCreateRequest,
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
            let settings = decode_settings(request.settings.clone())?;
            let mut state = lock(&self.state);
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
            let capabilities = match CodexProtocolMapper::capabilities(
                observer.generation().to_string(),
                models,
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
                    mutable.observer_session_id = Some(slot.id);
                    mutable.observer_generation = Some(observer_generation.clone());
                    mutable.pending_approvals.clear();
                    mutable.approval_history.clear();
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
            runtime.start_observer_forwarder(observer_generation, incoming);
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

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let limit = request.limit.map(u32::try_from).transpose().map_err(|_| {
                protocol_error(
                    "invalid_request",
                    "conversation list limit exceeds the Codex App Server range".to_string(),
                    false,
                )
            })?;
            let session = runtime.ready_observer()?;
            let page = tokio::task::spawn_blocking(move || {
                session.thread_list(CodexThreadListRequest {
                    cursor: request.cursor,
                    limit,
                    workspace_root: None,
                    search_term: None,
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
            let session = runtime.ready_observer()?;
            let snapshot = tokio::task::spawn_blocking(move || session.thread_read(&conversation_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
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
            let mapper = lock(&runtime.mapper);
            let conversation = mapper.conversation(&snapshot);
            let items = mapper.conversation_items(&snapshot, &approvals);
            Ok(ConversationGetResponse { conversation, items })
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
            if !slot.begin_spawn() {
                runtime.unregister_session(&slot);
                return Err(instance_session_cancelled_error("conversation creation session"));
            }
            let executable = runtime.settings.app_server_executable.clone();
            let args = runtime.settings.app_server_args.clone();
            let operation_runtime = runtime.clone();
            let operation_slot = slot.clone();
            let snapshot = tokio::task::spawn_blocking(move || {
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
                let outcome = session.thread_start_outcome_with_sender(CodexThreadStartRequest {
                    workspace_root: request.workspace_root,
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
                let result = match outcome {
                    CodexRequestOutcome::Success(snapshot) => Ok(snapshot),
                    outcome => Err(execution_outcome_error("thread/start", None, outcome)),
                };
                if let Err(error) = session.shutdown() {
                    eprintln!("Codex conversation creation session shutdown failed: {error}");
                }
                operation_runtime.unregister_session(&operation_slot);
                result
            })
            .await
            .map_err(provider_task_error)??;
            let conversation = lock(&runtime.mapper).conversation(&snapshot);
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
            let operation_runtime = runtime.clone();
            let operation_conversation_id = conversation_id.clone();
            let (turn, effective_selection) = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.start",
                    move |handle| {
                        let snapshot =
                            match handle.session.thread_read(&execution_conversation_id) {
                                Ok(snapshot) => snapshot,
                                Err(error) => {
                                    execution_runtime.release_execution(
                                        &execution_conversation_id,
                                        &handle.generation,
                                    );
                                    return Err(CodexProtocolMapper::error(error));
                                }
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
                            execution_runtime.release_execution(
                                &execution_conversation_id,
                                &handle.generation,
                            );
                            return Ok(ExecutionStep::Retry);
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
                                    execution_runtime.release_execution(
                                        &execution_conversation_id,
                                        &handle.generation,
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
                                    execution_runtime.release_execution(
                                        &execution_conversation_id,
                                        &handle.generation,
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
                                    execution_runtime.release_execution(
                                        &execution_conversation_id,
                                        &handle.generation,
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
            CodexNotification::TurnStarted { thread_id, .. }
            | CodexNotification::TurnCompleted { thread_id, .. }
            | CodexNotification::OutputDelta { thread_id, .. }
            | CodexNotification::ServerRequestResolved { thread_id, .. },
        ) => Some(thread_id),
        CodexIncoming::ApprovalRequested(request) => Some(&request.thread_id),
        CodexIncoming::Notification(CodexNotification::Unknown { .. })
        | CodexIncoming::UnsupportedServerRequest { .. } => None,
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
