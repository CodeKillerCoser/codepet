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
    start_codex_desktop_companion_event_bridge, start_runtime_gateway_event_bridge,
    CodexDesktopCompanionState, RuntimeGatewayState, CODEX_DESKTOP_COMPANION_EVENT,
    RUNTIME_GATEWAY_EVENT,
};
use code_pet_lib::agent::codex_thread_scope::CodexThreadScope;
use code_pet_lib::state::SharedState;
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor, PluginInstanceConfig,
    PluginManager, PluginManagerConfig, PluginRuntimeState, ProviderGatewayService,
    ProviderInstanceRegistry,
};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Listener;

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
async fn real_provider_faults_do_not_emit_tauri_pet_or_compat_channels_or_call_desktop_adapter() {
    let directory = tempfile::tempdir().unwrap();
    let device = DeviceRegistry::open(directory.path().join("device.json"), "Plugin Test Device")
        .unwrap();
    let device_id = device.identity().device_id.clone();
    let fixture_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("crates/Cargo.toml");
    let fixture_executable = fake_provider_executable(&fixture_manifest);
    let descriptor = PluginDescriptor {
        plugin_id: "dev.codepet.isolation".to_string(),
        display_name: "Isolation Fixture".to_string(),
        executable: fixture_executable,
        args: Vec::new(),
        env: [(
            "CODEPET_FAKE_PLUGIN_ID".to_string(),
            "dev.codepet.isolation".to_string(),
        )]
        .into_iter()
        .collect(),
        enabled: true,
        instances: vec![PluginInstanceConfig {
            instance_id: Some("instance-plugin-test".to_string()),
            instance_kind: "fake".to_string(),
            display_name: "Plugin Instance".to_string(),
            settings: Default::default(),
            enabled: true,
        }],
    };
    let plugin_root = directory.path().join("providers");
    let plugin_directory = plugin_root.join("isolation");
    std::fs::create_dir_all(&plugin_directory).unwrap();
    let mut manifest = serde_json::to_value(descriptor).unwrap();
    manifest
        .as_object_mut()
        .unwrap()
        .insert("manifestVersion".to_string(), serde_json::json!(1));
    std::fs::write(
        plugin_directory.join("codepet-provider.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let catalog = PluginCatalog::discover(
        PluginCatalogConfig::default().with_directory(plugin_root),
    );
    let instances = ProviderInstanceRegistry::open(
        directory.path().join("instances.json"),
        device_id.clone(),
    )
    .unwrap();
    let mut config = PluginManagerConfig::default();
    config.process.request_timeout = Duration::from_secs(30);
    config.process.shutdown_timeout = Duration::from_secs(2);
    let manager = Arc::new(
        PluginManager::new(device, catalog, instances, config).unwrap(),
    );
    let provider_gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    assert!(provider_gateway.start_event_forwarding());
    assert_start_enabled(&manager).await;

    let remote = RuntimeGatewayState::new(Arc::new(Gateway::default()));
    let companion = CodexDesktopCompanionState::new(Arc::new(Gateway::default()));
    let desktop_spy = Arc::new(FakeProvider::new("desktop-spy", ProviderStatus::Ready));
    companion
        .gateway()
        .registry()
        .register(desktop_spy.clone())
        .unwrap();
    let app = tauri::test::mock_app();
    let runtime_events = Arc::new(AtomicUsize::new(0));
    let companion_events = Arc::new(AtomicUsize::new(0));
    let pet_events = Arc::new(AtomicUsize::new(0));
    for (event_name, counter) in [
        (RUNTIME_GATEWAY_EVENT, runtime_events.clone()),
        (CODEX_DESKTOP_COMPANION_EVENT, companion_events.clone()),
        ("pet-event", pet_events.clone()),
    ] {
        app.listen(event_name, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
    }
    start_runtime_gateway_event_bridge(app.handle().clone(), &remote).unwrap();
    start_codex_desktop_companion_event_bridge(app.handle().clone(), &companion).unwrap();
    let pet_activity = SharedState::default();
    let mut plugin_events = provider_gateway.subscribe_events(None).unwrap();
    let resource = codepet_host::gateway_sdk::RoutedResourceId {
        device_id: device_id.clone(),
        provider_instance_id: "instance-plugin-test".to_string(),
        native_resource_id: "event-first".to_string(),
    };
    let response = codepet_host::gateway_sdk::ProtocolServer::conversation_get(
        provider_gateway.as_ref(),
        codepet_host::gateway_sdk::ConversationGetRequest {
            conversation: resource,
        },
    )
    .await
    .unwrap();
    assert_eq!(response.conversation.resource.native_resource_id, "event-first");
    let plugin_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = plugin_events.next_event().await.unwrap();
            if let codepet_host::gateway_sdk::ProtocolEvent::ConversationUpserted {
                payload,
                ..
            } = event
            {
                if payload.conversation.resource.native_resource_id
                    == "conversation-event-first"
                {
                    return payload.conversation.resource;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(plugin_event.device_id, device_id);
    assert_eq!(plugin_event.provider_instance_id, "instance-plugin-test");

    let malformed = codepet_host::gateway_sdk::ProtocolServer::conversation_get(
        provider_gateway.as_ref(),
        codepet_host::gateway_sdk::ConversationGetRequest {
            conversation: provider_resource(&device_id, "malformed"),
        },
    )
    .await
    .unwrap_err();
    assert!(malformed.code.contains("provider") || malformed.code.contains("rpc"));
    wait_for_plugin_state(&manager, PluginRuntimeState::Crashed).await;

    assert_start_enabled(&manager).await;
    let crashed = codepet_host::gateway_sdk::ProtocolServer::conversation_get(
        provider_gateway.as_ref(),
        codepet_host::gateway_sdk::ConversationGetRequest {
            conversation: provider_resource(&device_id, "crash"),
        },
    )
    .await
    .unwrap_err();
    assert!(crashed.code.contains("provider") || crashed.code.contains("rpc"));
    wait_for_plugin_state(&manager, PluginRuntimeState::Crashed).await;

    manager.shutdown().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(remote.transport().replay(None).unwrap().is_empty());
    assert!(companion.transport().replay(None).unwrap().is_empty());
    assert!(pet_activity.recent_events().is_empty());
    let companion_providers = companion.gateway().registry().list().unwrap();
    assert_eq!(companion_providers.len(), 1);
    assert_eq!(companion_providers[0].id, "desktop-spy");
    assert!(desktop_spy.calls().is_empty());
    assert_eq!(runtime_events.load(Ordering::SeqCst), 0);
    assert_eq!(companion_events.load(Ordering::SeqCst), 0);
    assert_eq!(pet_events.load(Ordering::SeqCst), 0);
}

fn cargo_executable() -> PathBuf {
    let configured = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"));
    if configured.is_file() {
        return configured;
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(&configured))
        .find(|candidate| candidate.is_file())
        .expect("cargo executable must be available for the real Provider fixture")
}

fn fake_provider_executable(fixture_manifest: &std::path::Path) -> PathBuf {
    let target_directory = fixture_manifest.parent().unwrap().join("target");
    let status = Command::new(cargo_executable())
        .arg("build")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(fixture_manifest)
        .arg("--target-dir")
        .arg(&target_directory)
        .arg("-p")
        .arg("codepet-host")
        .arg("--bin")
        .arg("codepet-host-fake-provider")
        .status()
        .unwrap();
    assert!(status.success(), "real Provider fixture must compile");
    let executable = target_directory
        .join("debug")
        .join(format!(
            "codepet-host-fake-provider{}",
            std::env::consts::EXE_SUFFIX
        ));
    assert!(executable.is_file());
    executable
}

fn provider_resource(
    device_id: &str,
    native_resource_id: &str,
) -> codepet_host::gateway_sdk::RoutedResourceId {
    codepet_host::gateway_sdk::RoutedResourceId {
        device_id: device_id.to_string(),
        provider_instance_id: "instance-plugin-test".to_string(),
        native_resource_id: native_resource_id.to_string(),
    }
}

async fn assert_start_enabled(manager: &PluginManager) {
    let outcomes = manager.start_enabled().await;
    assert_eq!(outcomes.len(), 1);
    if let Err(error) = outcomes.into_iter().next().unwrap().1 {
        let snapshot = manager.snapshot("dev.codepet.isolation").await.unwrap();
        panic!(
            "real Provider fixture failed to start: {error:?}; stderr={:?}",
            snapshot.stderr_diagnostics
        );
    }
    wait_for_plugin_state(manager, PluginRuntimeState::Ready).await;
}

async fn wait_for_plugin_state(manager: &PluginManager, expected: PluginRuntimeState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if manager
                .snapshot("dev.codepet.isolation")
                .await
                .unwrap()
                .state
                == expected
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
