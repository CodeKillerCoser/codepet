use crate::providers::manager::{
    HostUpdate, PluginManager, PluginRuntimeSnapshot, PluginRuntimeState,
    ProviderInstanceRuntimeSnapshot,
};
use crate::conversation_state::ConversationStateStore;
use crate::recent_conversations::RecentSnapshots;
use crate::{HostError, HostResult};
use codepet_gateway_sdk as gateway;
use codepet_gateway_sdk::ProtocolServer;
use codepet_provider_sdk as provider;
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

const EVENT_CURSOR_PREFIX: &str = "event-";
const TURN_SEND_CACHE_CAPACITY: usize = 1_024;
const DEFAULT_TURN_SEND_CALLER_SCOPE: &str = "provider-gateway-protocol-default";

mod recent;

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteHostIdentity {
    pub device_id: String,
    pub descriptor: gateway::DeviceDescriptor,
}

#[derive(Clone)]
struct GatewayProviderRuntime {
    route: provider::ProviderInstanceRoute,
    summary: gateway::ProviderSummary,
    capabilities: gateway::GatewayCapabilities,
    provider_mark_read: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TurnSendKey {
    caller_scope: String,
    provider_id: String,
    client_request_id: String,
}

#[derive(Clone)]
struct TurnSendCacheEntry {
    request: gateway::TurnSendRequest,
    result: Result<gateway::TurnSendResponse, gateway::ProtocolError>,
}

struct TurnSendCache {
    entries: BTreeMap<TurnSendKey, TurnSendCacheEntry>,
    order: VecDeque<TurnSendKey>,
}

impl TurnSendCache {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, key: TurnSendKey, entry: TurnSendCacheEntry) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.entries.insert(key, entry);
        while self.entries.len() > TURN_SEND_CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

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
    remote_host_identity: Option<RemoteHostIdentity>,
    turn_sends: AsyncMutex<TurnSendCache>,
    conversation_state: Arc<ConversationStateStore>,
    recent: Arc<Mutex<RecentSnapshots>>,
    recent_concurrency: tokio::sync::Semaphore,
    recent_builds: AsyncMutex<BTreeMap<crate::recent_conversations::ViewKey, std::sync::Weak<AsyncMutex<()>>>>,
}

impl ProviderGatewayService {
    pub fn new(manager: Arc<PluginManager>) -> HostResult<Self> {
        Self::build(manager, None, Arc::new(ConversationStateStore::memory()))
    }

    pub fn with_remote_identity(
        manager: Arc<PluginManager>,
        remote_host_identity: RemoteHostIdentity,
    ) -> HostResult<Self> {
        validate_remote_host_identity(&remote_host_identity)?;
        Self::build(
            manager,
            Some(remote_host_identity),
            Arc::new(ConversationStateStore::memory()),
        )
    }

    pub fn with_remote_identity_and_state_path(
        manager: Arc<PluginManager>,
        remote_host_identity: RemoteHostIdentity,
        state_path: impl AsRef<Path>,
    ) -> HostResult<Self> {
        validate_remote_host_identity(&remote_host_identity)?;
        Self::build(
            manager,
            Some(remote_host_identity),
            Arc::new(ConversationStateStore::open(state_path)?),
        )
    }

    fn build(
        manager: Arc<PluginManager>,
        remote_host_identity: Option<RemoteHostIdentity>,
        conversation_state: Arc<ConversationStateStore>,
    ) -> HostResult<Self> {
        let event_capacity = manager.event_capacity().max(1);
        if let Some(path) = conversation_state.path() {
            manager.set_conversation_state_path(path)?;
        }
        conversation_state.configure(manager.provider_state_paths()?)?;
        let updates = manager.take_updates()?;
        Ok(Self {
            manager,
            events: Arc::new(GatewayEventBus::new(event_capacity)),
            updates: Mutex::new(Some(updates)),
            forwarding_started: AtomicBool::new(false),
            server_name: "codepet-provider-gateway".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            remote_host_identity,
            turn_sends: AsyncMutex::new(TurnSendCache::new()),
            conversation_state,
            recent: Arc::new(Mutex::new(RecentSnapshots::default())),
            recent_concurrency: tokio::sync::Semaphore::new(4),
            recent_builds: AsyncMutex::new(BTreeMap::new()),
        })
    }

    pub fn remote_connections(&self) -> Arc<crate::RemoteConnections> { self.manager.remote_connections() }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    pub fn remote_host_identity(&self) -> Option<&RemoteHostIdentity> {
        self.remote_host_identity.as_ref()
    }

    pub fn current_event_cursor(&self) -> gateway::EventCursor {
        self.events.current_cursor()
    }

    fn observe_conversation_summary(&self, conversation: &gateway::Conversation) -> Result<(), gateway::ProtocolError> {
        // First discovery can add unread members without advancing the legacy activity clock.
        if self.conversation_state.observe_summary(conversation).map_err(gateway_error)? {
            self.invalidate_recent(&conversation.resource.provider_id)?;
        }
        Ok(())
    }

    pub async fn resolve_provider_route(
        &self,
        provider_id: &str,
    ) -> Result<provider::ProviderInstanceRoute, gateway::ProtocolError> {
        self.gateway_providers(None)
            .await?
            .into_iter()
            .find(|provider| provider.summary.id == provider_id)
            .map(|provider| provider.route)
            .ok_or_else(|| gateway::ProtocolError {
                code: "unknown_provider".to_string(),
                message: "Provider id does not identify a configured Provider".to_string(),
                retryable: false,
                details: None,
            })
    }

    async fn resolve_resource(
        &self,
        resource: gateway::RoutedResourceId,
    ) -> Result<provider::ProviderResourceId, gateway::ProtocolError> {
        validate_gateway_resource(&resource)?;
        let route = self.resolve_provider_route(&resource.provider_id).await?;
        Ok(provider::ProviderResourceId {
            device_id: route.device_id,
            provider_plugin_id: route.provider_plugin_id,
            provider_instance_id: route.provider_instance_id,
            native_resource_id: resource.native_resource_id,
        })
    }

