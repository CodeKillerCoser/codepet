use super::*;

pub(super) struct InstanceMutable {
    pub(super) atomic_facts_epoch: u64,
    pub(super) metadata_epoch: u64,
    pub(super) metadata_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) destroyed: bool,
    pub(super) interaction_released: bool,
    pub(super) failed_shutdown_sessions: Vec<CodexAppServerSession>,
    pub(super) cleanup_in_progress: bool,
    pub(super) status: InstanceStatus,
    pub(super) capabilities: ProviderCapabilities,
    pub(super) harness: HarnessDescriptor,
    pub(super) authentication: Option<ProviderAuthentication>,
    pub(super) usage: Option<ProviderUsage>,
    pub(super) lifecycle_generation: u64,
    pub(super) sessions: HashMap<u64, Arc<InstanceSessionSlot>>,
    pub(super) server_session_id: Option<u64>,
    pub(super) server_generation: Option<String>,
    pub(super) executions: HashMap<String, Arc<ExecutionSlot>>,
    pub(super) pending_approvals: HashMap<String, PendingApproval>,
    pub(super) approval_history: Vec<ObservedApproval>,
    pub(super) pending_materialization: HashMap<String, CodexConversationSnapshot>,
    pub(super) observed_thread_ids: HashSet<String>,
    pub(super) auto_title_attempted: HashSet<String>,
}

impl Drop for InstanceMutable {
    fn drop(&mut self) {
        if let Some(task) = self.metadata_task.take() {
            task.abort();
        }
    }
}

pub(super) enum InstanceSessionState {
    Pending,
    Spawning,
    Spawned(CodexAppServerSession),
    Finished,
}

pub(super) struct InstanceSessionSlot {
    pub(super) id: u64,
    pub(super) lifecycle_generation: u64,
    pub(super) cancelled: AtomicBool,
    pub(super) send_gate: Mutex<()>,
    pub(super) state: Mutex<InstanceSessionState>,
    pub(super) changed: Condvar,
}

impl InstanceSessionSlot {
    pub(super) fn new(lifecycle_generation: u64) -> Self {
        Self {
            id: NEXT_INSTANCE_SESSION.fetch_add(1, Ordering::SeqCst),
            lifecycle_generation,
            cancelled: AtomicBool::new(false),
            send_gate: Mutex::new(()),
            state: Mutex::new(InstanceSessionState::Pending),
            changed: Condvar::new(),
        }
    }

