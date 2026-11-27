use crate::conversation_state::ConversationStateStore;
use crate::providers::manager::{
    HostUpdate, PluginManager, PluginRuntimeSnapshot, PluginRuntimeState,
    ProviderInstanceRuntimeSnapshot,
};
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
const MAX_USAGE_DETAILS: usize = 8;
const MAX_USAGE_DETAIL_BYTES: usize = 16 * 1024;
const MAX_USAGE_DETAILS_BYTES: usize = 32 * 1024;

mod input_queue;
mod provider_updates;
mod provider_inventory;
mod notices;
pub use notices::NoticeActivator;
mod recent;
mod server;
mod mapping;
use mapping::*;
mod terminal;

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
    provider_ordered_ids: bool,
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
    events: VecDeque<Arc<gateway::ProtocolEvent>>,
}

struct GatewayEventBus {
    capacity: usize,
    state: Mutex<GatewayEventState>,
    sender: broadcast::Sender<Arc<gateway::ProtocolEvent>>,
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
        let (assigned, evicted) = {
            let mut state = self.state.lock().map_err(|_| gateway_state_error())?;
            let sequence = state
                .sequence
                .checked_add(1)
                .ok_or_else(|| gateway::ProtocolError {
                    code: "gateway_event_cursor_exhausted".to_string(),
                    message: "Gateway event cursor is exhausted".to_string(),
                    retryable: false,
                    details: None,
                })?;
            set_event_cursor(&mut event, event_cursor(sequence));
            state.sequence = sequence;
            let assigned = Arc::new(event);
            state.events.push_back(assigned.clone());
            let evicted = if state.events.len() > self.capacity {
                state.events.pop_front()
            } else {
                None
            };
            // Cursor assignment, replay insertion and broadcast order are one short commit.
            let _ = self.sender.send(assigned.clone());
            (assigned, evicted)
        };
        drop(evicted); // Heavy payload destruction is outside the shared cursor lock.
        Ok((*assigned).clone())
    }

    fn replay(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<Vec<gateway::ProtocolEvent>, gateway::ProtocolError> {
        Ok(self
            .replay_shared(after_cursor)?
            .into_iter()
            .map(|event| (*event).clone())
            .collect())
    }
    fn replay_shared(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<Vec<Arc<gateway::ProtocolEvent>>, gateway::ProtocolError> {
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
            if let Some(oldest) = state
                .events
                .front()
                .map(|event| protocol_event_sequence(event))
                .transpose()?
            {
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
        let replay = self.replay_shared(after_cursor)?;
        Ok(GatewayEventSubscription {
            replay: replay.into(),
            local: false,
            receiver,
            last_sequence: after_cursor
                .map(event_cursor_sequence)
                .transpose()?
                .unwrap_or(0),
        })
    }
}

pub struct GatewayEventSubscription {
    local: bool,
    replay: VecDeque<Arc<gateway::ProtocolEvent>>,
    receiver: broadcast::Receiver<Arc<gateway::ProtocolEvent>>,
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
                            message: format!("Gateway event subscriber skipped {skipped} messages"),
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
                if !self.local && matches!(event.as_ref(),gateway::ProtocolEvent::NoticeChanged{..}) {continue;}
                return Ok((*event).clone());
            }
        }
    }
}

