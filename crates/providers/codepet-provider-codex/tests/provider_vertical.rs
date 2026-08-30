use codepet_gateway_sdk::{self as gateway, ProtocolServer as GatewayProtocolServer};
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    ProviderGatewayService, ProviderInstanceRegistry,
};
use codepet_provider_codex::{CodexProvider, CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationCreateRequest, ConversationListRequest,
    ConversationGetRequest, InstanceCapabilitiesRequest, InstanceCreateRequest,
    InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest, JsonObject, ProtocolEvent,
    ProtocolServer as ProviderProtocolServer, ProviderInitializeRequest,
    ProviderInstanceRoute, ProviderShutdownRequest, TurnInterruptRequest, TurnStartRequest,
    TurnSteerRequest, VersionRange, PROTOCOL_VERSION,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

fn provider_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codepet-provider-codex"))
}

fn app_server_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codex-app-server-fixture"))
}

fn route(device_id: &str) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: device_id.to_string(),
        provider_instance_id: "codex".to_string(),
    }
}

fn instance_settings(app_server: &Path) -> JsonObject {
    [
        (
            "appServerExecutable".to_string(),
            json!(app_server.to_string_lossy()),
        ),
        ("appServerArgs".to_string(), json!([])),
        ("models".to_string(), json!(["gpt-fixture"])),
        ("reasoningEfforts".to_string(), json!(["high"])),
    ]
    .into_iter()
    .collect()
}

