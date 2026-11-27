use codepet_gateway_sdk::{
    ConversationAcquireInteractionRequest as GatewayConversationAcquireInteractionRequest,
    ConversationCreateRequest as GatewayConversationCreateRequest,
    ConversationGetRequest as GatewayConversationGetRequest,
    ConversationListRequest as GatewayConversationListRequest, ConversationProjectFilter,
    ConversationProjectFilterAll, ConversationProjectFilterAllKind,
    ConversationProjectFilterProject, ConversationProjectFilterProjectKind,
    ConversationSearchRequest as GatewayConversationSearchRequest, DeviceDescriptor,
    EventSubscribeRequest, FlatModelCatalogKind, FlatModelSelection, GroupedModelCatalogKind,
    GroupedModelSelection, HandshakeRequest, JsonRpcResponsePayload, ModelCatalog, ModelSelection,
    ProjectCreateRequest, ProjectDeleteRequest, ProjectGetRequest, ProjectListRequest, ProjectRoot,
    ProjectUpdateRequest, ProtocolEvent as GatewayEvent, ProtocolRequest as GatewayRequest,
    ProtocolServer as GatewayProtocolServer, ProviderDescribeRequest, ProviderListRequest,
    TurnInput as GatewayTurnInput, TurnInputKind as GatewayTurnInputKind,
    TurnSelection as GatewayTurnSelection, TurnSendRequest as GatewayTurnSendRequest, ProtocolVersionOffer,
};
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor, PluginInstanceConfig,
    PluginManager, PluginManagerConfig, PluginProcessOptions, PluginRuntimeState,
    ProviderGatewayService, ProviderInstanceRegistry, RemoteHostIdentity,
};
use codepet_provider_sdk::{
    ConversationGetRequest, JsonObject, ProviderInstanceRoute, ProviderResourceId, TurnInput,
    TurnInputKind, TurnSelection, TurnStartRequest,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

#[path = "manager_gateway/registry.rs"]
mod registry;
#[path = "manager_gateway/input_queue.rs"]
mod input_queue;
#[path = "manager_gateway/recent.rs"]
mod recent;
#[path = "manager_gateway/runtime_inventory.rs"]
mod runtime_inventory;
#[path = "manager_gateway/provider_failure_isolation.rs"]
mod provider_failure_isolation;
#[path = "manager_gateway/plugin_lifecycle.rs"]
mod plugin_lifecycle;
#[path = "manager_gateway/routing_validation.rs"]
mod routing_validation;
#[path = "manager_gateway/historical_recovery.rs"]
mod historical_recovery;

#[tokio::test]
async fn codepet_usage_routes_to_provider_and_host_assigns_private_storage_and_logs() {
    let directory=tempfile::tempdir().unwrap();
    let device=DeviceRegistry::open(directory.path().join("device.json"),"Usage test").unwrap();
    let device_id=device.identity().device_id.clone();
    let mut descriptor=plugin("dev.codepet.usage-fixture",&["usage-instance"]);
    descriptor.env.insert("CODEPET_FAKE_USAGE".into(),"1".into());
    let marker=directory.path().join("directories.json");
    descriptor.env.insert("CODEPET_FAKE_DIRECTORIES_MARKER".into(),marker.to_string_lossy().into_owned());
    let catalog_dir=directory.path().join("plugins");std::fs::create_dir_all(catalog_dir.join("fixture")).unwrap();
    let mut manifest=serde_json::to_value(&descriptor).unwrap();manifest["manifestVersion"]=serde_json::json!(2);manifest["transport"]=serde_json::json!("stdio-codepet-mux-v1");
    std::fs::write(catalog_dir.join("fixture/codepet-provider.json"),serde_json::to_vec(&manifest).unwrap()).unwrap();
    let catalog=PluginCatalog::discover(PluginCatalogConfig::default().with_directory(catalog_dir));
    let registry=ProviderInstanceRegistry::open(directory.path().join("instances.json"),device_id).unwrap();
    let data_root=directory.path().join("providers");
    let manager=Arc::new(PluginManager::new(device,catalog,registry,PluginManagerConfig{provider_data_root:Some(data_root.clone()),provider_logs_root:Some(directory.path().join("logs/providers")),..PluginManagerConfig::default()}).unwrap());
    manager.enable_connection_heartbeats();
    let gateway=Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    let request: GatewayRequest=serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":"usage","method":"codepet.usage.query","params":{"providerId":"usage-instance","query":{"datasetId":"observed-model-tokens","filter":{"time":{"kind":"all"}},"aggregation":{"timeBucket":"halfHour","timeZone":"UTC","groupBy":["model"]},"metrics":["totalTokens"],"summaries":["totals"]}}})).unwrap();
    // Presence is liveness data, not authorization or a prerequisite for local reads.
    // Remote channel admission still validates credentials before dispatch.
    let without_phone=gateway.dispatch_for_caller_scope("client",request.clone()).await;
    assert!(matches!(without_phone.response, JsonRpcResponsePayload::Ok { .. }),"{without_phone:?}");
    let response=gateway.dispatch_for_local_client("desktop-main",request).await;
    assert!(manager.remote_connections().snapshot().connections.is_empty());
    let JsonRpcResponsePayload::Ok{result}=response.response else{panic!("usage query failed: {response:?}")};
    assert_eq!(result["result"]["summaries"]["totals"]["totalTokens"]["value"],20);
    assert_eq!(result["result"]["rows"][0]["modelId"],"fixture-model");
    let describe=serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":"usage-description","method":"provider.describe","params":{"providerId":"usage-instance"}})).unwrap();
    let description=gateway.dispatch_for_local_client("desktop-main",describe).await;
    let JsonRpcResponsePayload::Ok{result:description}=description.response else{panic!("provider description failed")};
    assert_eq!(description["provider"]["runtime"]["usage"]["displayText"],"Fixture usage 42%");
    assert!(description["capabilities"]["methods"].as_array().unwrap().contains(&serde_json::json!("codepet.usage.query")));
    assert!(!description["provider"]["runtime"]["usage"].to_string().contains("accessToken"));
    let empty_request=serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":"empty-usage","method":"codepet.usage.query","params":{"providerId":"usage-instance","query":{"datasetId":"observed-model-tokens","filter":{"time":{"kind":"all"},"modelIds":["missing-model"]},"aggregation":{"timeBucket":"halfHour","timeZone":"UTC","groupBy":["model"]},"metrics":["totalTokens"],"summaries":["peakDaily"]}}})).unwrap();
    let empty=gateway.dispatch_for_local_client("desktop-main",empty_request).await;
    let JsonRpcResponsePayload::Ok{result:empty}=empty.response else{panic!("empty usage failed")};
    assert_eq!(empty["result"]["summaries"].get("peakDaily"),Some(&serde_json::Value::Null));
    let assigned:codepet_provider_sdk::ProviderDirectories=serde_json::from_slice(&std::fs::read(marker).unwrap()).unwrap();
    assert_eq!(std::path::Path::new(&assigned.database_path),data_root.join("dev.codepet.usage-fixture/usage.sqlite"));
    assert_eq!(std::path::Path::new(&assigned.logs), directory.path().join("logs/providers/dev.codepet.usage-fixture"));
    assert!(std::fs::read(&assigned.database_path).unwrap().starts_with(b"SQLite format 3"));
    assert!(!result.to_string().contains(&assigned.data));
    manager.shutdown().await;
    let log=std::fs::read_to_string(std::path::Path::new(&assigned.logs).join("provider.log")).unwrap();
    assert!(log.contains("fixture Provider directories initialized"));
}

fn plugin(plugin_id: &str, instances: &[&str]) -> PluginDescriptor {
    PluginDescriptor {
        plugin_id: plugin_id.to_string(),
        display_name: format!("Fake {plugin_id}"),
        icon: "https://example.com/fake.png".to_string(),
        executable: env!("CARGO_BIN_EXE_codepet-host-fake-provider").into(),
        args: Vec::new(),
        env: BTreeMap::from([(
            "CODEPET_FAKE_PLUGIN_ID".to_string(),
            plugin_id.to_string(),
        )]),
        enabled: true,
        instances: instances
            .iter()
            .map(|instance_id| PluginInstanceConfig {
                instance_id: Some((*instance_id).to_string()),
                instance_kind: "fake".to_string(),
                display_name: format!("Instance {instance_id}"),
                settings: JsonObject::new(),
                enabled: true,
            })
            .collect(),
    }
}

fn build_manager(device_id: &str, descriptors: Vec<PluginDescriptor>) -> Arc<PluginManager> {
    build_manager_with_event_capacity(
        device_id,
        descriptors,
        PluginManagerConfig::default().event_capacity,
    )
}

fn build_manager_with_event_capacity(
    device_id: &str,
    descriptors: Vec<PluginDescriptor>,
    event_capacity: usize,
) -> Arc<PluginManager> {
    build_manager_with_options(
        device_id,
        descriptors,
        event_capacity,
        PluginProcessOptions {
            request_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
            ..PluginProcessOptions::default()
        },
    )
}

fn build_manager_with_options(
    device_id: &str,
    descriptors: Vec<PluginDescriptor>,
    event_capacity: usize,
    process: PluginProcessOptions,
) -> Arc<PluginManager> {
    build_manager_at(device_id,descriptors,event_capacity,process,None)
}
thread_local! {
    static TEST_DATABASE_DIRS: std::cell::RefCell<Vec<tempfile::TempDir>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn build_manager_at(device_id: &str,descriptors: Vec<PluginDescriptor>,event_capacity: usize,process: PluginProcessOptions,state_path: Option<&std::path::Path>) -> Arc<PluginManager> {
    let directory = tempfile::tempdir().unwrap();
    let device_path = state_path.map(std::path::Path::to_owned).unwrap_or_else(||directory.path().join("app-state.sqlite"));
    codepet_provider_data::app_state::open_app_state(&device_path).unwrap().execute(
        "INSERT OR IGNORE INTO host_identity VALUES(1,?1,?2,1)",rusqlite::params![device_id,format!("Device {device_id}")]).unwrap();
    let device = DeviceRegistry::open(&device_path, format!("Device {device_id}")).unwrap();
    let plugin_directory = directory.path().join("providers");
    for (index, descriptor) in descriptors.into_iter().enumerate() {
        let directory = plugin_directory.join(index.to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let mut manifest = serde_json::to_value(descriptor).unwrap();
        manifest
            .as_object_mut()
            .unwrap()
            .insert("manifestVersion".to_string(), serde_json::json!(2));
        manifest["transport"] = manifest["env"].get("CODEPET_PROVIDER_TRANSPORT").cloned().unwrap_or(serde_json::json!("stdio-codepet-mux-v1"));
        std::fs::write(
            directory.join("codepet-provider.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }
    let catalog = PluginCatalog::discover(
        PluginCatalogConfig::default().with_directory(plugin_directory),
    );
    let instances = ProviderInstanceRegistry::open(
        device_path,
        device_id.to_string(),
    )
    .unwrap();
    TEST_DATABASE_DIRS.with(|directories|directories.borrow_mut().push(directory));
    Arc::new(
        PluginManager::new(
            device,
            catalog,
            instances,
            PluginManagerConfig {
                event_capacity,
                process,
                ..PluginManagerConfig::default()
            },
        )
        .unwrap(),
    )
}

fn resource(
    device_id: &str,
    plugin_id: &str,
    instance_id: &str,
    native_id: &str,
) -> ProviderResourceId {
    ProviderResourceId {
        device_id: device_id.to_string(),
        provider_plugin_id: plugin_id.to_string(),
        provider_instance_id: instance_id.to_string(),
        native_resource_id: native_id.to_string(),
    }
}

fn gateway_resource(
    _device_id: &str,
    _plugin_id: &str,
    instance_id: &str,
    native_id: &str,
) -> codepet_gateway_sdk::RoutedResourceId {
    codepet_gateway_sdk::RoutedResourceId {
        provider_id: instance_id.to_string(),
        native_resource_id: native_id.to_string(),
    }
}

fn event_cursor_sequence(cursor: &str) -> u64 {
    cursor
        .strip_prefix("event-")
        .unwrap()
        .parse::<u64>()
        .unwrap()
}

fn handshake_request() -> HandshakeRequest {
    HandshakeRequest {
        client_id: "remote-client-handshake".to_string(),
        device: device_descriptor("Remote Test"),
        client_version: "1.0.0".to_string(),
        protocol: ProtocolVersionOffer {
            min_version: codepet_gateway_sdk::PROTOCOL_VERSION,
            version: codepet_gateway_sdk::PROTOCOL_VERSION,
        },
        last_event_cursor: None,
    }
}

fn device_descriptor(name: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        device_name: name.to_string(),
        operating_system: "Test OS".to_string(),
        system_version: "1.0".to_string(),
    }
}

fn all_project_filter() -> ConversationProjectFilter {
    ConversationProjectFilter::ConversationProjectFilterAll(ConversationProjectFilterAll {
        kind: ConversationProjectFilterAllKind::All,
    })
}

#[tokio::test]
async fn gateway_handshake_without_a_transport_identity_fails_closed() {
    let manager = build_manager("device-no-remote-identity", Vec::new());
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();

    let error = gateway
        .protocol_handshake(handshake_request())
        .await
        .unwrap_err();

    assert_eq!(error.code, "remote_host_identity_unavailable");
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_handshake_returns_the_transport_injected_remote_host_identity() {
    let manager = build_manager("device-handshake", Vec::new());
    let device = RemoteHostIdentity {
        device_id: "device-handshake".to_string(),
        descriptor: device_descriptor("Device device-handshake"),
    };
    let gateway =
        ProviderGatewayService::with_remote_identity(manager.clone(), device.clone()).unwrap();

    let response = gateway
        .protocol_handshake(handshake_request())
        .await
        .unwrap();

    assert_eq!(response.device.name, device.descriptor.device_name);
    assert_eq!(response.device.operating_system, device.descriptor.operating_system);
    assert_eq!(response.device.system_version, device.descriptor.system_version);
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_remote_identity_requires_a_device_id_and_complete_descriptor() {
    let manager = build_manager("device-invalid-remote-identity", Vec::new());
    let error = ProviderGatewayService::with_remote_identity(
        manager.clone(),
        RemoteHostIdentity {
            device_id: String::new(),
            descriptor: device_descriptor("Device device-invalid-remote-identity"),
        },
    )
    .err()
    .expect("expected invalid Gateway identity to be rejected");
    assert_eq!(error.code, "invalid_remote_host_identity");

    let gateway = ProviderGatewayService::with_remote_identity(
        manager.clone(),
        RemoteHostIdentity {
            device_id: "device-invalid-remote-identity".to_string(),
            descriptor: device_descriptor("Device device-invalid-remote-identity"),
        },
    )
    .unwrap();
    drop(gateway);
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_dispatches_event_subscribe_with_the_exact_cursor_boundary() {
    let manager = build_manager("device-subscribe", Vec::new());
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    let after_cursor = gateway.current_event_cursor();

    let response = codepet_gateway_sdk::dispatch(
        &gateway,
        GatewayRequest::EventSubscribe {
            jsonrpc: "2.0".to_string(),
            id: "subscribe-current".to_string(),
            params: EventSubscribeRequest {
                after_cursor: after_cursor.clone(),
            },
        },
    )
    .await;
    let JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected a successful event.subscribe response");
    };
    let result: codepet_gateway_sdk::EventSubscribeResponse = serde_json::from_value(result).unwrap();
    assert_eq!(result.subscribed_after_cursor, after_cursor);

    let response = codepet_gateway_sdk::dispatch(
        &gateway,
        GatewayRequest::EventSubscribe {
            jsonrpc: "2.0".to_string(),
            id: "subscribe-ahead".to_string(),
            params: EventSubscribeRequest {
                after_cursor: "event-00000000000000000001".to_string(),
            },
        },
    )
    .await;
    let JsonRpcResponsePayload::Error { error } = response.response else {
        panic!("expected event.subscribe to reject an unknown future cursor");
    };
    assert_eq!(error.data.unwrap().get("code").unwrap(), "invalid_event_cursor");
    manager.shutdown().await;
}

#[tokio::test]
async fn host_manifest_launches_provider_binary_and_completes_gateway_rpc() {
    let manager = build_manager(
        "device-a",
        vec![plugin("dev.codepet.gateway", &["instance-a1", "instance-a2"])],
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    assert!(gateway.start_event_forwarding());
    let outcomes = manager.start_enabled().await;
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].1.is_ok());

    let providers = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap();
    assert_eq!(providers.providers.len(), 2);
    for provider in &providers.providers {
        assert_eq!(provider.identity.icon.as_deref(), Some("https://example.com/fake.png"));
        assert_eq!(
            provider.identity.default_workspace_root.as_deref(),
            Some("/workspace/fake")
        );
        assert_eq!(provider.runtime.status, codepet_gateway_sdk::ProviderStatus::Ready);
        let described = gateway
            .provider_describe(ProviderDescribeRequest { provider_id: provider.id.clone() })
            .await
            .unwrap();
        assert_eq!(&described.provider, provider);
        assert!(described
            .capabilities
            .methods
            .contains(&codepet_gateway_sdk::GatewayCapability::ConversationGet));
    }

    wait_for_gateway_cursor(&gateway, 8).await;
    let lifecycle_cursor = gateway.current_event_cursor();
    let mut lifecycle_events = gateway
        .subscribe_events(Some(&lifecycle_cursor))
        .unwrap();
    manager
        .stop_instance(&ProviderInstanceRoute {
            device_id: "device-a".to_string(),
            provider_plugin_id: "dev.codepet.gateway".to_string(),
            provider_instance_id: "instance-a1".to_string(),
        })
        .await
        .unwrap();
    let stopped_event = lifecycle_events.next_event().await.unwrap();
    let GatewayEvent::ProviderChanged { params, .. } = stopped_event else {
        panic!("expected one Provider change event");
    };
    assert_eq!(
        params.payload.provider.runtime.status,
        codepet_gateway_sdk::ProviderStatus::Stopped
    );
    assert!(tokio::time::timeout(
        Duration::from_millis(50),
        lifecycle_events.next_event()
    )
    .await
    .is_err());
    manager
        .start_instance(&ProviderInstanceRoute {
            device_id: "device-a".to_string(),
            provider_plugin_id: "dev.codepet.gateway".to_string(),
            provider_instance_id: "instance-a1".to_string(),
        })
        .await
        .unwrap();
    let ready_event = lifecycle_events.next_event().await.unwrap();
    let GatewayEvent::ProviderChanged { params, .. } = ready_event else {
        panic!("expected one Provider change event");
    };
    assert_eq!(params.payload.provider.runtime.status, codepet_gateway_sdk::ProviderStatus::Ready);

    let interaction = gateway
        .conversation_acquire_interaction(GatewayConversationAcquireInteractionRequest {
            conversation: gateway_resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-a1",
                "event-first",
            ),
        })
        .await
        .unwrap();
    assert_eq!(interaction.selection.access_mode_id.as_deref(), Some("workspace-write"));
    assert_eq!(interaction.selection.reasoning_effort_id.as_deref(), Some("high"));
    assert_eq!(interaction.lease_expires_at, Some(2_000));

    let after = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&after)).unwrap();
    let response = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-a1",
                "event-first",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.provider_id,
        "instance-a1"
    );
    assert_eq!(response.items.len(), 2);
    let codepet_gateway_sdk::ConversationItem::MessageConversationItem(user_item) = &response.items[0] else {
        panic!("expected user message item");
    };
    let codepet_gateway_sdk::ContentBlock::TextContentBlock(user_content) = &user_item.contents[0] else {
        panic!("expected user text content");
    };
    let codepet_gateway_sdk::ConversationItem::MessageConversationItem(assistant_item) = &response.items[1] else {
        panic!("expected assistant message item");
    };
    let codepet_gateway_sdk::ContentBlock::TextContentBlock(assistant_content) = &assistant_item.contents[0] else {
        panic!("expected assistant text content");
    };
    assert_eq!(
        user_item.resource.native_resource_id,
        "event-first-user"
    );
    assert_eq!(
        user_content.content_id,
        "event-first-user:input:0"
    );
    assert_eq!(
        assistant_content.content_id,
        "event-first-assistant:text"
    );
    let response = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-a2",
                "event-first",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.provider_id,
        "instance-a2"
    );

    let mut routed_event_cursors = Vec::new();
    let mut routed_instances = Vec::new();
    let mut upserted_content_ids = BTreeMap::new();
    let mut delta_content_ids = BTreeMap::new();
    while routed_event_cursors.len() < 4 {
        let event = tokio::time::timeout(Duration::from_secs(2), events.next_event())
            .await
            .unwrap()
            .unwrap();
        match &event {
            GatewayEvent::ConversationUpserted {
                params,
                ..
            } => {
                routed_instances.push(
                    params.payload.conversation.resource.provider_id.clone(),
                );
                routed_event_cursors.push(event_cursor_sequence(&params.event_cursor));
            }
            GatewayEvent::TurnOutputDelta {
                params,
                ..
            } => {
                routed_instances.push(params.payload.turn.provider_id.clone());
                routed_event_cursors.push(event_cursor_sequence(&params.event_cursor));
                delta_content_ids.insert(
                    params.payload.turn.provider_id.clone(),
                    (params.payload.item_id.clone(), params.payload.content_id.clone()),
                );
            }
            GatewayEvent::ConversationItemUpserted { params, .. } => {
                let Some(codepet_gateway_sdk::ConversationItem::MessageConversationItem(item)) = &params.payload.item else {
                    panic!("expected assistant message upsert");
                };
                let codepet_gateway_sdk::ContentBlock::TextContentBlock(content) = &item.contents[0] else {
                    panic!("expected assistant text upsert");
                };
                upserted_content_ids.insert(
                    item.resource.provider_id.clone(),
                    (item.resource.native_resource_id.clone(), content.content_id.clone()),
                );
            }
            _ => {}
        }
    }
    assert!(routed_event_cursors
        .windows(2)
        .all(|pair| pair[1] > pair[0]));
    assert_eq!(
        routed_instances
            .iter()
            .filter(|instance| instance.as_str() == "instance-a1")
            .count(),
        2
    );
    assert_eq!(upserted_content_ids, delta_content_ids);
    assert_eq!(
        upserted_content_ids.get("instance-a1"),
        Some(&("event-first-assistant".to_string(), "event-first-assistant:text".to_string()))
    );
    assert_eq!(
        routed_instances
            .iter()
            .filter(|instance| instance.as_str() == "instance-a2")
            .count(),
        2
    );

    let wrong_instance = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-missing",
                "conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_instance.code, "unknown_provider");
    let opaque_route = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-b",
                "dev.codepet.gateway",
                "instance-a1",
                "conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(opaque_route.conversation.resource.provider_id, "instance-a1");
    let device_b = build_manager(
        "device-b",
        vec![plugin("dev.codepet.device-b", &["instance-b1"])],
    );
    let gateway_b = Arc::new(ProviderGatewayService::new(device_b.clone()).unwrap());
    gateway_b.start_event_forwarding();
    assert!(device_b.start_enabled().await[0].1.is_ok());
    let response_b = gateway_b
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-b",
                "dev.codepet.device-b",
                "instance-b1",
                "device-b-conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(response_b.conversation.resource.provider_id, "instance-b1");
    manager.shutdown().await;
    device_b.shutdown().await;
}

#[tokio::test]
async fn gateway_release_interaction_rejects_provider_without_capability() {
    let manager = build_manager("device-release", vec![plugin("dev.codepet.release", &["instance-release"])]);
    manager.start_enabled().await;
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    let error = gateway.conversation_release_interaction(codepet_gateway_sdk::ConversationReleaseInteractionRequest {
        conversation: gateway_resource("device-release", "dev.codepet.release", "instance-release", "conversation"),
    }).await.unwrap_err();
    assert_eq!(error.code, "provider_capability_unsupported");
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_resume_returns_history_or_preserves_the_interaction_outcome() {
    let manager = build_manager("device-resume", vec![plugin("dev.codepet.resume", &["instance-resume"])]);
    manager.start_enabled().await;
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    for (native_id, acquired, has_history) in [
        ("resume-ok", true, true),
        ("resume-denied", false, false),
        ("response-wrong-native", true, false),
    ] {
        let result = gateway.conversation_resume(codepet_gateway_sdk::ConversationResumeRequest { force: None,
            conversation: gateway_resource("device-resume", "dev.codepet.resume", "instance-resume", native_id),
            limit: Some(20),
        }).await.unwrap();
        assert_eq!(result.interaction_acquired, acquired);
        assert_eq!(result.interaction.is_some(), acquired);
        assert_eq!(result.history.is_some(), has_history);
        if !acquired {
            assert_eq!(result.interaction_error.unwrap().code, "conversation_write_conflict");
            assert!(result.history_error.is_none());
        } else if !has_history {
            assert_eq!(result.history_error.unwrap().code, "provider_resource_identity_mismatch");
        } else {
            let history = result.history.unwrap();
            assert_eq!(history.conversation.resource.native_resource_id, native_id);
            assert!(!history.snapshot_cursor.is_empty());
        }
    }
    for (force, id, acquired, error) in [
        (None, "resume-force", false, Some("conversation_write_conflict")),
        (Some(false), "resume-force", false, Some("conversation_write_conflict")),
        (Some(true), "resume-force", true, None),
        (Some(true), "resume-force-active", false, Some("conversation_active")),
    ] {
        let result = gateway.conversation_resume(codepet_gateway_sdk::ConversationResumeRequest {
            conversation: gateway_resource("device-resume", "dev.codepet.resume", "instance-resume", id),
            force, limit: Some(20),
        }).await.unwrap();
        assert_eq!(result.interaction_acquired, acquired);
        assert_eq!(result.interaction_error.as_ref().map(|e| e.code.as_str()), error);
        assert_eq!(result.history.is_some(), acquired);
    }
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_routes_project_crud_filters_and_project_owned_conversation_create() {
    let manager = build_manager(
        "device-project",
        vec![plugin("dev.codepet.project", &["instance-project"])],
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    assert!(gateway.start_event_forwarding());
    assert!(manager.start_enabled().await[0].1.is_ok());
    let route = ProviderInstanceRoute {
        device_id: "device-project".to_string(),
        provider_plugin_id: "dev.codepet.project".to_string(),
        provider_instance_id: "instance-project".to_string(),
    };

    let providers = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap();
    let described = gateway
        .provider_describe(ProviderDescribeRequest { provider_id: providers.providers[0].id.clone() })
        .await
        .unwrap();
    assert!(described.capabilities
        .methods
        .contains(&codepet_gateway_sdk::GatewayCapability::ProjectList));

    let snapshot_cursor = gateway.current_event_cursor();
    let listed = gateway
        .project_list(ProjectListRequest {
            provider_id: route.provider_instance_id.clone(),
            cursor: None,
            limit: Some(10),
        })
        .await
        .unwrap();
    assert_eq!(listed.snapshot_cursor, snapshot_cursor);
    assert_eq!(listed.page_info.next_cursor.as_deref(), Some("project-next"));
    assert_eq!(listed.projects[0].metadata["fixture"], "true");
    let listed_project = listed.projects[0].resource.clone();

    let fetched = gateway
        .project_get(ProjectGetRequest {
            project: listed_project.clone(),
        })
        .await
        .unwrap();
    assert_eq!(fetched.project.resource, listed_project);

    let after_project_event = gateway.current_event_cursor();
    let mut project_events = gateway.subscribe_events(Some(&after_project_event)).unwrap();
    gateway
        .project_get(ProjectGetRequest {
            project: gateway_resource(
                "device-project",
                "dev.codepet.project",
                "instance-project",
                "event-project",
            ),
        })
        .await
        .unwrap();
    let event = project_events.next_event().await.unwrap();
    assert!(matches!(
        event,
        GatewayEvent::ProjectChanged { params, .. }
            if params.payload.project.native_resource_id == "event-project"
                && params.payload.change_type == codepet_gateway_sdk::ProjectChangeType::Updated
    ));

    let created = gateway
        .project_create(ProjectCreateRequest {
            provider_id: route.provider_instance_id.clone(),
            idempotency_key: "create-project".to_string(),
            name: "Created Project".to_string(),
            roots: vec![ProjectRoot {
                path: "/fixture/created".to_string(),
            }],
            metadata: BTreeMap::from([("owner".to_string(), "gateway".to_string())]),
        })
        .await
        .unwrap();
    assert_eq!(created.project.roots[0].path, "/fixture/created");
    assert_eq!(created.project.metadata["owner"], "gateway");

    let updated = gateway
        .project_update(ProjectUpdateRequest {
            project: created.project.resource.clone(),
            name: Some("Renamed Project".to_string()),
            roots: None,
            metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.project.name, "Renamed Project");

    let conversation = gateway
        .conversation_create(GatewayConversationCreateRequest {
            provider_id: route.provider_instance_id.clone(),
            project: Some(listed_project.clone()),
            title: None,
            permission_level: "workspace-write".to_string(),
            model: None,
            reasoning_effort: None,
            workspace_root: Some("/fixture/project".to_string()),
            workspace_mode: None,
        })
        .await
        .unwrap();
    assert_eq!(conversation.conversation.project.as_ref(), Some(&listed_project));

    let filtered = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: route.provider_instance_id.clone(),
            cursor: None,
            limit: Some(10),
            project_filter: ConversationProjectFilter::ConversationProjectFilterProject(
                ConversationProjectFilterProject {
                    kind: ConversationProjectFilterProjectKind::Project,
                    project: listed_project.clone(),
                },
            ),
        })
        .await
        .unwrap();
    assert_eq!(filtered.conversations[0].project.as_ref(), Some(&listed_project));

    gateway
        .project_delete(ProjectDeleteRequest {
            project: created.project.resource,
        })
        .await
        .unwrap();
    manager.shutdown().await;
}

#[tokio::test]
async fn conversation_snapshot_cursors_precede_events_emitted_during_provider_queries() {
    let release_directory = tempfile::tempdir().unwrap();
    let release_marker = release_directory.path().join("release-snapshot-query");
    let mut descriptor = plugin("dev.codepet.snapshot", &["instance-snapshot"]);
    descriptor.env.insert(
        "CODEPET_FAKE_SNAPSHOT_RELEASE_MARKER".to_string(),
        release_marker.display().to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_CONVERSATION_LIST_SNAPSHOT_RACE".to_string(),
        "1".to_string(),
    );
    let manager = build_manager("device-snapshot", vec![descriptor]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    wait_for_gateway_cursor(&gateway, 4).await;

    let before_list = gateway.current_event_cursor();
    let mut list_events = gateway.subscribe_events(Some(&before_list)).unwrap();
    let list_gateway = gateway.clone();
    let list_query = tokio::spawn(async move {
        list_gateway
            .conversation_list(GatewayConversationListRequest {
                provider_id: "instance-snapshot".to_string(),
                cursor: None,
                limit: Some(10),
                project_filter: all_project_filter(),
            })
            .await
    });
    let list_event_cursor = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let GatewayEvent::ConversationUpserted {
                params,
                ..
            } = list_events.next_event().await.unwrap()
            {
                if params.payload.conversation.resource.native_resource_id
                    == "conversation-list-event-first"
                {
                    return params.event_cursor;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(!list_query.is_finished());
    std::fs::write(&release_marker, b"release\n").unwrap();

    let list = tokio::time::timeout(Duration::from_secs(2), list_query)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(list.snapshot_cursor, before_list);
    assert!(
        event_cursor_sequence(&list.snapshot_cursor)
            < event_cursor_sequence(&list_event_cursor)
    );
    std::fs::remove_file(&release_marker).unwrap();

    let after_cursor = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&after_cursor)).unwrap();
    let query_gateway = gateway.clone();
    let query = tokio::spawn(async move {
        query_gateway
            .conversation_get(GatewayConversationGetRequest {
                conversation: gateway_resource(
                    "device-snapshot",
                    "dev.codepet.snapshot",
                    "instance-snapshot",
                    "snapshot-race",
                ),
                cursor: None,
                limit: None,
            })
            .await
    });

    let event_cursor = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let GatewayEvent::ConversationUpserted {
                params,
                ..
            } = events.next_event().await.unwrap()
            {
                if params.payload.conversation.resource.native_resource_id
                    == "snapshot-race"
                {
                    return params.event_cursor;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(!query.is_finished());
    std::fs::write(&release_marker, b"release\n").unwrap();

    let response = tokio::time::timeout(Duration::from_secs(2), query)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        event_cursor_sequence(&response.snapshot_cursor)
            < event_cursor_sequence(&event_cursor)
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_reports_replay_and_live_subscription_gaps() {
    let manager = build_manager_with_event_capacity(
        "device-gap",
        vec![plugin("dev.codepet.gap", &["instance-gap"])],
        2,
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    wait_for_gateway_cursor(&gateway, 4).await;

    let replay_error = gateway
        .replay_events(Some("event-00000000000000000000"))
        .unwrap_err();
    assert_eq!(replay_error.code, "event_replay_unavailable");

    let cursor = gateway.current_event_cursor();
    let starting_sequence = event_cursor_sequence(&cursor);
    let mut subscription = gateway.subscribe_events(Some(&cursor)).unwrap();
    let route = ProviderInstanceRoute {
        device_id: "device-gap".to_string(),
        provider_plugin_id: "dev.codepet.gap".to_string(),
        provider_instance_id: "instance-gap".to_string(),
    };
    for _ in 0..3 {
        manager.stop_instance(&route).await.unwrap();
        manager.start_instance(&route).await.unwrap();
    }
    wait_for_gateway_cursor(&gateway, starting_sequence + 6).await;
    let live_error = subscription.next_event().await.unwrap_err();
    assert_eq!(live_error.code, "gateway_event_subscription_lagged");
    manager.shutdown().await;
    let restart_error = manager.start_plugin("dev.codepet.gap").await.unwrap_err();
    assert_eq!(restart_error.code, "provider_manager_shutting_down");
}

#[tokio::test]
async fn shutdown_during_parallel_initialize_stops_processes_and_blocks_restart() {
    let directory = tempfile::tempdir().unwrap();
    let initialize_marker = directory.path().join("initialize-a");
    let first_pid = directory.path().join("pid-a");
    let second_pid = directory.path().join("pid-b");
    let mut first = plugin("dev.codepet.a-delayed", &["instance-a"]);
    first.env.insert(
        "CODEPET_FAKE_INITIALIZE_DELAY_MS".to_string(),
        "300".to_string(),
    );
    first.env.insert(
        "CODEPET_FAKE_INITIALIZE_MARKER".to_string(),
        initialize_marker.display().to_string(),
    );
    first.env.insert(
        "CODEPET_FAKE_PID_MARKER".to_string(),
        first_pid.display().to_string(),
    );
    let mut second = plugin("dev.codepet.b-late", &["instance-b"]);
    second.env.insert(
        "CODEPET_FAKE_PID_MARKER".to_string(),
        second_pid.display().to_string(),
    );
    let manager = build_manager("device-start-shutdown", vec![first, second]);
    let startup_manager = manager.clone();
    let startup = tokio::spawn(async move { startup_manager.start_enabled().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !initialize_marker.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let shutdown = manager.shutdown().await;
    assert!(shutdown.iter().all(|(_, outcome)| outcome.is_ok()));
    let startup = startup.await.unwrap();
    assert!(startup.iter().find(|(id, _)| id == "dev.codepet.a-delayed").unwrap().1.is_err());
    // B may finish before shutdown: batch startup no longer waits for A's Initialize.
    assert!(manager.snapshots().await.iter().all(|snapshot| snapshot.state == PluginRuntimeState::Stopped));
    let restart = manager
        .start_plugin("dev.codepet.b-late")
        .await
        .unwrap_err();
    assert_eq!(restart.code, "provider_manager_shutting_down");
    #[cfg(unix)]
    {
        assert!(!pid_is_alive(&first_pid));
        if second_pid.exists() { assert!(!pid_is_alive(&second_pid)); }
    }
}

#[cfg(unix)]
fn pid_is_alive(path: &std::path::Path) -> bool {
    extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    let pid = std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    unsafe { kill(pid, 0) == 0 }
}

async fn wait_for_gateway_cursor(gateway: &ProviderGatewayService, sequence: u64) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if event_cursor_sequence(&gateway.current_event_cursor()) >= sequence {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn conversation_search_is_route_scoped_and_preserves_pagination_and_snapshot_cursor() {
    let manager = build_manager(
        "device-search",
        vec![plugin("dev.codepet.search", &["instance-search"])],
    );
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    assert!(manager.start_enabled().await[0].1.is_ok());

    let providers = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap();
    let described = gateway
        .provider_describe(ProviderDescribeRequest { provider_id: providers.providers[0].id.clone() })
        .await
        .unwrap();
    assert!(described.capabilities
        .methods
        .contains(&codepet_gateway_sdk::GatewayCapability::ConversationSearch));

    let snapshot_cursor = gateway.current_event_cursor();
    let response = gateway
        .conversation_search(GatewayConversationSearchRequest {
            provider_id: "instance-search".to_string(),
            search_term: "gateway protocol".to_string(),
            cursor: Some("search-cursor".to_string()),
            limit: Some(7),
        })
        .await
        .unwrap();
    assert_eq!(response.snapshot_cursor, snapshot_cursor);
    assert_eq!(response.page_info.next_cursor.as_deref(), Some("search-next"));
    assert_eq!(
        response.conversations[0].resource.native_resource_id,
        "conversation-search"
    );

    let empty = gateway
        .conversation_search(GatewayConversationSearchRequest {
            provider_id: "instance-search".to_string(),
            search_term: String::new(),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(empty.code, "invalid_request");
    manager.shutdown().await;
}

#[tokio::test]
async fn handshake_list_and_ping_hide_undetected_builtins_but_keep_custom_providers() {
    let manager = build_manager("device-visible", vec![
        plugin("dev.codepet.codex", &["codex"]),
        plugin("dev.codepet.claude", &["claude"]),
        plugin("dev.codepet.opencode", &["opencode"]),
        plugin("custom.provider", &["custom"]),
    ]);
    let gateway = Arc::new(ProviderGatewayService::with_remote_identity(manager.clone(), RemoteHostIdentity {
        device_id:"device-visible".into(), descriptor:device_descriptor("Visible host"),
    }).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await.iter().all(|(_, result)| result.is_ok()));
    // Fixture binaries are online, but do not report installed built-in runtimes.
    let handshake = gateway.protocol_handshake(handshake_request()).await.unwrap();
    let list = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    let ping = gateway.protocol_ping(codepet_gateway_sdk::PingRequest { sequence:1 }).await.unwrap();
    for providers in [&handshake.providers, &list.providers, &ping.providers] {
        assert_eq!(providers.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["custom"]);
    }
    let cursor = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&cursor)).unwrap();
    manager.stop_instance(&codepet_provider_sdk::ProviderInstanceRoute {
        device_id:"device-visible".into(), provider_plugin_id:"dev.codepet.claude".into(), provider_instance_id:"claude".into(),
    }).await.unwrap();
    // No hidden built-in may be reintroduced through a lifecycle notification.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(80);
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.next_event()).await {
        if let GatewayEvent::ProviderChanged { params, .. } = event {
            assert_eq!(params.payload.provider.id, "custom");
        }
    }
    manager.shutdown().await;
}

#[tokio::test]
async fn failed_turn_error_survives_provider_event_and_gateway_history_without_items() {
    let manager = build_manager("device-failure", vec![plugin("dev.codepet.failure", &["failure-instance"])]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    let conversation = gateway_resource("device-failure", "dev.codepet.failure", "failure-instance", "failure-conversation");
    let mut events = gateway.subscribe_events(Some(&gateway.current_event_cursor())).unwrap();
    let receipt = gateway.turn_send(GatewayTurnSendRequest {
        conversation: conversation.clone(), client_request_id:"quota-input".into(),
        capability_revision:"fake-capabilities-v1".into(),
        input:GatewayTurnInput { kind:GatewayTurnInputKind::Text, text:"fail with quota".into() },
        selection:GatewayTurnSelection { access_mode_id:None, reasoning_effort_id:None, model:None },
    }).await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let GatewayEvent::TurnUpserted { params, .. } = events.next_event().await.unwrap() {
                let turn = params.payload.turn;
                if turn.resource == receipt.turn.resource && turn.status == codepet_gateway_sdk::TurnStatus::Failed { break turn; }
            }
        }
    }).await.unwrap();
    assert_eq!(terminal.error.as_ref().unwrap().code,"quota_exceeded");
    let history = gateway.conversation_get(GatewayConversationGetRequest { conversation, cursor:None,limit:Some(10) }).await.unwrap();
    assert!(history.items.is_empty());
    assert_eq!(history.turns.unwrap(),vec![terminal]);
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_provider_initialize_does_not_delay_starting_another_provider() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("slow-initialize");
    let mut slow = plugin("a-slow-start", &["a-slow-instance"]);
    slow.env
        .insert("CODEPET_FAKE_INITIALIZE_DELAY_MS".into(), "1500".into());
    slow.env.insert(
        "CODEPET_FAKE_INITIALIZE_MARKER".into(),
        marker.to_string_lossy().into_owned(),
    );
    let fast = plugin("b-fast-start", &["b-fast-instance"]);
    let manager = build_manager("parallel-start", vec![slow, fast]);
    let running = manager.clone();
    let startup = tokio::spawn(async move { running.start_enabled().await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        loop {
            if manager.snapshot("b-fast-start").await.unwrap().state == PluginRuntimeState::Ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fast provider must initialize while another provider is delayed");
    assert_eq!(
        manager.snapshot("a-slow-start").await.unwrap().state,
        PluginRuntimeState::Starting
    );
    assert!(startup
        .await
        .unwrap()
        .into_iter()
        .all(|(_, result)| result.is_ok()));
    manager.shutdown().await;
}
