use code_pet_lib::runtime_gateway::generated::{
    Approval, ApprovalDecision, ApprovalResolveRequest, ApprovalResolveResponse, ApprovalStatus,
    Conversation, ConversationCreateRequest, ConversationCreateResponse, ConversationGetRequest,
    ConversationGetResponse, ConversationListRequest, ConversationListResponse,
    ConversationStatus, ConversationUpsertedEvent, PermissionLevel, ProtocolEvent,
    ProtocolRequest, ProtocolResponse, ProtocolServer, Provider, ProviderCapabilities,
    ProviderListRequest, ProviderStatus, ResponsePayload, TurnInterruptRequest,
    TurnInterruptResponse, TurnSendRequest, TurnSendResponse, TurnTask, TurnTaskStatus,
    PROTOCOL_VERSION,
};
use code_pet_lib::runtime_gateway::{
    event_sequence, Gateway, LocalTransport, ProviderAdapter, ProviderFuture, ProviderRegistry,
    Transport,
};
use std::sync::{Arc, Mutex};

struct FakeProvider {
    provider: Provider,
    calls: Mutex<Vec<String>>,
}

impl FakeProvider {
    fn new(id: &str, status: ProviderStatus) -> Self {
        Self {
            provider: Provider {
                id: id.to_string(),
                provider_type: "fake".to_string(),
                display_name: format!("Fake {id}"),
                version: Some("test".to_string()),
                status,
                capabilities: ProviderCapabilities {
                    methods: Vec::new(),
                    permission_levels: vec![PermissionLevel::WorkspaceWrite],
                    models: Vec::new(),
                    reasoning_efforts: Vec::new(),
                    quick_replies: Vec::new(),
                    can_steer: true,
                    can_interrupt: true,
                    extension: None,
                },
                extension: None,
            },
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.to_string());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProviderAdapter for FakeProvider {
    fn provider(&self) -> Provider {
        self.provider.clone()
    }

    fn conversation_list<'a>(
        &'a self,
        _request: ConversationListRequest,
    ) -> ProviderFuture<'a, ConversationListResponse> {
        self.record("conversation.list");
        let conversation = conversation(&self.provider.id);
        Box::pin(async move {
            Ok(ConversationListResponse {
                conversations: vec![conversation],
                next_cursor: None,
                event_sequence: 999,
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        _request: ConversationGetRequest,
    ) -> ProviderFuture<'a, ConversationGetResponse> {
        self.record("conversation.get");
        let conversation = conversation(&self.provider.id);
        Box::pin(async move { Ok(ConversationGetResponse { conversation }) })
    }

    fn conversation_create<'a>(
        &'a self,
        _request: ConversationCreateRequest,
    ) -> ProviderFuture<'a, ConversationCreateResponse> {
        self.record("conversation.create");
        let conversation = conversation(&self.provider.id);
        Box::pin(async move { Ok(ConversationCreateResponse { conversation }) })
    }

    fn turn_send<'a>(
        &'a self,
        _request: TurnSendRequest,
    ) -> ProviderFuture<'a, TurnSendResponse> {
        self.record("turn.send");
        let turn = turn(&self.provider.id);
        Box::pin(async move { Ok(TurnSendResponse { turn }) })
    }

    fn turn_interrupt<'a>(
        &'a self,
        _request: TurnInterruptRequest,
    ) -> ProviderFuture<'a, TurnInterruptResponse> {
        self.record("turn.interrupt");
        let mut turn = turn(&self.provider.id);
        turn.status = TurnTaskStatus::Interrupted;
        Box::pin(async move { Ok(TurnInterruptResponse { turn }) })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProviderFuture<'a, ApprovalResolveResponse> {
        self.record("approval.resolve");
        let provider_id = self.provider.id.clone();
        Box::pin(async move {
            Ok(ApprovalResolveResponse {
                approval: Approval {
                    id: request.approval_id,
                    provider_id,
                    conversation_id: "conversation-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    kind: "test".to_string(),
                    title: "Test approval".to_string(),
                    description: None,
                    status: match request.decision {
                        ApprovalDecision::Approve => ApprovalStatus::Approved,
                        ApprovalDecision::Deny => ApprovalStatus::Denied,
                    },
                    decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
                    requested_at: 1,
                    resolved_at: Some(2),
                    decision: Some(request.decision),
                    extension: None,
                },
            })
        })
    }
}

fn conversation(provider_id: &str) -> Conversation {
    Conversation {
        id: format!("{provider_id}-conversation"),
        provider_id: provider_id.to_string(),
        title: format!("{provider_id} conversation"),
        preview: None,
        status: ConversationStatus::Idle,
        permission_level: PermissionLevel::WorkspaceWrite,
        model: None,
        reasoning_effort: None,
        workspace_root: None,
        created_at: 1,
        updated_at: 1,
        active_turn: None,
        extension: None,
    }
}

fn turn(provider_id: &str) -> TurnTask {
    TurnTask {
        id: "turn-1".to_string(),
        provider_id: provider_id.to_string(),
        conversation_id: "conversation-1".to_string(),
        status: TurnTaskStatus::Running,
        display_summary: None,
        started_at: Some(1),
        updated_at: 1,
        completed_at: None,
        extension: None,
    }
}

fn conversation_event(provider_id: &str) -> ProtocolEvent {
    ProtocolEvent::ConversationUpserted {
        protocol_version: PROTOCOL_VERSION,
        event_sequence: 0,
        payload: ConversationUpsertedEvent {
            conversation: conversation(provider_id),
        },
    }
}