    async fn resolve_project_filter(
        &self,
        filter: gateway::ConversationProjectFilter,
    ) -> Result<provider::ConversationProjectFilter, gateway::ProtocolError> {
        Ok(match filter {
            gateway::ConversationProjectFilter::ConversationProjectFilterAll(filter) => {
                provider::ConversationProjectFilter::ConversationProjectFilterAll(
                    provider::ConversationProjectFilterAll {
                        kind: match filter.kind {
                            gateway::ConversationProjectFilterAllKind::All => {
                                provider::ConversationProjectFilterAllKind::All
                            }
                        },
                    },
                )
            }
            gateway::ConversationProjectFilter::ConversationProjectFilterStandalone(filter) => {
                provider::ConversationProjectFilter::ConversationProjectFilterStandalone(
                    provider::ConversationProjectFilterStandalone {
                        kind: match filter.kind {
                            gateway::ConversationProjectFilterStandaloneKind::Standalone => {
                                provider::ConversationProjectFilterStandaloneKind::Standalone
                            }
                        },
                    },
                )
            }
            gateway::ConversationProjectFilter::ConversationProjectFilterProject(filter) => {
                provider::ConversationProjectFilter::ConversationProjectFilterProject(
                    provider::ConversationProjectFilterProject {
                        kind: match filter.kind {
                            gateway::ConversationProjectFilterProjectKind::Project => {
                                provider::ConversationProjectFilterProjectKind::Project
                            }
                        },
                        project: self.resolve_resource(filter.project).await?,
                    },
                )
            }
        })
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

    pub async fn dispatch_for_caller_scope(
        &self,
        caller_scope: &str,
        request: gateway::ProtocolRequest,
    ) -> gateway::JsonRpcResponse {
        match request {
            gateway::ProtocolRequest::ConversationRecent { jsonrpc, id, params } => {
                let result = self.conversation_recent_for_scope(caller_scope, params).await;
                json_rpc_response(jsonrpc, id, result)
            }
            gateway::ProtocolRequest::TurnSend {
                jsonrpc,
                id,
                params,
            } => {
                let result = self.turn_send_for_caller_scope(caller_scope, params).await;
                json_rpc_response(jsonrpc, id, result)
            }
            gateway::ProtocolRequest::ConversationList { jsonrpc, id, params } => {
                let result = self.conversation_list(params).await.and_then(|mut response| {
                    for conversation in &mut response.conversations {
                        self.conversation_state
                            .decorate(caller_scope, conversation)
                            .map_err(gateway_error)?;
                    }
                    Ok(response)
                });
                json_rpc_response(jsonrpc, id, result)
            }
            gateway::ProtocolRequest::ConversationSearch { jsonrpc, id, params } => {
                let result = self.conversation_search(params).await.and_then(|mut response| {
                    for conversation in &mut response.conversations {
                        self.conversation_state
                            .decorate(caller_scope, conversation)
                            .map_err(gateway_error)?;
                    }
                    Ok(response)
                });
                json_rpc_response(jsonrpc, id, result)
            }
            gateway::ProtocolRequest::ConversationGet { jsonrpc, id, params } => {
                let result = self.conversation_get(params).await.and_then(|mut response| {
                    self.conversation_state
                        .decorate(caller_scope, &mut response.conversation)
                        .map_err(gateway_error)?;
                    Ok(response)
                });
                json_rpc_response(jsonrpc, id, result)
            }
            gateway::ProtocolRequest::ConversationMarkRead { jsonrpc, id, params } => {
                let result = self.mark_read_for_scope(caller_scope, params).await;
                json_rpc_response(jsonrpc, id, result)
            }
            request => gateway::dispatch(self, request).await,
        }
    }

    pub async fn turn_send_for_caller_scope(
        &self,
        caller_scope: &str,
        request: gateway::TurnSendRequest,
    ) -> Result<gateway::TurnSendResponse, gateway::ProtocolError> {
        let key = turn_send_key(caller_scope, &request)?;
        let mut cache = self.turn_sends.lock().await;
        if let Some(entry) = cache.entries.get(&key) {
            if entry.request != request {
                return Err(gateway::ProtocolError {
                    code: "client_request_conflict".to_string(),
                    message: "clientRequestId was already used with a different turn.send request"
                        .to_string(),
                    retryable: false,
                    details: None,
                });
            }
            return entry.result.clone();
        }
        let result = self.perform_turn_send(request.clone()).await;
        cache.insert(
            key,
            TurnSendCacheEntry {
                request,
                result: result.clone(),
            },
        );
        result
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
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                let update = tokio::select! {
                    update = updates.recv() => match update { Some(update) => Some(update), None => return },
                    _ = tick.tick() => None,
                };
                let Some(service) = service.upgrade() else {
                    return;
                };
                let result = match update {
                    Some(update) => service.forward_host_update(update),
                    None => service.publish_recent_invalidations(),
                };
                if let Err(error) = result {
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
                match &event {
                    provider::ProtocolEvent::EventConversationActiveChanged { params, .. } => {
                        self.invalidate_recent(&params.conversation.provider_instance_id)?;
                        return Ok(());
                    }
                    provider::ProtocolEvent::EventConversationUnreadChanged { params, .. } => {
                        // Shared replay is broad: never forward another scope's read state or ID.
                        self.invalidate_recent(&params.conversation.provider_instance_id)?;
                        return Ok(());
                    }
                    provider::ProtocolEvent::EventConversationDeleted { params, .. } => {
                        self.invalidate_recent(&params.conversation.provider_instance_id)?;
                        return Ok(());
                    }
                    provider::ProtocolEvent::EventConversationUpserted { params, .. } => {
                        self.invalidate_recent(&params.conversation.resource.provider_id)?;
                    }
                    provider::ProtocolEvent::EventTurnUpserted { params, .. } => {
                        self.invalidate_recent(&params.turn.conversation.provider_id)?;
                    }
                    _ => {}
                }
                let activity = self
                    .conversation_state
                    .observe_provider_event(&event)
                    .map_err(gateway_error)?;
                self.events.publish(self.map_provider_event(event))?;
                if let Some((conversation, version)) = activity {
                    self.invalidate_recent(&conversation.provider_id)?;
                    self.events.publish(gateway::ProtocolEvent::ConversationActivityChanged {
                        jsonrpc: "2.0".to_string(),
                        params: gateway::ProtocolEventParams {
                            event_cursor: event_cursor(0),
                            payload: gateway::ConversationActivityChangedEvent {
                                conversation,
                                activity_version: format!("activity-{version}"),
                            },
                        },
                    })?;
                }
            }
            HostUpdate::PluginStateChanged {
                snapshot,
                previous_state,
            } => {
                let _ = previous_state;
                for instance in &snapshot.instances {
                    let provider = gateway_provider(&snapshot, instance).summary;
                    self.invalidate_recent(&provider.id)?;
                    self.events.publish(gateway::ProtocolEvent::ProviderChanged {
                        jsonrpc: "2.0".to_string(),
                        params: gateway::ProtocolEventParams {
                            event_cursor: event_cursor(0),
                            payload: gateway::ProviderChangedEvent { provider },
                        },
                    })?;
                }
            }
            HostUpdate::InstanceChanged {
                snapshot,
                instance_id,
                previous_status,
            } => {
                let _ = previous_status;
                self.conversation_state.configure(self.manager.provider_state_paths().map_err(gateway_error)?).map_err(gateway_error)?;
                self.invalidate_recent(&instance_id)?;
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
                self.events.publish(gateway::ProtocolEvent::ProviderChanged {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ProviderChangedEvent {
                            provider: gateway_provider(&snapshot, runtime).summary,
                        },
                    },
                })?;
            }
        }
        Ok(())
    }

    fn map_provider_event(&self, event: provider::ProtocolEvent) -> gateway::ProtocolEvent {
        match event {
            provider::ProtocolEvent::EventConversationActiveChanged { .. }
            | provider::ProtocolEvent::EventConversationUnreadChanged { .. }
            | provider::ProtocolEvent::EventConversationDeleted { .. } => unreachable!("recent facts become broad invalidations"),
            provider::ProtocolEvent::EventNotification { .. } | provider::ProtocolEvent::RuntimeInventoryChanged { .. } => unreachable!("subscription notifications are delivered only to their subscriber"),
            provider::ProtocolEvent::EventInstanceStatusChanged { .. } => {
                unreachable!("instance status events are converted to one Host state update")
            }
            provider::ProtocolEvent::EventProjectChanged { params, .. } => {
                gateway::ProtocolEvent::ProjectChanged {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ProjectChangedEvent {
                            project: gateway_resource(params.project),
                            change_type: params.change_type,
                        },
                    },
                }
            }
            provider::ProtocolEvent::EventConversationUpserted { params, .. } => {
                gateway::ProtocolEvent::ConversationUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ConversationUpsertedEvent {
                            conversation: sanitize_provider_conversation(params.conversation),
                        },
                    },
                }
            }
            provider::ProtocolEvent::EventConversationItemUpserted { params, .. } => {
                gateway::ProtocolEvent::ConversationItemUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ConversationItemUpsertedEvent {
                            conversation: params.conversation,
                            update_id: params.update_id,
                            item: params.item,
                        },
                    },
                }
            }
            provider::ProtocolEvent::EventTurnUpserted { params, .. } => {
                gateway::ProtocolEvent::TurnUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::TurnUpsertedEvent {
                            turn: params.turn,
                        },
                    },
                }
            }
            provider::ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                gateway::ProtocolEvent::TurnOutputDelta {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::TurnOutputDeltaEvent {
                            turn: gateway_resource(params.turn),
                            conversation: gateway_resource(params.conversation),
                            item_id: params.item_id,
                            content_id: params.content_id,
                            kind: params.kind,
                            delta: params.delta,
                        },
                    },
                }
            }
            provider::ProtocolEvent::EventApprovalRequested { params, .. } => {
                gateway::ProtocolEvent::ApprovalRequested {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ApprovalRequestedEvent {
                            approval: params.approval,
                        },
                    },
                }
            }
            provider::ProtocolEvent::EventApprovalResolved { params, .. } => {
                gateway::ProtocolEvent::ApprovalResolved {
                    jsonrpc: "2.0".to_string(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ApprovalResolvedEvent {
                            approval: params.approval,
                        },
                    },
                }
            }
        }
    }

    async fn gateway_providers(
        &self,
        device_id: Option<&str>,
    ) -> Result<Vec<GatewayProviderRuntime>, gateway::ProtocolError> {
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
                let mut runtime = gateway_provider(&snapshot, instance);
                if self.conversation_state.path().is_none() {
                    runtime.capabilities.methods.retain(|method| *method != gateway::GatewayCapability::ConversationRecent);
                }
                providers.push(runtime);
            }
        }
        providers.sort_by(|left, right| {
            left.summary.id.cmp(&right.summary.id)
        });
        Ok(providers)
    }

    async fn perform_turn_send(
        &self,
        request: gateway::TurnSendRequest,
    ) -> Result<gateway::TurnSendResponse, gateway::ProtocolError> {
        validate_gateway_resource(&request.conversation)?;
        let route = self.resolve_provider_route(&request.conversation.provider_id).await?;
        let expected_conversation = self.resolve_resource(request.conversation.clone()).await?;
        if request.input.text.trim().is_empty() {
            return Err(gateway::ProtocolError {
                code: "invalid_turn_input".to_string(),
                message: "turn.send text input must not be empty".to_string(),
                retryable: false,
                details: None,
            });
        }
        let provider_instance = self
            .gateway_providers(Some(&route.device_id))
            .await?
            .into_iter()
            .find(|provider| provider.route == route)
            .ok_or_else(|| gateway::ProtocolError {
                code: "unknown_provider_instance".to_string(),
                message: "turn.send route does not identify a registered Provider instance"
                    .to_string(),
                retryable: false,
                details: None,
            })?;
        if provider_instance.summary.runtime.status != gateway::ProviderStatus::Ready {
            return Err(gateway::ProtocolError {
                code: "provider_instance_unavailable".to_string(),
                message: "turn.send Provider instance is not ready".to_string(),
                retryable: true,
                details: None,
            });
        }
        if !provider_instance
            .capabilities
            .methods
            .contains(&gateway::GatewayCapability::TurnSend)
        {
            return Err(gateway::ProtocolError {
                code: "provider_capability_unsupported".to_string(),
                message: "Provider instance does not advertise turn.send".to_string(),
                retryable: false,
                details: None,
            });
        }
        if provider_instance.capabilities.revision != request.capability_revision {
            return Err(gateway::ProtocolError {
                code: "stale_capability_revision".to_string(),
                message: "turn.send capabilityRevision no longer matches the Provider route"
                    .to_string(),
                retryable: true,
                details: None,
            });
        }
        validate_gateway_turn_selection(
            &provider_instance.capabilities,
            &request.selection,
        )?;
        let conversation_snapshot = self
            .manager
            .conversation_get(provider::ConversationGetRequest {
                conversation: expected_conversation.clone(),
                cursor: None,
                limit: Some(1),
            })
            .await
            .map_err(gateway_error)?;
        if conversation_snapshot.conversation.active_turn.is_some() {
            return Err(gateway::ProtocolError {
                code: "turn_already_active".to_string(),
                message: "turn.send requires an idle conversation".to_string(),
                retryable: false,
                details: None,
            });
        }
        let response = self
            .manager
            .turn_start(provider::TurnStartRequest {
                conversation: expected_conversation.clone(),
                client_request_id: request.client_request_id,
                capability_revision: request.capability_revision,
                input: request.input,
                selection: request.selection,
            })
            .await
            .map_err(gateway_error)?;
        if !response.accepted {
            return Err(gateway::ProtocolError {
                code: "provider_response_invalid".to_string(),
                message: "Provider returned a successful turn.start response that was not accepted"
                    .to_string(),
                retryable: false,
                details: None,
            });
        }
        ensure_same_resource_identity(&response.turn.conversation, &expected_conversation)?;
        if let Some(user_item) = response.user_item.as_ref() {
            let provider_user_item = match user_item {
                provider::ConversationItem::MessageConversationItem(item) => item,
                _ => {
                    return Err(gateway::ProtocolError {
                        code: "provider_response_invalid".to_string(),
                        message: "Provider turn.start userItem must be canonical when present".to_string(),
                        retryable: false,
                        details: None,
                    });
                }
            };
            ensure_same_resource_identity(&provider_user_item.conversation, &expected_conversation)?;
            ensure_same_agent_resource_identity(&provider_user_item.turn, &response.turn.resource)?;
            ensure_same_provider_route(&provider_user_item.resource, &expected_conversation)?;
            if provider_user_item.role != gateway::ConversationItemRole::User {
                return Err(gateway::ProtocolError {
                    code: "provider_response_invalid".to_string(),
                    message: "Provider turn.start userItem must be canonical when present"
                        .to_string(),
                    retryable: false,
                    details: None,
                });
            }
        }
        let effective_selection = response.effective_selection;
        validate_gateway_turn_selection(&provider_instance.capabilities, &effective_selection)?;
        Ok(gateway::TurnSendResponse {
            accepted: true,
            turn: response.turn,
            user_item: response.user_item,
            effective_selection,
        })
    }
}

