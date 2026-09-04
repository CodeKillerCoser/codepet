use codepet_gateway_sdk::{
    ConversationAcquireInteractionRequest as GatewayConversationAcquireInteractionRequest,
    ConversationCreateRequest as GatewayConversationCreateRequest,
    ConversationGetRequest as GatewayConversationGetRequest,
    ConversationListRequest as GatewayConversationListRequest,
    ConversationProjectFilter, ConversationProjectFilterAll, ConversationProjectFilterAllKind,
    ConversationProjectFilterProject,
    ConversationProjectFilterProjectKind,
    ConversationSearchRequest as GatewayConversationSearchRequest, DeviceDescriptor,
    EventSubscribeRequest, HandshakeRequest,
    FlatModelCatalogKind, FlatModelSelection,
    GroupedModelCatalogKind, GroupedModelSelection, ModelCatalog, ModelSelection,
    JsonRpcResponsePayload, ProtocolEvent as GatewayEvent, ProtocolRequest as GatewayRequest,
    ProjectCreateRequest, ProjectDeleteRequest, ProjectGetRequest, ProjectListRequest, ProjectRoot,
    ProjectUpdateRequest,
    ProtocolServer as GatewayProtocolServer, ProviderDescribeRequest, ProviderListRequest,
    TurnInput as GatewayTurnInput, TurnInputKind as GatewayTurnInputKind,
    TurnSelection as GatewayTurnSelection, TurnSendRequest as GatewayTurnSendRequest, VersionRange,
};
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor, PluginInstanceConfig,
    PluginManager, PluginManagerConfig, PluginProcessOptions, PluginRuntimeState,
    ProviderGatewayService, ProviderInstanceRegistry, RemoteHostIdentity,
};
use codepet_provider_sdk::{
    ConversationGetRequest, JsonObject, ProviderInstanceRoute, RoutedResourceId, TurnInput,
    TurnInputKind, TurnSelection, TurnStartRequest,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn plugin(plugin_id: &str, instances: &[&str]) -> PluginDescriptor {
    PluginDescriptor {
        plugin_id: plugin_id.to_string(),
        display_name: format!("Fake {plugin_id}"),
        icon: Some("https://example.com/fake.png".to_string()),
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
    let directory = tempfile::tempdir().unwrap();
    let device_path = directory.path().join("device.json");
    std::fs::write(
        &device_path,
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "deviceId": device_id,
            "displayName": format!("Device {device_id}"),
            "createdAt": 1
        }))
        .unwrap(),
    )
    .unwrap();
    let device = DeviceRegistry::open(device_path, format!("Device {device_id}")).unwrap();
    let plugin_directory = directory.path().join("providers");
    for (index, descriptor) in descriptors.into_iter().enumerate() {
        let directory = plugin_directory.join(index.to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let mut manifest = serde_json::to_value(descriptor).unwrap();
        manifest
            .as_object_mut()
            .unwrap()
            .insert("manifestVersion".to_string(), serde_json::json!(1));
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
        directory.path().join("instances.json"),
        device_id.to_string(),
    )
    .unwrap();
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
) -> RoutedResourceId {
    RoutedResourceId {
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
        supported_versions: VersionRange {
            min_version: codepet_gateway_sdk::PROTOCOL_VERSION,
            max_version: codepet_gateway_sdk::PROTOCOL_VERSION,
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
    assert_eq!(
        response.items[0].resource.native_resource_id,
        "event-first-user"
    );
    assert_eq!(
        response.items[0].contents[0].content_id,
        "event-first-user:input:0"
    );
    assert_eq!(
        response.items[1].contents[0].content_id,
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
                    == "conversation-event-first"
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
async fn version_negotiation_rejects_a_plugin_with_an_inconsistent_reported_range() {
    let mut descriptor = plugin("dev.codepet.version-mismatch", &["instance-version"]);
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MIN_VERSION".to_string(),
        "2".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MAX_VERSION".to_string(),
        "2".to_string(),
    );
    let manager = build_manager("device-version", vec![descriptor]);

    let outcomes = manager.start_enabled().await;
    let error = outcomes[0].1.as_ref().unwrap_err();
    assert_eq!(error.code, "provider_protocol_version_mismatch");
    let snapshot = manager
        .snapshot("dev.codepet.version-mismatch")
        .await
        .unwrap();
    assert_eq!(snapshot.state, PluginRuntimeState::Crashed);
    assert!(snapshot.process_exit.is_some());
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    let providers = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap();
    assert_eq!(
        providers.providers[0].runtime.status,
        codepet_gateway_sdk::ProviderStatus::Error
    );
}

#[tokio::test]
async fn explicit_restart_continues_after_graceful_stop_error_when_process_was_killed() {
    let mut descriptor = plugin("dev.codepet.restart", &["instance-restart"]);
    descriptor.instances[0]
        .settings
        .insert("fixtureRevision".to_string(), serde_json::json!("before-restart"));
    descriptor.env.insert(
        "CODEPET_FAKE_SHUTDOWN_RESPONSE_DELAY_MS".to_string(),
        "200".to_string(),
    );
    let manager = build_manager_with_options(
        "device-restart",
        vec![descriptor],
        PluginManagerConfig::default().event_capacity,
        PluginProcessOptions {
            request_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_millis(20),
            ..PluginProcessOptions::default()
        },
    );
    let initial_start = manager.start_enabled().await;
    initial_start[0].1.as_ref().unwrap();

    let before_restart = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-restart",
                "dev.codepet.restart",
                "instance-restart",
                "before-restart",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(
        before_restart.conversation.preview.as_deref(),
        Some("before-restart")
    );
    assert_eq!(
        manager
            .replace_instance_setting(
                "dev.codepet.restart",
                "fake",
                "fixtureRevision",
                Some(serde_json::json!("after-restart")),
            )
            .await
            .unwrap(),
        1
    );

    manager.restart_plugin("dev.codepet.restart").await.unwrap();

    assert_eq!(
        manager
            .snapshot("dev.codepet.restart")
            .await
            .unwrap()
            .state,
        PluginRuntimeState::Ready
    );
    let response = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-restart",
                "dev.codepet.restart",
                "instance-restart",
                "after-restart",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.native_resource_id,
        "after-restart"
    );
    assert_eq!(
        response.conversation.preview.as_deref(),
        Some("after-restart")
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn failed_instance_start_is_unavailable_instead_of_stuck_connecting() {
    let marker_directory = tempfile::tempdir().unwrap();
    let start_marker = marker_directory.path().join("instance-starts");
    let mut descriptor = plugin(
        "dev.codepet.start-failure",
        &["instance-start-failure", "instance-start-healthy"],
    );
    descriptor.env.insert(
        "CODEPET_FAKE_INSTANCE_START_ERROR_ID".to_string(),
        "instance-start-failure".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_INSTANCE_START_MARKER".to_string(),
        start_marker.display().to_string(),
    );
    let manager = build_manager("device-start-failure", vec![descriptor]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();

    let outcomes = manager.start_enabled().await;
    assert!(outcomes[0].1.is_err());
    let providers = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap();
    let failed = providers
        .providers
        .iter()
        .find(|provider| provider.id == "instance-start-failure")
        .unwrap();
    let healthy = providers
        .providers
        .iter()
        .find(|provider| provider.id == "instance-start-healthy")
        .unwrap();
    assert_eq!(failed.runtime.status, codepet_gateway_sdk::ProviderStatus::Unavailable);
    assert_eq!(healthy.runtime.status, codepet_gateway_sdk::ProviderStatus::Ready);
    let initial_snapshot = manager
        .snapshot("dev.codepet.start-failure")
        .await
        .unwrap();
    assert_eq!(initial_snapshot.state, PluginRuntimeState::Ready);

    let failed_route = ProviderInstanceRoute {
        device_id: "device-start-failure".to_string(),
        provider_plugin_id: "dev.codepet.start-failure".to_string(),
        provider_instance_id: "instance-start-failure".to_string(),
    };
    for _ in 0..2 {
        let error = gateway
            .conversation_list(GatewayConversationListRequest {
                provider_id: failed_route.provider_instance_id.clone(),
                cursor: None,
                limit: Some(10),
                project_filter: all_project_filter(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "fixture_instance_start_failed");
    }
    let healthy_history = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: "instance-start-healthy".to_string(),
            cursor: None,
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap();
    assert_eq!(healthy_history.conversations.len(), 1);
    let final_snapshot = manager
        .snapshot("dev.codepet.start-failure")
        .await
        .unwrap();
    assert_eq!(final_snapshot.generation, initial_snapshot.generation);
    let starts = std::fs::read_to_string(&start_marker).unwrap();
    assert_eq!(starts.lines().count(), 2);
    assert_eq!(
        starts
            .lines()
            .filter(|instance| *instance == "instance-start-failure")
            .count(),
        1
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
async fn shutdown_during_delayed_initialize_prevents_late_plugin_spawn() {
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
    assert!(startup.iter().all(|(_, outcome)| outcome.is_err()));
    assert!(!second_pid.exists(), "shutdown gate must prevent the later spawn");
    let restart = manager
        .start_plugin("dev.codepet.b-late")
        .await
        .unwrap_err();
    assert_eq!(restart.code, "provider_manager_shutting_down");
    #[cfg(unix)]
    assert!(!pid_is_alive(&first_pid));
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
async fn gateway_turn_send_validates_controls_and_deduplicates_client_requests() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("turn-starts.txt");
    let mut descriptor = plugin("dev.codepet.turn-send", &["instance-turn-send"]);
    descriptor.env.insert(
        "CODEPET_FAKE_TURN_START_MARKER".to_string(),
        marker.to_string_lossy().to_string(),
    );
    let manager = build_manager("device-turn-send", vec![descriptor]);
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    assert!(manager.start_enabled().await[0].1.is_ok());

    let provider = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap()
        .providers
        .remove(0);
    assert_eq!(provider.runtime.version.as_deref(), Some("1.0.0-fixture"));
    assert_eq!(provider.runtime.executable_path.as_deref(), Some("/fixture/fake-harness"));
    assert_eq!(
        provider.runtime.authentication.as_ref().unwrap().status,
        codepet_gateway_sdk::ProviderAuthenticationStatus::SignedIn
    );
    let usage = provider.runtime.usage.as_ref().unwrap();
    assert_eq!(usage.display_text, "Fixture usage 42%");
    let details = usage.details.as_ref().unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].data.get("usedPercent"), Some(&serde_json::json!(42)));
    assert!(!details[0].data.contains_key("accessToken"));
    assert_eq!(details[0].data["nested"], serde_json::json!({"safe": true}));
    assert_eq!(provider.capabilities.revision, "fake-capabilities-v1");
    let described = gateway
        .provider_describe(ProviderDescribeRequest { provider_id: provider.id.clone() })
        .await
        .unwrap();
    let Some(turn_send) = described.capabilities.turn_send.as_ref() else {
        panic!("expected turn.send controls");
    };
    assert!(matches!(turn_send.model_catalog, Some(ModelCatalog::FlatModelCatalog(_))));

    let conversation = gateway_resource(
        "device-turn-send",
        "dev.codepet.turn-send",
        "instance-turn-send",
        "conversation-turn-send",
    );
    let request = GatewayTurnSendRequest {
        conversation: conversation.clone(),
        client_request_id: "remote-request-1".to_string(),
        capability_revision: "fake-capabilities-v1".to_string(),
        input: GatewayTurnInput {
            kind: GatewayTurnInputKind::Text,
            text: "hello".to_string(),
        },
        selection: GatewayTurnSelection {
            access_mode_id: Some("workspace-write".to_string()),
            reasoning_effort_id: Some("medium".to_string()),
            model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                kind: FlatModelCatalogKind::Flat,
                model_id: "fake-model".to_string(),
            })),
        },
    };
    let accepted = gateway
        .turn_send_for_caller_scope("remote-client-a", request.clone())
        .await
        .unwrap();
    assert!(accepted.accepted);
    let user_item = accepted.user_item.as_ref().unwrap();
    assert_eq!(user_item.turn, accepted.turn.resource);
    assert_eq!(user_item.conversation, conversation);
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", request.clone())
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-b", request.clone())
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);

    let mut conflict = request.clone();
    conflict.input.text = "different".to_string();
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", conflict)
            .await
            .unwrap_err()
            .code,
        "client_request_conflict"
    );

    let mut stale = request.clone();
    stale.client_request_id = "remote-request-stale".to_string();
    stale.capability_revision = "stale".to_string();
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", stale)
            .await
            .unwrap_err()
            .code,
        "stale_capability_revision"
    );

    let mut unknown = request.clone();
    unknown.client_request_id = "remote-request-unknown".to_string();
    unknown.selection.model = Some(ModelSelection::FlatModelSelection(FlatModelSelection {
        kind: FlatModelCatalogKind::Flat,
        model_id: "missing-model".to_string(),
    }));
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", unknown)
            .await
            .unwrap_err()
            .code,
        "unknown_turn_selection"
    );

    let mut wrong_shape = request;
    wrong_shape.client_request_id = "remote-request-shape".to_string();
    wrong_shape.selection.model = Some(ModelSelection::GroupedModelSelection(
        GroupedModelSelection {
            kind: GroupedModelCatalogKind::Grouped,
            provider_id: "openai".to_string(),
            model_id: "fake-model".to_string(),
        },
    ));
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", wrong_shape)
            .await
            .unwrap_err()
            .code,
        "turn_model_shape_mismatch"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn resource_identity_and_unknown_provider_fail_closed() {
    let manager = build_manager(
        "device-identity",
        vec![plugin("dev.codepet.identity", &["instance-identity"])],
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());

    for native_id in [
        "response-wrong-native",
        "response-empty-native",
        "response-wrong-route",
    ] {
        let error = manager
            .conversation_get(ConversationGetRequest {
                conversation: resource(
                    "device-identity",
                    "dev.codepet.identity",
                    "instance-identity",
                    native_id,
                ),
                cursor: None,
                limit: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error.code.as_str(),
            "provider_resource_identity_mismatch"
                | "invalid_provider_resource"
                | "provider_resource_route_mismatch"
        ));
    }
    let wrong_history_conversation = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-item-conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_history_conversation.code,
        "provider_resource_identity_mismatch"
    );
    let empty_native = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(empty_native.code, "invalid_provider_resource");
    let empty_device = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "",
                "dev.codepet.identity",
                "instance-identity",
                "conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(empty_device.code, "invalid_provider_route");
    let wrong_plugin = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.other",
                "instance-identity",
                "conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_plugin.code, "provider_instance_plugin_mismatch");

    let wrong_conversation = manager
        .turn_start(TurnStartRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-conversation",
            ),
            client_request_id: "message-1".to_string(),
            capability_revision: "fake-capabilities-v1".to_string(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "hello".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_conversation.code, "provider_resource_identity_mismatch");

    let wrong_user_item_conversation = manager
        .turn_start(TurnStartRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-user-item-conversation",
            ),
            client_request_id: "message-wrong-user-item".to_string(),
            capability_revision: "fake-capabilities-v1".to_string(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "hello".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_user_item_conversation.code,
        "provider_resource_identity_mismatch"
    );

    let wrong_gateway_conversation = gateway
        .turn_send(GatewayTurnSendRequest {
            conversation: gateway_resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-conversation",
            ),
            client_request_id: "message-gateway".to_string(),
            capability_revision: "fake-capabilities-v1".to_string(),
            input: GatewayTurnInput {
                kind: GatewayTurnInputKind::Text,
                text: "continue".to_string(),
            },
            selection: GatewayTurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_gateway_conversation.code,
        "provider_resource_identity_mismatch"
    );

    let unknown_provider = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: "missing-provider".to_string(),
            cursor: Some("provider-cursor".to_string()),
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        unknown_provider.code,
        "unknown_provider"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn a_crashed_plugin_does_not_change_another_plugin_or_instance_route() {
    let manager = build_manager(
        "device-isolation",
        vec![
            plugin("dev.codepet.alpha", &["instance-alpha"]),
            plugin("dev.codepet.beta", &["instance-beta"]),
        ],
    );
    let outcomes = manager.start_enabled().await;
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|(_, outcome)| outcome.is_ok()));

    let alpha_error = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-isolation",
                "dev.codepet.alpha",
                "instance-alpha",
                "crash",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        alpha_error.code.as_str(),
        "provider_stdout_eof" | "provider_process_exited"
    ));

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if manager
                .snapshot("dev.codepet.alpha")
                .await
                .unwrap()
                .state
                == PluginRuntimeState::Crashed
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let beta = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-isolation",
                "dev.codepet.beta",
                "instance-beta",
                "healthy",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(beta.conversation.resource.provider_instance_id, "instance-beta");
    assert_eq!(
        manager
            .snapshot("dev.codepet.beta")
            .await
            .unwrap()
            .state,
        PluginRuntimeState::Ready
    );
    assert!(manager
        .snapshot("dev.codepet.alpha")
        .await
        .unwrap()
        .stderr_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.line.contains("fixture crash requested")));

    manager.shutdown().await;
}

