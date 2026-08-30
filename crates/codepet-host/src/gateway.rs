use crate::manager::{
    HostUpdate, PluginManager, PluginRuntimeSnapshot, PluginRuntimeState,
    ProviderInstanceRuntimeSnapshot,
};
use crate::{HostError, HostResult};
use codepet_gateway_sdk as gateway;
use codepet_gateway_sdk::ProtocolServer;
use codepet_provider_sdk as provider;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc};

const EVENT_CURSOR_PREFIX: &str = "event-";

struct GatewayEventState {
    sequence: u64,
    events: VecDeque<gateway::ProtocolEvent>,
}

struct GatewayEventBus {
    capacity: usize,
    state: Mutex<GatewayEventState>,
    sender: broadcast::Sender<gateway::ProtocolEvent>,
}

impl GatewayEventBus {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let (sender, _) = broadcast::channel(capacity);
        Self {
            capacity,
            state: Mutex::new(GatewayEventState {
                sequence: 0,
                events: VecDeque::with_capacity(capacity),
            }),
            sender,
        }
    }

    fn current_cursor(&self) -> gateway::EventCursor {
        self.state
            .lock()
            .map(|state| event_cursor(state.sequence))
            .unwrap_or_else(|_| event_cursor(0))
    }

    fn publish(
        &self,
        mut event: gateway::ProtocolEvent,
    ) -> Result<gateway::ProtocolEvent, gateway::ProtocolError> {
        let assigned = {
            let mut state = self.state.lock().map_err(|_| gateway_state_error())?;
            let sequence = state.sequence.checked_add(1).ok_or_else(|| gateway::ProtocolError {
                code: "gateway_event_cursor_exhausted".to_string(),
                message: "Gateway event cursor is exhausted".to_string(),
                retryable: false,
                details: None,
            })?;
            set_event_cursor(&mut event, event_cursor(sequence));
            state.sequence = sequence;
            state.events.push_back(event.clone());
            while state.events.len() > self.capacity {
                state.events.pop_front();
            }
            event
        };
        let _ = self.sender.send(assigned.clone());
        Ok(assigned)
    }

    fn replay(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<Vec<gateway::ProtocolEvent>, gateway::ProtocolError> {
        let state = self.state.lock().map_err(|_| gateway_state_error())?;
        let after_sequence = after_cursor.map(event_cursor_sequence).transpose()?;
        if let Some(after_sequence) = after_sequence {
            if after_sequence > state.sequence {
                return Err(cursor_error(
                    "invalid_event_cursor",
                    "requested event cursor is ahead of the Gateway",
                    after_sequence,
                    state.sequence,
                ));
            }
            if let Some(oldest) = state.events.front().map(protocol_event_sequence).transpose()? {
                if after_sequence.saturating_add(1) < oldest {
                    return Err(cursor_error(
                        "event_replay_unavailable",
                        "requested events are outside the in-memory replay window",
                        after_sequence,
                        state.sequence,
                    ));
                }
            }
        }
        Ok(state
            .events
            .iter()
            .filter(|event| {
                after_sequence.map_or(true, |after| {
                    protocol_event_sequence(event)
                        .map(|sequence| sequence > after)
                        .unwrap_or(false)
                })
            })
            .cloned()
            .collect())
    }

    fn subscribe(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<GatewayEventSubscription, gateway::ProtocolError> {
        let receiver = self.sender.subscribe();
        let replay = self.replay(after_cursor)?;
        Ok(GatewayEventSubscription {
            replay: replay.into(),
            receiver,
            last_sequence: after_cursor.map(event_cursor_sequence).transpose()?.unwrap_or(0),
        })
    }
}

pub struct GatewayEventSubscription {
    replay: VecDeque<gateway::ProtocolEvent>,
    receiver: broadcast::Receiver<gateway::ProtocolEvent>,
    last_sequence: u64,
}

impl GatewayEventSubscription {
    pub async fn next_event(&mut self) -> Result<gateway::ProtocolEvent, gateway::ProtocolError> {
        loop {
            let event = match self.replay.pop_front() {
                Some(event) => event,
                None => match self.receiver.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        return Err(gateway::ProtocolError {
                            code: "gateway_event_subscription_lagged".to_string(),
                            message: format!(
                                "Gateway event subscriber skipped {skipped} messages"
                            ),
                            retryable: true,
                            details: None,
                        });
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(gateway::ProtocolError {
                            code: "gateway_event_subscription_closed".to_string(),
                            message: "Gateway event subscription is closed".to_string(),
                            retryable: true,
                            details: None,
                        });
                    }
                },
            };
            let sequence = protocol_event_sequence(&event)?;
            if sequence > self.last_sequence {
                self.last_sequence = sequence;
                return Ok(event);
            }
        }
    }
}

