//! Opt-in test: scripts/test_opencode_observation.py --remote-turns supplies isolated
//! OpenCode config and a localhost model fixture. It never uses the user's models.
use codepet_provider_opencode::{OpenCodeProvider, OPENCODE_INSTANCE_KIND, OPENCODE_PLUGIN_ID};
use codepet_provider_sdk::*;
use serde_json::json;
use std::{collections::BTreeMap, sync::{Arc, mpsc}, time::{Duration, Instant}};

#[tokio::test]
#[ignore = "requires isolated OpenCode 1.18.25 and a local model fixture"]
async fn native_remote_turns_keep_the_instance_ready() {
    let executable = std::env::var("CODEPET_OPENCODE_EXECUTABLE").unwrap();
    let workspace = std::env::var("CODEPET_OPENCODE_TEST_WORKSPACE").expect("isolated workspace required");
    let (sender, receiver) = mpsc::channel();
    let provider = OpenCodeProvider::new(Arc::new(move |event| { let _ = sender.send(event); Ok(()) }));
    provider.provider_initialize(ProviderInitializeRequest { directories: None,
        host_client_id:"native-smoke".into(), host_device_id:"native-smoke-device".into(), host_version:"0.1.0".into(),
        supported_versions:VersionRange { min_version:PROTOCOL_VERSION, max_version:PROTOCOL_VERSION },
    }).await.unwrap();
    let route = ProviderInstanceRoute { device_id:"native-smoke-device".into(), provider_plugin_id:OPENCODE_PLUGIN_ID.into(), provider_instance_id:"opencode".into() };
    provider.instance_create(InstanceCreateRequest { route:route.clone(), instance_kind:OPENCODE_INSTANCE_KIND.into(), display_name:"Native smoke".into(),
        settings:BTreeMap::from([("serverExecutable".into(),json!(executable)),("serverVersion".into(),json!("1.18.25")),
            ("serverArgs".into(),json!(["serve"])),("workspaceRoot".into(),json!(workspace))]),
    }).await.unwrap();
    let started = provider.instance_start(InstanceStartRequest { route:route.clone() }).await.unwrap();
    let conversation = provider.conversation_create(ConversationCreateRequest {
        route:route.clone(), project:None, title:None, permission_level:"build".into(), model:Some("codepet-smoke/smoke".into()),
        reasoning_effort:Some("default".into()), workspace_root:Some(workspace), workspace_mode:None, extension:None,
    }).await.unwrap().conversation;
    let resource = ProviderResourceId { device_id:route.device_id.clone(), provider_plugin_id:route.provider_plugin_id.clone(),
        provider_instance_id:route.provider_instance_id.clone(), native_resource_id:conversation.resource.native_resource_id.clone() };
    for index in 0..2 {
        let response = provider.turn_start(TurnStartRequest { conversation:resource.clone(), client_request_id:format!("native-turn-{index}"),
            capability_revision:started.instance.capabilities.revision.clone(),
            input:TurnInput { kind:TurnInputKind::Text, text:"remote-smoke".into() },
            selection:TurnSelection { access_mode_id:None, reasoning_effort_id:None, model:None },
        }).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let event = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("turn did not finish");
            match event {
                ProtocolEvent::EventInstanceStatusChanged { params, .. } => assert_ne!(params.instance.status, InstanceStatus::Error, "native instance failed while sending"),
                ProtocolEvent::EventTurnUpserted { params, .. } if params.turn.resource == response.turn.resource && matches!(params.turn.status, TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Interrupted) => {
                    assert_eq!(params.turn.status, TurnStatus::Completed); break;
                }
                ProtocolEvent::EventApprovalRequested { params, .. } => {
                    provider.approval_resolve(ApprovalResolveRequest { approval:ProviderResourceId { device_id:route.device_id.clone(),
                        provider_plugin_id:route.provider_plugin_id.clone(), provider_instance_id:route.provider_instance_id.clone(),
                        native_resource_id:params.approval.resource.native_resource_id }, decision:ApprovalDecision::Approve }).await.unwrap();
                }
                _ => {}
            }
        }
        let snapshot = provider.conversation_get(ConversationGetRequest { conversation:resource.clone(), cursor:None, limit:Some(100) }).await.unwrap();
        assert!(!snapshot.items.is_empty());
        let json = serde_json::to_value(snapshot.items).unwrap();
        fn collect(value: &serde_json::Value, ids: &mut std::collections::HashSet<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    if let Some(id) = map.get("contentId").and_then(serde_json::Value::as_str) { assert!(ids.insert(id.into()), "duplicate content identity {id}"); }
                    for v in map.values() { collect(v, ids); }
                }
                serde_json::Value::Array(values) => for v in values { collect(v, ids); },
                _ => {}
            }
        }
        collect(&json, &mut std::collections::HashSet::new());
        provider.conversation_acquire_interaction(ConversationAcquireInteractionRequest { conversation:resource.clone() }).await.unwrap();
    }
    provider.instance_stop(InstanceStopRequest { route }).await.unwrap();
}
