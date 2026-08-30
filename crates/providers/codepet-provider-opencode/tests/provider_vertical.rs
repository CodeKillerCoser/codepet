use codepet_provider_opencode::{
    OpenCodeProvider, OPENCODE_INSTANCE_KIND, OPENCODE_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationCreateRequest,
    ConversationGetRequest, ConversationListRequest, InstanceCreateRequest,
    InstanceStartRequest, InstanceStatus, InstanceStopRequest, ProtocolEvent,
    ProtocolServer, ProviderInitializeRequest, ProviderInstanceRoute, RoutedResourceId,
    TurnInterruptRequest, TurnStartRequest, TurnStatus, TurnSteerRequest, VersionRange,
    PROTOCOL_VERSION,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn official_v2_shapes_map_through_the_provider_protocol() {
    let (event_sender, event_receiver) = mpsc::channel();
    let provider = OpenCodeProvider::new(Arc::new(move |event| {
        event_sender.send(event).map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "test_event_sink_closed".to_string(),
            message: error.to_string(),
            retryable: false,
            details: None,
        })
    }));
    provider
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "host-fixture".to_string(),
            host_device_id: "device-fixture".to_string(),
            host_version: "0.1.0".to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        })
        .await
        .unwrap();
    let route = ProviderInstanceRoute {
        device_id: "device-fixture".to_string(),
        provider_plugin_id: OPENCODE_PLUGIN_ID.to_string(),
        provider_instance_id: "opencode".to_string(),
    };
    let settings = BTreeMap::from([
        (
            "serverExecutable".to_string(),
            json!(env!("CARGO_BIN_EXE_opencode-server-fixture")),
        ),
        ("serverArgs".to_string(), json!(["serve"])),
    ]);
    let created = provider
        .instance_create(InstanceCreateRequest {
            route: route.clone(),
            instance_kind: OPENCODE_INSTANCE_KIND.to_string(),
            display_name: "OpenCode Fixture".to_string(),
            settings,
        })
        .await
        .unwrap();
    assert_eq!(created.instance.status, InstanceStatus::Created);
    assert_eq!(created.instance.capabilities.models, Vec::<String>::new());
    assert_eq!(
        created.instance.capabilities.permission_levels,
        vec!["opencode-default".to_string()]
    );

    let started = provider
        .instance_start(InstanceStartRequest {
            route: route.clone(),
        })
        .await
        .unwrap();
    assert_eq!(started.instance.status, InstanceStatus::Ready);
    std::thread::sleep(Duration::from_millis(100));

    let listed = provider
        .conversation_list(ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(10),
        })
        .await
        .unwrap();
    assert_eq!(listed.conversations.len(), 1);
    assert_eq!(listed.conversations[0].resource.native_resource_id, "ses_fixture");
    let fixture_workspace = std::env::temp_dir()
        .join("opencode-fixture")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        listed.conversations[0].workspace_root.as_deref(),
        Some(fixture_workspace.as_str())
    );

    let fixture_conversation = resource(&route, "ses_fixture");
    let fetched = provider
        .conversation_get(ConversationGetRequest {
            conversation: fixture_conversation.clone(),
        })
        .await
        .unwrap();
    assert_eq!(fetched.conversation.title, "Fixture session");

    let created_workspace = std::env::temp_dir()
        .join("opencode-created")
        .to_string_lossy()
        .to_string();
    let new_conversation = provider
        .conversation_create(ConversationCreateRequest {
            route: route.clone(),
            title: None,
            permission_level: "opencode-default".to_string(),
            model: None,
            reasoning_effort: None,
            workspace_root: Some(created_workspace),
            extension: None,
        })
        .await
        .unwrap();
    assert_eq!(
        new_conversation.conversation.resource.native_resource_id,
        "ses_created"
    );

    let started_turn = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-start-1".to_string(),
            message: "needs approval".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(started_turn.turn.conversation, fixture_conversation);
    assert!(matches!(
        started_turn.turn.status,
        TurnStatus::Queued | TurnStatus::Running | TurnStatus::WaitingApproval
    ));

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_delta = false;
    let mut approval = None;
    while Instant::now() < deadline && (!saw_delta || approval.is_none()) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(event) = event_receiver.recv_timeout(remaining) else {
            break;
        };
        match event {
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.delta == "fixture output" && params.kind == "text" =>
            {
                saw_delta = true;
            }
            ProtocolEvent::EventApprovalRequested { params, .. } => {
                approval = Some(params.approval);
            }
            _ => {}
        }
    }
    assert!(saw_delta, "official text delta shape was not mapped");
    let approval = approval.expect("official permission.v2.asked shape was not mapped");
    assert_eq!(approval.kind, "bash");
    assert_eq!(approval.turn, started_turn.turn.resource);

    let resolved = provider
        .approval_resolve(ApprovalResolveRequest {
            approval: approval.resource,
            decision: ApprovalDecision::Approve,
        })
        .await
        .unwrap();
    assert_eq!(resolved.approval.decision, Some(ApprovalDecision::Approve));

    let steered = provider
        .turn_steer(TurnSteerRequest {
            conversation: fixture_conversation.clone(),
            turn: started_turn.turn.resource.clone(),
            client_message_id: "client-steer-1".to_string(),
            message: "steer fixture".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(steered.turn.resource, started_turn.turn.resource);

    let interrupted = provider
        .turn_interrupt(TurnInterruptRequest {
            conversation: fixture_conversation,
            turn: started_turn.turn.resource,
        })
        .await
        .unwrap();
    assert_eq!(interrupted.turn.status, TurnStatus::Interrupted);

    let stopped = provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
    assert_eq!(stopped.instance.status, InstanceStatus::Stopped);
}

#[tokio::test]
#[ignore = "requires CODEPET_OPENCODE_EXECUTABLE pointing to OpenCode 1.18.25 or newer"]
async fn provider_real_opencode_server_smoke() {
    let executable = std::env::var("CODEPET_OPENCODE_EXECUTABLE")
        .expect("CODEPET_OPENCODE_EXECUTABLE is required for the ignored smoke test");
    assert!(std::path::Path::new(&executable).is_absolute());
    let provider = OpenCodeProvider::new(Arc::new(|_event| Ok(())));
    provider
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "real-smoke-host".to_string(),
            host_device_id: "real-smoke-device".to_string(),
            host_version: "0.1.0".to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        })
        .await
        .unwrap();
    let route = ProviderInstanceRoute {
        device_id: "real-smoke-device".to_string(),
        provider_plugin_id: OPENCODE_PLUGIN_ID.to_string(),
        provider_instance_id: "opencode-real-smoke".to_string(),
    };
    provider
        .instance_create(InstanceCreateRequest {
            route: route.clone(),
            instance_kind: OPENCODE_INSTANCE_KIND.to_string(),
            display_name: "OpenCode Real Smoke".to_string(),
            settings: BTreeMap::from([
                ("serverExecutable".to_string(), json!(executable)),
                ("serverArgs".to_string(), json!(["serve"])),
            ]),
        })
        .await
        .unwrap();
    provider
        .instance_start(InstanceStartRequest {
            route: route.clone(),
        })
        .await
        .unwrap();
    provider
        .conversation_list(ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(1),
        })
        .await
        .unwrap();
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

fn resource(route: &ProviderInstanceRoute, native_resource_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_resource_id.to_string(),
    }
}