pub struct ProviderGatewayService {
    manager: Arc<PluginManager>,
    events: Arc<GatewayEventBus>,
    updates: Mutex<Option<mpsc::Receiver<HostUpdate>>>,
    forwarding_started: AtomicBool,
    server_name: String,
    server_version: String,
    remote_host_identity: Option<gateway::RemoteHostIdentity>,
}

impl ProviderGatewayService {
    pub fn new(manager: Arc<PluginManager>) -> HostResult<Self> {
        Self::build(manager, None)
    }

    pub fn with_remote_identity(
        manager: Arc<PluginManager>,
        remote_host_identity: gateway::RemoteHostIdentity,
    ) -> HostResult<Self> {
        validate_remote_host_identity(&remote_host_identity)?;
        Self::build(manager, Some(remote_host_identity))
    }

    fn build(
        manager: Arc<PluginManager>,
        remote_host_identity: Option<gateway::RemoteHostIdentity>,
    ) -> HostResult<Self> {
        let event_capacity = manager.event_capacity().max(1);
        let updates = manager.take_updates()?;
        Ok(Self {
            manager,
            events: Arc::new(GatewayEventBus::new(event_capacity)),
            updates: Mutex::new(Some(updates)),
            forwarding_started: AtomicBool::new(false),
            server_name: "codepet-provider-gateway".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            remote_host_identity,
        })
    }

    pub fn current_event_cursor(&self) -> gateway::EventCursor {
        self.events.current_cursor()
    }

    pub fn replay_events(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<Vec<gateway::ProtocolEvent>, gateway::ProtocolError> {
        self.events.replay(after_cursor)
    }

    pub fn subscribe_events(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<GatewayEventSubscription, gateway::ProtocolError> {
        self.events.subscribe(after_cursor)
    }

    pub fn start_event_forwarding(self: &Arc<Self>) -> bool {
        if self
            .forwarding_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let mut updates = match self.updates.lock() {
            Ok(mut updates) => match updates.take() {
                Some(updates) => updates,
                None => return false,
            },
            Err(_) => return false,
        };
        let service = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(update) = updates.recv().await {
                let Some(service) = service.upgrade() else {
                    return;
                };
                if let Err(error) = service.forward_host_update(update) {
                    eprintln!("Provider Gateway failed to forward Host update: {error:?}");
                }
            }
        });
        true
    }

    fn forward_host_update(
        &self,
        update: HostUpdate,
    ) -> Result<(), gateway::ProtocolError> {
        match update {
            HostUpdate::ProviderEvent(event) => {
                self.events
                    .publish(self.map_provider_event(event))
                    ?;
            }
            HostUpdate::PluginStateChanged {
                snapshot,
                previous_state,
            } => {
                for instance in &snapshot.instances {
                    let provider = gateway_instance(&snapshot, instance);
                    let previous_status = Some(provider_runtime_status(
                        previous_state,
                        instance.instance.as_ref(),
                    ));
                    self.events.publish(gateway::ProtocolEvent::ProviderStatusChanged {
                        protocol_version: gateway::PROTOCOL_VERSION,
                        event_cursor: event_cursor(0),
                        payload: gateway::ProviderStatusChangedEvent {
                            provider,
                            previous_status,
                        },
                    })?;
                }
            }
            HostUpdate::InstanceChanged {
                snapshot,
                instance_id,
                previous_status,
            } => {
                let runtime = snapshot
                    .instances
                    .iter()
                    .find(|runtime| runtime.record.instance_id == instance_id)
                    .ok_or_else(|| {
                        gateway::ProtocolError {
                            code: "unknown_provider_instance".to_string(),
                            message: "Provider instance update targets an unknown manifest instance".to_string(),
                            retryable: false,
                            details: None,
                        }
                    })?;
                self.events.publish(gateway::ProtocolEvent::ProviderStatusChanged {
                    protocol_version: gateway::PROTOCOL_VERSION,
                    event_cursor: event_cursor(0),
                    payload: gateway::ProviderStatusChangedEvent {
                        provider: gateway_instance(&snapshot, runtime),
                        previous_status: previous_status.map(instance_status_to_gateway),
                    },
                })?;
            }
        }
        Ok(())
    }

