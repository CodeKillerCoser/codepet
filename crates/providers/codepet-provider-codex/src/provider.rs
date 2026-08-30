use crate::client::CodexAppServerSession;
use crate::mapper::{parse_permission_level, CodexProtocolMapper};
use crate::protocol::{
    CodexAppServerError, CodexApprovalRequest, CodexIncoming, CodexNotification,
    CodexThreadListRequest, CodexThreadStartRequest, CodexTurnStartRequest,
    CodexTurnSteerRequest, CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalResolveRequest, ApprovalResolveResponse, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, InstanceCapabilitiesRequest,
    InstanceCapabilitiesResponse, InstanceCreateRequest, InstanceCreateResponse,
    InstanceDestroyRequest, InstanceDestroyResponse, InstanceStartRequest,
    InstanceStartResponse, InstanceStatus, InstanceStatusChangedEvent, InstanceStopRequest,
    InstanceStopResponse, PageInfo, ProtocolError, ProtocolEvent, ProtocolFuture,
    ProtocolServer, ProviderCapabilities, ProviderDescribeRequest, ProviderDescribeResponse,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance,
    ProviderInstanceRoute, ProviderPluginDescriptor, ProviderShutdownRequest,
    ProviderShutdownResponse, RoutedResourceId, TurnInterruptRequest, TurnInterruptResponse,
    TurnStartRequest, TurnStartResponse, TurnSteerRequest, TurnSteerResponse, VersionRange,
    PROTOCOL_VERSION,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct CodexInstanceSettings {
    app_server_executable: PathBuf,
    app_server_args: Vec<String>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    reasoning_efforts: Vec<String>,
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

struct InstanceMutable {
    status: InstanceStatus,
    session: Option<CodexAppServerSession>,
    pending_approvals: HashMap<String, CodexApprovalRequest>,
    turn_conversations: HashMap<String, String>,
}

struct CodexInstanceRuntime {
    route: ProviderInstanceRoute,
    instance_kind: String,
    display_name: String,
    settings: CodexInstanceSettings,
    capabilities: ProviderCapabilities,
    mutable: Mutex<InstanceMutable>,
    mapper: Mutex<CodexProtocolMapper>,
    events: Arc<dyn ProviderEventSink>,
}

impl CodexInstanceRuntime {
    fn new(
        request: InstanceCreateRequest,
        settings: CodexInstanceSettings,
        events: Arc<dyn ProviderEventSink>,
    ) -> Self {
        let capabilities = CodexProtocolMapper::capabilities(
            settings.models.clone(),
            settings.reasoning_efforts.clone(),
        );
        Self {
            route: request.route.clone(),
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            capabilities,
            mutable: Mutex::new(InstanceMutable {
                status: InstanceStatus::Created,
                session: None,
                pending_approvals: HashMap::new(),
                turn_conversations: HashMap::new(),
            }),
            mapper: Mutex::new(CodexProtocolMapper::new(request.route)),
            events,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        let status = lock(&self.mutable).status;
        lock(&self.mapper).instance(
            CODEX_PLUGIN_ID.to_string(),
            self.instance_kind.clone(),
            self.display_name.clone(),
            status,
            self.capabilities.clone(),
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
        let Some(previous_status) = previous_status else {
            return Ok(self.snapshot());
        };
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

    fn ready_session(&self) -> Result<CodexAppServerSession, ProtocolError> {
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
        mutable.session.clone().ok_or_else(|| {
            protocol_error(
                "provider_unavailable",
                "Codex App Server session is unavailable".to_string(),
                true,
            )
        })
    }

    fn remember_turn(&self, turn_id: String, conversation_id: String) {
        lock(&self.mutable)
            .turn_conversations
            .insert(turn_id, conversation_id);
    }

    fn remember_conversation(&self, conversation: &codepet_provider_sdk::ProviderConversation) {
        if let Some(turn) = conversation.active_turn.as_ref() {
            self.remember_turn(
                turn.resource.native_resource_id.clone(),
                conversation.resource.native_resource_id.clone(),
            );
        }
    }

    fn conversation_for_turn(&self, turn_id: &str) -> Result<String, ProtocolError> {
        lock(&self.mutable)
            .turn_conversations
            .get(turn_id)
            .cloned()
            .ok_or_else(|| {
                protocol_error(
                    "turn_context_unavailable",
                    format!(
                        "Codex turn {turn_id} has not been observed in this Provider instance"
                    ),
                    false,
                )
            })
    }

    fn start_event_forwarder(
        self: &Arc<Self>,
        incoming: Receiver<Result<CodexIncoming, CodexAppServerError>>,
    ) {
        let runtime = Arc::downgrade(self);
        thread::spawn(move || {
            while let Ok(message) = incoming.recv() {
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                match message {
                    Ok(incoming) => {
                        if !matches!(
                            runtime.status(),
                            InstanceStatus::Ready | InstanceStatus::Starting
                        ) {
                            return;
                        }
                        runtime.remember_incoming(&incoming);
                        let events = lock(&runtime.mapper).events(incoming, now_ms());
                        match events {
                            Ok(events) => {
                                for event in events {
                                    if let Err(error) = runtime.events.publish(event) {
                                        runtime.fail_from_event_forwarder(error);
                                        return;
                                    }
                                }
                            }
                            Err(error) => {
                                runtime.fail_from_event_forwarder(error);
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let should_fail = {
                            let mutable = lock(&runtime.mutable);
                            matches!(mutable.status, InstanceStatus::Ready | InstanceStatus::Starting)
                        };
                        if should_fail {
                            runtime.fail_from_event_forwarder(CodexProtocolMapper::error(error));
                        }
                        return;
                    }
                }
            }
        });
    }

    fn remember_incoming(&self, incoming: &CodexIncoming) {
        let mut mutable = lock(&self.mutable);
        match incoming {
            CodexIncoming::Notification(
                CodexNotification::TurnStarted { thread_id, turn }
                | CodexNotification::TurnCompleted { thread_id, turn },
            ) => {
                mutable
                    .turn_conversations
                    .insert(turn.id.clone(), thread_id.clone());
            }
            CodexIncoming::Notification(CodexNotification::OutputDelta {
                thread_id,
                turn_id,
                ..
            }) => {
                mutable
                    .turn_conversations
                    .insert(turn_id.clone(), thread_id.clone());
            }
            CodexIncoming::ApprovalRequested(approval) => {
                mutable
                    .turn_conversations
                    .insert(approval.turn_id.clone(), approval.thread_id.clone());
                mutable
                    .pending_approvals
                    .insert(approval.approval_id(), approval.clone());
            }
            CodexIncoming::Notification(CodexNotification::ServerRequestResolved {
                request_id,
                ..
            }) => {
                mutable.pending_approvals.remove(&request_id.approval_id());
            }
            CodexIncoming::Notification(CodexNotification::ThreadStarted { .. })
            | CodexIncoming::Notification(CodexNotification::Unknown { .. })
            | CodexIncoming::UnsupportedServerRequest { .. } => {}
        }
    }

    fn fail_from_event_forwarder(&self, error: ProtocolError) {
        eprintln!("Codex Provider event forwarding failed: {}", error.message);
        let previous_status = {
            let mut mutable = lock(&self.mutable);
            if !matches!(mutable.status, InstanceStatus::Ready | InstanceStatus::Starting) {
                return;
            }
            let previous = mutable.status;
            mutable.status = InstanceStatus::Error;
            mutable.session = None;
            mutable.pending_approvals.clear();
            mutable.turn_conversations.clear();
            previous
        };
        lock(&self.mapper).reset_session_state();
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
    instances: HashMap<String, Arc<CodexInstanceRuntime>>,
}

pub struct CodexProvider {
    state: Mutex<ProviderState>,
    events: Arc<dyn ProviderEventSink>,
    shutdown: AtomicBool,
}

impl CodexProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self {
            state: Mutex::new(ProviderState {
                host_device_id: None,
                initialized_client_id: None,
                instances: HashMap::new(),
            }),
            events,
            shutdown: AtomicBool::new(false),
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
            if runtime.status() == InstanceStatus::Ready {
                return Ok(InstanceStartResponse {
                    instance: runtime.snapshot(),
                });
            }
            if runtime.status() == InstanceStatus::Starting {
                return Err(protocol_error(
                    "provider_instance_starting",
                    "Codex Provider instance is already starting".to_string(),
                    true,
                ));
            }
            runtime.set_status(InstanceStatus::Starting)?;
            let executable = runtime.settings.app_server_executable.clone();
            let args = runtime.settings.app_server_args.clone();
            let session = tokio::task::spawn_blocking(move || {
                CodexAppServerSession::spawn(&executable, &args)
            })
            .await
            .map_err(|error| {
                protocol_error(
                    "provider_task_failed",
                    format!("Codex App Server start task failed: {error}"),
                    true,
                )
            })?;
            let session = match session {
                Ok(session) => session,
                Err(error) => {
                    let _ = runtime.set_status(InstanceStatus::Error);
                    return Err(CodexProtocolMapper::error(error));
                }
            };
            let incoming = session.subscribe();
            {
                let mut mutable = lock(&runtime.mutable);
                mutable.session = Some(session);
                mutable.pending_approvals.clear();
                mutable.turn_conversations.clear();
            }
            lock(&runtime.mapper).reset_session_state();
            runtime.start_event_forwarder(incoming);
            let instance = runtime.set_status(InstanceStatus::Ready)?;
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
                mutable.pending_approvals.clear();
                mutable.turn_conversations.clear();
                mutable.session.take()
            };
            if let Some(session) = session {
                tokio::task::spawn_blocking(move || session.shutdown())
                    .await
                    .map_err(|error| {
                        protocol_error(
                            "provider_task_failed",
                            format!("Codex App Server stop task failed: {error}"),
                            true,
                        )
                    })?
                    .map_err(CodexProtocolMapper::error)?;
            }
            {
                let mut mutable = lock(&runtime.mutable);
                mutable.pending_approvals.clear();
                mutable.turn_conversations.clear();
            }
            lock(&runtime.mapper).reset_session_state();
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
            if matches!(runtime.status(), InstanceStatus::Ready | InstanceStatus::Starting | InstanceStatus::Stopping) {
                return Err(protocol_error(
                    "provider_instance_running",
                    "Stop the Codex Provider instance before destroying it".to_string(),
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
            Ok(InstanceCapabilitiesResponse {
                capabilities: runtime.capabilities.clone(),
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
            let session = runtime.ready_session()?;
            let page = tokio::task::spawn_blocking(move || {
                session.thread_list(CodexThreadListRequest {
                    cursor: request.cursor,
                    limit,
                    workspace_root: None,
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
            for conversation in &conversations {
                runtime.remember_conversation(conversation);
            }
            Ok(ConversationListResponse {
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
            let session = runtime.ready_session()?;
            let snapshot = tokio::task::spawn_blocking(move || session.thread_read(&conversation_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
            let conversation = lock(&runtime.mapper).conversation(&snapshot);
            runtime.remember_conversation(&conversation);
            Ok(ConversationGetResponse { conversation })
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
            let session = runtime.ready_session()?;
            let snapshot = tokio::task::spawn_blocking(move || {
                session.thread_start(CodexThreadStartRequest {
                    workspace_root: request.workspace_root,
                    permission_level,
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                })
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let conversation = lock(&runtime.mapper).conversation(&snapshot);
            runtime.remember_conversation(&conversation);
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
            let session = runtime.ready_session()?;
            let native_conversation_id = conversation_id.clone();
            let turn = tokio::task::spawn_blocking(move || {
                session.turn_start(CodexTurnStartRequest {
                    thread_id: native_conversation_id,
                    message: request.message,
                    client_message_id: Some(request.client_message_id),
                    model: None,
                    reasoning_effort: None,
                })
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            runtime.remember_turn(turn.id.clone(), conversation_id.clone());
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn, now_ms());
            Ok(TurnStartResponse {
                turn: mapped_turn,
            })
        })
    }

    fn turn_steer<'a>(
        &'a self,
        request: TurnSteerRequest,
    ) -> ProtocolFuture<'a, TurnSteerResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.turn)?;
            let turn_id = request.turn.native_resource_id;
            let conversation_id = runtime.conversation_for_turn(&turn_id)?;
            let session = runtime.ready_session()?;
            let native_conversation_id = conversation_id.clone();
            let expected_turn_id = turn_id.clone();
            let turn = tokio::task::spawn_blocking(move || {
                session.turn_steer(CodexTurnSteerRequest {
                    thread_id: native_conversation_id,
                    expected_turn_id,
                    message: request.message,
                    client_message_id: Some(request.client_message_id),
                })
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            runtime.remember_turn(turn.id.clone(), conversation_id.clone());
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn, now_ms());
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
            let runtime = self.resource_instance(&request.turn)?;
            let turn_id = request.turn.native_resource_id;
            let conversation_id = runtime.conversation_for_turn(&turn_id)?;
            let session = runtime.ready_session()?;
            let native_conversation_id = conversation_id.clone();
            let native_turn_id = turn_id.clone();
            let turn = tokio::task::spawn_blocking(move || {
                session.turn_interrupt(&native_conversation_id, &native_turn_id)
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn, now_ms());
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
            let approval_id = request.approval.native_resource_id;
            let session = runtime.ready_session()?;
            let mut mapper = lock(&runtime.mapper);
            let mut mutable = lock(&runtime.mutable);
            let approval_request = mutable
                .pending_approvals
                .get(&approval_id)
                .cloned()
                .ok_or_else(|| {
                    protocol_error(
                        "approval_not_found",
                        format!("approval {approval_id} is not pending in this Provider instance"),
                        false,
                    )
                })?;
            session
                .respond_to_approval(&approval_request, request.decision)
                .map_err(CodexProtocolMapper::error)?;
            mutable.pending_approvals.remove(&approval_id);
            let (approval, event) = mapper.approval_resolved(
                &approval_id,
                request.decision,
                now_ms(),
            )?;
            drop(mutable);
            drop(mapper);
            runtime.events.publish(event)?;
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
            for runtime in instances {
                if matches!(
                    runtime.status(),
                    InstanceStatus::Ready
                        | InstanceStatus::Starting
                        | InstanceStatus::Stopping
                        | InstanceStatus::Error
                ) {
                    let session = {
                        let mut mutable = lock(&runtime.mutable);
                        mutable.status = InstanceStatus::Stopping;
                        mutable.session.take()
                    };
                    if let Some(session) = session {
                        tokio::task::spawn_blocking(move || session.shutdown())
                            .await
                            .map_err(provider_task_error)?
                            .map_err(CodexProtocolMapper::error)?;
                    }
                    {
                        let mut mutable = lock(&runtime.mutable);
                        mutable.status = InstanceStatus::Stopped;
                        mutable.pending_approvals.clear();
                        mutable.turn_conversations.clear();
                    }
                    lock(&runtime.mapper).reset_session_state();
                }
            }
            Ok(ProviderShutdownResponse { accepted: true })
        })
    }
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
    ensure_unique_non_empty(&settings.models, "models")?;
    ensure_unique_non_empty(&settings.reasoning_efforts, "reasoningEfforts")?;
    Ok(settings)
}

fn ensure_unique_non_empty(values: &[String], field: &str) -> Result<(), ProtocolError> {
    for (index, value) in values.iter().enumerate() {
        if value.trim().is_empty() || values[..index].contains(value) {
            return Err(protocol_error(
                "invalid_instance_settings",
                format!("{field} must contain unique non-empty strings"),
                false,
            ));
        }
    }
    Ok(())
}

fn validate_route(route: &ProviderInstanceRoute) -> Result<(), ProtocolError> {
    if route.device_id.trim().is_empty() || route.provider_instance_id.trim().is_empty() {
        return Err(protocol_error(
            "invalid_provider_route",
            "deviceId and providerInstanceId must not be empty".to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_resource(resource: &RoutedResourceId) -> Result<(), ProtocolError> {
    validate_route(&ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
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
