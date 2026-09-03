use crate::client::{OpenCodeClient, OpenCodeServerSession};
use crate::mapper::{protocol_error, OpenCodeProtocolMapper};
use crate::protocol::{
    OpenCodeDelivery, OpenCodeDeltaEventData, OpenCodeEvent,
    OpenCodeLocationRef, OpenCodePermissionAskedEventData, OpenCodePermissionRepliedEventData,
    OpenCodePermissionReply, OpenCodePrompt, OpenCodePromptAdmittedEventData,
    OpenCodeModel, OpenCodeModelRef, OpenCodePromptRequest, OpenCodeServerError, OpenCodeSession, OpenCodeSessionCreate,
    OpenCodeStepEndedEventData, OpenCodeStepFailedEventData, OpenCodeStepStartedEventData,
    OPENCODE_INSTANCE_KIND, OPENCODE_PERMISSION_LEVEL, OPENCODE_PLUGIN_ID,
    OPENCODE_VERIFIED_SERVER_VERSION,
};
use codepet_provider_sdk::{
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
    ProtocolFuture, Provider, ProviderApproval, ProviderCapabilities,
    ProviderDescribeRequest, ProviderDescribeResponse, ProviderInitializeRequest,
    ProviderInitializeResponse, ProviderInstance, ProviderInstanceRoute,
    ProviderPluginDescriptor, ProviderShutdownRequest, ProviderShutdownResponse,
    ProviderTurn, RoutedResourceId, TurnInterruptRequest, TurnInterruptResponse, TurnSelection,
    TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest, TurnSteerResponse,
    VersionRange, PROTOCOL_VERSION,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct OpenCodeInstanceSettings {
    server_executable: PathBuf,
    server_version: String,
    server_args: Vec<String>,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
}

use codepet_provider_sdk::ProviderEventSink;

#[derive(Clone)]
struct PendingApproval {
    session_generation: String,
    session_id: String,
    request_id: String,
    approval: ProviderApproval,
}

#[derive(Clone)]
struct ActiveTurnState {
    epoch: u64,
    turn: ProviderTurn,
    prompt_message_id: String,
    assistant_message_id: Option<String>,
    wait_started: bool,
    wait_confirmed: bool,
    latest_step_ended_at: Option<u64>,
}

struct InstanceMutable {
    status: InstanceStatus,
    session: Option<OpenCodeServerSession>,
    session_generation: Option<String>,
    generation_counter: u64,
    next_turn_epoch: u64,
    sessions: HashMap<String, OpenCodeSession>,
    active_turns: HashMap<String, ActiveTurnState>,
    pending_approvals: HashMap<String, PendingApproval>,
}

struct OpenCodeInstanceRuntime {
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
        Self {
            route: request.route.clone(),
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            capabilities: Mutex::new(OpenCodeProtocolMapper::base_capabilities()),
            boot_id,
            mutable: Mutex::new(InstanceMutable {
                status: InstanceStatus::Created,
                session: None,
                session_generation: None,
                generation_counter: 0,
                next_turn_epoch: 0,
                sessions: HashMap::new(),
                active_turns: HashMap::new(),
                pending_approvals: HashMap::new(),
            }),
            mapper: OpenCodeProtocolMapper::new(request.route),
            events,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        self.mapper.instance(
            OPENCODE_PLUGIN_ID.to_string(),
            self.instance_kind.clone(),
            self.display_name.clone(),
            HarnessDescriptor {
                id: self.instance_kind.clone(),
                display_name: "OpenCode".to_string(),
                version: Some(self.settings.server_version.clone()),
            },
            lock(&self.mutable).status,
            lock(&self.capabilities).clone(),
        )
    }

    fn status(&self) -> InstanceStatus {
        lock(&self.mutable).status
    }

    fn set_status(&self, status: InstanceStatus) -> Result<ProviderInstance, ProtocolError> {
        let previous_status = {
            let mut mutable = lock(&self.mutable);
            if mutable.status == status {
                None
            } else {
                let previous = mutable.status;
                mutable.status = status;
                Some(previous)
            }
        };
        let instance = self.snapshot();
        if let Some(previous_status) = previous_status {
            self.events.publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".to_string(),
                params: InstanceStatusChangedEvent {
                    instance: instance.clone(),
                    previous_status: Some(previous_status),
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
                let item_id = item_id.ok_or_else(|| {
                    event_shape_error(&event, "delta event is missing its item identifier")
                })?;
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
            let result = client.wait_session(&session_id);
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
}

pub struct OpenCodeProvider {
    state: Mutex<ProviderState>,
    events: Arc<dyn ProviderEventSink>,
    boot_id: String,
    shutdown: AtomicBool,
}

impl OpenCodeProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self {
            state: Mutex::new(ProviderState {
                host_device_id: None,
                initialized_client_id: None,
                instances: HashMap::new(),
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
        resource: &RoutedResourceId,
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
            let runtime = Arc::new(OpenCodeInstanceRuntime::new(
                request,
                settings,
                self.boot_id.clone(),
                self.events.clone(),
            ));
            let instance = runtime.snapshot();
            state
                .instances
                .insert(runtime.route.provider_instance_id.clone(), runtime);
            Ok(InstanceCreateResponse { instance })
        })
    }

    fn instance_start<'a>(
        &'a self,
        request: InstanceStartRequest,
    ) -> ProtocolFuture<'a, InstanceStartResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if runtime.status() == InstanceStatus::Ready {
                return Ok(InstanceStartResponse {
                    instance: runtime.snapshot(),
                });
            }
            if runtime.status() == InstanceStatus::Starting {
                return Err(protocol_error(
                    "provider_instance_starting",
                    "OpenCode Provider instance is already starting".to_string(),
                    true,
                ));
            }
            runtime.set_status(InstanceStatus::Starting)?;
            let executable = runtime.settings.server_executable.clone();
            let version = runtime.settings.server_version.clone();
            let args = runtime.settings.server_args.clone();
            let working_directory = runtime
                .settings
                .workspace_root
                .clone()
                .or_else(default_server_working_directory);
            let capability_executable = executable.clone();
            let capability_working_directory = working_directory.clone();
            let generation = {
                let mut mutable = lock(&runtime.mutable);
                mutable.generation_counter = mutable.generation_counter.saturating_add(1);
                format!("{}:{}", runtime.boot_id, mutable.generation_counter)
            };
            let session_generation = generation.clone();
            let session = tokio::task::spawn_blocking(move || {
                OpenCodeServerSession::spawn(
                    &executable,
                    &args,
                    &version,
                    session_generation,
                    working_directory.as_deref(),
                )
            })
            .await
            .map_err(provider_task_error)?;
            let session = match session {
                Ok(session) => session,
                Err(error) => {
                    let _ = runtime.set_status(InstanceStatus::Error);
                    return Err(OpenCodeProtocolMapper::error(error));
                }
            };
            let discovery_client = session.client();
            let discovered = tokio::task::spawn_blocking(move || {
                let mut models = discovery_client.list_models()?;
                if models.is_empty() {
                    models = discover_cli_models(
                        &capability_executable,
                        capability_working_directory.as_deref(),
                    )?;
                }
                Ok::<_, OpenCodeServerError>((
                    discovery_client.list_agents()?,
                    models,
                    discovery_client.list_providers()?,
                ))
            })
            .await
            .map_err(provider_task_error)?;
            let (agents, models, providers) = match discovered {
                Ok(discovered) => discovered,
                Err(error) => {
                    let _ = session.shutdown();
                    let _ = runtime.set_status(InstanceStatus::Error);
                    return Err(OpenCodeProtocolMapper::error(error));
                }
            };
            let capabilities = match OpenCodeProtocolMapper::capabilities(
                &agents,
                &models,
                &providers,
            ) {
                Ok(capabilities) => capabilities,
                Err(error) => {
                    let _ = session.shutdown();
                    let _ = runtime.set_status(InstanceStatus::Error);
                    return Err(error);
                }
            };
            *lock(&runtime.capabilities) = capabilities;
            let incoming = match session.subscribe() {
                Ok(incoming) => incoming,
                Err(error) => {
                    let _ = session.shutdown();
                    let _ = runtime.set_status(InstanceStatus::Error);
                    return Err(OpenCodeProtocolMapper::error(error));
                }
            };
            let generation = session.generation().to_string();
            {
                let mut mutable = lock(&runtime.mutable);
                mutable.session = Some(session);
                mutable.session_generation = Some(generation.clone());
                mutable.sessions.clear();
                mutable.active_turns.clear();
                mutable.pending_approvals.clear();
            }
            let instance = match runtime.set_status(InstanceStatus::Ready) {
                Ok(instance) => instance,
                Err(error) => {
                    let session = {
                        let mut mutable = lock(&runtime.mutable);
                        mutable.session_generation = None;
                        mutable.session.take()
                    };
                    if let Some(session) = session {
                        let _ = tokio::task::spawn_blocking(move || session.shutdown()).await;
                    }
                    return Err(error);
                }
            };
            runtime.start_event_forwarder(generation, incoming);
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
                mutable.session_generation = None;
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
            let capabilities = lock(&runtime.capabilities).clone();
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
            let session = runtime.ready_session()?;
            let client = session.client();
            let requested_id = conversation_id.clone();
            let (session, active, messages) = tokio::task::spawn_blocking(move || {
                let session = client.get_session(&requested_id)?;
                let active = client.active_sessions()?.contains_key(&requested_id);
                let messages = load_conversation_messages(&client, &requested_id)?;
                Ok::<_, OpenCodeServerError>((session, active, messages))
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
                .conversation_items(&conversation.resource, &messages);
            Ok(ConversationGetResponse {
                conversation,
                items,
                page_info: None,
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
            if request.model.is_some() {
                return Err(capability_unsupported(
                    "OpenCode Provider does not advertise model selection",
                ));
            }
            if request.reasoning_effort.is_some() {
                return Err(capability_unsupported(
                    "OpenCode Provider does not support reasoning effort selection",
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
            if request.permission_level != OPENCODE_PERMISSION_LEVEL {
                return Err(protocol_error(
                    "unsupported_permission_level",
                    format!(
                        "OpenCode Provider only supports permission level {OPENCODE_PERMISSION_LEVEL}"
                    ),
                    false,
                ));
            }
            let runtime = self.instance(&request.route)?;
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
            if !workspace_root.is_absolute() {
                return Err(protocol_error(
                    "invalid_workspace_root",
                    "OpenCode workspaceRoot must be an absolute path".to_string(),
                    false,
                ));
            }
            let directory = workspace_root.to_string_lossy().to_string();
            let session = runtime.ready_session()?;
            let client = session.client();
            let created = tokio::task::spawn_blocking(move || {
                client.create_session(&OpenCodeSessionCreate {
                    agent: None,
                    model: None,
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
            if request.capability_revision != "opencode-server-1.18.25-controls-v1" {
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
            let conversation_resource = request.conversation.clone();
            let runtime = self.resource_instance(&request.conversation)?;
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
            if expected_active.turn.resource != request.turn {
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
                            && active.turn.resource == request.turn
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
            if active.turn.resource != request.turn {
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
                        current.epoch == active.epoch && current.turn.resource == request.turn
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
                if current.epoch != active.epoch || current.turn.resource != request.turn {
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
            if pending.approval.resource != request.approval {
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
                    mutable.status = InstanceStatus::Stopping;
                    mutable.session_generation = None;
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
    if settings.server_version.trim() != OPENCODE_VERIFIED_SERVER_VERSION {
        return Err(protocol_error(
            "invalid_instance_settings",
            format!(
                "serverVersion must be exactly {OPENCODE_VERIFIED_SERVER_VERSION}"
            ),
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
    Ok(settings)
}

fn default_server_working_directory() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .find_map(std::env::var_os)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
}

fn discover_cli_models(
    executable: &std::path::Path,
    working_directory: Option<&std::path::Path>,
) -> Result<Vec<OpenCodeModel>, OpenCodeServerError> {
    const MAX_CATALOG_OUTPUT_BYTES: usize = 1024 * 1024;
    let mut command = Command::new(executable);
    command
        .arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
    }
    let output = command
        .output()
        .map_err(|error| OpenCodeServerError::Spawn(format!("run `opencode models`: {error}")))?;
    if !output.status.success() {
        return Err(OpenCodeServerError::ProcessExited(format!(
            "`opencode models` exited with {}",
            output.status
        )));
    }
    if output.stdout.len() > MAX_CATALOG_OUTPUT_BYTES {
        return Err(OpenCodeServerError::Protocol(
            "`opencode models` output exceeded 1 MiB".to_string(),
        ));
    }
    let output = String::from_utf8(output.stdout).map_err(|error| {
        OpenCodeServerError::Protocol(format!("`opencode models` returned non-UTF-8 output: {error}"))
    })?;
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for line in output.lines().map(str::trim).filter(|line| !line.is_empty()) {
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

fn load_conversation_messages(
    client: &OpenCodeClient,
    session_id: &str,
) -> Result<Vec<crate::protocol::OpenCodeMessage>, OpenCodeServerError> {
    const PAGE_SIZE: u64 = 100;
    const MAX_PAGES: usize = 100;
    const MAX_MESSAGES: usize = 10_000;
    let mut messages = Vec::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_PAGES {
        let page = client.list_messages(session_id, cursor.as_deref(), PAGE_SIZE)?;
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
        if page.data.is_empty() {
            return Ok(messages);
        }
        messages.extend(page.data);
        if messages.len() > MAX_MESSAGES {
            return Err(OpenCodeServerError::Protocol(format!(
                "OpenCode conversation exceeds the {MAX_MESSAGES} message history limit"
            )));
        }
        let Some(next_cursor) = page.cursor.next else {
            return Ok(messages);
        };
        if !seen_cursors.insert(next_cursor.clone()) {
            return Err(OpenCodeServerError::Protocol(
                "OpenCode message history repeated a pagination cursor".to_string(),
            ));
        }
        cursor = Some(next_cursor);
    }
    Err(OpenCodeServerError::Protocol(format!(
        "OpenCode conversation exceeds the {MAX_PAGES}-page history limit"
    )))
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
    use super::{approval_resource_id, decode_settings, message_id, turn_resource_id};
    use serde_json::json;

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

        let future = serde_json::from_value(json!({
            "serverExecutable": "/absolute/opencode",
            "serverVersion": "1.18.26",
            "serverArgs": ["serve"]
        }))
        .unwrap();
        assert!(decode_settings(future).is_err());
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
