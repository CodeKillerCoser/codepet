use codepet_gateway_sdk::{
    ConversationGetRequest as GatewayConversationGetRequest,
    ConversationListRequest as GatewayConversationListRequest, EventSubscribeRequest,
    HandshakeRequest,
    ProtocolEvent as GatewayEvent, ProtocolRequest as GatewayRequest,
    ProtocolResponse as GatewayResponse, ProtocolServer as GatewayProtocolServer,
    ProviderListRequest, RemoteHostIdentity, ResponsePayload,
    TurnSendRequest as GatewayTurnSendRequest, VersionRange,
};
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor, PluginInstanceConfig,
    PluginManager, PluginManagerConfig, PluginProcessOptions, PluginRuntimeState,
    ProviderGatewayService, ProviderInstanceRegistry,
};
use codepet_provider_sdk::{
    ConversationGetRequest, JsonObject, ProviderInstanceRoute, RoutedResourceId, TurnStartRequest,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn plugin(plugin_id: &str, instances: &[&str]) -> PluginDescriptor {
    PluginDescriptor {
        plugin_id: plugin_id.to_string(),
        display_name: format!("Fake {plugin_id}"),
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
    let device = DeviceRegistry::open(device_path, "unused").unwrap();
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

fn event_cursor_sequence(cursor: &str) -> u64 {
    cursor
        .strip_prefix("event-")
        .unwrap()
        .parse::<u64>()
        .unwrap()
}

#[tokio::test]
async fn gateway_handshake_returns_the_transport_injected_remote_host_identity() {
    let manager = build_manager("device-handshake", Vec::new());
    let device = RemoteHostIdentity {
        device_id: "device-handshake".to_string(),
        display_name: "Device device-handshake".to_string(),
        identity_fingerprint: "a".repeat(64),
    };
    let gateway = ProviderGatewayService::new_with_remote_host_identity(
        manager.clone(),
        device.clone(),
    )
    .unwrap();

    let response = gateway
        .protocol_handshake(HandshakeRequest {
            client_id: "remote-client-handshake".to_string(),
            client_name: "Remote Test".to_string(),
            client_version: "1.0.0".to_string(),
            supported_versions: VersionRange {
                min_version: codepet_gateway_sdk::PROTOCOL_VERSION,
                max_version: codepet_gateway_sdk::PROTOCOL_VERSION,
            },
            last_event_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(response.device, device);
    assert_eq!(response.devices.len(), 1);
    assert_eq!(response.devices[0].device_id, "device-handshake");
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
            protocol_version: codepet_gateway_sdk::PROTOCOL_VERSION,
            id: "subscribe-current".to_string(),
            params: EventSubscribeRequest {
                after_cursor: after_cursor.clone(),
            },
        },
    )
    .await;
    let GatewayResponse::EventSubscribe {
        response: ResponsePayload::Ok { result },
        ..
    } = response
    else {
        panic!("expected a successful event.subscribe response");
    };
    assert_eq!(result.subscribed_after_cursor, after_cursor);

    let response = codepet_gateway_sdk::dispatch(
        &gateway,
        GatewayRequest::EventSubscribe {
            protocol_version: codepet_gateway_sdk::PROTOCOL_VERSION,
            id: "subscribe-ahead".to_string(),
            params: EventSubscribeRequest {
                after_cursor: "event-00000000000000000001".to_string(),
            },
        },
    )
    .await;
    let GatewayResponse::EventSubscribe {
        response: ResponsePayload::Error { error },
        ..
    } = response
    else {
        panic!("expected event.subscribe to reject an unknown future cursor");
    };
    assert_eq!(error.code, "invalid_event_cursor");
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

    let devices = gateway
        .device_list(codepet_gateway_sdk::DeviceListRequest {})
        .await
        .unwrap();
    assert_eq!(devices.devices[0].device_id, "device-a");
    let providers = gateway
        .provider_list(ProviderListRequest {
            device_id: Some("device-a".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(providers.providers.len(), 2);
    assert!(providers.providers.iter().all(|provider| {
        provider.status == codepet_gateway_sdk::ProviderStatus::Ready
            && provider
                .capabilities
                .methods
                .contains(&codepet_gateway_sdk::GatewayCapability::ConversationGet)
    }));

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
    let GatewayEvent::ProviderStatusChanged { payload, .. } = stopped_event else {
        panic!("expected one lifecycle status event");
    };
    assert_eq!(
        payload.provider.status,
        codepet_gateway_sdk::ProviderStatus::Disconnected
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
    let GatewayEvent::ProviderStatusChanged { payload, .. } = ready_event else {
        panic!("expected one lifecycle status event");
    };
    assert_eq!(payload.provider.status, codepet_gateway_sdk::ProviderStatus::Ready);

    let after = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&after)).unwrap();
    let response = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-a1",
                "event-first",
            ),
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.provider_instance_id,
        "instance-a1"
    );
    let response = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-a2",
                "event-first",
            ),
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.provider_instance_id,
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
                event_cursor,
                payload,
                ..
            } => {
                assert_eq!(
                    payload.conversation.resource.device_id,
                    "device-a"
                );
                routed_instances.push(
                    payload.conversation.resource.provider_instance_id.clone(),
                );
                routed_event_cursors.push(event_cursor_sequence(event_cursor));
            }
            GatewayEvent::TurnOutputDelta {
                event_cursor,
                payload,
                ..
            } => {
                assert_eq!(payload.turn.device_id, "device-a");
                routed_instances.push(payload.turn.provider_instance_id.clone());
                routed_event_cursors.push(event_cursor_sequence(event_cursor));
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
            conversation: resource(
                "device-a",
                "dev.codepet.gateway",
                "instance-missing",
                "conversation",
            ),
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_instance.code, "unknown_provider_instance");
    let wrong_device = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource(
                "device-b",
                "dev.codepet.gateway",
                "instance-a1",
                "conversation",
            ),
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_device.code, "wrong_device_route");
    let wrong_device_list = gateway
        .provider_list(ProviderListRequest {
            device_id: Some("device-b".to_string()),
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_device_list.code, "unknown_device");

    let device_b = build_manager(
        "device-b",
        vec![plugin("dev.codepet.device-b", &["instance-b1"])],
    );
    let gateway_b = Arc::new(ProviderGatewayService::new(device_b.clone()).unwrap());
    gateway_b.start_event_forwarding();
    assert!(device_b.start_enabled().await[0].1.is_ok());
    let response_b = gateway_b
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource(
                "device-b",
                "dev.codepet.device-b",
                "instance-b1",
                "device-b-conversation",
            ),
        })
        .await
        .unwrap();
    assert_eq!(response_b.conversation.resource.device_id, "device-b");
    assert_eq!(response_b.conversation.resource.provider_instance_id, "instance-b1");
    manager.shutdown().await;
    device_b.shutdown().await;
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
                route: Some(codepet_gateway_sdk::GatewayProviderRoute {
                    device_id: "device-snapshot".to_string(),
                    provider_plugin_id: "dev.codepet.snapshot".to_string(),
                    provider_instance_id: "instance-snapshot".to_string(),
                }),
                cursor: None,
                limit: Some(10),
            })
            .await
    });
    let list_event_cursor = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let GatewayEvent::ConversationUpserted {
                event_cursor,
                payload,
                ..
            } = list_events.next_event().await.unwrap()
            {
                if payload.conversation.resource.native_resource_id
                    == "conversation-list-event-first"
                {
                    return event_cursor;
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
                conversation: resource(
                    "device-snapshot",
                    "dev.codepet.snapshot",
                    "instance-snapshot",
                    "snapshot-race",
                ),
            })
            .await
    });

    let event_cursor = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let GatewayEvent::ConversationUpserted {
                event_cursor,
                payload,
                ..
            } = events.next_event().await.unwrap()
            {
                if payload.conversation.resource.native_resource_id
                    == "conversation-event-first"
                {
                    return event_cursor;
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
        .provider_list(ProviderListRequest {
            device_id: Some("device-version".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(
        providers.providers[0].status,
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
    let mut descriptor = plugin(
        "dev.codepet.start-failure",
        &["instance-start-failure", "instance-start-healthy"],
    );
    descriptor.env.insert(
        "CODEPET_FAKE_INSTANCE_START_ERROR_ID".to_string(),
        "instance-start-failure".to_string(),
    );
    let manager = build_manager("device-start-failure", vec![descriptor]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();

    let outcomes = manager.start_enabled().await;
    assert!(outcomes[0].1.is_err());
    let providers = gateway
        .provider_list(ProviderListRequest {
            device_id: Some("device-start-failure".to_string()),
        })
        .await
        .unwrap();
    let failed = providers
        .providers
        .iter()
        .find(|provider| provider.route.provider_instance_id == "instance-start-failure")
        .unwrap();
    let healthy = providers
        .providers
        .iter()
        .find(|provider| provider.route.provider_instance_id == "instance-start-healthy")
        .unwrap();
    assert_eq!(failed.status, codepet_gateway_sdk::ProviderStatus::Unavailable);
    assert_eq!(healthy.status, codepet_gateway_sdk::ProviderStatus::Ready);
    assert_eq!(
        manager
            .snapshot("dev.codepet.start-failure")
            .await
            .unwrap()
            .state,
        PluginRuntimeState::Ready
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
async fn resource_identity_and_route_less_pagination_fail_closed() {
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
    let empty_native = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "",
            ),
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
            client_message_id: "message-1".to_string(),
            message: "hello".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_conversation.code, "provider_resource_identity_mismatch");

    let wrong_steer_conversation = gateway
        .turn_send(GatewayTurnSendRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "conversation-a",
            ),
            client_message_id: "message-steer".to_string(),
            message: "continue".to_string(),
            steer_turn: Some(resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "steer-wrong-conversation",
            )),
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_steer_conversation.code,
        "provider_resource_identity_mismatch"
    );

    let aggregate_cursor = gateway
        .conversation_list(GatewayConversationListRequest {
            route: None,
            cursor: Some("provider-cursor".to_string()),
            limit: Some(10),
        })
        .await
        .unwrap_err();
    assert_eq!(
        aggregate_cursor.code,
        "aggregate_conversation_cursor_unsupported"
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
