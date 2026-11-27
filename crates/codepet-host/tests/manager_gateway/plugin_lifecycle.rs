use super::*;

#[tokio::test]
async fn version_negotiation_rejects_a_plugin_with_an_inconsistent_reported_range() {
    let mut descriptor = plugin("dev.codepet.version-mismatch", &["instance-version"]);
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MIN_VERSION".to_string(),
        "3".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MAX_VERSION".to_string(),
        "3".to_string(),
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
