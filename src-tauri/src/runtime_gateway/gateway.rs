use super::event_bus::{
    EventSubscription, GatewayEventBus, ProviderEventSink, DEFAULT_EVENT_WINDOW_CAPACITY,
};
use super::generated::{
    ApprovalResolveRequest, ApprovalResolveResponse, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, EventSequence, HandshakeRequest,
    HandshakeResponse, ProtocolError, ProtocolEvent, ProtocolFuture, ProtocolServer,
    Provider, ProviderListRequest, ProviderListResponse, TurnInterruptRequest,
    TurnInterruptResponse, TurnSendRequest, TurnSendResponse, PROTOCOL_VERSION,
};
use super::provider::ProviderAdapter;
use super::registry::ProviderRegistry;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Gateway {
    registry: ProviderRegistry,
    events: Arc<GatewayEventBus>,
    server_name: String,
    server_version: String,
}

impl Default for Gateway {
    fn default() -> Self {
        Self::new(ProviderRegistry::default())
    }
}

impl Gateway {
    pub fn new(registry: ProviderRegistry) -> Self {
        Self::with_event_capacity(registry, DEFAULT_EVENT_WINDOW_CAPACITY)
    }

    pub fn with_event_capacity(registry: ProviderRegistry, event_capacity: usize) -> Self {
        Self {
            registry,
            events: Arc::new(GatewayEventBus::new(event_capacity)),
            server_name: "code-pet-runtime-gateway".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn event_sink(&self) -> ProviderEventSink {
        ProviderEventSink::new(self.events.clone())
    }

    pub fn publish_event(&self, event: ProtocolEvent) -> Result<ProtocolEvent, ProtocolError> {
        self.events.publish(event)
    }

    pub fn current_event_sequence(&self) -> EventSequence {
        self.events.current_sequence()
    }

    pub fn replay_events(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        self.events.replay(after_sequence)
    }

    pub fn subscribe_events(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<EventSubscription, ProtocolError> {
        self.events.subscribe(after_sequence)
    }

    fn resolve_method(
        &self,
        provider_id: &str,
        method: &str,
    ) -> Result<Arc<dyn ProviderAdapter>, ProtocolError> {
        let adapter = self.registry.resolve(provider_id)?;
        let provider = adapter.provider();
        ensure_method_capability(&provider, method)?;
        Ok(adapter)
    }
}

impl ProtocolServer for Gateway {
    fn protocol_handshake<'a>(
        &'a self,
        request: HandshakeRequest,
    ) -> ProtocolFuture<'a, HandshakeResponse> {
        Box::pin(async move {
            if request.min_protocol_version > request.max_protocol_version {
                return Err(protocol_version_error(
                    "invalid_protocol_range",
                    "minimum protocol version exceeds maximum protocol version",
                    &request,
                ));
            }
            if PROTOCOL_VERSION < request.min_protocol_version
                || PROTOCOL_VERSION > request.max_protocol_version
            {
                return Err(protocol_version_error(
                    "unsupported_protocol_version",
                    "gateway protocol version is outside the client-supported range",
                    &request,
                ));
            }
            Ok(HandshakeResponse {
                protocol_version: PROTOCOL_VERSION,
                server_name: self.server_name.clone(),
                server_version: self.server_version.clone(),
                providers: self.registry.list()?,
                event_sequence: self.current_event_sequence(),
            })
        })
    }

    fn provider_list<'a>(
        &'a self,
        _request: ProviderListRequest,
    ) -> ProtocolFuture<'a, ProviderListResponse> {
        Box::pin(async move {
            Ok(ProviderListResponse {
                providers: self.registry.list()?,
            })
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            if let Some(provider_id) = request.provider_id.as_deref() {
                let adapter = self.resolve_method(provider_id, "conversation.list")?;
                let mut response = adapter.conversation_list(request).await?;
                response.event_sequence = self.current_event_sequence();
                return Ok(response);
            }

            let adapters = self
                .registry
                .ready_adapters()?
                .into_iter()
                .filter(|adapter| {
                    provider_advertises_method(&adapter.provider(), "conversation.list")
                })
                .collect::<Vec<_>>();
            let single_provider = adapters.len() == 1;
            let mut conversations = Vec::new();
            let mut next_cursor = None;
            for adapter in adapters {
                let mut provider_request = request.clone();
                provider_request.provider_id = Some(adapter.provider().id);
                let mut response = adapter.conversation_list(provider_request).await?;
                conversations.append(&mut response.conversations);
                if single_provider {
                    next_cursor = response.next_cursor;
                }
            }
            if let Some(limit) = request.limit.and_then(|limit| usize::try_from(limit).ok()) {
                conversations.truncate(limit);
            }
            Ok(ConversationListResponse {
                conversations,
                next_cursor,
                event_sequence: self.current_event_sequence(),
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProtocolFuture<'a, ConversationGetResponse> {
        Box::pin(async move {
            self.resolve_method(&request.provider_id, "conversation.get")?
                .conversation_get(request)
                .await
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
            self.resolve_method(&request.provider_id, "conversation.create")?
                .conversation_create(request)
                .await
        })
    }

    fn turn_send<'a>(
        &'a self,
        request: TurnSendRequest,
    ) -> ProtocolFuture<'a, TurnSendResponse> {
        Box::pin(async move {
            let adapter = self.resolve_method(&request.provider_id, "turn.send")?;
            if request.steer_turn_id.is_some() && !adapter.provider().capabilities.can_steer {
                return Err(capability_error(
                    &request.provider_id,
                    "turn.send",
                    Some("canSteer"),
                ));
            }
            adapter.turn_send(request).await
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProtocolFuture<'a, TurnInterruptResponse> {
        Box::pin(async move {
            let adapter = self.resolve_method(&request.provider_id, "turn.interrupt")?;
            if !adapter.provider().capabilities.can_interrupt {
                return Err(capability_error(
                    &request.provider_id,
                    "turn.interrupt",
                    Some("canInterrupt"),
                ));
            }
            adapter.turn_interrupt(request).await
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async move {
            self.resolve_method(&request.provider_id, "approval.resolve")?
                .approval_resolve(request)
                .await
        })
    }
}

fn ensure_method_capability(provider: &Provider, method: &str) -> Result<(), ProtocolError> {
    if provider_advertises_method(provider, method) {
        return Ok(());
    }
    Err(capability_error(&provider.id, method, None))
}

fn provider_advertises_method(provider: &Provider, method: &str) -> bool {
    provider
        .capabilities
        .methods
        .iter()
        .any(|candidate| candidate == method)
}

fn capability_error(
    provider_id: &str,
    method: &str,
    capability: Option<&str>,
) -> ProtocolError {
    let mut details = BTreeMap::from([
        (
            "providerId".to_string(),
            serde_json::Value::String(provider_id.to_string()),
        ),
        (
            "method".to_string(),
            serde_json::Value::String(method.to_string()),
        ),
    ]);
    if let Some(capability) = capability {
        details.insert(
            "capability".to_string(),
            serde_json::Value::String(capability.to_string()),
        );
    }
    let message = capability
        .map(|capability| {
            format!("provider does not advertise {capability} for method: {method}")
        })
        .unwrap_or_else(|| format!("provider does not advertise method: {method}"));
    ProtocolError {
        code: "capability_unsupported".to_string(),
        message,
        retryable: false,
        details: Some(details),
    }
}

fn protocol_version_error(code: &str, message: &str, request: &HandshakeRequest) -> ProtocolError {
    let mut details = BTreeMap::new();
    details.insert(
        "gatewayProtocolVersion".to_string(),
        serde_json::Value::from(PROTOCOL_VERSION),
    );
    details.insert(
        "clientMinProtocolVersion".to_string(),
        serde_json::Value::from(request.min_protocol_version),
    );
    details.insert(
        "clientMaxProtocolVersion".to_string(),
        serde_json::Value::from(request.max_protocol_version),
    );
    ProtocolError {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        details: Some(details),
    }
}
