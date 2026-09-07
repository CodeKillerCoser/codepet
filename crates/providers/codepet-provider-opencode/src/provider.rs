use codepet_provider_sdk::conversation_atoms::{self, ConversationAtoms};
use codepet_provider_sdk::local_runtime;
use crate::client::OpenCodeServerSession;
use crate::mapper::{protocol_error, OpenCodeProtocolMapper};
use crate::protocol::{
    OpenCodeDelivery, OpenCodeDeltaEventData, OpenCodeEvent,
    OpenCodeLocationRef, OpenCodePermissionAskedEventData, OpenCodePermissionRepliedEventData,
    OpenCodePermissionReply, OpenCodePrompt, OpenCodePromptAdmittedEventData,
    OpenCodeModel, OpenCodeModelRef, OpenCodePromptRequest, OpenCodeServerError, OpenCodeSession, OpenCodeSessionCreate,
    OpenCodeStepEndedEventData, OpenCodeStepFailedEventData, OpenCodeStepStartedEventData,
    OPENCODE_INSTANCE_KIND, OPENCODE_PERMISSION_LEVEL, OPENCODE_PLUGIN_ID,
};
use codepet_provider_sdk::{
    Conversation,
    ApprovalDecision, ApprovalResolveRequest, ApprovalResolveResponse,
    ConversationAcquireInteractionRequest, ConversationAcquireInteractionResponse,
    ConversationContentKind, ConversationCreateRequest, ConversationCreateResponse,
    ConversationGetRequest, ConversationGetResponse, ConversationListRequest,
    ConversationListResponse, ConversationUpsertedEvent,
    InstanceCapabilitiesRequest, InstanceCapabilitiesResponse, InstanceCreateRequest,
    InstanceCreateResponse, InstanceDestroyRequest, InstanceDestroyResponse,
    InstanceStartRequest, InstanceStartResponse, InstanceStatus, InstanceStatusChangedEvent,
    GroupedModelCatalogKind, GroupedModelSelection, HarnessDescriptor, InstanceStopRequest,
    InstanceStopResponse, ModelCatalog, ModelSelection, PageInfo, ProtocolError, ProtocolEvent,
    ProtocolFuture, Provider, Approval, ProviderAuthentication,
    ProviderAuthenticationStatus, ProviderCapabilities,
    ProviderDescribeRequest, ProviderDescribeResponse, ProviderInitializeRequest,
    ProviderInitializeResponse, ProviderInstance, ProviderInstanceRoute, ProviderResourceId,
    ProviderPluginDescriptor, ProviderShutdownRequest, ProviderShutdownResponse,
    TurnTask, ProviderUsage, ProviderUsageDetail, RuntimeCandidate, RuntimeGetInstalledRequest,
    RuntimeGetInstalledResponse, RuntimeInstallation, RuntimeSelectRequest, RuntimeSelectResponse,
    TurnInterruptRequest, TurnInterruptResponse, TurnSelection,
    TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest, TurnSteerResponse,
    VersionRange, PROTOCOL_VERSION,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct OpenCodeInstanceSettings {
    server_executable: PathBuf,
    #[serde(default)]
    server_version: String,
    server_args: Vec<String>,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
    #[serde(default)]
    data_directory: Option<PathBuf>,
}

use codepet_provider_sdk::ProviderEventSink;

const DEFAULT_CONVERSATION_GET_MESSAGE_LIMIT: u64 = 40;
const MAX_CONVERSATION_GET_MESSAGE_LIMIT: u64 = 100;

#[derive(Clone)]
struct PendingApproval {
    session_generation: String,
    session_id: String,
    request_id: String,
    approval: Approval,
}

#[derive(Clone)]
struct ActiveTurnState {
    epoch: u64,
    turn: TurnTask,
    prompt_message_id: String,
    assistant_message_id: Option<String>,
    wait_started: bool,
    wait_confirmed: bool,
    latest_step_ended_at: Option<u64>,
}

struct InstanceMutable {
    atomic_task: Option<tokio::task::JoinHandle<()>>,
    atomic_facts_ready: bool,
    atomic_facts_epoch: u64,
    status: InstanceStatus,
    session: Option<OpenCodeServerSession>,
    session_generation: Option<String>,
    generation_counter: u64,
    next_turn_epoch: u64,
    sessions: HashMap<String, OpenCodeSession>,
    active_turns: HashMap<String, ActiveTurnState>,
    pending_approvals: HashMap<String, PendingApproval>,
    metadata_epoch: u64,
    metadata_task: Option<tokio::task::JoinHandle<()>>,
    version: Option<String>,
    authentication: Option<ProviderAuthentication>,
    usage: Option<ProviderUsage>,
}

impl Drop for InstanceMutable {
    fn drop(&mut self) { if let Some(task) = self.metadata_task.take() { task.abort(); } if let Some(task) = self.atomic_task.take() { task.abort(); } }
}

struct OpenCodeInstanceRuntime {
    atoms: ConversationAtoms,
    route: ProviderInstanceRoute,
    instance_kind: String,
    display_name: String,
    settings: OpenCodeInstanceSettings,
    capabilities: Mutex<ProviderCapabilities>,
    boot_id: String,
    mutable: Mutex<InstanceMutable>,
    mapper: OpenCodeProtocolMapper,
    events: Arc<dyn ProviderEventSink>,
}

impl OpenCodeInstanceRuntime {
    fn new(
        request: InstanceCreateRequest,
        settings: OpenCodeInstanceSettings,
        boot_id: String,
        events: Arc<dyn ProviderEventSink>,
    ) -> Self {
        let atoms = ConversationAtoms::default();
        let events = atoms.event_sink(request.route.clone(), events);
        Self {
            atoms,
            route: request.route.clone(),
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings: settings.clone(),
            capabilities: Mutex::new(OpenCodeProtocolMapper::base_capabilities()),
            boot_id,
            mutable: Mutex::new(InstanceMutable {
                atomic_task: None, atomic_facts_ready: false, atomic_facts_epoch: 0,
                status: InstanceStatus::Created,
                session: None,
                session_generation: None,
                generation_counter: 0,
                next_turn_epoch: 0,
                sessions: HashMap::new(),
                active_turns: HashMap::new(),
                pending_approvals: HashMap::new(),
                metadata_epoch: 0,
                metadata_task: None,
                version: (!settings.server_version.is_empty()).then(|| settings.server_version.clone()),
                authentication: None,
                usage: None,
            }),
            mapper: OpenCodeProtocolMapper::new(request.route),
            events,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        let mutable = lock(&self.mutable);
        self.snapshot_locked(&mutable)
    }

    fn snapshot_locked(&self, mutable: &InstanceMutable) -> ProviderInstance {
        self.mapper.instance(
            OPENCODE_PLUGIN_ID.to_string(),
            self.instance_kind.clone(),
            self.display_name.clone(),
            HarnessDescriptor {
                id: self.instance_kind.clone(),
                display_name: "OpenCode".to_string(),
                version: mutable.version.clone(),
                executable_path: Some(
                    self.settings
                        .server_executable
                        .to_string_lossy()
                        .into_owned(),
                ),
            },
            mutable.status,
            conversation_atoms::observed_capabilities(lock(&self.capabilities).clone(), mutable.atomic_facts_ready, mutable.atomic_facts_epoch),
            mutable.authentication.clone(),
            mutable.usage.clone(),
        )
    }

    fn cancel_metadata(mutable: &mut InstanceMutable) {
        mutable.metadata_epoch = mutable.metadata_epoch.saturating_add(1);
        if let Some(task) = mutable.metadata_task.take() {
            task.abort();
        }
    }

    fn refresh_metadata(self: &Arc<Self>, force: bool) {
        let mut mutable = lock(&self.mutable);
        if !matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready) {
            return;
        }
        if (!force || mutable.status == InstanceStatus::Starting) && mutable.metadata_task.is_some() {
            return;
        }
        Self::cancel_metadata(&mut mutable);
        let epoch = mutable.metadata_epoch;
        let owner = Arc::downgrade(self);
        let settings = self.settings.clone();
        let session = mutable.session.clone();
        mutable.metadata_task = Some(tokio::spawn(async move {
            let cwd = settings
                .workspace_root
                .clone()
                .or_else(default_server_working_directory);
            let probe = |args| {
                crate::background_probe::run(
                    &settings.server_executable,
                    args,
                    cwd.as_deref(),
                    settings.data_directory.as_deref(),
                    Duration::from_secs(30),
                )
            };
            let auth = async {
                let result = probe(&["auth", "list"]).await.ok();
                authentication_from_output(result.as_deref())
            };
            let usage = async {
                let result = probe(&["stats", "--days", "30"]).await.ok();
                usage_from_output(result.as_deref())
            };
            let catalog = async {
                let session = session?;
                let client = session.client();
                let discovered = tokio::task::spawn_blocking(move || {
                    std::thread::scope(|scope| {
                        let models = scope.spawn(|| client.list_models());
                        let agents = scope.spawn(|| client.list_agents());
                        let providers = scope.spawn(|| client.list_providers());
                        Some((agents.join().ok()?.ok()?, models.join().ok()?.ok()?, providers.join().ok()?.ok()?))
                    })
                }).await;
                let (agents, mut models, providers) = discovered.ok().flatten()?;
                if models.is_empty() {
                    models = discover_cli_models(&settings.server_executable, cwd.as_deref(),
                        settings.data_directory.as_deref()).await.ok()?;
                }
                OpenCodeProtocolMapper::capabilities(&agents, &models, &providers).ok()
            };
            let version = async {
                let output = probe(&["--version"]).await.ok()?;
                output.split_whitespace().find(|part| part.trim_start_matches('v').chars()
                    .next().is_some_and(|c| c.is_ascii_digit()))
                    .map(|version| version.trim_start_matches('v').to_string())
            };
            let (authentication, usage, capabilities, version) = tokio::join!(auth, usage, catalog, version);
            if let Some(owner) = owner.upgrade() {
                owner.apply_metadata(epoch, Some(authentication), Some(usage), capabilities, version);
            }
        }));
    }

    fn apply_metadata(
        &self,
        epoch: u64,
        authentication: Option<ProviderAuthentication>,
        usage: Option<ProviderUsage>,
        capabilities: Option<ProviderCapabilities>,
        version: Option<String>,
    ) {
        let mut mutable = lock(&self.mutable);
        if mutable.metadata_epoch != epoch || !matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready) {
            return;
        }
        if let Some(authentication) = authentication {
            mutable.authentication = Some(authentication);
        }
        if let Some(usage) = usage {
            mutable.usage = Some(usage);
        }
        if let Some(capabilities) = capabilities { *lock(&self.capabilities) = capabilities; }
        if let Some(version) = version { mutable.version = Some(version); }
        let previous = mutable.status;
        mutable.status = InstanceStatus::Ready;
        // Serialize publication with stop/refresh so an old scan cannot overwrite a new generation.
        let _ = self
            .events
            .publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".into(),
                params: InstanceStatusChangedEvent {
                    instance: self.snapshot_locked(&mutable),
                    previous_status: Some(previous),
                },
            });
    }

    fn starting_is_current(&self, attempt: u64) -> bool {
        let mutable = lock(&self.mutable);
        mutable.status == InstanceStatus::Starting && mutable.generation_counter == attempt
    }

    fn set_start_status(
        &self,
        attempt: u64,
        status: InstanceStatus,
    ) -> Result<ProviderInstance, ProtocolError> {
        let mut mutable = lock(&self.mutable);
        if mutable.status != InstanceStatus::Starting || mutable.generation_counter != attempt {
            return Err(protocol_error(
                "provider_start_cancelled",
                "Server startup cancelled".into(),
                true,
            ));
        }
        if !matches!(status, InstanceStatus::Ready) {
            Self::cancel_metadata(&mut mutable);
        }
        mutable.status = status;
        let instance = self.snapshot_locked(&mutable);
        self.events
            .publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".into(),
                params: InstanceStatusChangedEvent {
                    instance: instance.clone(),
                    previous_status: Some(InstanceStatus::Starting),
                },
            })?;
        Ok(instance)
    }

    fn status(&self) -> InstanceStatus { lock(&self.mutable).status }

    fn set_status(&self, status: InstanceStatus) -> Result<ProviderInstance, ProtocolError> {
        let mut mutable = lock(&self.mutable);
        let previous = mutable.status;
        if !matches!(status, InstanceStatus::Ready) {
            Self::cancel_metadata(&mut mutable);
        }
        mutable.status = status;
        let instance = self.snapshot_locked(&mutable);
        if previous != status {
            self.events
                .publish(ProtocolEvent::EventInstanceStatusChanged {
                    jsonrpc: "2.0".into(),
                    params: InstanceStatusChangedEvent {
                        instance: instance.clone(),
                        previous_status: Some(previous),
                    },
                })?;
        }
        Ok(instance)
    }

    fn ready_session(&self) -> Result<OpenCodeServerSession, ProtocolError> {
        let mutable = lock(&self.mutable);
        if mutable.status != InstanceStatus::Ready {
            return Err(protocol_error(
                "provider_unavailable",
                format!(
                    "OpenCode Provider instance {} is not ready",
                    self.route.provider_instance_id
                ),
                true,
            ));
        }
        mutable.session.clone().ok_or_else(|| {
            protocol_error(
                "provider_unavailable",
                "OpenCode Server session is unavailable".to_string(),
                true,
            )
        })
    }

    fn start_event_forwarder(
        self: &Arc<Self>,
        generation: String,
        incoming: Receiver<Result<OpenCodeEvent, OpenCodeServerError>>,
    ) {
        let runtime = Arc::downgrade(self);
        thread::spawn(move || {
            loop {
                let message = match incoming.recv() {
                    Ok(message) => message,
                    Err(_) => {
                        let Some(runtime) = runtime.upgrade() else {
                            return;
                        };
                        let current = {
                            let mutable = lock(&runtime.mutable);
                            mutable.session_generation.as_deref() == Some(generation.as_str())
                                && matches!(
                                    mutable.status,
                                    InstanceStatus::Ready | InstanceStatus::Starting
                                )
                        };
                        if current {
                            runtime.fail_from_event_forwarder(
                                &generation,
                                protocol_error(
                                    "opencode_event_stream_closed",
                                    "OpenCode event stream closed unexpectedly".to_string(),
                                    true,
                                ),
                            );
                        }
                        return;
                    }
                };
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let current = {
                    let mutable = lock(&runtime.mutable);
                    mutable.session_generation.as_deref() == Some(generation.as_str())
                        && matches!(
                            mutable.status,
                            InstanceStatus::Ready | InstanceStatus::Starting
                        )
                };
                if !current {
                    return;
                }
                match message {
                    Ok(event) => match runtime.map_event(&generation, event) {
                        Ok(events) => {
                            for event in events {
                                if let Err(error) = runtime.events.publish(event) {
                                    runtime.fail_from_event_forwarder(&generation, error);
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            runtime.fail_from_event_forwarder(&generation, error);
                            return;
                        }
                    },
                    Err(error) => {
                        runtime.fail_from_event_forwarder(
                            &generation,
                            OpenCodeProtocolMapper::error(error),
                        );
                        return;
                    }
                }
            }
        });
    }

    fn map_event(
        self: &Arc<Self>,
        generation: &str,
        event: OpenCodeEvent,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        match event.kind.as_str() {
            "session.next.prompt.admitted" => {
                let data: OpenCodePromptAdmittedEventData = decode_event(&event)?;
                if data.delivery != "queue" && data.delivery != "steer" {
                    return Err(event_shape_error(&event, "unknown prompt delivery"));
                }
                if data.delivery == "steer" {
                    return Ok(Vec::new());
                }
                let turn = {
                    let mut mutable = lock(&self.mutable);
                    let Some(active) = mutable.active_turns.get_mut(&data.session_id) else {
                        return Ok(Vec::new());
                    };
                    if active.prompt_message_id != data.message_id {
                        return Ok(Vec::new());
                    }
                    active.turn.updated_at = Some(data.timestamp);
                    active.turn.clone()
                };
                Ok(vec![self.mapper.turn_event(turn)])
            }
            "session.next.step.started" => {
                let data: OpenCodeStepStartedEventData = decode_event(&event)?;
                let turn = {
                    let mut mutable = lock(&self.mutable);
                    let Some(active) = mutable.active_turns.get_mut(&data.session_id) else {
                        return Ok(Vec::new());
                    };
                    if active
                        .assistant_message_id
                        .as_ref()
                        .is_some_and(|assistant| assistant != &data.assistant_message_id)
                    {
                        return Ok(Vec::new());
                    }
                    active.assistant_message_id = Some(data.assistant_message_id);
                    active.turn.status = TurnStatus::Running;
                    active.turn.started_at = active.turn.started_at.or(Some(data.timestamp));
                    active.turn.updated_at = Some(data.timestamp);
                    active.turn.clone()
                };
                Ok(vec![self.mapper.turn_event(turn)])
            }
            "session.next.text.delta" | "session.next.reasoning.delta" => {
                let data: OpenCodeDeltaEventData = decode_event(&event)?;
                let turn = lock(&self.mutable).active_turns.get(&data.session_id).and_then(
                    |active| {
                        (active.assistant_message_id.as_deref()
                            == Some(data.assistant_message_id.as_str()))
                        .then(|| active.turn.clone())
                    },
                );
                let Some(turn) = turn else {
                    return Ok(Vec::new());
                };
                let (item_id, content_suffix, kind) = if event.kind == "session.next.text.delta" {
                    (data.text_id, "text", ConversationContentKind::Text)
                } else {
                    (
                        data.reasoning_id,
                        "summary:0",
                        ConversationContentKind::ReasoningSummary,
                    )
                };
                let native_item_id = item_id.ok_or_else(|| {
                    event_shape_error(&event, "delta event is missing its item identifier")
                })?;
                let item_id = format!("{}:{native_item_id}", data.assistant_message_id);
                let content_id = format!("{item_id}:{content_suffix}");
                Ok(vec![self.mapper.delta_event(
                    &turn,
                    item_id,
                    content_id,
                    kind,
                    data.delta,
                )])
            }
            "session.next.step.ended" => {
                let data: OpenCodeStepEndedEventData = decode_event(&event)?;
                let (turn, should_start_waiter, should_finish, epoch) = {
                    let mut mutable = lock(&self.mutable);
                    let Some(active) = mutable.active_turns.get_mut(&data.session_id) else {
                        return Ok(Vec::new());
                    };
                    if active.assistant_message_id.as_deref()
                        != Some(data.assistant_message_id.as_str())
                    {
                        return Ok(Vec::new());
                    }
                    active.assistant_message_id = None;
                    active.turn.updated_at = Some(data.timestamp);
                    active.latest_step_ended_at = Some(data.timestamp);
                    let terminal_step = data.finish != "tool-calls";
                    let should_finish = terminal_step && active.wait_confirmed;
                    let should_start_waiter = terminal_step
                        && !active.wait_started
                        && !should_finish;
                    if should_start_waiter {
                        active.wait_started = true;
                    }
                    (
                        active.turn.clone(),
                        should_start_waiter,
                        should_finish,
                        active.epoch,
                    )
                };
                let turn_resource_id = turn.resource.native_resource_id.clone();
                if should_finish {
                    let events = self.finish_turn(
                        &data.session_id,
                        &turn_resource_id,
                        epoch,
                        TurnStatus::Completed,
                        Some(data.timestamp),
                    );
                    return Ok(events);
                } else if should_start_waiter {
                    self.start_completion_waiter(
                        generation.to_string(),
                        data.session_id,
                        turn_resource_id,
                        epoch,
                    )?;
                }
                Ok(vec![self.mapper.turn_event(turn)])
            }
            "session.next.step.failed" => {
                let data: OpenCodeStepFailedEventData = decode_event(&event)?;
                let expected = lock(&self.mutable)
                    .active_turns
                    .get(&data.session_id)
                    .and_then(|active| {
                        (active.assistant_message_id.as_deref()
                            == Some(data.assistant_message_id.as_str()))
                        .then(|| (active.turn.resource.native_resource_id.clone(), active.epoch))
                    });
                let Some((turn_resource_id, epoch)) = expected else {
                    return Ok(Vec::new());
                };
                Ok(self.finish_turn(
                    &data.session_id,
                    &turn_resource_id,
                    epoch,
                    TurnStatus::Failed,
                    Some(data.timestamp),
                ))
            }
            "permission.v2.asked" => {
                let data: OpenCodePermissionAskedEventData = decode_event(&event)?;
                if data
                    .source
                    .as_ref()
                    .is_some_and(|source| source.kind != "tool")
                {
                    return Err(event_shape_error(
                        &event,
                        "permission source type is not the official tool variant",
                    ));
                }
                let turn = {
                    let mut mutable = lock(&self.mutable);
                    if let Some(active) = mutable.active_turns.get_mut(&data.session_id) {
                        active.turn.status = TurnStatus::WaitingApproval;
                        Some(active.turn.clone())
                    } else {
                        None
                    }
                };
                let Some(turn) = turn else {
                    let client = self.ready_session()?.client();
                    client
                        .reply_permission(
                            &data.session_id,
                            &data.id,
                            OpenCodePermissionReply::Reject,
                        )
                        .map_err(OpenCodeProtocolMapper::error)?;
                    eprintln!(
                        "OpenCode Provider rejected unroutable permission request {}",
                        data.id
                    );
                    return Ok(Vec::new());
                };
                let native_approval_id = approval_resource_id(
                    generation,
                    &data.session_id,
                    &data.id,
                );
                let approval = self.mapper.approval(
                    &data,
                    &turn,
                    native_approval_id.clone(),
                );
                lock(&self.mutable).pending_approvals.insert(
                    native_approval_id,
                    PendingApproval {
                        session_generation: generation.to_string(),
                        session_id: data.session_id,
                        request_id: data.id,
                        approval: approval.clone(),
                    },
                );
                Ok(vec![
                    self.mapper.turn_event(turn),
                    self.mapper.approval_requested_event(approval),
                ])
            }
            "permission.v2.replied" => {
                let data: OpenCodePermissionRepliedEventData = decode_event(&event)?;
                let native_approval_id = approval_resource_id(
                    generation,
                    &data.session_id,
                    &data.request_id,
                );
                let pending = lock(&self.mutable)
                    .pending_approvals
                    .remove(&native_approval_id);
                let Some(pending) = pending else {
                    return Ok(Vec::new());
                };
                if pending.session_id != data.session_id {
                    return Err(event_shape_error(
                        &event,
                        "permission reply sessionID does not match the pending request",
                    ));
                }
                let decision = match data.reply.as_str() {
                    "once" | "always" => ApprovalDecision::Approve,
                    "reject" => ApprovalDecision::Deny,
                    _ => return Err(event_shape_error(&event, "unknown permission reply")),
                };
                let (approval, approval_event) = self.mapper.resolve_approval(
                    pending.approval,
                    decision,
                    None,
                );
                let mut events = vec![approval_event];
                let turn = {
                    let mut mutable = lock(&self.mutable);
                    let still_waiting = has_pending_approval(
                        &mutable.pending_approvals,
                        &data.session_id,
                    );
                    mutable.active_turns.get_mut(&data.session_id).map(|active| {
                        if !still_waiting {
                            active.turn.status = TurnStatus::Running;
                        }
                        if approval.resolved_at.is_some() {
                            active.turn.updated_at = approval.resolved_at;
                        }
                        active.turn.clone()
                    })
                };
                if let Some(turn) = turn {
                    events.push(self.mapper.turn_event(turn));
                }
                Ok(events)
            }
            _ => Ok(Vec::new()),
        }
    }

    fn start_completion_waiter(
        self: &Arc<Self>,
        generation: String,
        session_id: String,
        turn_resource_id: String,
        epoch: u64,
    ) -> Result<(), ProtocolError> {
        let client = self.ready_session()?.client();
        let runtime = Arc::downgrade(self);
        thread::spawn(move || {
            let result = client.wait_session(&session_id, || {
                runtime.upgrade().is_some_and(|runtime| {
                    let mutable = lock(&runtime.mutable);
                    mutable.session_generation.as_deref() == Some(generation.as_str())
                        && mutable.active_turns.get(&session_id).is_some_and(|active|
                            active.epoch == epoch && active.turn.resource.native_resource_id == turn_resource_id)
                })
            });
            let Some(runtime) = runtime.upgrade() else {
                return;
            };
            match result {
                Ok(()) => {
                    let completed_at = {
                        let mut mutable = lock(&runtime.mutable);
                        if mutable.session_generation.as_deref() != Some(generation.as_str()) {
                            return;
                        }
                        let Some(active) = mutable.active_turns.get_mut(&session_id) else {
                            return;
                        };
                        if active.epoch != epoch
                            || active.turn.resource.native_resource_id != turn_resource_id
                            || !active.wait_started
                        {
                            return;
                        }
                        if active.assistant_message_id.is_some() {
                            active.wait_confirmed = true;
                            return;
                        }
                        active.latest_step_ended_at
                    };
                    let events = runtime.finish_turn(
                        &session_id,
                        &turn_resource_id,
                        epoch,
                        TurnStatus::Completed,
                        completed_at,
                    );
                    for event in events {
                        if let Err(error) = runtime.events.publish(event) {
                            runtime.fail_from_event_forwarder(&generation, error);
                            return;
                        }
                    }
                }
                Err(error) => {
                    let current = {
                        let mutable = lock(&runtime.mutable);
                        mutable.session_generation.as_deref() == Some(generation.as_str())
                            && mutable.active_turns.get(&session_id).is_some_and(|active| {
                                active.epoch == epoch
                                    && active.turn.resource.native_resource_id == turn_resource_id
                            })
                    };
                    if current {
                        runtime.fail_from_event_forwarder(
                            &generation,
                            OpenCodeProtocolMapper::error(error),
                        );
                    }
                }
            }
        });
        Ok(())
    }

    fn finish_turn(
        &self,
        session_id: &str,
        expected_turn_resource_id: &str,
        expected_epoch: u64,
        status: TurnStatus,
        completed_at: Option<u64>,
    ) -> Vec<ProtocolEvent> {
        let (turn, approvals) = {
            let mut mutable = lock(&self.mutable);
            if mutable.active_turns.get(session_id).is_none_or(|active| {
                active.epoch != expected_epoch
                    || active.turn.resource.native_resource_id != expected_turn_resource_id
            }) {
                return Vec::new();
            }
            let turn = mutable.active_turns.remove(session_id).map(|active| {
                let mut turn = active.turn;
                turn.status = status;
                if completed_at.is_some() {
                    turn.updated_at = completed_at;
                }
                turn.completed_at = completed_at;
                turn
            });
            let mut approvals = Vec::new();
            mutable.pending_approvals.retain(|_, pending| {
                if pending.session_id == session_id {
                    approvals.push(pending.approval.clone());
                    false
                } else {
                    true
                }
            });
            (turn, approvals)
        };
        let mut events = turn
            .into_iter()
            .map(|turn| self.mapper.turn_event(turn))
            .collect::<Vec<_>>();
        events.extend(approvals.into_iter().map(|approval| {
            self.mapper.expire_approval(approval, completed_at).1
        }));
        events
    }

    fn fail_from_event_forwarder(&self, generation: &str, error: ProtocolError) {
        eprintln!("OpenCode Provider event forwarding failed: {}", error.message);
        let now = now_ms();
        let (previous_status, session, turns, approvals) = {
            let mut mutable = lock(&self.mutable);
            if mutable.session_generation.as_deref() != Some(generation)
                || !matches!(
                    mutable.status,
                    InstanceStatus::Ready | InstanceStatus::Starting
                )
            {
                return;
            }
            let previous_status = mutable.status;
            Self::cancel_metadata(&mut mutable);
            mutable.status = InstanceStatus::Error;
            mutable.session_generation = None;
            let session = mutable.session.take();
            let turns = mutable
                .active_turns
                .drain()
                .map(|(_, active)| {
                    let mut turn = active.turn;
                    turn.status = TurnStatus::Failed;
                    turn.updated_at = Some(now);
                    turn.completed_at = Some(now);
                    turn
                })
                .collect::<Vec<_>>();
            let approvals = mutable
                .pending_approvals
                .drain()
                .map(|(_, pending)| pending.approval)
                .collect::<Vec<_>>();
            (previous_status, session, turns, approvals)
        };
        if let Some(session) = session {
            let _ = session.shutdown();
        }
        for turn in turns {
            let _ = self.events.publish(self.mapper.turn_event(turn));
        }
        for approval in approvals {
            let (_, event) = self.mapper.expire_approval(approval, Some(now));
            let _ = self.events.publish(event);
        }
        let _ = self.events.publish(ProtocolEvent::EventInstanceStatusChanged {
            jsonrpc: "2.0".to_string(),
            params: InstanceStatusChangedEvent {
                instance: self.snapshot(),
                previous_status: Some(previous_status),
            },
        });
    }
}

struct ProviderState {
    host_device_id: Option<String>,
    initialized_client_id: Option<String>,
    instances: HashMap<String, Arc<OpenCodeInstanceRuntime>>,
    selected_runtime: Option<RuntimeInstallation>,
}

pub struct OpenCodeProvider {
    observation: codepet_observation::Observation,
    scanner: local_runtime::RuntimeScanner,
    state: Mutex<ProviderState>,
    events: Arc<dyn ProviderEventSink>,
    boot_id: String,
    shutdown: AtomicBool,
}

impl OpenCodeProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self {
            scanner: local_runtime::RuntimeScanner::new(events.clone()),
            observation: codepet_observation::Observation::new(codepet_observation::Definition {
                windows_command_override: false,
                name: "opencode", config: codepet_observation::config_home("XDG_CONFIG_HOME", codepet_observation::home().join(".config")).join("opencode/opencode.json"), events: &["session.created", "session.updated", "session.deleted", "session.status", "session.idle", "session.error", "permission.asked", "permission.replied", "question.asked", "question.replied", "question.rejected"], plugin: Some(include_str!("observation-plugin.ts")),
            }, events.clone()),
            state: Mutex::new(ProviderState {
                host_device_id: None,
                initialized_client_id: None,
                instances: HashMap::new(),
                selected_runtime: None,
            }),
            events,
            boot_id: Uuid::new_v4().to_string(),
            shutdown: AtomicBool::new(false),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn descriptor() -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            plugin_id: OPENCODE_PLUGIN_ID.to_string(),
            display_name: "OpenCode".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_workspace_root: default_remote_workspace_root("opencode"),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
            instance_kinds: vec![OPENCODE_INSTANCE_KIND.to_string()],
        }
    }

    fn instance(
        &self,
        route: &ProviderInstanceRoute,
    ) -> Result<Arc<OpenCodeInstanceRuntime>, ProtocolError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(protocol_error(
                "provider_shutdown",
                "OpenCode Provider is shut down".to_string(),
                false,
            ));
        }
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
                        "unknown OpenCode Provider instance: {}",
                        route.provider_instance_id
                    ),
                    false,
                )
            })
    }

    fn resource_instance(
        &self,
        resource: &ProviderResourceId,
    ) -> Result<Arc<OpenCodeInstanceRuntime>, ProtocolError> {
        validate_resource(resource)?;
        self.instance(&ProviderInstanceRoute {
            device_id: resource.device_id.clone(),
            provider_plugin_id: resource.provider_plugin_id.clone(),
            provider_instance_id: resource.provider_instance_id.clone(),
        })
    }
}

