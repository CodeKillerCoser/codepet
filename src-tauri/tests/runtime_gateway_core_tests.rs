use code_pet_lib::runtime_gateway::generated::{
    Approval, ApprovalDecision, ApprovalResolveRequest, ApprovalResolveResponse, ApprovalStatus,
    ApprovalRequestedEvent,
    Conversation, ConversationCreateRequest, ConversationCreateResponse, ConversationGetRequest,
    ConversationGetResponse, ConversationListRequest, ConversationListResponse,
    ConversationStatus, ConversationUpsertedEvent, PermissionLevel, ProtocolEvent,
    ProtocolRequest, ProtocolResponse, ProtocolServer, Provider, ProviderCapabilities,
    ProviderListRequest, ProviderStatus, ResponsePayload, TurnInterruptRequest,
    TurnInterruptResponse, TurnSendRequest, TurnSendResponse, TurnTask, TurnTaskStatus,
    TurnUpsertedEvent,
    PROTOCOL_VERSION,
};
use code_pet_lib::runtime_gateway::{
    event_sequence, Gateway, LocalTransport, ProviderAdapter, ProviderFuture, ProviderRegistry,
    Transport,
};
use code_pet_lib::runtime_gateway::tauri_bridge::{
    CodexDesktopCompanionState, RuntimeGatewayState,
};
use code_pet_lib::agent::codex_thread_scope::CodexThreadScope;
use code_pet_lib::state::SharedState;
use codepet_host::{
    DeviceIdentity, DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
    PluginInstanceConfig, PluginManager, PluginManagerConfig, ProviderGatewayService,
    ProviderInstanceRegistry,
};
use codepet_host::provider_sdk::{
    ConversationStatus as ProviderConversationStatus,
    ConversationUpsertedEvent as ProviderConversationUpsertedEvent,
    ProtocolEvent as ProviderProtocolEvent, ProviderConversation, RoutedResourceId,
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
                    methods: vec![
                        "conversation.list".to_string(),
                        "conversation.get".to_string(),
                        "conversation.create".to_string(),
                        "turn.send".to_string(),
                        "turn.interrupt".to_string(),
                        "approval.resolve".to_string(),
                    ],
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

    fn advertise_only(mut self, methods: &[&str]) -> Self {
        self.provider.capabilities.methods = methods.iter().map(|method| method.to_string()).collect();
        self
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
async fn gateway_fails_closed_before_dispatching_unadvertised_actions() {
    let registry = ProviderRegistry::default();
    let provider = Arc::new(
        FakeProvider::new("read-only", ProviderStatus::Ready)
            .advertise_only(&["conversation.list", "conversation.get"]),
    );
    registry.register(provider.clone()).unwrap();
    let gateway = Gateway::new(registry);

    let create_error = gateway
        .conversation_create(ConversationCreateRequest {
            provider_id: "read-only".to_string(),
            title: None,
            permission_level: PermissionLevel::WorkspaceWrite,
            model: None,
            reasoning_effort: None,
            workspace_root: None,
        })
        .await
        .unwrap_err();
    let send_error = gateway
        .turn_send(TurnSendRequest {
            provider_id: "read-only".to_string(),
            conversation_id: "conversation-1".to_string(),
            client_message_id: "message-1".to_string(),
            message: "hello".to_string(),
            quick_reply_id: None,
            steer_turn_id: None,
        })
        .await
        .unwrap_err();
    let interrupt_error = gateway
        .turn_interrupt(TurnInterruptRequest {
            provider_id: "read-only".to_string(),
            conversation_id: "conversation-1".to_string(),
            turn_id: "turn-1".to_string(),
        })
        .await
        .unwrap_err();
    let approval_error = gateway
        .approval_resolve(ApprovalResolveRequest {
            provider_id: "read-only".to_string(),
            approval_id: "approval-1".to_string(),
            decision: ApprovalDecision::Deny,
        })
        .await
        .unwrap_err();

    assert_eq!(create_error.code, "capability_unsupported");
    assert_eq!(send_error.code, "capability_unsupported");
    assert_eq!(interrupt_error.code, "capability_unsupported");
    assert_eq!(approval_error.code, "capability_unsupported");
    assert_eq!(provider.calls(), Vec::<String>::new());
}

#[tokio::test]
async fn gateway_requires_action_specific_capability_flags() {
    let registry = ProviderRegistry::default();
    let mut provider = FakeProvider::new("limited", ProviderStatus::Ready);
    provider.provider.capabilities.can_steer = false;
    provider.provider.capabilities.can_interrupt = false;
    let provider = Arc::new(provider);
    registry.register(provider.clone()).unwrap();
    let gateway = Gateway::new(registry);

    let steer_error = gateway
        .turn_send(TurnSendRequest {
            provider_id: "limited".to_string(),
            conversation_id: "conversation-1".to_string(),
            client_message_id: "message-1".to_string(),
            message: "hello".to_string(),
            quick_reply_id: None,
            steer_turn_id: Some("turn-1".to_string()),
        })
        .await
        .unwrap_err();
    let interrupt_error = gateway
        .turn_interrupt(TurnInterruptRequest {
            provider_id: "limited".to_string(),
            conversation_id: "conversation-1".to_string(),
            turn_id: "turn-1".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(steer_error.code, "capability_unsupported");
    assert_eq!(interrupt_error.code, "capability_unsupported");
    assert_eq!(provider.calls(), Vec::<String>::new());
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

#[tokio::test]
async fn remote_and_desktop_companion_lifecycles_are_independent() {
    let remote_registry = ProviderRegistry::default();
    remote_registry
        .register(Arc::new(FakeProvider::new("codex", ProviderStatus::Ready)))
        .unwrap();
    let companion_registry = ProviderRegistry::default();
    companion_registry
        .register(Arc::new(FakeProvider::new(
            "codex",
            ProviderStatus::Unavailable,
        )))
        .unwrap();
    let remote = RuntimeGatewayState::new(Arc::new(Gateway::new(remote_registry)));
    let companion = CodexDesktopCompanionState::new(Arc::new(Gateway::new(companion_registry)));

    let listed = remote
        .gateway()
        .conversation_list(ConversationListRequest {
            provider_id: Some("codex".to_string()),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(listed.conversations.len(), 1);
    assert_eq!(
        companion
            .gateway()
            .provider_list(ProviderListRequest {})
            .await
            .unwrap()
            .providers[0]
            .status,
        ProviderStatus::Unavailable
    );
}

#[tokio::test]
async fn remote_initialization_placeholder_does_not_block_companion_construction() {
    let thread_scope = CodexThreadScope::default();
    let remote = RuntimeGatewayState::with_thread_scope(thread_scope.clone());
    let companion = CodexDesktopCompanionState::with_thread_scope(thread_scope);

    let remote_provider = remote
        .gateway()
        .provider_list(ProviderListRequest {})
        .await
        .unwrap()
        .providers
        .remove(0);
    let companion_provider = companion
        .gateway()
        .provider_list(ProviderListRequest {})
        .await
        .unwrap()
        .providers
        .remove(0);

    assert_eq!(remote_provider.status, ProviderStatus::Unavailable);
    assert_eq!(remote_provider.id, "codex");
    assert_eq!(companion_provider.id, "codex");
}

#[test]
fn remote_events_never_enter_the_desktop_companion_transport() {
    let remote = RuntimeGatewayState::new(Arc::new(Gateway::default()));
    let companion = CodexDesktopCompanionState::new(Arc::new(Gateway::default()));
    let remote_turn = turn("codex");
    let remote_approval = Approval {
        id: "remote-approval".to_string(),
        provider_id: "codex".to_string(),
        conversation_id: "codex-conversation".to_string(),
        turn_id: remote_turn.id.clone(),
        kind: "command-execution".to_string(),
        title: "Remote approval".to_string(),
        description: None,
        status: ApprovalStatus::Pending,
        decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
        requested_at: 1,
        resolved_at: None,
        decision: None,
        extension: None,
    };

    remote
        .gateway()
        .publish_event(conversation_event("codex"))
        .unwrap();
    remote
        .gateway()
        .publish_event(ProtocolEvent::TurnUpserted {
            protocol_version: PROTOCOL_VERSION,
            event_sequence: 0,
            payload: TurnUpsertedEvent { turn: remote_turn },
        })
        .unwrap();
    remote
        .gateway()
        .publish_event(ProtocolEvent::ApprovalRequested {
            protocol_version: PROTOCOL_VERSION,
            event_sequence: 0,
            payload: ApprovalRequestedEvent {
                approval: remote_approval,
            },
        })
        .unwrap();

    assert_eq!(remote.transport().replay(None).unwrap().len(), 3);
    assert!(companion.transport().replay(None).unwrap().is_empty());

    companion
        .gateway()
        .publish_event(conversation_event("codex"))
        .unwrap();
    assert_eq!(companion.transport().replay(None).unwrap().len(), 1);
    assert_eq!(remote.transport().replay(None).unwrap().len(), 3);
}

#[tokio::test]
async fn provider_plugin_events_stay_out_of_compat_companion_and_pet_activity_state() {
    let device = DeviceRegistry::from_identity(DeviceIdentity {
        version: 1,
        device_id: "device-plugin-test".to_string(),
        display_name: "Plugin Test Device".to_string(),
        created_at: 1,
    })
    .unwrap();
    let catalog = PluginCatalog::discover(
        PluginCatalogConfig::default().with_descriptor(PluginDescriptor {
            plugin_id: "dev.codepet.isolation".to_string(),
            display_name: "Isolation Fixture".to_string(),
            executable: "/not-started/codepet-provider-fixture".into(),
            args: Vec::new(),
            env: Default::default(),
            enabled: true,
            instances: vec![PluginInstanceConfig {
                instance_id: Some("instance-plugin-test".to_string()),
                instance_kind: "fake".to_string(),
                display_name: "Plugin Instance".to_string(),
                settings: Default::default(),
                enabled: true,
            }],
        }),
    );
    let instances = ProviderInstanceRegistry::in_memory("device-plugin-test".to_string()).unwrap();
    let manager = Arc::new(
        PluginManager::new(
            device,
            catalog,
            instances,
            PluginManagerConfig::default(),
        )
        .unwrap(),
    );
    let provider_gateway = Arc::new(ProviderGatewayService::new(manager.clone()));
    provider_gateway.start_event_forwarding();
    let current_cursor = provider_gateway.current_event_cursor();
    let mut plugin_events = provider_gateway
        .subscribe_events(Some(&current_cursor))
        .unwrap();

    let remote = RuntimeGatewayState::new(Arc::new(Gateway::default()))
        .with_provider_gateway_v1(provider_gateway.clone());
    let companion = CodexDesktopCompanionState::new(Arc::new(Gateway::default()));
    let pet_activity = SharedState::default();
    let resource = RoutedResourceId {
        device_id: "device-plugin-test".to_string(),
        provider_instance_id: "instance-plugin-test".to_string(),
        native_resource_id: "plugin-conversation".to_string(),
    };
    manager
        .accept_provider_event(
            "dev.codepet.isolation",
            ProviderProtocolEvent::EventConversationUpserted {
                jsonrpc: "2.0".to_string(),
                params: ProviderConversationUpsertedEvent {
                    conversation: ProviderConversation {
                        resource: resource.clone(),
                        title: "Plugin conversation".to_string(),
                        preview: None,
                        status: ProviderConversationStatus::Idle,
                        permission_level: "workspace-write".to_string(),
                        model: None,
                        reasoning_effort: None,
                        workspace_root: None,
                        created_at: 1,
                        updated_at: 2,
                        active_turn: None,
                        extension: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    let event = plugin_events.next_event().await.unwrap();
    let codepet_gateway_sdk::ProtocolEvent::ConversationUpserted { payload, .. } = event else {
        panic!("expected v1 Provider conversation event");
    };
    assert_eq!(payload.conversation.resource, resource);
    assert!(remote.transport().replay(None).unwrap().is_empty());
    assert!(companion.transport().replay(None).unwrap().is_empty());
    assert!(pet_activity.recent_events().is_empty());
    assert!(remote.provider_manager().is_some());
    assert!(companion.gateway().registry().list().unwrap().is_empty());
}
