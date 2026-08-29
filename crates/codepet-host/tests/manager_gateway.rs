use codepet_gateway_sdk::{
    ConversationGetRequest as GatewayConversationGetRequest, ProtocolEvent as GatewayEvent,
    ProtocolServer as GatewayProtocolServer, ProviderListRequest,
};
use codepet_host::{
    event_cursor_sequence, DeviceIdentity, DeviceRegistry, PluginCatalog, PluginCatalogConfig,
    PluginDescriptor, PluginInstanceConfig, PluginManager, PluginManagerConfig,
    PluginProcessOptions, PluginRuntimeState, ProviderGatewayService,
    ProviderInstanceRegistry,
};
use codepet_provider_sdk::{
    ConversationGetRequest, JsonObject, ProviderInstanceRoute, RoutedResourceId,
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
    let device = DeviceRegistry::from_identity(DeviceIdentity {
        version: 1,
        device_id: device_id.to_string(),
        display_name: format!("Device {device_id}"),
        created_at: 1,
    })
    .unwrap();
    let mut catalog_config = PluginCatalogConfig::default();
    catalog_config.descriptors = descriptors;
    let catalog = PluginCatalog::discover(catalog_config);
    let instances = ProviderInstanceRegistry::in_memory(device_id.to_string()).unwrap();
    Arc::new(
        PluginManager::new(
            device,
            catalog,
            instances,
            PluginManagerConfig {
                process: PluginProcessOptions {
                    request_timeout: Duration::from_secs(2),
                    shutdown_timeout: Duration::from_secs(2),
                    ..PluginProcessOptions::default()
                },
                ..PluginManagerConfig::default()
            },
        )
        .unwrap(),
    )
}

fn resource(device_id: &str, instance_id: &str, native_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        device_id: device_id.to_string(),
        provider_instance_id: instance_id.to_string(),
        native_resource_id: native_id.to_string(),
    }
}

#[tokio::test]
async fn gateway_lists_devices_instances_capabilities_and_keeps_event_routes_monotonic() {
    let manager = build_manager(
        "device-a",
        vec![plugin("dev.codepet.gateway", &["instance-a1", "instance-a2"])],
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()));
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

    let after = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&after)).unwrap();
    let response = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource("device-a", "instance-a1", "event-first"),
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.provider_instance_id,
        "instance-a1"
    );
    let response = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource("device-a", "instance-a2", "event-first"),
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
                routed_event_cursors.push(event_cursor_sequence(event_cursor).unwrap());
            }
            GatewayEvent::TurnOutputDelta {
                event_cursor,
                payload,
                ..
            } => {
                assert_eq!(payload.turn.device_id, "device-a");
                routed_instances.push(payload.turn.provider_instance_id.clone());
                routed_event_cursors.push(event_cursor_sequence(event_cursor).unwrap());
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
            conversation: resource("device-a", "instance-missing", "conversation"),
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_instance.code, "unknown_provider_instance");
    let wrong_device = gateway
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource("device-b", "instance-a1", "conversation"),
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
    let gateway_b = Arc::new(ProviderGatewayService::new(device_b.clone()));
    gateway_b.start_event_forwarding();
    assert!(device_b.start_enabled().await[0].1.is_ok());
    let response_b = gateway_b
        .conversation_get(GatewayConversationGetRequest {
            conversation: resource("device-b", "instance-b1", "device-b-conversation"),
        })
        .await
        .unwrap();
    assert_eq!(response_b.conversation.resource.device_id, "device-b");
    assert_eq!(response_b.conversation.resource.provider_instance_id, "instance-b1");
    assert_ne!(
        manager.device().identity().device_id,
        device_b.device().identity().device_id
    );
    manager.shutdown().await;
    device_b.shutdown().await;
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
    assert_eq!(snapshot.state, PluginRuntimeState::Degraded);
    assert!(snapshot.process_exit.is_some());
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
            conversation: resource("device-isolation", "instance-alpha", "crash"),
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
            conversation: resource("device-isolation", "instance-beta", "healthy"),
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

    let plugin_mismatch = manager
        .instance_registry()
        .resolve_route(
            &ProviderInstanceRoute {
                device_id: "device-isolation".to_string(),
                provider_instance_id: "instance-beta".to_string(),
            },
            Some("dev.codepet.alpha"),
        )
        .unwrap_err();
    assert_eq!(plugin_mismatch.code, "provider_instance_plugin_mismatch");
    manager.shutdown().await;
}