    fn map_provider_event(&self, event: provider::ProtocolEvent) -> gateway::ProtocolEvent {
        match event {
            provider::ProtocolEvent::EventInstanceStatusChanged { .. } => {
                unreachable!("instance status events are converted to one Host state update")
            }
            provider::ProtocolEvent::EventConversationUpserted { params, .. } => {
                gateway::ProtocolEvent::ConversationUpserted {
                    protocol_version: gateway::PROTOCOL_VERSION,
                    event_cursor: event_cursor(0),
                    payload: gateway::ConversationUpsertedEvent {
                        conversation: map_conversation(params.conversation),
                    },
                }
            }
            provider::ProtocolEvent::EventTurnUpserted { params, .. } => {
                gateway::ProtocolEvent::TurnUpserted {
                    protocol_version: gateway::PROTOCOL_VERSION,
                    event_cursor: event_cursor(0),
                    payload: gateway::TurnUpsertedEvent {
                        turn: map_turn(params.turn),
                    },
                }
            }
            provider::ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                gateway::ProtocolEvent::TurnOutputDelta {
                    protocol_version: gateway::PROTOCOL_VERSION,
                    event_cursor: event_cursor(0),
                    payload: gateway::TurnOutputDeltaEvent {
                        turn: params.turn,
                        conversation: params.conversation,
                        output_id: params.output_id,
                        kind: params.kind,
                        delta: params.delta,
                    },
                }
            }
            provider::ProtocolEvent::EventApprovalRequested { params, .. } => {
                gateway::ProtocolEvent::ApprovalRequested {
                    protocol_version: gateway::PROTOCOL_VERSION,
                    event_cursor: event_cursor(0),
                    payload: gateway::ApprovalRequestedEvent {
                        approval: map_approval(params.approval),
                    },
                }
            }
            provider::ProtocolEvent::EventApprovalResolved { params, .. } => {
                gateway::ProtocolEvent::ApprovalResolved {
                    protocol_version: gateway::PROTOCOL_VERSION,
                    event_cursor: event_cursor(0),
                    payload: gateway::ApprovalResolvedEvent {
                        approval: map_approval(params.approval),
                    },
                }
            }
        }
    }

    async fn gateway_instances(
        &self,
        device_id: Option<&str>,
    ) -> Result<Vec<gateway::ProviderInstance>, gateway::ProtocolError> {
        let local_device_id = &self.manager.device().identity().device_id;
        if device_id.is_some_and(|requested| requested != local_device_id) {
            return Err(gateway::ProtocolError {
                code: "unknown_device".to_string(),
                message: "requested device is not registered on this Gateway".to_string(),
                retryable: false,
                details: None,
            });
        }
        let mut providers = Vec::new();
        for snapshot in self.manager.snapshots().await {
            for instance in &snapshot.instances {
                providers.push(gateway_instance(&snapshot, instance));
            }
        }
        providers.sort_by(|left, right| {
            left.route
                .device_id
                .cmp(&right.route.device_id)
                .then_with(|| {
                    left.route
                        .provider_instance_id
                        .cmp(&right.route.provider_instance_id)
                })
        });
        Ok(providers)
    }

    fn local_device(&self) -> gateway::Device {
        let identity = self.manager.device().identity();
        gateway::Device {
            device_id: identity.device_id.clone(),
            display_name: identity.display_name.clone(),
            status: gateway::DeviceStatus::Online,
            last_seen_at: Some(now_ms()),
        }
    }
}