    pub(super) fn begin_spawn(&self) -> bool {
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

    pub(super) fn register_spawned(&self, session: CodexAppServerSession) -> bool {
        let mut state = lock(&self.state);
        if !matches!(*state, InstanceSessionState::Spawning) {
            return false;
        }
        *state = InstanceSessionState::Spawned(session);
        self.changed.notify_all();
        !self.cancelled.load(Ordering::SeqCst)
    }

    pub(super) fn session(&self) -> Option<CodexAppServerSession> {
        match &*lock(&self.state) {
            InstanceSessionState::Spawned(session) => Some(session.clone()),
            InstanceSessionState::Pending
            | InstanceSessionState::Spawning
            | InstanceSessionState::Finished => None,
        }
    }

    pub(super) fn cancel_and_take(&self) -> Option<CodexAppServerSession> {
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

    pub(super) fn finish(&self) {
        let mut state = lock(&self.state);
        *state = InstanceSessionState::Finished;
        self.changed.notify_all();
    }
}

#[derive(Clone)]
pub(super) struct ExecutionSession {
    pub(super) session: CodexAppServerSession,
    pub(super) generation: String,
    pub(super) active_turn_id: Option<String>,
}

pub(super) enum ExecutionSlotState {
    Creating(Option<CodexAppServerSession>),
    Ready(ExecutionSession),
    Failed(ProtocolError),
    Closed,
}

pub(super) struct ExecutionSlot {
    pub(super) attempt_generation: u64,
    pub(super) cancelled: AtomicBool,
    pub(super) resume_send: Mutex<()>,
    pub(super) state: Mutex<ExecutionSlotState>,
    pub(super) changed: Condvar,
    pub(super) operation: Mutex<()>,
}

#[derive(Clone)]
pub(super) struct ExecutionHandle {
    pub(super) slot: Arc<ExecutionSlot>,
    pub(super) session: CodexAppServerSession,
    pub(super) generation: String,
}

pub(super) enum ExecutionStep<T> {
    Complete(T),
    Retry,
}

impl ExecutionSlot {
    pub(super) fn new() -> Self {
        Self {
            attempt_generation: NEXT_EXECUTION_ATTEMPT.fetch_add(1, Ordering::SeqCst),
            cancelled: AtomicBool::new(false),
            resume_send: Mutex::new(()),
            state: Mutex::new(ExecutionSlotState::Creating(None)),
            changed: Condvar::new(),
            operation: Mutex::new(()),
        }
    }

    pub(super) fn wait_ready(self: &Arc<Self>) -> Result<Option<ExecutionHandle>, ProtocolError> {
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

    pub(super) fn set_starting_session(&self, session: CodexAppServerSession) -> bool {
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

    pub(super) fn set_ready(&self, execution: ExecutionSession) -> bool {
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

    pub(super) fn fail(&self, error: ProtocolError) {
        let mut state = lock(&self.state);
        if matches!(*state, ExecutionSlotState::Creating(_)) {
            *state = ExecutionSlotState::Failed(error);
            self.changed.notify_all();
        }
    }

    pub(super) fn force_close(&self) -> Option<CodexAppServerSession> {
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

    pub(super) fn creation_is_current(
        &self,
        attempt_generation: u64,
        session_generation: &str,
    ) -> bool {
        self.attempt_generation == attempt_generation
            && !self.cancelled.load(Ordering::SeqCst)
            && matches!(
                &*lock(&self.state),
                ExecutionSlotState::Creating(Some(session))
                    if session.generation() == session_generation
            )
    }

    pub(super) fn matches_generation(&self, generation: &str) -> bool {
        matches!(
            &*lock(&self.state),
            ExecutionSlotState::Ready(execution) if execution.generation == generation
        )
    }

    pub(super) fn mark_active_turn(&self, generation: &str, turn_id: &str) -> bool {
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

    pub(super) fn has_active_turn(&self, generation: &str) -> bool {
        matches!(
            &*lock(&self.state),
            ExecutionSlotState::Ready(execution)
                if execution.generation == generation && execution.active_turn_id.is_some()
        )
    }

    pub(super) fn finish_active_turn(&self, generation: &str, completed_turn_id: Option<&str>) {
        let mut state = lock(&self.state);
        if let ExecutionSlotState::Ready(execution) = &mut *state {
            if execution.generation == generation
                && completed_turn_id.is_none_or(|id| {
                    execution
                        .active_turn_id
                        .as_deref()
                        .is_none_or(|active| active == id)
                })
            {
                execution.active_turn_id = None;
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct PendingApproval {
    pub(super) request: CodexApprovalRequest,
    pub(super) approval: Approval,
}

#[derive(Clone)]
pub(super) struct ObservedApproval {
    pub(super) item_id: String,
    pub(super) approval: Approval,
}

pub(super) struct CodexInstanceRuntime {
    pub(super) hook_activity: Arc<Mutex<hook_observation::ActivityProjection>>,
    pub(super) hook_ready: AtomicBool,
    pub(super) atoms: ConversationAtoms,
    pub(super) directory_pages: directory_list::DirectoryPages,
    pub(super) ordered_ids: ordered_ids::Pages,
    pub(super) route: ProviderInstanceRoute,
    pub(super) instance_kind: String,
    pub(super) display_name: String,
    pub(super) settings: CodexInstanceSettings,
    pub(super) lifecycle_transition: Mutex<()>,
    pub(super) interaction_transition: tokio::sync::Mutex<()>,
    pub(super) release_transition: tokio::sync::Mutex<()>,
    pub(super) lifecycle_changed: Condvar,
    pub(super) title_slots: Arc<tokio::sync::Semaphore>,
    pub(super) mutable: Mutex<InstanceMutable>,
    pub(super) mapper: Mutex<CodexProtocolMapper>,
    pub(super) events: Arc<dyn ProviderEventSink>,
    pub(super) lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
}

impl CodexInstanceRuntime {
    pub(super) fn new(
        request: InstanceCreateRequest,
        settings: CodexInstanceSettings,
        events: Arc<dyn ProviderEventSink>,
        lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
    ) -> Self {
        let executable_path = settings
            .app_server_executable
            .to_string_lossy()
            .into_owned();
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
            directory_pages: Default::default(),
            ordered_ids: Default::default(),
            route: request.route.clone(),
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            lifecycle_transition: Mutex::new(()),
            interaction_transition: tokio::sync::Mutex::new(()),
            release_transition: tokio::sync::Mutex::new(()),
            lifecycle_changed: Condvar::new(),
            title_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            mutable: Mutex::new(InstanceMutable {
                atomic_facts_epoch: 0,
                metadata_epoch: 0,
                metadata_task: None,
                destroyed: false,
                interaction_released: false,
                failed_shutdown_sessions: Vec::new(),
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

    pub(super) fn snapshot(&self) -> ProviderInstance {
        let mutable = lock(&self.mutable);
        self.snapshot_locked(&mutable)
    }

    pub(super) fn snapshot_locked(&self, mutable: &InstanceMutable) -> ProviderInstance {
        let mut capabilities = conversation_atoms::observed_capabilities(
            mutable.capabilities.clone(),
            self.hook_ready.load(Ordering::SeqCst),
            mutable.atomic_facts_epoch,
        );
        if !self.hook_ready.load(Ordering::SeqCst) {
            capabilities
                .methods
                .retain(|method| *method != ProviderCapability::ConversationActiveList);
        }
        lock(&self.mapper).instance(
            CODEX_PLUGIN_ID.to_string(),
            self.instance_kind.clone(),
            self.display_name.clone(),
            mutable.harness.clone(),
            mutable.status.clone(),
            mutable.authentication.clone(),
            mutable.usage.clone(),
            capabilities,
        )
    }

    pub(super) fn publish_status_change(
        &self,
        previous_status: InstanceStatus,
    ) -> Result<ProviderInstance, ProtocolError> {
        let instance = self.snapshot();
        self.events
            .publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".to_string(),
                params: InstanceStatusChangedEvent {
                    instance: instance.clone(),
                    previous_status: Some(previous_status),
                },
            })?;
        Ok(instance)
    }

    pub(super) fn cancel_metadata(mutable: &mut InstanceMutable) {
        mutable.metadata_epoch = mutable.metadata_epoch.wrapping_add(1);
        if let Some(task) = mutable.metadata_task.take() {
            task.abort();
        }
    }

    pub(super) fn refresh_metadata(self: &Arc<Self>) {
        let _transition = lock(&self.lifecycle_transition);
        let mut mutable = lock(&self.mutable);
        if !matches!(
            mutable.status.clone(),
            InstanceStatus::Starting | InstanceStatus::Ready
        ) || (mutable.status == InstanceStatus::Starting && mutable.metadata_task.is_some())
        {
            return;
        }
        let Some(server) = mutable
            .server_session_id
            .and_then(|id| mutable.sessions.get(&id))
            .and_then(|slot| slot.session())
        else {
            return;
        };
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
                let Ok(Ok(models)) = models else {
                    return None;
                };
                let supported = match projects {
                    Ok(Ok(_)) => true,
                    Ok(Err(error)) if error.is_method_not_found("project/list") => false,
                    Ok(Err(error)) => {
                        eprintln!("Codex capability discovery failed: method=project/list code={}; chat discovery retained", CodexProtocolMapper::error(error).code);
                        false
                    }
                    Err(_) => {
                        eprintln!("Codex capability discovery failed: method=project/list code=worker_failed; chat discovery retained");
                        false
                    }
                };
                match CodexProtocolMapper::capabilities(
                    server.generation().to_string(),
                    models,
                    supported,
                ) {
                    Ok(capabilities) => Some(capabilities),
                    Err(error) => {
                        eprintln!(
                            "Codex capability discovery failed: method=model/list code={}",
                            error.code
                        );
                        None
                    }
                }
            };
            let authentication = async {
                account
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .map(|account| codex_authentication(&account))
            };
            let consumption = async {
                let (limits, usage) = tokio::join!(limits, usage);
                codex_usage(
                    limits.ok().and_then(Result::ok).as_ref(),
                    usage.ok().and_then(Result::ok).as_ref(),
                )
            };
            let (capabilities, authentication, usage) =
                tokio::join!(catalog, authentication, consumption);
            if let Some(owner) = owner.upgrade() {
                owner.apply_metadata(epoch, |state| {
                    if let Some(capabilities) = capabilities {
                        state.capabilities = capabilities;
                    }
                    state.authentication = authentication.flatten();
                    state.usage = usage;
                });
            }
        }));
    }

    pub(super) fn apply_metadata(&self, epoch: u64, update: impl FnOnce(&mut InstanceMutable)) {
        let _transition = lock(&self.lifecycle_transition);
        let mut mutable = lock(&self.mutable);
        if !matches!(
            mutable.status.clone(),
            InstanceStatus::Starting | InstanceStatus::Ready
        ) || mutable.metadata_epoch != epoch
        {
            return;
        }
        let previous = mutable.status.clone();
        update(&mut mutable);
        mutable.status = InstanceStatus::Ready;
        self.lifecycle_changed.notify_all();
        drop(mutable);
        let _ = self.publish_status_change(previous);
    }

    pub(super) fn session_is_current(&self, slot: &Arc<InstanceSessionSlot>) -> bool {
        let mutable = lock(&self.mutable);
        mutable.lifecycle_generation == slot.lifecycle_generation
            && mutable
                .sessions
                .get(&slot.id)
                .is_some_and(|current| Arc::ptr_eq(current, slot))
    }

    pub(super) fn unregister_session(&self, slot: &Arc<InstanceSessionSlot>) {
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

    pub(super) fn cancel_sessions(
        slots: Vec<Arc<InstanceSessionSlot>>,
    ) -> Result<(), CodexAppServerError> {
        let sessions = slots
            .into_iter()
            .filter_map(|slot| slot.cancel_and_take())
            .collect::<Vec<_>>();
        shutdown_sessions(sessions)
    }

    pub(super) fn mark_start_failed(
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
            let previous = mutable.status.clone();
            mutable.status = InstanceStatus::Error;
            self.lifecycle_changed.notify_all();
            previous
        };
        slot.finish();
        self.publish_status_change(previous_status)?;
        Ok(true)
    }

    pub(super) async fn stop(self: &Arc<Self>) -> Result<ProviderInstance, ProtocolError> {
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
                    match mutable.status.clone() {
                        InstanceStatus::Stopped => {
                            drop(mutable);
                            StopAction::Return(Box::new(self.snapshot()))
                        }
                        InstanceStatus::Stopping => StopAction::Wait,
                        InstanceStatus::Created => {
                            let previous = mutable.status.clone();
                            Self::cancel_metadata(&mut mutable);
                            mutable.lifecycle_generation =
                                mutable.lifecycle_generation.wrapping_add(1);
                            mutable.status = InstanceStatus::Stopped;
                            self.lifecycle_changed.notify_all();
                            drop(mutable);
                            StopAction::Return(Box::new(self.publish_status_change(previous)?))
                        }
                        _ => {
                            let previous = mutable.status.clone();
                            Self::cancel_metadata(&mut mutable);
                            mutable.lifecycle_generation =
                                mutable.lifecycle_generation.wrapping_add(1);
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
                    let cleanup_runtime = self.clone();
                    let shutdown_result = tokio::task::spawn_blocking(move || {
                        let lifecycle_result = cleanup_runtime.shutdown_owned_sessions(sessions);
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
                                let previous = mutable.status.clone();
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

    pub(super) fn shutdown_owned_sessions(
        &self,
        slots: Vec<Arc<InstanceSessionSlot>>,
    ) -> Result<(), CodexAppServerError> {
        let mut sessions = std::mem::take(&mut lock(&self.mutable).failed_shutdown_sessions);
        sessions.extend(slots.into_iter().filter_map(|slot| slot.cancel_and_take()));
        let mut failure = None;
        for session in sessions {
            if let Err(error) = session.shutdown() {
                lock(&self.mutable).failed_shutdown_sessions.push(session);
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub(super) fn ready_server(&self) -> Result<CodexAppServerSession, ProtocolError> {
        let mutable = lock(&self.mutable);
        if mutable.status.clone() != InstanceStatus::Ready {
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

    pub(super) fn acquire_execution_at_generation(
        self: &Arc<Self>,
        conversation_id: &str,
        expected: Option<u64>,
    ) -> Result<ExecutionHandle, ProtocolError> {
        loop {
            let (slot, creator) = {
                let mut mutable = lock(&self.mutable);
                if expected.is_some_and(|generation| generation != mutable.lifecycle_generation)
                    || mutable.interaction_released
                {
                    return Err(protocol_error(
                        "interaction_released",
                        "Conversation interaction was released during acquire".into(),
                        false,
                    ));
                }
                let server_ready = mutable
                    .server_session_id
                    .and_then(|id| mutable.sessions.get(&id))
                    .and_then(|slot| slot.session())
                    .is_some();
                if mutable.status.clone() != InstanceStatus::Ready || !server_ready {
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
                Err(error) => {
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
            let resume_outcome =
                session.ensure_thread_loaded_outcome_with_sender(conversation_id, |message| {
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
                });
            if let outcome @ (CodexRequestOutcome::NotSent(_)
            | CodexRequestOutcome::ExplicitRpcReject(_)
            | CodexRequestOutcome::SentOutcomeUnknown(_)) = resume_outcome
            {
                let error =
                    execution_outcome_error("thread/resume", Some(conversation_id), outcome);
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

    pub(super) fn execution_creation_is_current(
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

    pub(super) fn with_current_execution<T>(
        self: &Arc<Self>,
        conversation_id: &str,
        operation_name: &str,
        operation: impl FnMut(&ExecutionHandle) -> Result<ExecutionStep<T>, ProtocolError>,
    ) -> Result<T, ProtocolError> {
        let _gate = DESKTOP_TAKEOVER_GATE
            .read()
            .unwrap_or_else(|e| e.into_inner());
        self.with_current_execution_inner(conversation_id, operation_name, None, operation)
    }

    pub(super) fn check_takeover_inactive(
        &self,
        conversation_id: &str,
    ) -> Result<(), ProtocolError> {
        // Hook facts include desktop-origin activity which a separate app-server may not load.
        if !lock(&self.hook_activity)
            .active_rows(&self.route)
            .is_empty()
        {
            return Err(protocol_error(
                "conversation_active",
                "A Codex conversation is active; desktop takeover is unavailable".into(),
                true,
            ));
        }
        if lock(&self.mutable).executions.values().any(|slot| {
            matches!(&*lock(&slot.state),
            ExecutionSlotState::Ready(execution) if execution.active_turn_id.is_some())
        }) {
            return Err(protocol_error(
                "conversation_active",
                "A Codepet Codex turn is active; desktop takeover is unavailable".into(),
                true,
            ));
        }
        let snapshot = self
            .ready_server()?
            .thread_read(conversation_id)
            .map_err(|_| {
                protocol_error(
                    "force_takeover_failed",
                    "Cannot read current conversation activity".into(),
                    true,
                )
            })?;
        if matches!(
            snapshot.thread.status,
            crate::protocol::CodexThreadStatus::Active { .. }
        ) || snapshot
            .thread
            .turns
            .iter()
            .any(|t| t.status == CodexTurnStatus::InProgress)
        {
            return Err(protocol_error(
                "conversation_active",
                "Conversation has an active turn or pending interaction".into(),
                true,
            ));
        }
        if matches!(
            snapshot.thread.status,
            crate::protocol::CodexThreadStatus::SystemError
        ) {
            return Err(protocol_error(
                "force_takeover_failed",
                "Cannot establish that the conversation is inactive".into(),
                true,
            ));
        }
        Ok(())
    }

    pub(super) fn prepare_forced_takeover(
        self: &Arc<Self>,
        conversation_id: &str,
        expected: u64,
    ) -> Result<(), ProtocolError> {
        self.check_takeover_inactive(conversation_id)?;
        match self.acquire_execution_at_generation(conversation_id, Some(expected)) {
            Ok(_) => return self.check_takeover_inactive(conversation_id),
            Err(error) if error.code == "conversation_write_conflict" => {}
            Err(error) => return Err(error),
        }
        self.lifecycle_hook
            .close_desktop_for_takeover(&mut || self.check_takeover_inactive(conversation_id))?;
        for attempt in 0..10 {
            self.check_takeover_inactive(conversation_id)?;
            match self.acquire_execution_at_generation(conversation_id, Some(expected)) {
                Ok(_) => return self.check_takeover_inactive(conversation_id),
                Err(error) if error.code == "conversation_write_conflict" && attempt < 9 => {
                    thread::sleep(Duration::from_millis(100))
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    pub(super) fn with_current_execution_inner<T>(
        self: &Arc<Self>,
        conversation_id: &str,
        operation_name: &str,
        expected: Option<u64>,
        mut operation: impl FnMut(&ExecutionHandle) -> Result<ExecutionStep<T>, ProtocolError>,
    ) -> Result<T, ProtocolError> {
        loop {
            let handle = self.acquire_execution_at_generation(conversation_id, expected)?;
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

    pub(super) fn execution_handle_is_current(
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

    pub(super) fn fail_execution_creation(
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

    pub(super) fn execution_slot(
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

    pub(super) fn has_execution_generation(&self, generation: &str) -> bool {
        lock(&self.mutable)
            .executions
            .values()
            .any(|slot| slot.matches_generation(generation))
    }

    pub(super) fn finish_turn_execution(
        &self,
        _conversation_id: &str,
        handle: &ExecutionHandle,
        completed_turn_id: Option<&str>,
    ) {
        handle
            .slot
            .finish_active_turn(&handle.generation, completed_turn_id);
    }

    pub(super) fn map_execution_incoming(
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
            CodexIncoming::Notification(CodexNotification::ProjectChanged { .. }) => Ok(Vec::new()),
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
                let (approval, event) =
                    lock(&self.mapper).approval_expired(pending.approval, now_ms());
                {
                    let mut mutable = lock(&self.mutable);
                    update_recorded_approval(&mut mutable, &approval);
                }
                Ok(vec![event])
            }
            incoming => lock(&self.mapper).events(incoming),
        }
    }

    pub(super) fn conversation_upsert_event(
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

    pub(super) fn conversation_status_upsert_event(
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

    pub(super) fn fail_server(&self, server_generation: &str, error: ProtocolError) {
        eprintln!("Codex server App Server failed: {}", error.message);
        let (failure_generation, sessions, execution_slots) = {
            let _transition = lock(&self.lifecycle_transition);
            let mut mutable = lock(&self.mutable);
            if mutable.server_generation.as_deref() != Some(server_generation)
                || !matches!(
                    mutable.status.clone(),
                    InstanceStatus::Ready | InstanceStatus::Starting
                )
            {
                return;
            }
            let previous = mutable.status.clone();
            Self::cancel_metadata(&mut mutable);
            mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            let failure_generation = mutable.lifecycle_generation;
            mutable.cleanup_in_progress = true;
            mutable.status = InstanceStatus::Error;
            mutable.server_session_id = None;
            mutable.server_generation = None;
            let sessions = mutable.sessions.drain().map(|(_, slot)| slot).collect();
            let execution_slots = mutable.executions.drain().collect::<Vec<_>>();
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
            let active_turn_id = match &*lock(&slot.state) {
                ExecutionSlotState::Ready(execution) => execution.active_turn_id.clone(),
                _ => None,
            };
            if let Some(id) = active_turn_id {
                let mapper = lock(&self.mapper);
                let turn = TurnTask {
                    resource: mapper.resource(id),
                    conversation: mapper.resource(conversation_id.clone()),
                    status: TurnStatus::Failed,
                    display_summary: Some(error.message.clone()),
                    error: Some(error.clone()),
                    started_at: None,
                    updated_at: Some(now_ms()),
                    completed_at: Some(now_ms()),
                };
                drop(mapper);
                let _ = self.events.publish(ProtocolEvent::EventTurnUpserted {
                    jsonrpc: "2.0".into(),
                    params: codepet_provider_sdk::TurnUpsertedEvent { turn },
                });
            }
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