impl ProtocolServer for ProviderGatewayService {
    fn conversation_recent<'a>(
        &'a self, request: gateway::ConversationRecentRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationRecentResponse> {
        Box::pin(self.conversation_recent_for_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn protocol_describe<'a>(
        &'a self,
        _request: gateway::ProtocolDescribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProtocolDescribeResponse> {
        Box::pin(async move {
            Ok(gateway::ProtocolDescribeResponse {
                features: vec![gateway::ProtocolFeature::TraceContextV1],
            })
        })
    }

    fn protocol_ping<'a>(&'a self, request: gateway::PingRequest) -> gateway::ProtocolFuture<'a, gateway::PingResponse> {
        Box::pin(async move {
            Ok(gateway::PingResponse { sequence: request.sequence,
                providers: self.gateway_providers(None).await?.into_iter().map(|p| p.summary).collect() })
        })
    }

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
                || request.client_version.trim().is_empty()
                || !device_descriptor_is_valid(&request.device)
            {
                return Err(gateway::ProtocolError {
                    code: "invalid_gateway_client".to_string(),
                    message: "Gateway client identity and device descriptor fields must not be empty"
                        .to_string(),
                    retryable: false,
                    details: None,
                });
            }
            if let Some(cursor) = request.last_event_cursor.as_deref() {
                self.events.replay(Some(cursor))?;
            }
            // Capture the replay boundary before reading Provider snapshots. Any concurrent
            // update is therefore either reflected in the snapshot or replayable after it.
            let event_cursor = self.current_event_cursor();
            Ok(gateway::HandshakeResponse {
                protocol: gateway::GatewayProtocol {
                    version: gateway::PROTOCOL_VERSION,
                },
                device: gateway::GatewayDevice {
                    name: remote_host_identity.descriptor.device_name,
                    operating_system: remote_host_identity.descriptor.operating_system,
                    system_version: remote_host_identity.descriptor.system_version,
                },
                providers: self
                    .gateway_providers(None)
                    .await?
                    .into_iter()
                    .map(|provider| provider.summary)
                    .collect(),
                event_cursor,
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

    fn provider_list<'a>(
        &'a self,
        _request: gateway::ProviderListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProviderListResponse> {
        Box::pin(async move {
            Ok(gateway::ProviderListResponse {
                providers: self
                    .gateway_providers(None)
                    .await?
                    .into_iter()
                    .map(|provider| provider.summary)
                    .collect(),
            })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        request: gateway::ProviderDescribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProviderDescribeResponse> {
        Box::pin(async move {
            let provider = self
                .gateway_providers(None)
                .await?
                .into_iter()
                .find(|provider| provider.summary.id == request.provider_id)
                .ok_or_else(|| gateway::ProtocolError {
                    code: "unknown_provider".to_string(),
                    message: "provider.describe id does not identify a configured Provider".to_string(),
                    retryable: false,
                    details: None,
                })?;
            Ok(gateway::ProviderDescribeResponse {
                provider: provider.summary,
                capabilities: provider.capabilities,
            })
        })
    }

    fn project_list<'a>(
        &'a self,
        request: gateway::ProjectListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectListResponse> {
        Box::pin(async move {
            let snapshot_cursor = self.current_event_cursor();
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let response = self
                .manager
                .project_list(provider::ProjectListRequest {
                    route,
                    cursor: request.cursor,
                    limit: request.limit,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectListResponse {
                projects: response.projects,
                page_info: gateway::PageInfo {
                    next_cursor: response.page_info.next_cursor,
                },
                snapshot_cursor,
            })
        })
    }

    fn project_get<'a>(
        &'a self,
        request: gateway::ProjectGetRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectGetResponse> {
        Box::pin(async move {
            let project = self.resolve_resource(request.project).await?;
            let response = self
                .manager
                .project_get(provider::ProjectGetRequest {
                    project,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectGetResponse {
                project: response.project,
            })
        })
    }

    fn project_create<'a>(
        &'a self,
        request: gateway::ProjectCreateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectCreateResponse> {
        Box::pin(async move {
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let response = self
                .manager
                .project_create(provider::ProjectCreateRequest {
                    route,
                    idempotency_key: request.idempotency_key,
                    name: request.name,
                    roots: request
                        .roots
                        .into_iter()
                        .collect(),
                    metadata: request.metadata,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectCreateResponse {
                project: response.project,
            })
        })
    }

    fn project_update<'a>(
        &'a self,
        request: gateway::ProjectUpdateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectUpdateResponse> {
        Box::pin(async move {
            let project = self.resolve_resource(request.project).await?;
            let response = self
                .manager
                .project_update(provider::ProjectUpdateRequest {
                    project,
                    name: request.name,
                    roots: request.roots.map(|roots| {
                        roots.into_iter().collect()
                    }),
                    metadata: request.metadata,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectUpdateResponse {
                project: response.project,
            })
        })
    }

    fn project_delete<'a>(
        &'a self,
        request: gateway::ProjectDeleteRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectDeleteResponse> {
        Box::pin(async move {
            let project = self.resolve_resource(request.project).await?;
            self.manager
                .project_delete(provider::ProjectDeleteRequest {
                    project,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectDeleteResponse {})
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: gateway::ConversationListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationListResponse> {
        Box::pin(async move {
            let project_provider_id = match &request.project_filter {
                gateway::ConversationProjectFilter::ConversationProjectFilterProject(filter) => {
                    validate_gateway_resource(&filter.project)?;
                    Some(filter.project.provider_id.as_str())
                }
                _ => None,
            };
            if project_provider_id.is_some_and(|id| id != request.provider_id) {
                return Err(gateway::ProtocolError {
                    code: "mismatched_provider_route".to_string(),
                    message: "conversation project filter must target the requested Provider"
                        .to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let project_filter = self.resolve_project_filter(request.project_filter).await?;
            let snapshot_cursor = self.current_event_cursor();
            let response = self
                .manager
                .conversation_list(provider::ConversationListRequest {
                    route,
                    cursor: request.cursor,
                    limit: request.limit,
                    project_filter,
                    query: None,
                    reader_scope: None,
                })
                .await
                .map_err(gateway_error)?;
            let mut conversations = Vec::with_capacity(response.conversations.len());
            for conversation in response.conversations {
                let mut conversation = sanitize_provider_conversation(conversation);
                self.observe_conversation_summary(&conversation)?;
                self.conversation_state
                    .decorate(DEFAULT_TURN_SEND_CALLER_SCOPE, &mut conversation)
                    .map_err(gateway_error)?;
                conversations.push(conversation);
            }
            Ok(gateway::ConversationListResponse {
                conversations,
                page_info: gateway::PageInfo {
                    next_cursor: response.page_info.next_cursor,
                },
                snapshot_cursor,
            })
        })
    }

    fn conversation_search<'a>(
        &'a self,
        request: gateway::ConversationSearchRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationSearchResponse> {
        Box::pin(async move {
            if request.search_term.trim().is_empty() {
                return Err(gateway::ProtocolError {
                    code: "invalid_request".to_string(),
                    message: "conversation.search requires a non-empty searchTerm".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let snapshot_cursor = self.current_event_cursor();
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let response = self
                .manager
                .conversation_search(provider::ConversationSearchRequest {
                    route,
                    search_term: request.search_term,
                    cursor: request.cursor,
                    limit: request.limit,
                })
                .await
                .map_err(gateway_error)?;
            let mut conversations = Vec::with_capacity(response.conversations.len());
            for conversation in response.conversations {
                let mut conversation = sanitize_provider_conversation(conversation);
                self.observe_conversation_summary(&conversation)?;
                self.conversation_state
                    .decorate(DEFAULT_TURN_SEND_CALLER_SCOPE, &mut conversation)
                    .map_err(gateway_error)?;
                conversations.push(conversation);
            }
            Ok(gateway::ConversationSearchResponse {
                conversations,
                page_info: gateway::PageInfo {
                    next_cursor: response.page_info.next_cursor,
                },
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
            let conversation = self.resolve_resource(request.conversation).await?;
            let response = self
                .manager
                .conversation_get(provider::ConversationGetRequest {
                    conversation,
                    cursor: request.cursor,
                    limit: request.limit,
                })
                .await
                .map_err(gateway_error)?;
            let mut conversation = sanitize_provider_conversation(response.conversation);
            let items = response.items;
            self.observe_conversation_summary(&conversation)?;
            if self.conversation_state
                .observe_detail(&conversation.resource, &items)
                .map_err(gateway_error)?.is_some() {
                self.invalidate_recent(&conversation.resource.provider_id)?;
            }
            self.conversation_state
                .decorate(DEFAULT_TURN_SEND_CALLER_SCOPE, &mut conversation)
                .map_err(gateway_error)?;
            Ok(gateway::ConversationGetResponse {
                conversation,
                items,
                page_info: response.page_info.map(|page_info| gateway::PageInfo {
                    next_cursor: page_info.next_cursor,
                }),
                snapshot_cursor,
            })
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: gateway::ConversationAcquireInteractionRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            let conversation = self.resolve_resource(request.conversation).await?;
            let response = self
                .manager
                .conversation_acquire_interaction(
                    provider::ConversationAcquireInteractionRequest {
                        conversation,
                    },
                )
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationAcquireInteractionResponse {
                selection: response.selection,
                lease_expires_at: response.lease_expires_at,
            })
        })
    }

    fn conversation_resume<'a>(
        &'a self,
        request: gateway::ConversationResumeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationResumeResponse> {
        Box::pin(async move {
            let limit = request.limit.unwrap_or(20);
            if !(1..=100).contains(&limit) {
                return Err(gateway::ProtocolError {
                    code: "invalid_request".to_string(),
                    message: "conversation.resume limit must be between 1 and 100".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let interaction = match self
                .conversation_acquire_interaction(gateway::ConversationAcquireInteractionRequest {
                    conversation: request.conversation.clone(),
                })
                .await
            {
                Ok(interaction) => interaction,
                Err(error) => {
                    return Ok(gateway::ConversationResumeResponse {
                        interaction_acquired: false,
                        interaction: None,
                        interaction_error: Some(error),
                        history: None,
                        history_error: None,
                    });
                }
            };
            let history = self
                .conversation_get(gateway::ConversationGetRequest {
                    conversation: request.conversation,
                    cursor: None,
                    limit: Some(limit),
                })
                .await;
            let (history, history_error) = match history {
                Ok(history) => (Some(history), None),
                Err(error) => (None, Some(error)),
            };
            Ok(gateway::ConversationResumeResponse {
                interaction_acquired: true,
                interaction: Some(interaction),
                interaction_error: None,
                history,
                history_error,
            })
        })
    }

    fn codepet_usage_query<'a>(&'a self, request: gateway::UsageQueryRequest) -> gateway::ProtocolFuture<'a, gateway::UsageQueryResponse> {
        Box::pin(async move {
            let route=self.resolve_provider_route(&request.provider_id).await?;
            let grouped=!request.query.aggregation.group_by.is_empty();
            let peak_requested=request.query.summaries.as_ref().is_some_and(|v|v.contains(&gateway::UsageQuerySummariesItems::PeakDaily));
            let mut response=self.manager.usage_query(provider::UsageQueryRequest { route, query:request.query }).await.map_err(gateway_error)?;
            // The generated Rust nested Option collapses explicit null on Provider decoding.
            // Restore requested nullable fields before forwarding the Gateway wire response.
            if grouped {for row in &mut response.result.rows {row.model_id.get_or_insert(None);}}
            if peak_requested {if let Some(summary)=&mut response.result.summaries {summary.peak_daily.get_or_insert(None);}}
            Ok(gateway::UsageQueryResponse { result:response.result })
        })
    }

    fn conversation_mark_read<'a>(
        &'a self,
        request: gateway::ConversationMarkReadRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationMarkReadResponse> {
        Box::pin(self.mark_read_for_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }

    fn conversation_create<'a>(
        &'a self,
        request: gateway::ConversationCreateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationCreateResponse> {
        Box::pin(async move {
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let project = match request.project {
                Some(project) => Some(self.resolve_resource(project).await?),
                None => None,
            };
            let response = self
                .manager
                .conversation_create(provider::ConversationCreateRequest {
                    route,
                    project,
                    title: request.title,
                    permission_level: request.permission_level,
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                    workspace_root: request.workspace_root,
                    workspace_mode: request.workspace_mode,
                    extension: None,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationCreateResponse {
                conversation: sanitize_provider_conversation(response.conversation),
            })
        })
    }

    fn turn_send<'a>(
        &'a self,
        request: gateway::TurnSendRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnSendResponse> {
        Box::pin(async move {
            self.turn_send_for_caller_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request)
                .await
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: gateway::TurnInterruptRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnInterruptResponse> {
        Box::pin(async move {
            ensure_same_gateway_route(&request.conversation, &request.turn)?;
            let conversation = self.resolve_resource(request.conversation).await?;
            let turn = self.resolve_resource(request.turn).await?;
            let response = self
                .manager
                .turn_interrupt(provider::TurnInterruptRequest {
                    conversation,
                    turn,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::TurnInterruptResponse {
                turn: response.turn,
            })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: gateway::ApprovalResolveRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ApprovalResolveResponse> {
        Box::pin(async move {
            let approval = self.resolve_resource(request.approval).await?;
            let response = self
                .manager
                .approval_resolve(provider::ApprovalResolveRequest {
                    approval,
                    decision: request.decision,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ApprovalResolveResponse {
                approval: response.approval,
            })
        })
    }
}

fn gateway_provider(
    plugin: &PluginRuntimeSnapshot,
    runtime: &ProviderInstanceRuntimeSnapshot,
) -> GatewayProviderRuntime {
    let capabilities = runtime
        .instance
        .as_ref()
        .map(|instance| map_capabilities(&instance.capabilities))
        .unwrap_or_else(|| {
            empty_gateway_capabilities(format!("provider-unavailable-{}", plugin.generation))
        });
    let harness_version = runtime
        .instance
        .as_ref()
        .and_then(|instance| instance.harness.version.clone());
    let executable_path = runtime
        .instance
        .as_ref()
        .and_then(|instance| instance.harness.executable_path.clone())
        .or_else(|| configured_executable_path(&runtime.record.settings));
    GatewayProviderRuntime {
        provider_mark_read: runtime.instance.as_ref().is_some_and(|instance|
            instance.capabilities.methods.contains(&provider::ProviderCapability::ConversationMarkRead)),
        route: provider::ProviderInstanceRoute {
            device_id: runtime.record.device_id.clone(),
            provider_plugin_id: plugin.catalog.plugin_id.clone(),
            provider_instance_id: runtime.record.instance_id.clone(),
        },
        summary: gateway::ProviderSummary {
            id: runtime.record.instance_id.clone(),
            identity: gateway::ProviderIdentity {
                display_name: runtime.record.display_name.clone(),
                icon: plugin.catalog.icon.clone(),
                default_workspace_root: plugin
                    .reported
                    .as_ref()
                    .and_then(|descriptor| descriptor.default_workspace_root.clone()),
            },
            runtime: gateway::ProviderRuntime {
                connection_status: Some(plugin.connection_status),
                generation: Some(plugin.generation),
                status: if plugin.catalog.enabled && runtime.record.enabled {
                    provider_runtime_status(plugin.state, runtime.instance.as_ref())
                } else {
                    gateway::ProviderStatus::Unavailable
                },
                version: harness_version,
                executable_path,
                authentication: runtime
                    .instance
                    .as_ref()
                    .and_then(|instance| instance.authentication.as_ref())
                    .cloned(),
            },
            capabilities: gateway::ProviderCapabilitiesSummary {
                revision: capabilities.revision.clone(),
            },
        },
        capabilities,
    }
}

fn configured_executable_path(settings: &provider::JsonObject) -> Option<String> {
    ["appServerExecutable", "claudeExecutable", "serverExecutable"]
        .into_iter()
        .find_map(|key| settings.get(key).and_then(|value| value.as_str()))
        .filter(|path| !path.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn provider_runtime_status(
    state: PluginRuntimeState,
    instance: Option<&provider::ProviderInstance>,
) -> gateway::ProviderStatus {
    match state {
        PluginRuntimeState::Stopped => gateway::ProviderStatus::Stopped,
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
        provider::InstanceStatus::Stopped => gateway::ProviderStatus::Stopped,
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
            provider::ProviderCapability::ProjectList => {
                Some(gateway::GatewayCapability::ProjectList)
            }
            provider::ProviderCapability::ProjectGet => {
                Some(gateway::GatewayCapability::ProjectGet)
            }
            provider::ProviderCapability::ProjectCreate => {
                Some(gateway::GatewayCapability::ProjectCreate)
            }
            provider::ProviderCapability::ProjectUpdate => {
                Some(gateway::GatewayCapability::ProjectUpdate)
            }
            provider::ProviderCapability::ProjectDelete => {
                Some(gateway::GatewayCapability::ProjectDelete)
            }
            provider::ProviderCapability::ConversationList => {
                Some(gateway::GatewayCapability::ConversationList)
            }
            provider::ProviderCapability::ConversationSearch => {
                Some(gateway::GatewayCapability::ConversationSearch)
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
            provider::ProviderCapability::UsageQuery => Some(gateway::GatewayCapability::CodepetUsageQuery),
            provider::ProviderCapability::TurnSteer => None,
            provider::ProviderCapability::ConversationActiveList
            | provider::ProviderCapability::ConversationUnreadList
            | provider::ProviderCapability::ConversationMarkRead => None,
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
    if capabilities.conversation_list_query.as_ref().is_some_and(|query| query.updated_after && query.ids)
        && [provider::ProviderCapability::ConversationList, provider::ProviderCapability::ConversationActiveList,
            provider::ProviderCapability::ConversationUnreadList, provider::ProviderCapability::ConversationMarkRead]
            .iter().all(|required| capabilities.methods.contains(required)) {
        methods.push(gateway::GatewayCapability::ConversationRecent);
    }
    gateway::GatewayCapabilities { usage_datasets: capabilities.usage_datasets.clone(),
        revision: capabilities.revision.clone(),
        methods,
        turn_send: capabilities.turn_send.clone(),
        conversation_create: capabilities.conversation_create.clone(),
    }
}

fn empty_gateway_capabilities(revision: String) -> gateway::GatewayCapabilities {
    gateway::GatewayCapabilities { usage_datasets: None,
        revision,
        methods: Vec::new(),
        turn_send: None,
        conversation_create: None,
    }
}

fn sanitize_provider_conversation(mut conversation: gateway::Conversation) -> gateway::Conversation {
    conversation.read_state = None;
    conversation
}

fn gateway_resource(resource: provider::ProviderResourceId) -> gateway::RoutedResourceId {
    gateway::RoutedResourceId {
        provider_id: resource.provider_instance_id,
        native_resource_id: resource.native_resource_id,
    }
}

fn turn_send_key(
    caller_scope: &str,
    request: &gateway::TurnSendRequest,
) -> Result<TurnSendKey, gateway::ProtocolError> {
    if caller_scope.trim().is_empty() {
        return Err(gateway::ProtocolError {
            code: "invalid_caller_scope".to_string(),
            message: "turn.send internal caller scope must not be empty".to_string(),
            retryable: false,
            details: None,
        });
    }
    validate_gateway_resource(&request.conversation)?;
    if request.client_request_id.trim().is_empty() {
        return Err(gateway::ProtocolError {
            code: "invalid_client_request_id".to_string(),
            message: "turn.send clientRequestId must not be empty".to_string(),
            retryable: false,
            details: None,
        });
    }
    Ok(TurnSendKey {
        caller_scope: caller_scope.to_string(),
        provider_id: request.conversation.provider_id.clone(),
        client_request_id: request.client_request_id.clone(),
    })
}

fn json_rpc_response<T: serde::Serialize>(
    jsonrpc: String,
    id: gateway::RequestId,
    result: Result<T, gateway::ProtocolError>,
) -> gateway::JsonRpcResponse {
    let response = match result {
        Ok(result) => match serde_json::to_value(result) {
            Ok(result) => gateway::JsonRpcResponsePayload::Ok { result },
            Err(error) => gateway::JsonRpcResponsePayload::Error {
                error: gateway::RpcError {
                    code: gateway::JSON_RPC_INTERNAL_ERROR,
                    message: format!("encode Gateway response: {error}"),
                    data: None,
                },
            },
        },
        Err(error) => {
            let message = error.message.clone();
            let data = serde_json::to_value(error).ok().and_then(|value| {
                value.as_object().cloned().map(|entries| entries.into_iter().collect())
            });
            gateway::JsonRpcResponsePayload::Error {
                error: gateway::RpcError {
                    code: -32000,
                    message,
                    data,
                },
            }
        }
    };
    gateway::JsonRpcResponse {
        jsonrpc,
        id: Some(id),
        response,
    }
}

fn validate_gateway_turn_selection(
    capabilities: &gateway::GatewayCapabilities,
    selection: &gateway::TurnSelection,
) -> Result<(), gateway::ProtocolError> {
    let turn_send = capabilities.turn_send.as_ref();
    validate_choice_selection(
        "access mode",
        selection.access_mode_id.as_deref(),
        turn_send.and_then(|capabilities| capabilities.access_mode.as_ref()),
    )?;
    validate_choice_selection(
        "reasoning effort",
        selection.reasoning_effort_id.as_deref(),
        turn_send.and_then(|capabilities| capabilities.reasoning_effort.as_ref()),
    )?;
    let catalog = turn_send.and_then(|capabilities| capabilities.model_catalog.as_ref());
    match (&selection.model, catalog) {
        (None, _) => Ok(()),
        (Some(_), None) => Err(gateway::ProtocolError {
            code: "unsupported_turn_control".to_string(),
            message: "Provider does not advertise a model selector for turn.send".to_string(),
            retryable: false,
            details: None,
        }),
        (
            Some(gateway::ModelSelection::FlatModelSelection(selection)),
            Some(gateway::ModelCatalog::FlatModelCatalog(catalog)),
        ) => validate_choice_option("model", &selection.model_id, &catalog.models),
        (
            Some(gateway::ModelSelection::GroupedModelSelection(selection)),
            Some(gateway::ModelCatalog::GroupedModelCatalog(catalog)),
        ) => {
            let group = catalog
                .providers
                .iter()
                .find(|group| group.id == selection.provider_id)
                .ok_or_else(|| gateway::ProtocolError {
                    code: "unknown_turn_selection".to_string(),
                    message: format!(
                        "Provider does not advertise model provider {}",
                        selection.provider_id
                    ),
                    retryable: false,
                    details: None,
                })?;
            validate_choice_option("model", &selection.model_id, &group.models)
        }
        _ => Err(gateway::ProtocolError {
            code: "turn_model_shape_mismatch".to_string(),
            message: "turn.send model selection kind does not match the advertised model catalog"
                .to_string(),
            retryable: false,
            details: None,
        }),
    }
}

fn validate_choice_selection(
    label: &str,
    selected_id: Option<&str>,
    choices: Option<&gateway::ChoiceSet>,
) -> Result<(), gateway::ProtocolError> {
    let Some(selected_id) = selected_id else {
        return Ok(());
    };
    let Some(choices) = choices else {
        return Err(gateway::ProtocolError {
            code: "unsupported_turn_control".to_string(),
            message: format!("Provider does not advertise a {label} selector for turn.send"),
            retryable: false,
            details: None,
        });
    };
    validate_choice_option(label, selected_id, &choices.options)
}

fn validate_choice_option(
    label: &str,
    selected_id: &str,
    options: &[gateway::ChoiceOption],
) -> Result<(), gateway::ProtocolError> {
    let option = options
        .iter()
        .find(|option| option.id == selected_id)
        .ok_or_else(|| gateway::ProtocolError {
            code: "unknown_turn_selection".to_string(),
            message: format!("Provider does not advertise {label} {selected_id}"),
            retryable: false,
            details: None,
        })?;
    if option.enabled == Some(false) {
        return Err(gateway::ProtocolError {
            code: "disabled_turn_selection".to_string(),
            message: option
                .disabled_reason
                .clone()
                .unwrap_or_else(|| format!("Provider disabled {label} {selected_id}")),
            retryable: false,
            details: None,
        });
    }
    Ok(())
}

fn ensure_same_gateway_route(
    left: &gateway::RoutedResourceId,
    right: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    validate_gateway_resource(left)?;
    validate_gateway_resource(right)?;
    if left.provider_id == right.provider_id {
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
    expected: &provider::ProviderResourceId,
) -> Result<(), gateway::ProtocolError> {
    if actual.provider_id == expected.provider_instance_id
        && actual.native_resource_id == expected.native_resource_id
    {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "provider_resource_identity_mismatch".to_string(),
        message: "Provider returned a turn for a different conversation".to_string(),
        retryable: false,
        details: None,
    })
}

fn ensure_same_agent_resource_identity(
    actual: &gateway::RoutedResourceId,
    expected: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    if actual == expected {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "provider_resource_identity_mismatch".to_string(),
        message: "Provider returned a resource with a different identity".to_string(),
        retryable: false,
        details: None,
    })
}

fn ensure_same_provider_route(
    left: &gateway::RoutedResourceId,
    right: &provider::ProviderResourceId,
) -> Result<(), gateway::ProtocolError> {
    if left.provider_id == right.provider_instance_id {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "gateway_route_mismatch".to_string(),
        message: "Provider returned resources owned by different routes".to_string(),
        retryable: false,
        details: None,
    })
}

fn validate_remote_host_identity(identity: &RemoteHostIdentity) -> HostResult<()> {
    if identity.device_id.trim().is_empty() || !device_descriptor_is_valid(&identity.descriptor) {
        return Err(HostError::new(
            "invalid_remote_host_identity",
            "Gateway Host identity requires a device ID and complete descriptor",
        ));
    }
    Ok(())
}

fn device_descriptor_is_valid(descriptor: &gateway::DeviceDescriptor) -> bool {
    !descriptor.device_name.trim().is_empty()
        && !descriptor.operating_system.trim().is_empty()
        && !descriptor.system_version.trim().is_empty()
}

fn validate_gateway_resource(
    resource: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    if resource.provider_id.trim().is_empty() || resource.native_resource_id.trim().is_empty()
    {
        return Err(gateway::ProtocolError {
            code: "invalid_gateway_resource".to_string(),
            message: "providerId and nativeResourceId must not be empty".to_string(),
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
    event_cursor_sequence(event.event_cursor())
}

fn set_event_cursor(event: &mut gateway::ProtocolEvent, cursor: gateway::EventCursor) {
    event.set_event_cursor(cursor);
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