#[tokio::test]
async fn provider_scoped_history_starts_on_demand_and_recovers_the_crashed_plugin_generation() {
    let manager = build_manager(
        "device-history-recovery",
        vec![plugin("dev.codepet.history", &["instance-history"])],
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    assert!(gateway.start_event_forwarding());
    let route = ProviderInstanceRoute {
        device_id: "device-history-recovery".to_string(),
        provider_plugin_id: "dev.codepet.history".to_string(),
        provider_instance_id: "instance-history".to_string(),
    };

    let listed = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: route.provider_instance_id.clone(),
            cursor: None,
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap();
    assert_eq!(listed.conversations.len(), 1);
    assert_eq!(
        listed.conversations[0].resource.provider_id,
        route.provider_instance_id
    );
    let initial_generation = manager
        .snapshot("dev.codepet.history")
        .await
        .unwrap()
        .generation;

    let crashed = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-history-recovery",
                "dev.codepet.history",
                "instance-history",
                "crash",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        crashed.code.as_str(),
        "provider_stdout_eof" | "provider_process_exited"
    ));

    let recovered_list = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: route.provider_instance_id.clone(),
            cursor: None,
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap();
    assert_eq!(recovered_list.conversations.len(), 1);
    let recovered = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: gateway_resource(
                "device-history-recovery",
                "dev.codepet.history",
                "instance-history",
                "healthy-after-restart",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(
        recovered.conversation.resource.provider_id,
        route.provider_instance_id
    );
    assert!(
        manager
            .snapshot("dev.codepet.history")
            .await
            .unwrap()
            .generation
            > initial_generation
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn provider_scoped_history_isolates_healthy_and_failed_providers() {
    let mut failing = plugin("dev.codepet.aggregate-failing", &["instance-aggregate-failing"]);
    failing.env.insert(
        "CODEPET_FAKE_INSTANCE_START_ERROR_ID".to_string(),
        "instance-aggregate-failing".to_string(),
    );
    let manager = build_manager(
        "device-aggregate-partial",
        vec![
            failing,
            plugin("dev.codepet.aggregate-healthy", &["instance-aggregate-healthy"]),
        ],
    );
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();

    let response = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: "instance-aggregate-healthy".to_string(),
            cursor: None,
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap();
    assert_eq!(response.conversations.len(), 1);
    assert_eq!(
        response.conversations[0].resource.provider_id,
        "instance-aggregate-healthy"
    );
    manager.shutdown().await;

    let mut only_failing = plugin(
        "dev.codepet.aggregate-all-failing",
        &["instance-aggregate-all-failing"],
    );
    only_failing.env.insert(
        "CODEPET_FAKE_INSTANCE_START_ERROR_ID".to_string(),
        "instance-aggregate-all-failing".to_string(),
    );
    let all_failed_manager = build_manager("device-aggregate-all-failed", vec![only_failing]);
    let all_failed_gateway = ProviderGatewayService::new(all_failed_manager.clone()).unwrap();
    let error = all_failed_gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: "instance-aggregate-all-failing".to_string(),
            cursor: None,
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "fixture_instance_start_failed");
    all_failed_manager.shutdown().await;
}

#[tokio::test]
async fn historical_recovery_fails_closed_for_disabled_plugin_and_instance() {
    let mut disabled_plugin = plugin(
        "dev.codepet.disabled-plugin",
        &["instance-disabled-plugin"],
    );
    disabled_plugin.enabled = false;
    let mut disabled_instance = plugin(
        "dev.codepet.disabled-instance",
        &["instance-disabled-instance"],
    );
    disabled_instance.instances[0].enabled = false;
    let manager = build_manager(
        "device-disabled-history",
        vec![disabled_plugin, disabled_instance],
    );
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();

    for (plugin_id, instance_id, expected_code) in [
        (
            "dev.codepet.disabled-plugin",
            "instance-disabled-plugin",
            "provider_plugin_disabled",
        ),
        (
            "dev.codepet.disabled-instance",
            "instance-disabled-instance",
            "provider_instance_disabled",
        ),
    ] {
        let error = gateway
            .conversation_list(GatewayConversationListRequest {
                provider_id: instance_id.to_string(),
                cursor: None,
                limit: Some(10),
                project_filter: all_project_filter(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, expected_code);
        assert_eq!(manager.snapshot(plugin_id).await.unwrap().generation, 0);
    }
    manager.shutdown().await;
}

#[tokio::test]
async fn nonretryable_plugin_misconfiguration_is_not_restarted_by_history() {
    let mut descriptor = plugin(
        "dev.codepet.history-misconfigured",
        &["instance-history-misconfigured"],
    );
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MIN_VERSION".to_string(),
        "2".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MAX_VERSION".to_string(),
        "2".to_string(),
    );
    let manager = build_manager("device-history-misconfigured", vec![descriptor]);
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    let request = || GatewayConversationListRequest {
        provider_id: "instance-history-misconfigured".to_string(),
        cursor: None,
        limit: Some(10),
        project_filter: all_project_filter(),
    };

    let first = gateway.conversation_list(request()).await.unwrap_err();
    assert_eq!(first.code, "provider_protocol_version_mismatch");
    let generation = manager
        .snapshot("dev.codepet.history-misconfigured")
        .await
        .unwrap()
        .generation;
    let second = gateway.conversation_list(request()).await.unwrap_err();
    assert_eq!(second.code, "provider_protocol_version_mismatch");
    assert_eq!(
        manager
            .snapshot("dev.codepet.history-misconfigured")
            .await
            .unwrap()
            .generation,
        generation
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn concurrent_history_requests_start_one_plugin_generation() {
    let mut descriptor = plugin(
        "dev.codepet.concurrent-history",
        &["instance-concurrent-history"],
    );
    descriptor.env.insert(
        "CODEPET_FAKE_INITIALIZE_DELAY_MS".to_string(),
        "100".to_string(),
    );
    let manager = build_manager("device-concurrent-history", vec![descriptor]);
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    let request = || GatewayConversationListRequest {
        provider_id: "instance-concurrent-history".to_string(),
        cursor: None,
        limit: Some(10),
        project_filter: all_project_filter(),
    };

    let (first, second) = tokio::join!(
        gateway.conversation_list(request()),
        gateway.conversation_list(request())
    );
    assert_eq!(first.unwrap().conversations.len(), 1);
    assert_eq!(second.unwrap().conversations.len(), 1);
    assert_eq!(
        manager
            .snapshot("dev.codepet.concurrent-history")
            .await
            .unwrap()
            .generation,
        1
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn history_recovery_racing_shutdown_does_not_resurrect_the_plugin() {
    let directory = tempfile::tempdir().unwrap();
    let initialize_marker = directory.path().join("history-initialize");
    let pid_marker = directory.path().join("history-pid");
    let mut descriptor = plugin(
        "dev.codepet.history-shutdown",
        &["instance-history-shutdown"],
    );
    descriptor.env.insert(
        "CODEPET_FAKE_INITIALIZE_DELAY_MS".to_string(),
        "300".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_INITIALIZE_MARKER".to_string(),
        initialize_marker.display().to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_PID_MARKER".to_string(),
        pid_marker.display().to_string(),
    );
    let manager = build_manager("device-history-shutdown", vec![descriptor]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    let query_gateway = gateway.clone();
    let query = tokio::spawn(async move {
        query_gateway
            .conversation_list(GatewayConversationListRequest {
                provider_id: "instance-history-shutdown".to_string(),
                cursor: None,
                limit: Some(10),
                project_filter: all_project_filter(),
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !initialize_marker.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert!(manager
        .shutdown()
        .await
        .iter()
        .all(|(_, outcome)| outcome.is_ok()));
    assert!(query.await.unwrap().is_err());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        manager
            .snapshot("dev.codepet.history-shutdown")
            .await
            .unwrap()
            .state,
        PluginRuntimeState::Stopped
    );
    let retry = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: "instance-history-shutdown".to_string(),
            cursor: None,
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap_err();
    assert_eq!(retry.code, "provider_manager_shutting_down");
    #[cfg(unix)]
    assert!(!pid_is_alive(&pid_marker));
}