impl Provider for OpenCodeProvider {
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
            self.scanner.start_cancellable("opencode", "opencode-ai", || discover_path_candidates("opencode"), inspect_runtime_candidate);
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

    fn runtime_get_installed<'a>(&'a self, request: RuntimeGetInstalledRequest) -> ProtocolFuture<'a, RuntimeGetInstalledResponse> {
        Box::pin(async move {
            if request.refresh == Some(true) && self.scanner.snapshot().scanning != Some(true) {
                self.scanner.stop();
                self.scanner.start_cancellable("opencode", "opencode-ai", || discover_path_candidates("opencode"), inspect_runtime_candidate);
            }
            if request.refresh == Some(true) {
                let instances = lock(&self.state).instances.values().cloned().collect::<Vec<_>>();
                for instance in instances { instance.refresh_metadata(true); }
            }
            Ok(self.scanner.snapshot())
        })
    }
    fn runtime_select<'a>(&'a self, request: RuntimeSelectRequest) -> ProtocolFuture<'a, RuntimeSelectResponse> {
        Box::pin(async move {
            let mut candidate = request.candidate;
            candidate.executable_path = local_runtime::resolve_executable(std::path::Path::new(&candidate.executable_path), "opencode", "opencode-ai")
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
            let selected = if request.settings.contains_key("serverExecutable") { None } else {
                let current = { lock(&self.state).selected_runtime.clone() };
                let installation = match current {
                    Some(selected) => Some(selected),
                    None => {
                        let inventory=self.scanner.snapshot();
                        if inventory.scanning==Some(true) {return Err(protocol_error("runtime_scanning", "Runtime discovery is still in progress".into(), true));}
                        inventory.installed.into_iter().next()
                    },
                };
                Some(installation.ok_or_else(|| protocol_error("provider_unavailable", "OpenCode Provider did not find a local runtime".to_string(), true))?)
            };
            if let Some(selected) = selected.as_ref() {
                request.settings.insert("serverExecutable".to_string(), json!(selected.executable_path.clone()));
                request.settings.insert("serverVersion".to_string(), json!(selected.version.clone()));
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
            let runtime = Arc::new(OpenCodeInstanceRuntime::new(
                request,
                settings,
                self.boot_id.clone(),
                self.events.clone(),
            ));
            let instance = runtime.snapshot();
            state
                .instances
                .insert(runtime.route.provider_instance_id.clone(), runtime.clone());
            Ok(InstanceCreateResponse { instance })
        })
    }

    fn instance_start<'a>(
        &'a self,
        request: InstanceStartRequest,
    ) -> ProtocolFuture<'a, InstanceStartResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let (attempt, previous_status) = {
                let mut mutable = lock(&runtime.mutable);
                if matches!(mutable.status, InstanceStatus::Ready | InstanceStatus::Starting) {
                    drop(mutable);
                    return Ok(InstanceStartResponse { instance: runtime.snapshot() });
                }
                if mutable.status == InstanceStatus::Stopping {
                    return Err(protocol_error("provider_instance_starting", "OpenCode lifecycle transition is in progress".into(), true));
                }
                let previous = mutable.status;
                mutable.status = InstanceStatus::Starting;
                mutable.generation_counter = mutable.generation_counter.saturating_add(1);
                (mutable.generation_counter, previous)
            };
            runtime.events.publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent {
                    instance: runtime.snapshot(), previous_status: Some(previous_status),
                },
            })?;
            let executable = runtime.settings.server_executable.clone();
            let version = runtime.settings.server_version.clone();
            let args = runtime.settings.server_args.clone();
            let working_directory = runtime
                .settings
                .workspace_root
                .clone()
                .or_else(default_server_working_directory);
            let generation = format!("{}:{attempt}", runtime.boot_id);
            let session_generation = generation.clone();
            let owner = runtime.clone();
            let session = tokio::task::spawn_blocking(move || {
                let session = OpenCodeServerSession::spawn_while_current(
                    &executable,
                    &args,
                    &version,
                    session_generation,
                    working_directory.as_deref(),
                    owner.settings.data_directory.as_deref(),
                    || owner.starting_is_current(attempt),
                )?;
                // Register before returning from the blocking task. Dropping the
                // awaiting future must not orphan a successfully spawned child.
                let mut mutable = lock(&owner.mutable);
                if mutable.status != InstanceStatus::Starting || mutable.generation_counter != attempt {
                    drop(mutable);
                    let _ = session.shutdown();
                    return Err(OpenCodeServerError::Protocol("Server startup cancelled".into()));
                }
                mutable.session = Some(session.clone());
                mutable.session_generation = Some(session.generation().to_string());
                Ok(session)
            })
            .await
            .map_err(provider_task_error)?;
            let session = match session {
                Ok(session) => session,
                Err(error) => {
                    let _ = runtime.set_start_status(attempt, InstanceStatus::Error);
                    return Err(OpenCodeProtocolMapper::error(error));
                }
            };
            let incoming = match session.subscribe() {
                Ok(incoming) => incoming,
                Err(error) => {
                    let _ = session.shutdown();
                    let _ = runtime.set_start_status(attempt, InstanceStatus::Error);
                    return Err(OpenCodeProtocolMapper::error(error));
                }
            };
            let generation = session.generation().to_string();
            {
                let mut mutable = lock(&runtime.mutable);
                if mutable.status != InstanceStatus::Starting || mutable.generation_counter != attempt {
                    drop(mutable);
                    let _ = session.shutdown();
                    return Err(protocol_error("provider_start_cancelled", "Server startup cancelled".into(), true));
                }
                *lock(&runtime.capabilities) = OpenCodeProtocolMapper::base_capabilities();
                mutable.session = Some(session.clone());
                mutable.session_generation = Some(generation.clone());
                mutable.sessions.clear();
                mutable.active_turns.clear();
                mutable.pending_approvals.clear();

            }
            let instance = runtime.snapshot();
            runtime.start_atomic_poll(generation.clone());
            runtime.start_event_forwarder(generation, incoming);
            runtime.refresh_metadata(false);
            Ok(InstanceStartResponse { instance })
        })
    }

    fn instance_stop<'a>(
        &'a self,
        request: InstanceStopRequest,
    ) -> ProtocolFuture<'a, InstanceStopResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if matches!(runtime.status(), InstanceStatus::Created | InstanceStatus::Stopped) {
                return Ok(InstanceStopResponse {
                    instance: runtime.set_status(InstanceStatus::Stopped)?,
                });
            }
            runtime.set_status(InstanceStatus::Stopping)?;
            let session = {
                let mut mutable = lock(&runtime.mutable);
                mutable.generation_counter = mutable.generation_counter.saturating_add(1);
                mutable.session_generation = None;
                mutable.atomic_facts_ready = false;
                if let Some(task) = mutable.atomic_task.take() { task.abort(); }
                mutable.active_turns.clear();
                mutable.pending_approvals.clear();
                mutable.session.take()
            };
            if let Some(session) = session {
                tokio::task::spawn_blocking(move || session.shutdown())
                    .await
                    .map_err(provider_task_error)?
                    .map_err(OpenCodeProtocolMapper::error)?;
            }
            Ok(InstanceStopResponse {
                instance: runtime.set_status(InstanceStatus::Stopped)?,
            })
        })
    }

    fn instance_destroy<'a>(
        &'a self,
        request: InstanceDestroyRequest,
    ) -> ProtocolFuture<'a, InstanceDestroyResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if matches!(
                runtime.status(),
                InstanceStatus::Ready | InstanceStatus::Starting | InstanceStatus::Stopping
            ) {
                return Err(protocol_error(
                    "provider_instance_running",
                    "Stop the OpenCode Provider instance before destroying it".to_string(),
                    false,
                ));
            }
            lock(&self.state)
                .instances
                .remove(&request.route.provider_instance_id);
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

    fn conversation_active_list<'a>(&'a self, request: codepet_provider_sdk::ConversationActiveListRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationActiveListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if !lock(&runtime.mutable).atomic_facts_ready { return Err(protocol_error("unsupported", "complete native activity observation is not ready".into(), true)); }
            let generation = runtime.query_generation()?;
            if let Some(page) = runtime.atoms.active_cached(&generation, &request)? { return Ok(page); }
            let event_epoch = runtime.atoms.event_epoch();
            let rows = self.complete_conversation_summaries(&request.route).await?;
            if generation != runtime.query_generation()? || event_epoch != runtime.atoms.event_epoch() { return Err(conversation_atoms::generation_changed()); }
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
            if request.query.is_some() {
                let runtime = self.instance(&request.route)?;
                let generation = runtime.query_generation()?;
                if let Some(page) = runtime.atoms.list_cached(&generation, &request)? { return Ok(page); }
                let event_epoch = runtime.atoms.event_epoch();
                let mut rows = self.complete_conversation_summaries(&request.route).await?;
                if let Some(codepet_provider_sdk::ConversationListQuery::ConversationIdsQuery(query)) = &request.query {
                    self.complete_requested_summaries(&request.route, &query.ids, &mut rows).await?;
                }
                if generation != runtime.query_generation()? || event_epoch != runtime.atoms.event_epoch() { return Err(conversation_atoms::generation_changed()); }
                return runtime.atoms.list(&generation, &request, rows);
            }
            if let Some(scope) = request.reader_scope.clone() {
                let mut native_request = request;
                native_request.reader_scope = None;
                let mut response = self.conversation_list(native_request).await?;
                codepet_provider_sdk::conversation_state::SharedConversationStateStore::from_env()?.decorate_many(&scope, &mut response.conversations)?;
                return Ok(response);
            }

            if request.limit == Some(0) {
                return Err(protocol_error(
                    "invalid_request",
                    "conversation list limit must be greater than zero".to_string(),
                    false,
                ));
            }
            let runtime = self.instance(&request.route)?;
            let session = runtime.ready_session()?;
            let client = session.client();
            let cursor = request.cursor;
            let limit = request.limit;
            let (page, active) = tokio::task::spawn_blocking(move || {
                let page = client.list_sessions(cursor.as_deref(), limit)?;
                let active = client.active_sessions()?;
                Ok::<_, OpenCodeServerError>((page, active))
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            for session in page.data.iter() {
                validate_opencode_session(session)?;
            }
            if active.keys().any(|session_id| session_id.trim().is_empty()) {
                return Err(protocol_error(
                    "opencode_protocol_error",
                    "OpenCode active session map contains an empty session id".to_string(),
                    false,
                ));
            }
            let conversations = {
                let mut mutable = lock(&runtime.mutable);
                for session in page.data.iter() {
                    mutable.sessions.insert(session.id.clone(), session.clone());
                }
                page.data
                    .iter()
                    .map(|session| {
                        let turn = mutable
                            .active_turns
                            .get(&session.id)
                            .map(|active| active.turn.clone());
                        let waiting = has_pending_approval(
                            &mutable.pending_approvals,
                            &session.id,
                        );
                        runtime.mapper.conversation(
                            session,
                            active.contains_key(&session.id),
                            turn,
                            waiting,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            Ok(ConversationListResponse {
                conversations,
                page_info: PageInfo {
                    next_cursor: page.cursor.next,
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
            let cursor = request.cursor;
            let limit = conversation_get_limit(request.limit)?;
            let session = runtime.ready_session()?;
            let client = session.client();
            let requested_id = conversation_id.clone();
            let (session, active, message_page) = tokio::task::spawn_blocking(move || {
                let session = client.get_session(&requested_id)?;
                let active = client.active_sessions()?.contains_key(&requested_id);
                let page = client.list_messages(&requested_id, cursor.as_deref(), limit)?;
                validate_conversation_message_page(&page, cursor.as_deref(), limit)?;
                Ok::<_, OpenCodeServerError>((session, active, page))
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            if session.id != conversation_id {
                return Err(protocol_error(
                    "opencode_protocol_error",
                    "OpenCode session response id does not match the requested id".to_string(),
                    false,
                ));
            }
            validate_opencode_session(&session)?;
            let conversation = {
                let mut mutable = lock(&runtime.mutable);
                mutable.sessions.insert(session.id.clone(), session.clone());
                let turn = mutable
                    .active_turns
                    .get(&session.id)
                    .map(|active| active.turn.clone());
                let waiting = has_pending_approval(&mutable.pending_approvals, &session.id);
                runtime
                    .mapper
                    .conversation(&session, active, turn, waiting)
            };
            let items = runtime
                .mapper
                .conversation_items(&conversation.resource, &message_page.data);
            Ok(ConversationGetResponse {
                conversation,
                items,
                page_info: Some(PageInfo {
                    next_cursor: message_page.cursor.next,
                }),
            })
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: ConversationAcquireInteractionRequest,
    ) -> ProtocolFuture<'a, ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            self.resource_instance(&request.conversation)?;
            Ok(ConversationAcquireInteractionResponse {
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
                lease_expires_at: None,
            })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
            if request.project.is_some() {
                return Err(capability_unsupported(
                    "OpenCode Provider does not support project-owned conversations",
                ));
            }
            if request.title.is_some() {
                return Err(capability_unsupported(
                    "OpenCode V2 session.create does not support setting a title",
                ));
            }
            if request.extension.is_some() {
                return Err(capability_unsupported(
                    "OpenCode Provider does not define conversation.create extensions",
                ));
            }
            if request.workspace_mode.as_deref().unwrap_or("main") != "main" {
                return Err(protocol_error(
                    "unsupported_workspace_mode",
                    "OpenCode Provider only supports main workspace mode".to_string(),
                    false,
                ));
            }
            let runtime = self.instance(&request.route)?;
            let (agent, model) = resolve_create_selection(
                &lock(&runtime.capabilities), &request.permission_level,
                request.model, request.reasoning_effort,
            )?;
            let workspace_root = request
                .workspace_root
                .map(PathBuf::from)
                .or_else(|| runtime.settings.workspace_root.clone())
                .ok_or_else(|| {
                    protocol_error(
                        "workspace_required",
                        "OpenCode session.create requires an absolute workspaceRoot".to_string(),
                        false,
                    )
                })?;
            let workspace_root = ensure_opencode_workspace(workspace_root)?;
            let directory = workspace_root.to_string_lossy().to_string();
            let session = runtime.ready_session()?;
            let client = session.client();
            let created = tokio::task::spawn_blocking(move || {
                client.create_session(&OpenCodeSessionCreate {
                    agent,
                    model,
                    location: OpenCodeLocationRef {
                        directory,
                        workspace_id: None,
                    },
                })
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            validate_opencode_session(&created)?;
            let conversation = runtime
                .mapper
                .conversation(&created, false, None, false);
            lock(&runtime.mutable)
                .sessions
                .insert(created.id.clone(), created);
            runtime
                .events
                .publish(ProtocolEvent::EventConversationUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: ConversationUpsertedEvent {
                        conversation: conversation.clone(),
                    },
                })?;
            Ok(ConversationCreateResponse { conversation })
        })
    }

    fn turn_start<'a>(
        &'a self,
        request: TurnStartRequest,
    ) -> ProtocolFuture<'a, TurnStartResponse> {
        Box::pin(async move {
            if request.input.text.is_empty() {
                return Err(protocol_error(
                    "invalid_request",
                    "turn.start message must not be empty".to_string(),
                    false,
                ));
            }
            let runtime = self.resource_instance(&request.conversation)?;
            let base_revision = lock(&runtime.capabilities).revision.clone();
            if request.capability_revision != base_revision && request.capability_revision != runtime.snapshot().capabilities.revision {
                return Err(protocol_error(
                    "stale_capability_revision",
                    "turn.start capabilityRevision no longer matches the Provider instance"
                        .to_string(),
                    true,
                ));
            }
            let requested_selection = request.selection;
            let input_text = request.input.text;
            let client_request_id = request.client_request_id;
            let conversation_resource = runtime
                .mapper
                .resource(request.conversation.native_resource_id.clone());
            let conversation_id = request.conversation.native_resource_id;
            let session = runtime.ready_session()?;
            let generation = session.generation().to_string();
            let client = session.client();
            let active_client = client.clone();
            let active_conversation_id = conversation_id.clone();
            let server_active = tokio::task::spawn_blocking(move || {
                active_client
                    .active_sessions()
                    .map(|active| active.contains_key(&active_conversation_id))
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            if server_active {
                return Err(protocol_error(
                    "turn_already_active",
                    "OpenCode session already has active execution".to_string(),
                    false,
                ));
            }
            let configuration_client = client.clone();
            let configuration_conversation_id = conversation_id.clone();
            let current_session = tokio::task::spawn_blocking(move || {
                configuration_client.get_session(&configuration_conversation_id)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            let effective_selection = resolve_opencode_selection(
                &lock(&runtime.capabilities),
                requested_selection,
                &current_session,
            )?;
            let selected_agent = effective_selection
                .access_mode_id
                .clone()
                .ok_or_else(|| protocol_error(
                    "provider_capability_invalid",
                    "OpenCode access mode has no selection".to_string(),
                    false,
                ))?;
            let selected_model = selected_opencode_model(&effective_selection)?;
            let switch_client = client.clone();
            let switch_conversation_id = conversation_id.clone();
            let configured_session = tokio::task::spawn_blocking(move || {
                let mut configured = current_session;
                if configured.agent.as_deref() != Some(selected_agent.as_str()) {
                    switch_client.switch_agent(
                        &switch_conversation_id,
                        selected_agent.clone(),
                    )?;
                    configured.agent = Some(selected_agent);
                }
                if configured.model.as_ref() != Some(&selected_model) {
                    switch_client.switch_model(
                        &switch_conversation_id,
                        selected_model.clone(),
                    )?;
                    configured.model = Some(selected_model);
                }
                Ok::<_, OpenCodeServerError>(configured)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            let (epoch, turn_resource_id, message_id) = {
                let mut mutable = lock(&runtime.mutable);
                if mutable.session_generation.as_deref() != Some(generation.as_str()) {
                    return Err(protocol_error(
                        "provider_unavailable",
                        "OpenCode Server session changed while starting a turn".to_string(),
                        true,
                    ));
                }
                if mutable.active_turns.contains_key(&conversation_id) {
                    return Err(protocol_error(
                        "turn_already_active",
                        "OpenCode session already has active execution".to_string(),
                        false,
                    ));
                }
                mutable
                    .sessions
                    .insert(conversation_id.clone(), configured_session);
                mutable.next_turn_epoch = mutable.next_turn_epoch.saturating_add(1);
                let epoch = mutable.next_turn_epoch;
                let turn_resource_id = turn_resource_id(
                    &generation,
                    epoch,
                    &conversation_id,
                    &client_request_id,
                );
                let message_id = message_id(
                    "start",
                    &generation,
                    epoch,
                    &conversation_id,
                    &client_request_id,
                );
                let provisional = runtime.mapper.turn(
                    &conversation_id,
                    &turn_resource_id,
                    TurnStatus::Queued,
                    None,
                    None,
                    None,
                );
                mutable.active_turns.insert(
                    conversation_id.clone(),
                    ActiveTurnState {
                        epoch,
                        turn: provisional.clone(),
                        prompt_message_id: message_id.clone(),
                        assistant_message_id: None,
                        wait_started: false,
                        wait_confirmed: false,
                        latest_step_ended_at: None,
                    },
                );
                (epoch, turn_resource_id, message_id)
            };
            let native_conversation_id = conversation_id.clone();
            let sent_message_id = message_id.clone();
            let sent_text = input_text.clone();
            let admission = tokio::task::spawn_blocking(move || {
                client.prompt(
                    &native_conversation_id,
                    &OpenCodePromptRequest {
                        id: sent_message_id,
                        prompt: OpenCodePrompt {
                            text: sent_text,
                        },
                        delivery: OpenCodeDelivery::Queue,
                    },
                )
            })
            .await
            .map_err(provider_task_error)?;
            let admission = match admission {
                Ok(admission) => admission,
                Err(error) => {
                    let mut mutable = lock(&runtime.mutable);
                    if mutable.active_turns.get(&conversation_id).is_some_and(|active| {
                        active.epoch == epoch
                            && active.turn.resource.native_resource_id == turn_resource_id
                    }) {
                        mutable.active_turns.remove(&conversation_id);
                    }
                    return Err(OpenCodeProtocolMapper::error(error));
                }
            };
            if let Err(error) = validate_admission(
                &admission,
                &conversation_id,
                &message_id,
                "queue",
            ) {
                let mut mutable = lock(&runtime.mutable);
                if mutable.active_turns.get(&conversation_id).is_some_and(|active| {
                    active.epoch == epoch
                        && active.turn.resource.native_resource_id == turn_resource_id
                }) {
                    mutable.active_turns.remove(&conversation_id);
                }
                return Err(error);
            }
            let turn = {
                let mut mutable = lock(&runtime.mutable);
                let active = mutable
                    .active_turns
                    .get_mut(&conversation_id)
                    .filter(|active| {
                        active.epoch == epoch
                            && active.turn.resource.native_resource_id == turn_resource_id
                            && active.prompt_message_id == message_id
                    })
                    .ok_or_else(|| {
                        protocol_error(
                            "turn_not_active",
                            "OpenCode turn completed while the prompt response was in flight"
                                .to_string(),
                            false,
                        )
                    })?;
                active.turn.updated_at = Some(
                    active
                        .turn
                        .updated_at
                        .unwrap_or(0)
                        .max(admission.time_created),
                );
                active.turn.clone()
            };
            let user_item = runtime.mapper.user_message_item(
                &conversation_resource,
                &turn,
                message_id,
                input_text,
            );
            Ok(TurnStartResponse {
                accepted: true,
                turn,
                user_item: Some(user_item),
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
            if request.message.is_empty() {
                return Err(protocol_error(
                    "invalid_request",
                    "turn.steer message must not be empty".to_string(),
                    false,
                ));
            }
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            let expected_active = {
                let mutable = lock(&runtime.mutable);
                mutable.active_turns.get(&conversation_id).cloned()
            }
            .ok_or_else(|| {
                protocol_error(
                    "turn_not_active",
                    "OpenCode session has no active Provider turn to steer".to_string(),
                    false,
                )
            })?;
            if expected_active.turn.resource.native_resource_id
                != request.turn.native_resource_id
            {
                return Err(protocol_error(
                    "stale_turn",
                    "turn.steer targets a stale OpenCode Provider turn".to_string(),
                    false,
                ));
            }
            let session = runtime.ready_session()?;
            let generation = session.generation().to_string();
            let client = session.client();
            let message_id = message_id(
                "steer",
                &generation,
                expected_active.epoch,
                &conversation_id,
                &request.client_message_id,
            );
            let native_conversation_id = conversation_id.clone();
            let sent_message_id = message_id.clone();
            let admission = tokio::task::spawn_blocking(move || {
                client.prompt(
                    &native_conversation_id,
                    &OpenCodePromptRequest {
                        id: sent_message_id,
                        prompt: OpenCodePrompt {
                            text: request.message,
                        },
                        delivery: OpenCodeDelivery::Steer,
                    },
                )
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            validate_admission(&admission, &conversation_id, &message_id, "steer")?;
            let turn = {
                let mut mutable = lock(&runtime.mutable);
                let active = mutable
                    .active_turns
                    .get_mut(&conversation_id)
                    .filter(|active| {
                        active.epoch == expected_active.epoch
                            && active.turn.resource.native_resource_id
                                == request.turn.native_resource_id
                    })
                    .ok_or_else(|| {
                        protocol_error(
                            "turn_not_active",
                            "OpenCode turn completed while the steer request was in flight"
                                .to_string(),
                            false,
                        )
                    })?;
                active.turn.updated_at = Some(
                    active
                        .turn
                        .updated_at
                        .unwrap_or(0)
                        .max(admission.time_created),
                );
                active.turn.clone()
            };
            Ok(TurnSteerResponse { turn })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProtocolFuture<'a, TurnInterruptResponse> {
        Box::pin(async move {
            validate_same_resource_route(&request.conversation, &request.turn)?;
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            let active = lock(&runtime.mutable)
                .active_turns
                .get(&conversation_id)
                .cloned()
                .ok_or_else(|| {
                    protocol_error(
                        "turn_not_active",
                        "OpenCode session has no active Provider turn to interrupt".to_string(),
                        false,
                    )
                })?;
            if active.turn.resource.native_resource_id != request.turn.native_resource_id {
                return Err(protocol_error(
                    "stale_turn",
                    "turn.interrupt targets a stale OpenCode Provider turn".to_string(),
                    false,
                ));
            }
            let session = runtime.ready_session()?;
            let client = session.client();
            let native_conversation_id = conversation_id.clone();
            let interrupted = tokio::task::spawn_blocking(move || {
                let was_active = client
                    .active_sessions()?
                    .contains_key(&native_conversation_id);
                client.interrupt(&native_conversation_id)?;
                Ok::<_, OpenCodeServerError>(was_active)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(OpenCodeProtocolMapper::error)?;
            if !interrupted {
                let turn = lock(&runtime.mutable)
                    .active_turns
                    .get(&conversation_id)
                    .filter(|current| {
                        current.epoch == active.epoch
                            && current.turn.resource.native_resource_id
                                == request.turn.native_resource_id
                    })
                    .map(|current| current.turn.clone())
                    .ok_or_else(|| {
                        protocol_error(
                            "turn_not_active",
                            "OpenCode turn completed while the interrupt request was in flight"
                                .to_string(),
                            false,
                        )
                    })?;
                return Ok(TurnInterruptResponse { turn });
            }
            let completed_at = now_ms();
            let (turn, expired_approvals) = {
                let mut mutable = lock(&runtime.mutable);
                let current = mutable.active_turns.get(&conversation_id).ok_or_else(|| {
                    protocol_error(
                        "turn_not_active",
                        "OpenCode turn completed while the interrupt request was in flight"
                            .to_string(),
                        false,
                    )
                })?;
                if current.epoch != active.epoch
                    || current.turn.resource.native_resource_id
                        != request.turn.native_resource_id
                {
                    return Err(protocol_error(
                        "stale_turn",
                        "turn.interrupt targets a stale OpenCode Provider turn".to_string(),
                        false,
                    ));
                }
                let mut turn = mutable
                    .active_turns
                    .remove(&conversation_id)
                    .expect("active turn was checked under the same lock")
                    .turn;
                turn.status = TurnStatus::Interrupted;
                turn.updated_at = Some(completed_at);
                turn.completed_at = Some(completed_at);
                let mut approvals = Vec::new();
                mutable.pending_approvals.retain(|_, pending| {
                    if pending.session_id == conversation_id {
                        approvals.push(pending.approval.clone());
                        false
                    } else {
                        true
                    }
                });
                (turn, approvals)
            };
            runtime
                .events
                .publish(runtime.mapper.turn_event(turn.clone()))?;
            for approval in expired_approvals {
                runtime.events.publish(
                    runtime
                        .mapper
                        .expire_approval(approval, Some(completed_at))
                        .1,
                )?;
            }
            Ok(TurnInterruptResponse { turn })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.approval)?;
            let native_approval_id = request.approval.native_resource_id.clone();
            let pending = lock(&runtime.mutable)
                .pending_approvals
                .remove(&native_approval_id)
                .ok_or_else(|| {
                    protocol_error(
                        "approval_not_found",
                        format!(
                            "approval {native_approval_id} is not pending in this Provider instance"
                        ),
                        false,
                    )
                })?;
            if pending.approval.resource.native_resource_id
                != request.approval.native_resource_id
            {
                return Err(protocol_error(
                    "stale_approval_session",
                    "approval route does not match the pending OpenCode permission".to_string(),
                    false,
                ));
            }
            let session = runtime.ready_session()?;
            if pending.session_generation != session.generation() {
                return Err(protocol_error(
                    "stale_approval_session",
                    "approval belongs to a previous OpenCode Server session".to_string(),
                    false,
                ));
            }
            let reply = match request.decision {
                ApprovalDecision::Approve => OpenCodePermissionReply::Once,
                ApprovalDecision::Deny => OpenCodePermissionReply::Reject,
            };
            let client = session.client();
            let session_id = pending.session_id.clone();
            let request_id = pending.request_id.clone();
            let reply_result = tokio::task::spawn_blocking(move || {
                client.reply_permission(&session_id, &request_id, reply)
            })
            .await
            .map_err(provider_task_error)?;
            if let Err(error) = reply_result {
                let mut mutable = lock(&runtime.mutable);
                if mutable.session_generation.as_deref() == Some(pending.session_generation.as_str()) {
                    mutable
                        .pending_approvals
                        .entry(native_approval_id)
                        .or_insert(pending);
                }
                return Err(OpenCodeProtocolMapper::error(error));
            }
            let (approval, event) = runtime.mapper.resolve_approval(
                pending.approval,
                request.decision,
                Some(now_ms()),
            );
            let turn = {
                let mut mutable = lock(&runtime.mutable);
                let still_waiting = has_pending_approval(
                    &mutable.pending_approvals,
                    &pending.session_id,
                );
                mutable.active_turns.get_mut(&pending.session_id).map(|active| {
                    if !still_waiting {
                        active.turn.status = TurnStatus::Running;
                    }
                    if approval.resolved_at.is_some() {
                        active.turn.updated_at = approval.resolved_at;
                    }
                    active.turn.clone()
                })
            };
            runtime.events.publish(event)?;
            if let Some(turn) = turn {
                runtime.events.publish(runtime.mapper.turn_event(turn))?;
            }
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
                return Ok(ProviderShutdownResponse { accepted: true });
            }
            let instances = lock(&self.state)
                .instances
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let mut cleanup_error = None;
            for runtime in instances {
                let session = {
                    let mut mutable = lock(&runtime.mutable);
                    OpenCodeInstanceRuntime::cancel_metadata(&mut mutable);
                    mutable.status = InstanceStatus::Stopping;
                    mutable.generation_counter = mutable.generation_counter.saturating_add(1);
                    mutable.session_generation = None;
                mutable.atomic_facts_ready = false;
                if let Some(task) = mutable.atomic_task.take() { task.abort(); }
                    mutable.active_turns.clear();
                    mutable.pending_approvals.clear();
                    mutable.session.take()
                };
                if let Some(session) = session {
                    let result = match tokio::task::spawn_blocking(move || session.shutdown()).await {
                        Ok(result) => result.map_err(OpenCodeProtocolMapper::error),
                        Err(error) => Err(provider_task_error(error)),
                    };
                    if cleanup_error.is_none() {
                        cleanup_error = result.err();
                    }
                }
                lock(&runtime.mutable).status = InstanceStatus::Stopped;
            }
            if let Some(error) = cleanup_error {
                Err(error)
            } else {
                Ok(ProviderShutdownResponse { accepted: true })
            }
        })
    }
}

fn decode_settings(
    settings: codepet_provider_sdk::JsonObject,
) -> Result<OpenCodeInstanceSettings, ProtocolError> {
    let value = Value::Object(settings.into_iter().collect());
    let settings: OpenCodeInstanceSettings = serde_json::from_value(value).map_err(|error| {
        protocol_error(
            "invalid_instance_settings",
            format!("invalid OpenCode instance settings: {error}"),
            false,
        )
    })?;
    if !settings.server_executable.is_absolute() {
        return Err(protocol_error(
            "invalid_instance_settings",
            "serverExecutable must be an absolute path resolved by the Host".to_string(),
            false,
        ));
    }
    if settings.server_args != ["serve"] {
        return Err(protocol_error(
            "invalid_instance_settings",
            "serverArgs must be exactly [\"serve\"]".to_string(),
            false,
        ));
    }
    if settings
        .workspace_root
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(protocol_error(
            "invalid_instance_settings",
            "workspaceRoot must be an absolute path when configured".to_string(),
            false,
        ));
    }
    if settings.data_directory.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(protocol_error("invalid_instance_settings", "dataDirectory must be an absolute storage root".into(), false));
    }
    Ok(settings)
}

fn default_server_working_directory() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .find_map(std::env::var_os)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
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

fn ensure_opencode_workspace(workspace: PathBuf) -> Result<PathBuf, ProtocolError> {
    if !workspace.is_absolute() {
        return Err(protocol_error(
            "invalid_workspace_root",
            "OpenCode workspaceRoot must be an absolute path".to_string(),
            false,
        ));
    }
    std::fs::create_dir_all(&workspace).map_err(|error| {
        protocol_error(
            "workspace_create_failed",
            format!("failed to create OpenCode workspaceRoot: {error}"),
            false,
        )
    })?;
    Ok(workspace)
}

fn authentication_from_output(output: Option<&str>) -> ProviderAuthentication {
    let credential_count = output.and_then(parse_credential_count);
    let authentication = ProviderAuthentication {
        status: match credential_count {
            Some(0) => ProviderAuthenticationStatus::SignedOut,
            Some(_) => ProviderAuthenticationStatus::SignedIn,
            None => ProviderAuthenticationStatus::Unknown,
        },
        display_text: Some(match credential_count {
            Some(0) => "No stored credentials".to_string(),
            Some(1) => "1 credential provider".to_string(),
            Some(count) => format!("{count} credential providers"),
            None => "Authentication status unavailable".to_string(),
        }),
    };
    authentication
}

fn usage_from_output(stats: Option<&str>) -> ProviderUsage {
    let total_cost = stats.as_deref().and_then(|text| statistic_value(text, "Total Cost"));
    let input = stats.as_deref().and_then(|text| statistic_value(text, "Input"));
    let output = stats.as_deref().and_then(|text| statistic_value(text, "Output"));
    let mut data = codepet_provider_sdk::JsonObject::new();
    if let Some(value) = total_cost.as_ref() { data.insert("totalCost".to_string(), json!(value)); }
    if let Some(value) = input.as_ref() { data.insert("inputTokens".to_string(), json!(value)); }
    if let Some(value) = output.as_ref() { data.insert("outputTokens".to_string(), json!(value)); }
    let usage = ProviderUsage {
        display_text: total_cost.map(|cost| format!("Last 30 days · {cost}"))
            .unwrap_or_else(|| "Local usage statistics unavailable".to_string()),
        observed_at: Some(now_ms()),
        details: (!data.is_empty()).then_some(vec![ProviderUsageDetail {
            namespace: "opencode.local-stats".to_string(),
            schema_version: "1".to_string(),
            data,
        }]),
    };
    usage
}

fn parse_credential_count(text: &str) -> Option<u64> {
    text.lines().find(|line| line.contains("credential"))
        .and_then(|line| line.split(|character: char| !character.is_ascii_digit()).find(|part| !part.is_empty()))
        .and_then(|value| value.parse().ok())
}

fn statistic_value(text: &str, label: &str) -> Option<String> {
    text.lines().find(|line| line.contains(label)).and_then(|line| {
        let index = line.find(label)? + label.len();
        let value = line[index..].trim().trim_end_matches('│').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn discover_path_candidates(command: &str) -> Vec<RuntimeCandidate> {
    let mut candidates = local_runtime::discover(command, "opencode-ai");
    if let Some(path) = std::env::var_os("CODE_PET_OPENCODE_BIN").filter(|value| !value.is_empty()) {
        candidates.insert(0, local_runtime::candidate(PathBuf::from(path), codepet_provider_sdk::RuntimeCandidateSource::Environment));
    }
    if let Some(home) = local_runtime::home_dir() {
        for directory in [home.join(".local").join("bin"), home.join(".opencode").join("bin")] {
            candidates.extend(local_runtime::candidates_in(&directory, command, "opencode-ai").into_iter().map(|path| local_runtime::candidate(path, codepet_provider_sdk::RuntimeCandidateSource::CurrentPath)));
        }
    }
    candidates
}

fn inspect_runtime_candidate(candidate: RuntimeCandidate, timeout: Duration, control: local_runtime::RuntimeProbeControl) -> Result<RuntimeInstallation, ProtocolError> {
    let canonical = local_runtime::resolve_executable(Path::new(&candidate.executable_path), "opencode", "opencode-ai")
        .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?;
    let version = bounded_opencode_version(&canonical, timeout, control)?;
    Ok(RuntimeInstallation { executable_path: canonical.to_string_lossy().into_owned(), version, source: candidate.source })
}

fn bounded_opencode_version(executable: &std::path::Path, timeout: Duration, control: local_runtime::RuntimeProbeControl) -> Result<String, ProtocolError> {
    let mut child = codepet_provider_sdk::local_runtime::command(executable).arg("--version")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| protocol_error("invalid_runtime_selection", format!("Run runtime executable {}: {error}", executable.display()), false))?;
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
                return Ok(stdout.lines().chain(stderr.lines())
                    .find_map(|line| line.split_whitespace().find(|part| part.trim_start_matches('v').chars().next().is_some_and(|character| character.is_ascii_digit())))
                    .unwrap_or_default().trim_start_matches('v').to_string());
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

async fn discover_cli_models(
    executable: &std::path::Path,
    working_directory: Option<&std::path::Path>,
    data_directory: Option<&std::path::Path>,
) -> Result<Vec<OpenCodeModel>, OpenCodeServerError> {
    let output = crate::background_probe::run(
        executable,
        &["models"],
        working_directory,
        data_directory,
        Duration::from_secs(30),
    )
    .await
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            OpenCodeServerError::Timeout(error.to_string())
        } else {
            OpenCodeServerError::Protocol(format!("probe models: {error}"))
        }
    })?;
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((provider_id, model_id)) = line.split_once('/') else {
            continue;
        };
        if provider_id.is_empty()
            || model_id.is_empty()
            || !seen.insert((provider_id.to_string(), model_id.to_string()))
        {
            continue;
        }
        models.push(OpenCodeModel {
            id: model_id.to_string(),
            provider_id: provider_id.to_string(),
            name: model_id.to_string(),
            status: "active".to_string(),
            enabled: true,
            variants: Vec::new(),
        });
    }
    if models.is_empty() {
        return Err(OpenCodeServerError::Protocol(
            "`opencode models` returned no parseable provider/model entries".to_string(),
        ));
    }
    Ok(models)
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
    if route.provider_plugin_id != OPENCODE_PLUGIN_ID {
        return Err(protocol_error(
            "wrong_provider_plugin_route",
            format!(
                "OpenCode Provider cannot serve plugin {}",
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

fn validate_admission(
    admission: &crate::protocol::OpenCodePromptAdmission,
    session_id: &str,
    message_id: &str,
    delivery: &str,
) -> Result<(), ProtocolError> {
    if admission.session_id != session_id
        || admission.id != message_id
        || admission.delivery != delivery
    {
        return Err(protocol_error(
            "opencode_protocol_error",
            "OpenCode prompt admission does not match the submitted prompt".to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_opencode_session(session: &OpenCodeSession) -> Result<(), ProtocolError> {
    if session.id.trim().is_empty() {
        return Err(protocol_error(
            "opencode_protocol_error",
            "OpenCode session response contains an empty id".to_string(),
            false,
        ));
    }
    let workspace_root = session.workspace_root();
    if workspace_root.trim().is_empty() || !PathBuf::from(&workspace_root).is_absolute() {
        return Err(protocol_error(
            "opencode_protocol_error",
            "OpenCode session response contains a non-absolute workspace directory".to_string(),
            false,
        ));
    }
    Ok(())
}

fn conversation_get_limit(limit: Option<u64>) -> Result<u64, ProtocolError> {
    let limit = limit.unwrap_or(DEFAULT_CONVERSATION_GET_MESSAGE_LIMIT);
    if !(1..=MAX_CONVERSATION_GET_MESSAGE_LIMIT).contains(&limit) {
        return Err(protocol_error(
            "invalid_request",
            format!(
                "conversation.get limit must be between 1 and {MAX_CONVERSATION_GET_MESSAGE_LIMIT}"
            ),
            false,
        ));
    }
    Ok(limit)
}

fn validate_conversation_message_page(
    page: &crate::protocol::OpenCodeMessagePage,
    cursor: Option<&str>,
    limit: u64,
) -> Result<(), OpenCodeServerError> {
    if page.data.len() as u64 > limit {
        return Err(OpenCodeServerError::Protocol(format!(
            "OpenCode message history returned {} messages for limit {limit}",
            page.data.len()
        )));
    }
    if page.cursor.next.as_deref().is_some_and(|next| Some(next) == cursor) {
        return Err(OpenCodeServerError::Protocol(
            "OpenCode message history repeated a pagination cursor".to_string(),
        ));
    }
    for message in &page.data {
        if message.id.trim().is_empty() || message.kind.trim().is_empty() {
            return Err(OpenCodeServerError::Protocol(
                "OpenCode message history contains an empty id or type".to_string(),
            ));
        }
        if message.content.as_ref().is_some_and(|contents| {
            contents
                .iter()
                .any(|content| content.id.trim().is_empty() || content.kind.trim().is_empty())
        }) {
            return Err(OpenCodeServerError::Protocol(
                "OpenCode message history contains an empty content id or type".to_string(),
            ));
        }
    }
    Ok(())
}

fn decode_event<T: DeserializeOwned>(event: &OpenCodeEvent) -> Result<T, ProtocolError> {
    serde_json::from_value(event.data.clone()).map_err(|error| {
        event_shape_error(event, &format!("invalid event data: {error}"))
    })
}

fn event_shape_error(event: &OpenCodeEvent, message: &str) -> ProtocolError {
    protocol_error(
        "opencode_event_shape_invalid",
        format!("OpenCode event {} ({}) {message}", event.id, event.kind),
        false,
    )
}

fn has_pending_approval(
    approvals: &HashMap<String, PendingApproval>,
    session_id: &str,
) -> bool {
    approvals
        .values()
        .any(|approval| approval.session_id == session_id)
}

fn approval_resource_id(
    generation: &str,
    conversation_id: &str,
    request_id: &str,
) -> String {
    format!(
        "opencode-permission:{generation}:{:016x}:{:016x}",
        stable_hash(&[conversation_id]),
        stable_hash(&[request_id]),
    )
}

fn turn_resource_id(
    generation: &str,
    epoch: u64,
    conversation_id: &str,
    client_message_id: &str,
) -> String {
    format!(
        "opencode-turn:{generation}:{epoch}:{:016x}:{:016x}",
        stable_hash(&[conversation_id]),
        stable_hash(&[client_message_id]),
    )
}

fn message_id(
    kind: &str,
    generation: &str,
    epoch: u64,
    conversation_id: &str,
    client_message_id: &str,
) -> String {
    let epoch = epoch.to_string();
    format!(
        "msg_codepet_{:016x}",
        stable_hash(&[
            kind,
            generation,
            &epoch,
            conversation_id,
            client_message_id,
        ])
    )
}

fn stable_hash(parts: &[&str]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn capability_unsupported(message: &str) -> ProtocolError {
    protocol_error("capability_unsupported", message.to_string(), false)
}

fn provider_task_error(error: tokio::task::JoinError) -> ProtocolError {
    protocol_error(
        "provider_task_failed",
        format!("OpenCode Provider task failed: {error}"),
        true,
    )
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

fn resolve_opencode_selection(
    capabilities: &ProviderCapabilities,
    requested: TurnSelection,
    current: &OpenCodeSession,
) -> Result<TurnSelection, ProtocolError> {
    let controls = capabilities.turn_send.as_ref().ok_or_else(|| protocol_error(
        "provider_capability_invalid",
        "OpenCode turn controls are unavailable".to_string(),
        false,
    ))?;
    let access_mode_id = resolve_control_choice(
        controls.access_mode.as_ref(),
        requested.access_mode_id,
        current.agent.clone(),
        "accessModeId",
    )?;
    let reasoning_effort_id = resolve_control_choice(
        controls.reasoning_effort.as_ref(),
        requested.reasoning_effort_id,
        current.model.as_ref().and_then(|model| model.variant.clone()),
        "reasoningEffortId",
    )?;
    let catalog = match controls.model_catalog.as_ref() {
        Some(ModelCatalog::GroupedModelCatalog(catalog)) => catalog,
        Some(ModelCatalog::FlatModelCatalog(_)) => {
            return Err(protocol_error(
                "provider_capability_invalid",
                "OpenCode requires a grouped model catalog".to_string(),
                false,
            ));
        }
        None => {
            return Err(protocol_error(
                "provider_capability_invalid",
                "OpenCode model catalog is unavailable".to_string(),
                false,
            ));
        }
    };
    let requested_model = match requested.model {
        Some(ModelSelection::GroupedModelSelection(selection)) => Some(selection),
        Some(ModelSelection::FlatModelSelection(_)) => {
            return Err(protocol_error(
                "invalid_turn_selection",
                "OpenCode requires a grouped model selection".to_string(),
                false,
            ));
        }
        None => None,
    };
    let current_model = current.model.as_ref().map(|model| GroupedModelSelection {
        kind: GroupedModelCatalogKind::Grouped,
        provider_id: model.provider_id.clone(),
        model_id: model.id.clone(),
    });
    let model = requested_model
        .or_else(|| current_model.filter(|selection| grouped_model_enabled(catalog, selection)))
        .or_else(|| catalog.default_selection.clone())
        .ok_or_else(|| protocol_error(
            "provider_capability_invalid",
            "OpenCode model catalog has no default selection".to_string(),
            false,
        ))?;
    if !grouped_model_enabled(catalog, &model) {
        return Err(protocol_error(
            "invalid_turn_selection",
            format!("unknown or disabled OpenCode model: {}/{}", model.provider_id, model.model_id),
            false,
        ));
    }
    Ok(TurnSelection {
        access_mode_id,
        reasoning_effort_id,
        model: Some(ModelSelection::GroupedModelSelection(model)),
    })
}

fn resolve_create_selection(
    capabilities: &ProviderCapabilities,
    permission_level: &str,
    model: Option<String>,
    reasoning_effort: Option<String>,
) -> Result<(Option<String>, Option<OpenCodeModelRef>), ProtocolError> {
    let controls = capabilities.conversation_create.as_ref().and_then(|create| create.selection.as_ref())
        .ok_or_else(|| capability_unsupported("OpenCode conversation creation controls are unavailable"))?;
    let agent = if permission_level == OPENCODE_PERMISSION_LEVEL {
        None // Preserve the native default for older callers.
    } else {
        resolve_control_choice(controls.access_mode.as_ref(), Some(permission_level.to_string()), None, "accessModeId")?
    };
    if model.is_none() && reasoning_effort.is_none() { return Ok((agent, None)); }
    let variant = resolve_control_choice(controls.reasoning_effort.as_ref(), reasoning_effort, None, "reasoningEffortId")?;
    let Some(ModelCatalog::FlatModelCatalog(catalog)) = controls.model_catalog.as_ref() else {
        return Err(capability_unsupported("OpenCode conversation creation model catalog is unavailable"));
    };
    let model = model.or_else(|| catalog.default_selection.as_ref().map(|selection| selection.model_id.clone()))
        .ok_or_else(|| capability_unsupported("OpenCode model catalog has no default selection"))?;
    if !catalog.models.iter().any(|option| option.id == model && option.enabled != Some(false)) {
        return Err(protocol_error("invalid_turn_selection", format!("unknown or disabled OpenCode model: {model}"), false));
    }
    let (provider_id, id) = model.split_once('/').ok_or_else(|| protocol_error(
        "provider_capability_invalid", "OpenCode create model must include its provider".into(), false,
    ))?;
    Ok((agent, Some(OpenCodeModelRef { id:id.into(), provider_id:provider_id.into(), variant })))
}

fn resolve_control_choice(
    choices: Option<&codepet_provider_sdk::ChoiceSet>,
    requested: Option<String>,
    current: Option<String>,
    field: &str,
) -> Result<Option<String>, ProtocolError> {
    let choices = choices.ok_or_else(|| protocol_error(
        "provider_capability_invalid",
        format!("OpenCode {field} choices are unavailable"),
        false,
    ))?;
    let selected = requested
        .or_else(|| current.filter(|value| choices.options.iter().any(|option| option.id == *value)))
        .or_else(|| choices.default_id.clone());
    if let Some(selected) = selected.as_deref() {
        match choices.options.iter().find(|option| option.id == selected) {
            Some(option) if option.enabled != Some(false) => {}
            Some(option) => {
                return Err(protocol_error(
                    "invalid_turn_selection",
                    option.disabled_reason.clone().unwrap_or_else(|| format!("{field} is disabled: {selected}")),
                    false,
                ));
            }
            None => {
                return Err(protocol_error(
                    "invalid_turn_selection",
                    format!("unknown {field}: {selected}"),
                    false,
                ));
            }
        }
    }
    Ok(selected)
}

fn grouped_model_enabled(
    catalog: &codepet_provider_sdk::GroupedModelCatalog,
    selection: &GroupedModelSelection,
) -> bool {
    catalog.providers.iter().any(|provider| {
        provider.id == selection.provider_id
            && provider.models.iter().any(|model| {
                model.id == selection.model_id && model.enabled != Some(false)
            })
    })
}

fn selected_opencode_model(selection: &TurnSelection) -> Result<OpenCodeModelRef, ProtocolError> {
    let model = match selection.model.as_ref() {
        Some(ModelSelection::GroupedModelSelection(model)) => model,
        _ => {
            return Err(protocol_error(
                "invalid_turn_selection",
                "OpenCode model selection is missing".to_string(),
                false,
            ));
        }
    };
    Ok(OpenCodeModelRef {
        id: model.model_id.clone(),
        provider_id: model.provider_id.clone(),
        variant: selection.reasoning_effort_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        approval_resource_id, decode_settings, ensure_opencode_workspace, message_id,
        turn_resource_id,
    };
    use serde_json::json;

    #[test]
    fn creates_a_missing_standalone_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("task-1");

        let prepared = ensure_opencode_workspace(workspace.clone()).unwrap();

        assert_eq!(prepared, workspace);
        assert!(workspace.is_dir());
    }

    #[test]
    fn settings_require_host_resolved_absolute_executable() {
        let missing = serde_json::from_value(json!({"serverArgs": ["serve"]})).unwrap();
        assert!(decode_settings(missing).is_err());

        let relative = serde_json::from_value(json!({
            "serverExecutable": "opencode",
            "serverVersion": "1.18.25",
            "serverArgs": ["serve"]
        }))
        .unwrap();
        assert!(decode_settings(relative).is_err());

        for version in ["1.18.24", "1.18.29", "v1.18.25", "development"] {
            let value = serde_json::from_value(json!({
                "serverExecutable": std::env::current_exe().unwrap(),
                "serverVersion": version,
                "serverArgs": ["serve"]
            })).unwrap();
            assert!(decode_settings(value).is_ok(), "version must not gate runtime: {version}");
        }
    }

    #[test]
    fn provider_resource_ids_are_stable_and_session_scoped() {
        assert_eq!(
            message_id("start", "1", 1, "ses_1", "request-1"),
            message_id("start", "1", 1, "ses_1", "request-1")
        );
        assert_ne!(
            turn_resource_id("1", 1, "ses_1", "same-client"),
            turn_resource_id("1", 1, "ses_2", "same-client")
        );
        assert_ne!(
            turn_resource_id("1", 1, "ses_1", "same-client"),
            turn_resource_id("2", 1, "ses_1", "same-client")
        );
        assert_ne!(
            turn_resource_id("1", 1, "ses_1", "same-client"),
            turn_resource_id("1", 2, "ses_1", "same-client")
        );
        assert_ne!(
            approval_resource_id("1", "ses_1", "per_1"),
            approval_resource_id("2", "ses_1", "per_1")
        );
        assert_ne!(
            approval_resource_id("1", "ses_1", "per_1"),
            approval_resource_id("1", "ses_2", "per_1")
        );
    }
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
            "serverExecutable": executable, "serverArgs": ["serve"],
            "dataDirectory": data,
        })).unwrap()).unwrap();
        assert_eq!(settings.data_directory.as_deref(), Some(data.as_path()));
        let invalid = decode_settings(serde_json::from_value(json!({
            "serverExecutable": executable, "serverArgs": ["serve"],
            "dataDirectory": "relative-storage",
        })).unwrap());
        assert!(invalid.is_err());
    }
}

impl OpenCodeInstanceRuntime {
    fn query_generation(&self) -> Result<String, ProtocolError> {
        self.ready_session()?;
        Ok(lock(&self.mutable).generation_counter.to_string())
    }
}

impl OpenCodeProvider {
    async fn complete_conversation_summaries(&self, route: &ProviderInstanceRoute) -> Result<Vec<Conversation>, ProtocolError> {
        self.instance(route)?.collect_atomic_summaries().await
    }
}

impl OpenCodeProvider {
    async fn complete_requested_summaries(&self, route: &ProviderInstanceRoute, ids: &[String], rows: &mut Vec<Conversation>) -> Result<(), ProtocolError> {
        let runtime = self.instance(route)?;
        for id in ids {
            if rows.iter().any(|row| &row.resource.native_resource_id == id) { continue; }
            let client = runtime.ready_session()?.client();
            let requested = id.clone();
            let session = match tokio::task::spawn_blocking(move || client.get_session(&requested)).await.map_err(provider_task_error)? {
                Ok(session) => session,
                Err(OpenCodeServerError::Http { status: 404, .. }) => continue,
                Err(error) => return Err(OpenCodeProtocolMapper::error(error)),
            };
            validate_opencode_session(&session)?;
            let mutable = lock(&runtime.mutable);
            rows.push(runtime.mapper.conversation(&session, false, mutable.active_turns.get(id).map(|run| run.turn.clone()), has_pending_approval(&mutable.pending_approvals, id)));
        }
        Ok(())
    }
}

impl OpenCodeInstanceRuntime {
    async fn collect_atomic_summaries(&self) -> Result<Vec<Conversation>, ProtocolError> {
        let client = self.ready_session()?.client();
        let mut sessions = HashMap::new();
        let mut cursor = None;
        let mut progress = codepet_provider_sdk::conversation_query::EnumerationProgress::default();
        loop {
            let page_client = client.clone();
            let page = tokio::task::spawn_blocking(move || page_client.list_sessions(cursor.as_deref(), Some(100)))
                .await.map_err(provider_task_error)?.map_err(OpenCodeProtocolMapper::error)?;
            for session in page.data { validate_opencode_session(&session)?; sessions.insert(session.id.clone(), session); }
            cursor = progress.advance(page.cursor.next)?;
            if cursor.is_none() { break; }
            tokio::task::yield_now().await;
        }
        let active_client = client.clone();
        let active = tokio::task::spawn_blocking(move || active_client.active_sessions()).await.map_err(provider_task_error)?.map_err(OpenCodeProtocolMapper::error)?;
        for id in active.keys() {
            if id.trim().is_empty() { return Err(protocol_error("opencode_protocol_error", "active session has an empty ID".into(), false)); }
            if sessions.contains_key(id) { continue; }
            let client = client.clone(); let id = id.clone();
            let session = tokio::task::spawn_blocking(move || client.get_session(&id)).await.map_err(provider_task_error)?.map_err(OpenCodeProtocolMapper::error)?;
            validate_opencode_session(&session)?;
            sessions.insert(session.id.clone(), session);
        }
        let mut mutable = lock(&self.mutable);
        for (id, session) in &sessions { mutable.sessions.insert(id.clone(), session.clone()); }
        for id in mutable.active_turns.keys() {
            if let Some(session) = mutable.sessions.get(id) { sessions.entry(id.clone()).or_insert_with(|| session.clone()); }
        }
        let mut rows = sessions.values().map(|session| self.mapper.conversation(session, active.contains_key(&session.id),
            mutable.active_turns.get(&session.id).map(|run| run.turn.clone()), has_pending_approval(&mutable.pending_approvals, &session.id))).collect::<Vec<_>>();
        conversation_atoms::sort_summaries(&mut rows);
        Ok(rows)
    }

    fn start_atomic_poll(self: &Arc<Self>, generation: String) {
        let owner = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            let mut previous = HashMap::<String, Conversation>::new();
            loop {
                let Some(runtime) = owner.upgrade() else { return; };
                let (current_generation, status) = {
                    let state = lock(&runtime.mutable);
                    (state.session_generation.clone(), state.status)
                };
                if current_generation.as_deref() != Some(generation.as_str()) { return; }
                if status != InstanceStatus::Ready {
                    drop(runtime);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                let event_epoch = runtime.atoms.event_epoch();
                let result = runtime.poll_atomic_changes(&previous).await;
                if lock(&runtime.mutable).session_generation.as_deref() != Some(generation.as_str()) { return; }
                match result {
                    Ok(current) => match runtime.apply_atomic_snapshot(&generation, event_epoch, &previous, &current) {
                        Ok(true) => previous = current,
                        Ok(false) => {},
                        Err(_) => { runtime.set_atomic_readiness_for(&generation, false); return; }
                    },
                    Err(error) => { eprintln!("OpenCode atomic discovery unavailable: {}", error.message); runtime.set_atomic_readiness_for(&generation, false); }
                }
                drop(runtime);
                // One scan in flight, no overlapping catch-up and no UI policy.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        let mut state = lock(&self.mutable);
        state.atomic_facts_ready = false;
        if let Some(previous) = state.atomic_task.replace(task) { previous.abort(); }
    }

    fn set_atomic_readiness_for(&self, generation: &str, ready: bool) -> bool {
        let mut state = lock(&self.mutable);
        if state.session_generation.as_deref() != Some(generation) || state.status != InstanceStatus::Ready { return false; }
        if state.atomic_facts_ready == ready { return true; }
        state.atomic_facts_ready = ready;
        state.atomic_facts_epoch = state.atomic_facts_epoch.saturating_add(1);
        let _ = self.events.publish(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent {
            instance: self.snapshot_locked(&state), previous_status: Some(state.status),
        } });
        true
    }

    async fn poll_atomic_changes(&self, previous: &HashMap<String, Conversation>) -> Result<HashMap<String, Conversation>, ProtocolError> {
        let mut current = self.collect_atomic_summaries().await?.into_iter().map(|row| (row.resource.native_resource_id.clone(), row)).collect::<HashMap<_, _>>();
        // List absence is not sufficient deletion evidence. Confirm every old
        // missing identity through the summary endpoint; only HTTP 404 deletes.
        for id in previous.keys().filter(|id| !current.contains_key(*id)).cloned().collect::<Vec<_>>() {
            let client = self.ready_session()?.client(); let requested = id.clone();
            match tokio::task::spawn_blocking(move || client.get_session(&requested)).await.map_err(provider_task_error)? {
                Ok(session) => {
                    validate_opencode_session(&session)?;
                    let state = lock(&self.mutable);
                    current.insert(id.clone(), self.mapper.conversation(&session, false, state.active_turns.get(&id).map(|run| run.turn.clone()), has_pending_approval(&state.pending_approvals, &id)));
                }
                Err(OpenCodeServerError::Http { status: 404, .. }) => {}
                Err(error) => return Err(OpenCodeProtocolMapper::error(error)),
            }
        }
        Ok(current)
    }
}

impl OpenCodeInstanceRuntime {
    fn apply_atomic_snapshot(&self, generation: &str, epoch: u64, previous: &HashMap<String, Conversation>, current: &HashMap<String, Conversation>) -> Result<bool, ProtocolError> {
        let mut state = lock(&self.mutable);
        if state.session_generation.as_deref() != Some(generation) || state.status != InstanceStatus::Ready { return Ok(false); }
        let old_ready = state.atomic_facts_ready;
        let old_epoch = state.atomic_facts_epoch;
        let mut events = Vec::new();
        if !old_ready {
            state.atomic_facts_ready = true;
            state.atomic_facts_epoch = old_epoch.saturating_add(1);
            events.push(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent { instance: self.snapshot_locked(&state), previous_status: Some(state.status) } });
        }
        let delta = conversation_atoms::SummaryDelta {
            event_epoch: epoch,
            upserted: current.iter().filter(|(id, row)| previous.get(*id) != Some(*row)).map(|(_, row)| row.clone()).collect(),
            deleted: previous.keys().filter(|id| !current.contains_key(*id)).cloned().collect(),
        };
        events.extend(conversation_atoms::summary_delta_events(&self.route, delta));
        match self.atoms.commit_events(epoch, events) {
            Ok(true) => Ok(true),
            Ok(false) => { state.atomic_facts_ready = old_ready; state.atomic_facts_epoch = old_epoch; Ok(false) },
            Err(error) => { state.atomic_facts_ready = false; Err(error) },
        }
    }
}