impl ProtocolServer for ProviderGatewayService {
    fn protocol_handshake<'a>(
        &'a self,
        request: gateway::HandshakeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::HandshakeResponse> {
        Box::pin(async move {
            let remote_host_identity = self.remote_host_identity.clone().ok_or_else(|| {
                gateway::ProtocolError {
                    code: "remote_host_identity_unavailable".to_string(),
                    message: "Remote Gateway handshake requires a transport-injected Host identity"
                        .to_string(),
                    retryable: false,
                    details: None,
                }
            })?;
            validate_gateway_version_range(&request.supported_versions)?;
            if request.client_id.trim().is_empty()
                || request.client_name.trim().is_empty()
                || request.client_version.trim().is_empty()
            {
                return Err(gateway::ProtocolError {
                    code: "invalid_gateway_client".to_string(),
                    message: "Gateway client identity fields must not be empty".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            if let Some(cursor) = request.last_event_cursor.as_deref() {
                self.events.replay(Some(cursor))?;
            }
            Ok(gateway::HandshakeResponse {
                selected_version: gateway::PROTOCOL_VERSION,
                server_name: self.server_name.clone(),
                server_version: self.server_version.clone(),
                device: remote_host_identity,
                devices: vec![self.local_device()],
                providers: self.gateway_instances(None).await?,
                event_cursor: self.current_event_cursor(),
            })
        })
    }

    fn event_subscribe<'a>(
        &'a self,
        request: gateway::EventSubscribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::EventSubscribeResponse> {
        Box::pin(async move {
            self.events.replay(Some(&request.after_cursor))?;
            Ok(gateway::EventSubscribeResponse {
                subscribed_after_cursor: request.after_cursor,
            })
        })
    }

    fn device_list<'a>(
        &'a self,
        _request: gateway::DeviceListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::DeviceListResponse> {
        Box::pin(async move {
            Ok(gateway::DeviceListResponse {
                devices: vec![self.local_device()],
            })
        })
    }

    fn provider_list<'a>(
        &'a self,
        request: gateway::ProviderListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProviderListResponse> {
        Box::pin(async move {
            Ok(gateway::ProviderListResponse {
                providers: self.gateway_instances(request.device_id.as_deref()).await?,
            })
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: gateway::ConversationListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationListResponse> {
        Box::pin(async move {
            if request.route.is_none() && request.cursor.is_some() {
                return Err(gateway::ProtocolError {
                    code: "aggregate_conversation_cursor_unsupported".to_string(),
                    message: "route-less conversation.list does not support Provider cursors"
                        .to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let snapshot_cursor = self.current_event_cursor();
            let mut conversations = Vec::new();
            let mut next_cursor = None;
            if let Some(route) = request.route {
                let response = self
                    .manager
                    .conversation_list(provider::ConversationListRequest {
                        route: provider_route(route),
                        cursor: request.cursor,
                        limit: request.limit,
                    })
                    .await
                    .map_err(gateway_error)?;
                conversations.extend(response.conversations.into_iter().map(map_conversation));
                next_cursor = response.page_info.next_cursor;
            } else {
                let providers = self.gateway_instances(None).await?;
                for instance in providers
                    .into_iter()
                    .filter(|instance| instance.status == gateway::ProviderStatus::Ready)
                    .filter(|instance| {
                        instance
                            .capabilities
                            .methods
                            .contains(&gateway::GatewayCapability::ConversationList)
                    })
                {
                    let response = self
                        .manager
                        .conversation_list(provider::ConversationListRequest {
                            route: provider_route(instance.route),
                            cursor: request.cursor.clone(),
                            limit: request.limit,
                        })
                        .await
                        .map_err(gateway_error)?;
                    conversations.extend(response.conversations.into_iter().map(map_conversation));
                }
                if let Some(limit) = request.limit.and_then(|limit| usize::try_from(limit).ok()) {
                    conversations.truncate(limit);
                }
            }
            Ok(gateway::ConversationListResponse {
                conversations,
                page_info: gateway::PageInfo { next_cursor },
                snapshot_cursor,
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: gateway::ConversationGetRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationGetResponse> {
        Box::pin(async move {
            let snapshot_cursor = self.current_event_cursor();
            let response = self
                .manager
                .conversation_get(provider::ConversationGetRequest {
                    conversation: request.conversation,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationGetResponse {
                conversation: map_conversation(response.conversation),
                snapshot_cursor,
            })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: gateway::ConversationCreateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationCreateResponse> {
        Box::pin(async move {
            let response = self
                .manager
                .conversation_create(provider::ConversationCreateRequest {
                    route: provider_route(request.route),
                    title: request.title,
                    permission_level: request.permission_level,
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                    workspace_root: request.workspace_root,
                    extension: None,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationCreateResponse {
                conversation: map_conversation(response.conversation),
            })
        })
    }

    fn turn_send<'a>(
        &'a self,
        request: gateway::TurnSendRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnSendResponse> {
        Box::pin(async move {
            if let Some(steer_turn) = request.steer_turn {
                ensure_same_gateway_route(&request.conversation, &steer_turn)?;
                let expected_conversation = request.conversation;
                let response = self
                    .manager
                    .turn_steer(provider::TurnSteerRequest {
                        conversation: expected_conversation.clone(),
                        turn: steer_turn,
                        client_message_id: request.client_message_id,
                        message: request.message,
                    })
                    .await
                    .map_err(gateway_error)?;
                ensure_same_resource_identity(
                    &response.turn.conversation,
                    &expected_conversation,
                )?;
                return Ok(gateway::TurnSendResponse {
                    turn: map_turn(response.turn),
                });
            }
            let response = self
                .manager
                .turn_start(provider::TurnStartRequest {
                    conversation: request.conversation,
                    client_message_id: request.client_message_id,
                    message: request.message,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::TurnSendResponse {
                turn: map_turn(response.turn),
            })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: gateway::TurnInterruptRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnInterruptResponse> {
        Box::pin(async move {
            ensure_same_gateway_route(&request.conversation, &request.turn)?;
            let response = self
                .manager
                .turn_interrupt(provider::TurnInterruptRequest {
                    conversation: request.conversation,
                    turn: request.turn,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::TurnInterruptResponse {
                turn: map_turn(response.turn),
            })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: gateway::ApprovalResolveRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ApprovalResolveResponse> {
        Box::pin(async move {
            let response = self
                .manager
                .approval_resolve(provider::ApprovalResolveRequest {
                    approval: request.approval,
                    decision: match request.decision {
                        gateway::ApprovalDecision::Approve => provider::ApprovalDecision::Approve,
                        gateway::ApprovalDecision::Deny => provider::ApprovalDecision::Deny,
                    },
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ApprovalResolveResponse {
                approval: map_approval(response.approval),
            })
        })
    }
}

fn gateway_instance(
    plugin: &PluginRuntimeSnapshot,
    runtime: &ProviderInstanceRuntimeSnapshot,
) -> gateway::ProviderInstance {
    gateway::ProviderInstance {
        route: gateway::GatewayProviderRoute {
            device_id: runtime.record.device_id.clone(),
            provider_plugin_id: plugin.catalog.plugin_id.clone(),
            provider_instance_id: runtime.record.instance_id.clone(),
        },
        plugin_id: plugin.catalog.plugin_id.clone(),
        display_name: runtime.record.display_name.clone(),
        version: plugin.reported.as_ref().map(|reported| reported.version.clone()),
        status: if plugin.catalog.enabled && runtime.record.enabled {
            provider_runtime_status(plugin.state, runtime.instance.as_ref())
        } else {
            gateway::ProviderStatus::Unavailable
        },
        capabilities: runtime
            .instance
            .as_ref()
            .map(|instance| map_capabilities(&instance.capabilities))
            .unwrap_or_else(empty_gateway_capabilities),
    }
}

fn provider_runtime_status(
    state: PluginRuntimeState,
    instance: Option<&provider::ProviderInstance>,
) -> gateway::ProviderStatus {
    match state {
        PluginRuntimeState::Stopped => gateway::ProviderStatus::Disconnected,
        PluginRuntimeState::Starting => gateway::ProviderStatus::Connecting,
        PluginRuntimeState::Ready => instance
            .map(|instance| instance_status_to_gateway(instance.status))
            .unwrap_or(gateway::ProviderStatus::Unavailable),
        PluginRuntimeState::Crashed => gateway::ProviderStatus::Error,
    }
}

fn instance_status_to_gateway(status: provider::InstanceStatus) -> gateway::ProviderStatus {
    match status {
        provider::InstanceStatus::Created => gateway::ProviderStatus::Unavailable,
        provider::InstanceStatus::Stopped => gateway::ProviderStatus::Disconnected,
        provider::InstanceStatus::Starting | provider::InstanceStatus::Stopping => {
            gateway::ProviderStatus::Connecting
        }
        provider::InstanceStatus::Ready => gateway::ProviderStatus::Ready,
        provider::InstanceStatus::Error => gateway::ProviderStatus::Error,
    }
}

fn map_capabilities(capabilities: &provider::ProviderCapabilities) -> gateway::GatewayCapabilities {
    let mut methods = Vec::new();
    for method in &capabilities.methods {
        let mapped = match method {
            provider::ProviderCapability::ConversationList => {
                Some(gateway::GatewayCapability::ConversationList)
            }
            provider::ProviderCapability::ConversationGet => {
                Some(gateway::GatewayCapability::ConversationGet)
            }
            provider::ProviderCapability::ConversationCreate => {
                Some(gateway::GatewayCapability::ConversationCreate)
            }
            provider::ProviderCapability::TurnStart => {
                Some(gateway::GatewayCapability::TurnSend)
            }
            provider::ProviderCapability::TurnSteer => None,
            provider::ProviderCapability::TurnInterrupt => {
                Some(gateway::GatewayCapability::TurnInterrupt)
            }
            provider::ProviderCapability::ApprovalResolve => {
                Some(gateway::GatewayCapability::ApprovalResolve)
            }
        };
        if let Some(mapped) = mapped {
            if !methods.contains(&mapped) {
                methods.push(mapped);
            }
        }
    }
    gateway::GatewayCapabilities {
        methods,
        permission_levels: capabilities.permission_levels.clone(),
        models: capabilities.models.clone(),
        reasoning_efforts: capabilities.reasoning_efforts.clone(),
    }
}

fn empty_gateway_capabilities() -> gateway::GatewayCapabilities {
    gateway::GatewayCapabilities {
        methods: Vec::new(),
        permission_levels: Vec::new(),
        models: Vec::new(),
        reasoning_efforts: Vec::new(),
    }
}

fn map_conversation(conversation: provider::ProviderConversation) -> gateway::Conversation {
    gateway::Conversation {
        resource: conversation.resource,
        title: conversation.title,
        preview: conversation.preview,
        status: match conversation.status {
            provider::ConversationStatus::Idle => gateway::ConversationStatus::Idle,
            provider::ConversationStatus::Running => gateway::ConversationStatus::Running,
            provider::ConversationStatus::WaitingApproval => {
                gateway::ConversationStatus::WaitingApproval
            }
            provider::ConversationStatus::WaitingUserInput => {
                gateway::ConversationStatus::WaitingUserInput
            }
            provider::ConversationStatus::Error => gateway::ConversationStatus::Error,
            provider::ConversationStatus::Archived => gateway::ConversationStatus::Archived,
        },
        permission_level: conversation.permission_level,
        model: conversation.model,
        reasoning_effort: conversation.reasoning_effort,
        workspace_root: conversation.workspace_root,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
        active_turn: conversation.active_turn.map(map_turn),
    }
}

fn map_turn(turn: provider::ProviderTurn) -> gateway::TurnTask {
    gateway::TurnTask {
        resource: turn.resource,
        conversation: turn.conversation,
        status: match turn.status {
            provider::TurnStatus::Queued => gateway::TurnStatus::Queued,
            provider::TurnStatus::Running => gateway::TurnStatus::Running,
            provider::TurnStatus::WaitingApproval => gateway::TurnStatus::WaitingApproval,
            provider::TurnStatus::Completed => gateway::TurnStatus::Completed,
            provider::TurnStatus::Failed => gateway::TurnStatus::Failed,
            provider::TurnStatus::Interrupted => gateway::TurnStatus::Interrupted,
        },
        display_summary: turn.display_summary,
        started_at: turn.started_at,
        updated_at: turn.updated_at,
        completed_at: turn.completed_at,
    }
}

fn map_approval(approval: provider::ProviderApproval) -> gateway::Approval {
    gateway::Approval {
        resource: approval.resource,
        conversation: approval.conversation,
        turn: approval.turn,
        kind: approval.kind,
        title: approval.title,
        description: approval.description,
        status: match approval.status {
            provider::ApprovalStatus::Pending => gateway::ApprovalStatus::Pending,
            provider::ApprovalStatus::Approved => gateway::ApprovalStatus::Approved,
            provider::ApprovalStatus::Denied => gateway::ApprovalStatus::Denied,
            provider::ApprovalStatus::Expired => gateway::ApprovalStatus::Expired,
        },
        decisions: approval
            .decisions
            .into_iter()
            .map(|decision| match decision {
                provider::ApprovalDecision::Approve => gateway::ApprovalDecision::Approve,
                provider::ApprovalDecision::Deny => gateway::ApprovalDecision::Deny,
            })
            .collect(),
        requested_at: approval.requested_at,
        resolved_at: approval.resolved_at,
        decision: approval.decision.map(|decision| match decision {
            provider::ApprovalDecision::Approve => gateway::ApprovalDecision::Approve,
            provider::ApprovalDecision::Deny => gateway::ApprovalDecision::Deny,
        }),
    }
}

fn provider_route(route: gateway::GatewayProviderRoute) -> provider::ProviderInstanceRoute {
    provider::ProviderInstanceRoute {
        device_id: route.device_id,
        provider_plugin_id: route.provider_plugin_id,
        provider_instance_id: route.provider_instance_id,
    }
}

fn ensure_same_gateway_route(
    left: &gateway::RoutedResourceId,
    right: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    validate_gateway_resource(left)?;
    validate_gateway_resource(right)?;
    if left.device_id == right.device_id
        && left.provider_plugin_id == right.provider_plugin_id
        && left.provider_instance_id == right.provider_instance_id
    {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "gateway_route_mismatch".to_string(),
        message: "conversation and steer turn target different Provider routes".to_string(),
        retryable: false,
        details: None,
    })
}

fn ensure_same_resource_identity(
    actual: &gateway::RoutedResourceId,
    expected: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    validate_gateway_resource(actual)?;
    if actual == expected {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "provider_resource_identity_mismatch".to_string(),
        message: "Provider returned a turn for a different conversation".to_string(),
        retryable: false,
        details: None,
    })
}

fn validate_remote_host_identity(identity: &gateway::RemoteHostIdentity) -> HostResult<()> {
    let fingerprint = identity.identity_fingerprint.as_bytes();
    let is_lowercase_sha256 = fingerprint.len() == 64
        && fingerprint
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    let is_all_zero = fingerprint.iter().all(|byte| *byte == b'0');
    if !is_lowercase_sha256 || is_all_zero {
        return Err(HostError::new(
            "invalid_remote_host_identity",
            "Remote Host identity fingerprint must be a non-zero 64-character lowercase hexadecimal SHA-256 digest",
        ));
    }
    Ok(())
}

fn validate_gateway_resource(
    resource: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    if resource.device_id.trim().is_empty()
        || resource.provider_plugin_id.trim().is_empty()
        || resource.provider_instance_id.trim().is_empty()
        || resource.native_resource_id.trim().is_empty()
    {
        return Err(gateway::ProtocolError {
            code: "invalid_gateway_resource".to_string(),
            message: "deviceId, providerPluginId, providerInstanceId, and nativeResourceId must not be empty".to_string(),
            retryable: false,
            details: None,
        });
    }
    Ok(())
}

fn validate_gateway_version_range(
    supported: &gateway::VersionRange,
) -> Result<(), gateway::ProtocolError> {
    if supported.min_version > supported.max_version {
        return Err(gateway::ProtocolError {
            code: "invalid_protocol_range".to_string(),
            message: "minimum Gateway version exceeds maximum version".to_string(),
            retryable: false,
            details: None,
        });
    }
    if gateway::PROTOCOL_VERSION < supported.min_version
        || gateway::PROTOCOL_VERSION > supported.max_version
    {
        return Err(gateway::ProtocolError {
            code: "unsupported_protocol_version".to_string(),
            message: "Gateway protocol version is outside the client-supported range".to_string(),
            retryable: false,
            details: None,
        });
    }
    Ok(())
}

fn gateway_error(error: HostError) -> gateway::ProtocolError {
    error.into_protocol_error()
}

fn event_cursor(sequence: u64) -> gateway::EventCursor {
    format!("{EVENT_CURSOR_PREFIX}{sequence:020}")
}

fn event_cursor_sequence(cursor: &str) -> Result<u64, gateway::ProtocolError> {
    let sequence = cursor
        .strip_prefix(EVENT_CURSOR_PREFIX)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| gateway::ProtocolError {
            code: "invalid_event_cursor".to_string(),
            message: format!("invalid Gateway event cursor: {cursor}"),
            retryable: false,
            details: None,
        })?;
    if event_cursor(sequence) != cursor {
        return Err(gateway::ProtocolError {
            code: "invalid_event_cursor".to_string(),
            message: format!("non-canonical Gateway event cursor: {cursor}"),
            retryable: false,
            details: None,
        });
    }
    Ok(sequence)
}

fn protocol_event_sequence(event: &gateway::ProtocolEvent) -> Result<u64, gateway::ProtocolError> {
    let cursor = match event {
        gateway::ProtocolEvent::DeviceStatusChanged { event_cursor, .. }
        | gateway::ProtocolEvent::ProviderStatusChanged { event_cursor, .. }
        | gateway::ProtocolEvent::ConversationUpserted { event_cursor, .. }
        | gateway::ProtocolEvent::TurnUpserted { event_cursor, .. }
        | gateway::ProtocolEvent::TurnOutputDelta { event_cursor, .. }
        | gateway::ProtocolEvent::ApprovalRequested { event_cursor, .. }
        | gateway::ProtocolEvent::ApprovalResolved { event_cursor, .. } => event_cursor,
    };
    event_cursor_sequence(cursor)
}

fn set_event_cursor(event: &mut gateway::ProtocolEvent, cursor: gateway::EventCursor) {
    match event {
        gateway::ProtocolEvent::DeviceStatusChanged {
            protocol_version,
            event_cursor,
            ..
        }
        | gateway::ProtocolEvent::ProviderStatusChanged {
            protocol_version,
            event_cursor,
            ..
        }
        | gateway::ProtocolEvent::ConversationUpserted {
            protocol_version,
            event_cursor,
            ..
        }
        | gateway::ProtocolEvent::TurnUpserted {
            protocol_version,
            event_cursor,
            ..
        }
        | gateway::ProtocolEvent::TurnOutputDelta {
            protocol_version,
            event_cursor,
            ..
        }
        | gateway::ProtocolEvent::ApprovalRequested {
            protocol_version,
            event_cursor,
            ..
        }
        | gateway::ProtocolEvent::ApprovalResolved {
            protocol_version,
            event_cursor,
            ..
        } => {
            *protocol_version = gateway::PROTOCOL_VERSION;
            *event_cursor = cursor;
        }
    }
}

fn gateway_state_error() -> gateway::ProtocolError {
    gateway::ProtocolError {
        code: "gateway_state_error".to_string(),
        message: "Gateway event state lock is unavailable".to_string(),
        retryable: true,
        details: None,
    }
}

fn cursor_error(
    code: &str,
    message: &str,
    requested: u64,
    current: u64,
) -> gateway::ProtocolError {
    let mut details = BTreeMap::new();
    details.insert("requestedSequence".to_string(), requested.into());
    details.insert("currentSequence".to_string(), current.into());
    gateway::ProtocolError {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        details: Some(details),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
