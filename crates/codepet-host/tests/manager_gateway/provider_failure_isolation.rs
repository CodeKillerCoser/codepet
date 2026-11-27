use super::*;

#[tokio::test]
async fn provider_business_faults_leave_gateway_and_sibling_requests_available() {
    for fault in ["invalid", "error", "hang", "exit"] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("fault-request-started");
        let mut bad = plugin("faulty", &["faulty"]);
        bad.env.insert("CODEPET_FAKE_PROJECT_FAULT".into(), fault.into());
        bad.env.insert("CODEPET_FAKE_PROJECT_MARKER".into(), marker.to_string_lossy().into());
        let manager = build_manager_with_options(
            "device-provider-isolation", vec![bad, plugin("healthy", &["healthy"])],
            PluginManagerConfig::default().event_capacity,
            PluginProcessOptions {
                request_timeout: Duration::from_secs(2),
                summary_query_timeout: Duration::from_millis(750),
                shutdown_timeout: Duration::from_millis(100),
                ..PluginProcessOptions::default()
            },
        );
        assert!(manager.start_enabled().await.iter().all(|(_, result)| result.is_ok()));
        let gateway = Arc::new(ProviderGatewayService::with_remote_identity(manager.clone(), RemoteHostIdentity {
            device_id: "device-provider-isolation".into(),
            descriptor: device_descriptor("Device"),
        }).unwrap());
        gateway.start_event_forwarding();
        let healthy_generation = manager.snapshot("healthy").await.unwrap().generation;
        gateway.protocol_handshake(handshake_request()).await.unwrap();
        let failed_gateway = gateway.clone();
        let failed = tokio::spawn(async move {
            failed_gateway.project_list(ProjectListRequest {
                provider_id: "faulty".into(), cursor: None, limit: Some(20),
            }).await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !marker.exists() { tokio::task::yield_now().await; }
        }).await.expect("fault injection never reached its isolated process");

        // These requests must finish before the stalled Provider's deadline,
        // and neither discovery nor application heartbeat calls that Provider.
        tokio::time::timeout(Duration::from_millis(600), async {
            let pong = gateway.protocol_ping(codepet_gateway_sdk::PingRequest { sequence: 1 }).await.unwrap();
            assert_eq!(pong.sequence, 1);
            gateway.provider_describe(ProviderDescribeRequest { provider_id: "healthy".into() }).await.unwrap();
            let page = gateway.project_list(ProjectListRequest {
                provider_id: "healthy".into(), cursor: None, limit: Some(20),
            }).await.unwrap();
            assert_eq!(page.projects.len(), 1);
            gateway.conversation_list(GatewayConversationListRequest {
                provider_id: "healthy".into(), project_filter: ConversationProjectFilter::ConversationProjectFilterAll(
                    ConversationProjectFilterAll { kind: ConversationProjectFilterAllKind::All }),
                cursor: None, limit: Some(20),
            }).await.unwrap();
            manager.turn_start(TurnStartRequest {
                conversation: resource("device-provider-isolation", "healthy", "healthy", "isolated-conversation"),
                client_request_id: format!("healthy-send-{fault}"),
                capability_revision: "fake-capabilities-v1".into(),
                input: TurnInput { kind: TurnInputKind::Text, text: "healthy request".into() },
                selection: TurnSelection { access_mode_id: None, reasoning_effort_id: None, model: None },
            }).await.unwrap();
            gateway.protocol_handshake(handshake_request()).await.unwrap();
        }).await.unwrap_or_else(|_| panic!("{fault} in one Provider blocked another Provider or Gateway health"));
        if fault == "hang" { assert!(!failed.is_finished(), "healthy requests waited for fault timeout"); }
        assert!(tokio::time::timeout(Duration::from_secs(3), failed).await.unwrap().unwrap().is_err());
        assert_eq!(manager.snapshot("healthy").await.unwrap().generation, healthy_generation);
        gateway.project_list(ProjectListRequest {
            provider_id: "healthy".into(), cursor: None, limit: Some(20),
        }).await.unwrap();
        manager.shutdown().await;
    }
}