pub struct ProviderGatewayService {
    terminals: crate::terminal::TerminalManager,
    notice_activator: Mutex<Option<Arc<dyn NoticeActivator>>>,
    manager: Arc<PluginManager>,
    events: Arc<GatewayEventBus>,
    provider_inventory: AsyncMutex<provider_inventory::Inventory>,
    updates: Mutex<Option<mpsc::Receiver<HostUpdate>>>,
    forwarding_started: AtomicBool,
    server_name: String,
    server_version: String,
    remote_host_identity: Option<RemoteHostIdentity>,
    turn_sends: AsyncMutex<TurnSendCache>,
    input_queue: Option<crate::input_queue::InputQueueStore>,
    queue_locks: Mutex<BTreeMap<(String, String), std::sync::Weak<AsyncMutex<()>>>>,
    input_request_locks:
        Mutex<BTreeMap<(String, String, String, String), std::sync::Weak<AsyncMutex<()>>>>,
    queue_wake: Arc<tokio::sync::Notify>,
    turn_steers: AsyncMutex<
        BTreeMap<
            (String, String, String),
            (
                gateway::TurnSteerRequest,
                Result<gateway::TurnSteerResponse, gateway::ProtocolError>,
            ),
        >,
    >,
    conversation_state: Arc<ConversationStateStore>,
    recent: Arc<Mutex<RecentSnapshots>>,
    recent_concurrency: tokio::sync::Semaphore,
    recent_builds:
        AsyncMutex<BTreeMap<crate::recent_conversations::ViewKey, std::sync::Weak<AsyncMutex<()>>>>,
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
        let input_queue = conversation_state.path().map(crate::input_queue::InputQueueStore::open).transpose()?;
        let updates = manager.take_updates()?;
        Ok(Self {
            terminals: crate::terminal::TerminalManager::new(),
            notice_activator: Mutex::new(None),
            manager,
            events: Arc::new(GatewayEventBus::new(event_capacity)),
            provider_inventory: AsyncMutex::new(provider_inventory::Inventory::default()),
            updates: Mutex::new(Some(updates)),
            forwarding_started: AtomicBool::new(false),
            server_name: "codepet-provider-gateway".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            remote_host_identity,
            turn_sends: AsyncMutex::new(TurnSendCache::new()),
            input_queue, queue_locks:Mutex::new(BTreeMap::new()), input_request_locks:Mutex::new(BTreeMap::new()), queue_wake:Arc::new(tokio::sync::Notify::new()), turn_steers:AsyncMutex::new(BTreeMap::new()),
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
            gateway::ConversationProjectFilter::Unrecognized(_) => return Err(gateway::ProtocolError { code: "unsupported_filter".into(), message: "Unknown project filter".into(), retryable: false, details: None }),
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
        self.events.replay(after_cursor).map(|events|events.into_iter().filter(|event|!matches!(event,gateway::ProtocolEvent::NoticeChanged{..})).collect())
    }

    pub fn subscribe_events(
        &self,
        after_cursor: Option<&str>,
    ) -> Result<GatewayEventSubscription, gateway::ProtocolError> {
        self.events.subscribe(after_cursor)
    }
    pub fn subscribe_local_events(&self,after_cursor:Option<&str>)->Result<GatewayEventSubscription,gateway::ProtocolError>{let mut stream=self.events.subscribe(after_cursor)?;stream.local=true;Ok(stream)}


    /// Dispatch from the trusted in-process desktop channel. Presence is scoped
    /// to the request (including cancellation), and shared with Provider pings.
    /// Remote channels must authenticate and register their own connection.
    pub async fn dispatch_for_local_client(
        &self,
        client_id: &str,
        request: gateway::ProtocolRequest,
    ) -> gateway::JsonRpcResponse {
        if let Some(response)=self.local_notice_request(&request).await {return response;}
        let _connection = self.manager.remote_connections().register(client_id.to_owned());
        self.dispatch_for_caller_scope(client_id, request).await
    }

    pub async fn dispatch_for_caller_scope(
        &self,
        caller_scope: &str,
        request: gateway::ProtocolRequest,
    ) -> gateway::JsonRpcResponse {
        let request = match self.terminal_request(caller_scope, request).await {
            Ok(response) => return response,
            Err(request) => request,
        };
        match request {
            gateway::ProtocolRequest::NoticeList {id,..}|gateway::ProtocolRequest::NoticeGet{id,..}|gateway::ProtocolRequest::NoticeActionExecute{id,..}|gateway::ProtocolRequest::NoticeSourceSetEnabled{id,..} => json_rpc_response("2.0".into(),id,Err::<serde_json::Value,_>(gateway::ProtocolError{code:"local_only".into(),message:"Notice methods require the trusted local channel".into(),retryable:false,details:None})),
            gateway::ProtocolRequest::TurnSteer { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.turn_steer_for_caller_scope(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationInputQueueGet { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.input_queue_get(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationInputQueueEnqueue { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.input_queue_enqueue(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationInputQueueWithdraw { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.input_queue_withdraw(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationInputQueueSteer { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.input_queue_steer(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationInputQueueResume { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.input_queue_resume(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationInputQueueAcknowledgeEdit { jsonrpc,id,params } => json_rpc_response(jsonrpc,id,self.input_queue_acknowledge_edit(caller_scope,params).await),
            gateway::ProtocolRequest::ConversationRecent { jsonrpc, id, params } => {
                // Recent can concurrently enumerate two Provider streams. Keep
                // those futures off the stack of every other RPC dispatcher.
                let result = Box::pin(self.conversation_recent_for_scope(caller_scope, params)).await;
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
        let request_lock = self.input_request_lock(caller_scope, "turn.send", &request.conversation.provider_id, &request.client_request_id)?;
        let _request_guard = request_lock.lock().await;
        let lock=self.input_lock(&request.conversation)?;
        let _guard=lock.lock().await;
        let cache = self.turn_sends.lock().await;
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
        drop(cache);
        if self.input_queue.as_ref().is_some_and(|store|store.has_pending(&request.conversation).unwrap_or(true)) {
            return Err(crate::input_queue::error("input_queue_pending","Submit this input through the conversation queue while it has pending messages"));
        }
        let result = self.perform_turn_send(request.clone()).await;
        self.turn_sends.lock().await.insert(
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
        self.start_notice_forwarding();
        let mut updates = match self.updates.lock() {
            Ok(mut updates) => match updates.take() {
                Some(updates) => updates,
                None => return false,
            },
            Err(_) => return false,
        };
        self.start_input_queue_worker();
        self.start_provider_inventory();
        let service = Arc::downgrade(self);
        let worker_service = service.clone();
        tokio::spawn(async move {
            let mut workers = provider_updates::Workers::new(64, move |update| {
                if let Some(service) = worker_service.upgrade() {
                    if let Err(error) = service.forward_host_update(update) {
                        eprintln!("Provider Gateway failed to process update: {error:?}");
                    }
                }
            });
            let mut failed = std::collections::BTreeSet::new();
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                let update = tokio::select! {
                    update = updates.recv() => match update { Some(update) => Some(update), None => return },
                    _ = tick.tick() => None,
                };
                let Some(service) = service.upgrade() else {
                    return;
                };
                match update {
                    Some(update) => {
                        let (plugin_id, generation) = match &update {
                            HostUpdate::ProviderEvent {
                                plugin_id,
                                generation,
                                ..
                            } => (plugin_id.clone(), *generation),
                            HostUpdate::PluginStateChanged { snapshot, .. }
                            | HostUpdate::InstanceChanged { snapshot, .. } => {
                                (snapshot.catalog.plugin_id.clone(), snapshot.generation)
                            }
                        };
                        if failed.contains(&(plugin_id.clone(), generation)) {
                            continue;
                        }
                        if workers.dispatch(&plugin_id, update).is_err() {
                            failed.insert((plugin_id.clone(), generation));
                            let manager = service.manager.clone();
                            tokio::spawn(async move {
                                manager.fail_event_delivery(&plugin_id, generation).await;
                            });
                        }
                    }
                    None => {
                        if let Err(error) = service.publish_recent_invalidations() {
                            eprintln!("Provider Gateway invalidation failed: {error:?}");
                        }
                    }
                }
            }
        });
        true
    }

    fn forward_host_update(&self, update: HostUpdate) -> Result<(), gateway::ProtocolError> {
        match update {
            HostUpdate::ProviderEvent {
                event,
                permit: _permit,
                ..
            } => {
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
                        if let Some(store) = &self.input_queue {
                            store.observe_turn(&params.turn)?;
                            self.publish_queue_changed(&params.turn.conversation)?;
                            self.queue_wake.notify_one();
                        }
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
                    self.events
                        .publish(gateway::ProtocolEvent::ConversationActivityChanged {
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
                if let Some(store) = &self.input_queue {
                    for instance in &snapshot.instances {
                        let provider = gateway_provider(&snapshot, instance).summary;
                        if provider.runtime.status != gateway::ProviderStatus::Ready {
                            for route in store.pause_provider(&provider.id)? {
                                self.publish_queue_changed(&route)?;
                            }
                        }
                    }
                }
                if !gateway_plugin_visible(&snapshot) {
                    return Ok(());
                }
                for instance in &snapshot.instances {
                    let provider = gateway_provider(&snapshot, instance).summary;
                    self.invalidate_recent(&provider.id)?;

                }
            }
            HostUpdate::InstanceChanged {
                snapshot,
                instance_id,
                previous_status,
            } => {
                let _ = previous_status;
                if let Some(store) = &self.input_queue {
                    for instance in &snapshot.instances {
                        let provider = gateway_provider(&snapshot, instance).summary;
                        if provider.runtime.status != gateway::ProviderStatus::Ready {
                            for route in store.pause_provider(&provider.id)? {
                                self.publish_queue_changed(&route)?;
                            }
                        }
                    }
                }
                if !gateway_plugin_visible(&snapshot) {
                    return Ok(());
                }
                self.conversation_state
                    .configure(self.manager.provider_state_paths().map_err(gateway_error)?)
                    .map_err(gateway_error)?;
                self.invalidate_recent(&instance_id)?;

            }
        }
        Ok(())
    }

    fn map_provider_event(&self, event: provider::ProtocolEvent) -> gateway::ProtocolEvent {
        match event {
            provider::ProtocolEvent::EventConversationActiveChanged { .. }
            | provider::ProtocolEvent::EventConversationUnreadChanged { .. }
            | provider::ProtocolEvent::EventConversationDeleted { .. } => unreachable!("recent facts become broad invalidations"),
            provider::ProtocolEvent::RuntimeInventoryChanged { .. } => unreachable!("subscription notifications are delivered only to their subscriber"),
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

    async fn gateway_providers(&self, device_id: Option<&str>) -> Result<Vec<GatewayProviderRuntime>, gateway::ProtocolError> {
        Ok(self.provider_inventory_snapshot(device_id).await?.2)
    }

    async fn collect_gateway_providers(
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
            if !gateway_plugin_visible(&snapshot) { continue; }
            for instance in &snapshot.instances {
                let mut runtime = gateway_provider(&snapshot, instance);
                self.add_queue_capabilities(&mut runtime.capabilities);
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


#[cfg(test)]
mod visibility_tests {
    use super::*;
    #[test]
    fn builtin_visibility_uses_inventory_not_process_or_instance_liveness() {
        let mut plugin = PluginRuntimeSnapshot {
            runtime_inventory_diagnostic: None,
            catalog: serde_json::from_value(serde_json::json!({
                "pluginId":"dev.codepet.codex", "displayName":"Codex", "command":"provider", "icon":"https://example.com/codex.png"
            })).unwrap(),
            runtime_inventory: None, reported: None, state: PluginRuntimeState::Ready,
            diagnostic: None, generation: 1, connection_status: codepet_provider_sdk::ConnectionStatus::Online,
            process_exit: None, stderr_diagnostics: vec![], instances: vec![],
        };
        for id in ["dev.codepet.codex", "dev.codepet.claude", "dev.codepet.opencode"] {
            plugin.catalog.plugin_id = id.into();
            assert!(!gateway_plugin_visible(&plugin));
            plugin.runtime_inventory = Some(serde_json::from_value(serde_json::json!({"harnessList":[]})).unwrap());
            assert!(!gateway_plugin_visible(&plugin));
            plugin.runtime_inventory = None;
        }
        plugin.runtime_inventory = Some(serde_json::from_value(serde_json::json!({"harnessList":[{
            "executablePath":"/runtime/codex", "version":"1", "source":"current-path"
        }]})).unwrap());
        plugin.state = PluginRuntimeState::Stopped;
        assert!(gateway_plugin_visible(&plugin), "installed idle runtime must remain visible");
        plugin.catalog.plugin_id = "custom.example".into();
        plugin.runtime_inventory = None;
        assert!(gateway_plugin_visible(&plugin), "custom provider need not expose built-in inventory");
    }
}