fn gateway_with_two_providers() -> (Gateway, Arc<FakeProvider>, Arc<FakeProvider>) {
    let registry = ProviderRegistry::default();
    let alpha = Arc::new(FakeProvider::new("alpha", ProviderStatus::Ready));
    let beta = Arc::new(FakeProvider::new("beta", ProviderStatus::Ready));
    registry.register(alpha.clone()).unwrap();
    registry.register(beta.clone()).unwrap();
    (Gateway::new(registry), alpha, beta)
}

#[tokio::test]
async fn gateway_routes_provider_scoped_methods_and_aggregates_provider_lists() {
    let (gateway, alpha, beta) = gateway_with_two_providers();

    let providers = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    assert_eq!(
        providers.providers.iter().map(|provider| provider.id.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );

    let conversations = gateway
        .conversation_list(ConversationListRequest {
            provider_id: None,
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(conversations.conversations.len(), 2);
    assert_eq!(conversations.event_sequence, 0);

    gateway
        .conversation_get(ConversationGetRequest {
            provider_id: "beta".to_string(),
            conversation_id: "conversation-1".to_string(),
        })
        .await
        .unwrap();
    gateway
        .conversation_create(ConversationCreateRequest {
            provider_id: "beta".to_string(),
            title: None,
            permission_level: PermissionLevel::WorkspaceWrite,
            model: None,
            reasoning_effort: None,
            workspace_root: None,
        })
        .await
        .unwrap();
    gateway
        .turn_interrupt(TurnInterruptRequest {
            provider_id: "beta".to_string(),
            conversation_id: "conversation-1".to_string(),
            turn_id: "turn-1".to_string(),
        })
        .await
        .unwrap();
    gateway
        .approval_resolve(ApprovalResolveRequest {
            provider_id: "beta".to_string(),
            approval_id: "approval-1".to_string(),
            decision: ApprovalDecision::Approve,
        })
        .await
        .unwrap();

    assert_eq!(alpha.calls(), vec!["conversation.list"]);
    assert_eq!(
        beta.calls(),
        vec![
            "conversation.list",
            "conversation.get",
            "conversation.create",
            "turn.interrupt",
            "approval.resolve"
        ]
    );
}

#[tokio::test]
async fn gateway_returns_clear_unknown_and_unavailable_provider_errors() {
    let registry = ProviderRegistry::default();
    registry
        .register(Arc::new(FakeProvider::new(
            "offline",
            ProviderStatus::Unavailable,
        )))
        .unwrap();
    let gateway = Gateway::new(registry);

    let unknown = gateway
        .conversation_get(ConversationGetRequest {
            provider_id: "missing".to_string(),
            conversation_id: "conversation-1".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code, "unknown_provider");

    let unavailable = gateway
        .conversation_get(ConversationGetRequest {
            provider_id: "offline".to_string(),
            conversation_id: "conversation-1".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(unavailable.code, "provider_unavailable");
    assert!(unavailable.retryable);
}

#[tokio::test]
async fn gateway_assigns_monotonic_sequences_and_replays_the_retained_window() {
    let gateway = Gateway::with_event_capacity(ProviderRegistry::default(), 2);

    let first = gateway.publish_event(conversation_event("alpha")).unwrap();
    let second = gateway.publish_event(conversation_event("alpha")).unwrap();
    let third = gateway.publish_event(conversation_event("alpha")).unwrap();
    assert_eq!(event_sequence(&first), 1);
    assert_eq!(event_sequence(&second), 2);
    assert_eq!(event_sequence(&third), 3);

    let replay = gateway.replay_events(Some(1)).unwrap();
    assert_eq!(replay.iter().map(event_sequence).collect::<Vec<_>>(), vec![2, 3]);
    assert_eq!(
        gateway.replay_events(Some(0)).unwrap_err().code,
        "event_replay_unavailable"
    );

    let mut subscription = gateway.subscribe_events(Some(3)).unwrap();
    gateway.publish_event(conversation_event("alpha")).unwrap();
    assert_eq!(event_sequence(&subscription.next_event().await.unwrap()), 4);
}

#[tokio::test]
async fn local_transport_dispatches_requests_and_subscribes_to_gateway_events() {
    let (gateway, _alpha, beta) = gateway_with_two_providers();
    let gateway = Arc::new(gateway);
    let transport = LocalTransport::new(gateway.clone());
    let mut subscription = transport.subscribe(Some(0)).unwrap();

    let response = transport
        .request(ProtocolRequest::TurnSend {
            protocol_version: PROTOCOL_VERSION,
            id: "request-1".to_string(),
            params: TurnSendRequest {
                provider_id: "beta".to_string(),
                conversation_id: "conversation-1".to_string(),
                client_message_id: "message-1".to_string(),
                message: "hello".to_string(),
                quick_reply_id: None,
                steer_turn_id: None,
            },
        })
        .await;

    match response {
        ProtocolResponse::TurnSend {
            id,
            response: ResponsePayload::Ok { result },
            ..
        } => {
            assert_eq!(id, "request-1");
            assert_eq!(result.turn.provider_id, "beta");
        }
        response => panic!("unexpected response: {response:?}"),
    }
    assert_eq!(beta.calls(), vec!["turn.send"]);

    gateway.publish_event(conversation_event("beta")).unwrap();
    let event = subscription.next_event().await.unwrap();
    assert_eq!(event_sequence(&event), 1);
    assert_eq!(transport.replay(Some(0)).unwrap(), vec![event]);
}
