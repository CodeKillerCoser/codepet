use super::*;

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
        "provider_mux_error" | "provider_mux_driver_failed" | "provider_process_exited" | "provider_process_closed"
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
    assert_eq!(beta.conversation.resource.provider_id, "instance-beta");
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
        "provider_mux_error" | "provider_mux_driver_failed" | "provider_process_exited" | "provider_process_closed"
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
        "3".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_SUPPORTED_MAX_VERSION".to_string(),
        "3".to_string(),
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