#[tokio::test]
async fn provider_v1_round_trips_fixture_app_server_lifecycle_and_approval() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("approval.txt");
    std::env::set_var("CODEPET_FIXTURE_APPROVAL_MARKER", &marker);
    let (event_sender, event_receiver) = mpsc::channel();
    let provider = CodexProvider::new(Arc::new(move |event| {
        event_sender.send(event).map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "test_event_sink_closed".to_string(),
            message: error.to_string(),
            retryable: false,
            details: None,
        })
    }));
    let device_id = "device-provider-fixture";
    let route = route(device_id);

    let initialized = ProviderProtocolServer::provider_initialize(
        &provider,
        ProviderInitializeRequest {
            host_client_id: "client-provider-fixture".to_string(),
            host_device_id: device_id.to_string(),
            host_version: "test".to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(initialized.plugin.plugin_id, CODEX_PLUGIN_ID);

    let created = ProviderProtocolServer::instance_create(
        &provider,
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CODEX_INSTANCE_KIND.to_string(),
            display_name: "Codex Fixture".to_string(),
            settings: instance_settings(&app_server_executable()),
        },
    )
    .await
    .unwrap();
    assert_eq!(created.instance.route, route);
    let started = ProviderProtocolServer::instance_start(
        &provider,
        InstanceStartRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(started.instance.status, codepet_provider_sdk::InstanceStatus::Ready);
    let capabilities = ProviderProtocolServer::instance_capabilities(
        &provider,
        InstanceCapabilitiesRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ApprovalResolve));

    let listed = ProviderProtocolServer::conversation_list(
        &provider,
        ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.conversations[0].resource.device_id, device_id);
    assert_eq!(
        listed.conversations[0].resource.provider_instance_id,
        "codex"
    );
    assert_eq!(
        listed.conversations[0].resource.native_resource_id,
        "thread-listed"
    );
    let fetched = ProviderProtocolServer::conversation_get(
        &provider,
        ConversationGetRequest {
            conversation: listed.conversations[0].resource.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fetched.conversation.resource.native_resource_id,
        "thread-listed"
    );

    let unsupported_title = ProviderProtocolServer::conversation_create(
        &provider,
        ConversationCreateRequest {
            route: route.clone(),
            title: Some("unsupported title".to_string()),
            permission_level: "workspace-write".to_string(),
            model: None,
            reasoning_effort: None,
            workspace_root: None,
            extension: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(unsupported_title.code, "capability_unsupported");

    let conversation = ProviderProtocolServer::conversation_create(
        &provider,
        ConversationCreateRequest {
            route: route.clone(),
            title: None,
            permission_level: "workspace-write".to_string(),
            model: Some("gpt-fixture".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some("/fixture/workspace".to_string()),
            extension: None,
        },
    )
    .await
    .unwrap()
    .conversation;
    let turn = ProviderProtocolServer::turn_start(
        &provider,
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_message_id: "message-one".to_string(),
            message: "run fixture".to_string(),
        },
    )
    .await
    .unwrap()
    .turn;
    assert_eq!(turn.resource.native_resource_id, "turn-started");

    let mut approval = None;
    let mut saw_delta = false;
    for _ in 0..8 {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        match event {
            ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                saw_delta = params.delta == "fixture output";
            }
            ProtocolEvent::EventApprovalRequested { params, .. } => {
                approval = Some(params.approval);
                break;
            }
            _ => {}
        }
    }
    assert!(saw_delta);
    let approval = approval.expect("fixture approval event");
    let resolved = ProviderProtocolServer::approval_resolve(
        &provider,
        ApprovalResolveRequest {
            approval: approval.resource.clone(),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        resolved.approval.status,
        codepet_provider_sdk::ApprovalStatus::Approved
    );
    let resolved_approval_id = resolved.approval.resource.native_resource_id.clone();
    let mut saw_resolved_event = false;
    for _ in 0..8 {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if matches!(
            event,
            ProtocolEvent::EventApprovalResolved { params, .. }
                if params.approval.resource.native_resource_id == resolved_approval_id
        ) {
            saw_resolved_event = true;
            break;
        }
    }
    assert!(saw_resolved_event);
    wait_for_file(&marker).await;
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "accept");

    let steered = ProviderProtocolServer::turn_steer(
        &provider,
        TurnSteerRequest {
            turn: turn.resource.clone(),
            client_message_id: "message-two".to_string(),
            message: "steer fixture".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(steered.turn.resource.native_resource_id, "turn-started");
    let interrupted = ProviderProtocolServer::turn_interrupt(
        &provider,
        TurnInterruptRequest {
            turn: turn.resource,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        interrupted.turn.status,
        codepet_provider_sdk::TurnStatus::Interrupted
    );

    let stopped = ProviderProtocolServer::instance_stop(
        &provider,
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(stopped.instance.status, codepet_provider_sdk::InstanceStatus::Stopped);
    assert!(ProviderProtocolServer::instance_destroy(
        &provider,
        InstanceDestroyRequest { route },
    )
    .await
    .unwrap()
    .destroyed);
    assert!(ProviderProtocolServer::provider_shutdown(
        &provider,
        ProviderShutdownRequest {},
    )
    .await
    .unwrap()
    .accepted);
    std::env::remove_var("CODEPET_FIXTURE_APPROVAL_MARKER");
}

#[tokio::test]
async fn host_launches_codex_manifest_and_completes_gateway_rpc() {
    let directory = tempfile::tempdir().unwrap();
    let plugin_directory = directory.path().join("plugins/codex");
    std::fs::create_dir_all(&plugin_directory).unwrap();
    let marker = directory.path().join("host-approval.txt");
    let manifest = json!({
        "manifestVersion": 1,
        "pluginId": CODEX_PLUGIN_ID,
        "displayName": "Codex Fixture",
        "executable": provider_executable(),
        "args": [],
        "env": {
            "CODEPET_FIXTURE_APPROVAL_MARKER": marker
        },
        "enabled": true,
        "instances": [{
            "instanceId": "codex",
            "instanceKind": CODEX_INSTANCE_KIND,
            "displayName": "Codex Fixture",
            "settings": instance_settings(&app_server_executable()),
            "enabled": true
        }]
    });
    std::fs::write(
        plugin_directory.join("codepet-provider.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let device = DeviceRegistry::open(directory.path().join("device.json"), "Fixture Device")
        .unwrap();
    let device_id = device.identity().device_id.clone();
    let catalog = PluginCatalog::discover(
        PluginCatalogConfig::default().with_directory(directory.path().join("plugins")),
    );
    assert!(catalog.diagnostics().is_empty());
    let instances = ProviderInstanceRegistry::open(
        directory.path().join("instances.json"),
        device_id.clone(),
    )
    .unwrap();
    let mut config = PluginManagerConfig::default();
    config.process.request_timeout = Duration::from_secs(5);
    config.process.shutdown_timeout = Duration::from_secs(2);
    let manager = Arc::new(PluginManager::new(device, catalog, instances, config).unwrap());
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    assert!(gateway.start_event_forwarding());
    let outcomes = manager.start_enabled().await;
    assert_eq!(outcomes.len(), 1);
    outcomes.into_iter().next().unwrap().1.unwrap();

    let listed = GatewayProtocolServer::conversation_list(
        gateway.as_ref(),
        gateway::ConversationListRequest {
            route: Some(gateway::GatewayProviderRoute {
                device_id: device_id.clone(),
                provider_instance_id: "codex".to_string(),
            }),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.conversations[0].resource.native_resource_id, "thread-listed");
    let conversation = GatewayProtocolServer::conversation_create(
        gateway.as_ref(),
        gateway::ConversationCreateRequest {
            route: gateway::GatewayProviderRoute {
                device_id: device_id.clone(),
                provider_instance_id: "codex".to_string(),
            },
            title: None,
            permission_level: "workspace-write".to_string(),
            model: Some("gpt-fixture".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some("/fixture/workspace".to_string()),
        },
    )
    .await
    .unwrap()
    .conversation;
    let turn = GatewayProtocolServer::turn_send(
        gateway.as_ref(),
        gateway::TurnSendRequest {
            conversation: conversation.resource,
            client_message_id: "host-message".to_string(),
            message: "host vertical".to_string(),
            steer_turn: None,
        },
    )
    .await
    .unwrap()
    .turn;
    assert_eq!(turn.resource.native_resource_id, "turn-started");

    let mut events = gateway.subscribe_events(None).unwrap();
    let approval = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let gateway::ProtocolEvent::ApprovalRequested { payload, .. } =
                events.next_event().await.unwrap()
            {
                return payload.approval;
            }
        }
    })
    .await
    .unwrap();
    let resolved = GatewayProtocolServer::approval_resolve(
        gateway.as_ref(),
        gateway::ApprovalResolveRequest {
            approval: approval.resource,
            decision: gateway::ApprovalDecision::Deny,
        },
    )
    .await
    .unwrap();
    assert_eq!(resolved.approval.status, gateway::ApprovalStatus::Denied);
    wait_for_file(&marker).await;
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "decline");
    let interrupted = GatewayProtocolServer::turn_interrupt(
        gateway.as_ref(),
        gateway::TurnInterruptRequest {
            turn: turn.resource,
        },
    )
    .await
    .unwrap();
    assert_eq!(interrupted.turn.status, gateway::TurnStatus::Interrupted);

    for (_, outcome) in manager.shutdown().await {
        if let Err(error) = outcome {
            let snapshot = manager.snapshot(CODEX_PLUGIN_ID).await.unwrap();
            panic!(
                "Codex Provider shutdown failed: {error:?}; stderr={:?}",
                snapshot.stderr_diagnostics
            );
        }
    }
}

async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !path.is_file() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
