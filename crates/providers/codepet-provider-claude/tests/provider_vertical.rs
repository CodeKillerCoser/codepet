use codepet_provider_claude::{
    decode_claude_output, ClaudeOutput, ClaudeProvider, CLAUDE_INSTANCE_KIND,
    CLAUDE_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationCreateRequest,
    ConversationGetRequest, ConversationListRequest, InstanceCapabilitiesRequest,
    InstanceCreateRequest, InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest,
    JsonObject, ProtocolEvent, ProtocolServer as ProviderProtocolServer,
    ProviderCapability, ProviderInitializeRequest, ProviderInstanceRoute,
    ProviderShutdownRequest, TurnInterruptRequest, TurnStartRequest, TurnStatus,
    TurnSteerRequest, VersionRange, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn provider_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codepet-provider-claude"))
}

fn fixture_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_claude-stream-fixture"))
}

fn route(device_id: &str) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: device_id.to_string(),
        provider_plugin_id: CLAUDE_PLUGIN_ID.to_string(),
        provider_instance_id: "claude".to_string(),
    }
}

fn instance_settings(executable: &Path) -> JsonObject {
    [(
        "claudeExecutable".to_string(),
        json!(executable.to_string_lossy()),
    )]
    .into_iter()
    .collect()
}

async fn ready_provider(
    workspace_root: &Path,
) -> (
    Arc<ClaudeProvider>,
    mpsc::Receiver<ProtocolEvent>,
    ProviderInstanceRoute,
    codepet_provider_sdk::ProviderConversation,
) {
    let (event_sender, event_receiver) = mpsc::channel();
    let provider = Arc::new(ClaudeProvider::new(Arc::new(move |event| {
        event_sender
            .send(event)
            .map_err(|error| codepet_provider_sdk::ProtocolError {
                code: "test_event_sink_closed".to_string(),
                message: error.to_string(),
                retryable: false,
                details: None,
            })
    })));
    let device_id = "device-claude-fixture";
    let route = route(device_id);
    let initialized = ProviderProtocolServer::provider_initialize(
        provider.as_ref(),
        ProviderInitializeRequest {
            host_client_id: "client-claude-fixture".to_string(),
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
    assert_eq!(initialized.plugin.plugin_id, CLAUDE_PLUGIN_ID);

    let described = ProviderProtocolServer::provider_describe(
        provider.as_ref(),
        codepet_provider_sdk::ProviderDescribeRequest {},
    )
    .await
    .unwrap();
    assert_eq!(described.plugin, initialized.plugin);

    let created = ProviderProtocolServer::instance_create(
        provider.as_ref(),
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CLAUDE_INSTANCE_KIND.to_string(),
            display_name: "Claude Fixture".to_string(),
            settings: instance_settings(&fixture_executable()),
        },
    )
    .await
    .unwrap();
    assert_eq!(created.instance.route, route);
    let started = ProviderProtocolServer::instance_start(
        provider.as_ref(),
        InstanceStartRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(started.instance.status, codepet_provider_sdk::InstanceStatus::Ready);

    let conversation = ProviderProtocolServer::conversation_create(
        provider.as_ref(),
        ConversationCreateRequest {
            route: route.clone(),
            title: Some("Fixture conversation".to_string()),
            permission_level: "workspace-write".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            extension: None,
        },
    )
    .await
    .unwrap()
    .conversation;
    (provider, event_receiver, route, conversation)
}

#[tokio::test]
async fn provider_maps_claude_stream_json_and_fails_closed_for_missing_methods() {
    let workspace = tempfile::tempdir().unwrap();
    let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
    assert_resource_route(&conversation.resource, &route);

    let capabilities = ProviderProtocolServer::instance_capabilities(
        provider.as_ref(),
        InstanceCapabilitiesRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap()
    .capabilities;
    assert!(capabilities.methods.contains(&ProviderCapability::ConversationCreate));
    assert!(capabilities.methods.contains(&ProviderCapability::TurnStart));
    #[cfg(unix)]
    assert!(capabilities.methods.contains(&ProviderCapability::TurnInterrupt));
    assert!(!capabilities.methods.contains(&ProviderCapability::ConversationList));
    assert!(!capabilities.methods.contains(&ProviderCapability::ConversationGet));
    assert!(!capabilities.methods.contains(&ProviderCapability::TurnSteer));
    assert!(!capabilities.methods.contains(&ProviderCapability::ApprovalResolve));

    let first_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_message_id: "message-first".to_string(),
            message: "run fixture".to_string(),
        },
    )
    .await
    .unwrap()
    .turn;
    assert_resource_route(&first_turn.resource, &route);
    assert_eq!(first_turn.conversation, conversation.resource);
    let first = terminal_turn(&events, &first_turn.resource.native_resource_id);
    assert_eq!(first.status, TurnStatus::Completed);
    assert_eq!(first.output, "fixture output");

    let second_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_message_id: "message-second".to_string(),
            message: "run fixture again".to_string(),
        },
    )
    .await
    .unwrap()
    .turn;
    let second = terminal_turn(&events, &second_turn.resource.native_resource_id);
    assert_eq!(second.status, TurnStatus::Completed);
    assert_eq!(second.output, "fixture resumed");

    let failed_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_message_id: "message-failed".to_string(),
            message: "fail".to_string(),
        },
    )
    .await
    .unwrap()
    .turn;
    let failed = terminal_turn(&events, &failed_turn.resource.native_resource_id);
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(failed.output, "Not logged in");

    let list_error = ProviderProtocolServer::conversation_list(
        provider.as_ref(),
        ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(list_error.code, "capability_unsupported");
    let get_error = ProviderProtocolServer::conversation_get(
        provider.as_ref(),
        ConversationGetRequest {
            conversation: conversation.resource.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(get_error.code, "capability_unsupported");
    let steer_error = ProviderProtocolServer::turn_steer(
        provider.as_ref(),
        TurnSteerRequest {
            conversation: conversation.resource.clone(),
            turn: failed_turn.resource.clone(),
            client_message_id: "message-steer".to_string(),
            message: "steer".to_string(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(steer_error.code, "capability_unsupported");
    let approval_error = ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: failed_turn.resource,
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(approval_error.code, "capability_unsupported");

    let stopped = ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(stopped.instance.status, codepet_provider_sdk::InstanceStatus::Stopped);
    assert!(ProviderProtocolServer::instance_destroy(
        provider.as_ref(),
        InstanceDestroyRequest { route },
    )
    .await
    .unwrap()
    .destroyed);
    assert!(ProviderProtocolServer::provider_shutdown(
        provider.as_ref(),
        ProviderShutdownRequest {},
    )
    .await
    .unwrap()
    .accepted);
}

#[cfg(unix)]
#[tokio::test]
async fn provider_interrupts_an_active_claude_process_with_sigint() {
    let workspace = tempfile::tempdir().unwrap();
    let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
    let turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_message_id: "message-interrupt".to_string(),
            message: "wait for interrupt".to_string(),
        },
    )
    .await
    .unwrap()
    .turn;
    wait_for_claude_init(&events, &turn.resource.native_resource_id);

    let interrupted = ProviderProtocolServer::turn_interrupt(
        provider.as_ref(),
        TurnInterruptRequest {
            conversation: conversation.resource,
            turn: turn.resource.clone(),
        },
    )
    .await
    .unwrap()
    .turn;
    assert_eq!(interrupted.status, TurnStatus::Interrupted);
    let terminal = terminal_turn(&events, &turn.resource.native_resource_id);
    assert_eq!(terminal.status, TurnStatus::Interrupted);

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(
        provider.as_ref(),
        ProviderShutdownRequest {},
    )
    .await
    .unwrap();
}

#[test]
fn captured_claude_2_1_251_output_decodes_without_guessed_fields() {
    let fixture = include_str!("fixtures/claude-2.1.251-no-auth.ndjson");
    let mut unknown = 0;
    let mut saw_init = false;
    let mut saw_auth_error = false;
    for line in fixture.lines() {
        match decode_claude_output(line.as_bytes()).unwrap() {
            ClaudeOutput::Unknown => unknown += 1,
            ClaudeOutput::System {
                subtype,
                session_id,
                model,
                capabilities,
                ..
            } if subtype == "init" => {
                saw_init = true;
                assert_eq!(session_id.as_deref(), Some("22222222-2222-4222-8222-222222222222"));
                assert_eq!(model.as_deref(), Some("claude-sonnet-5"));
                assert!(capabilities.iter().any(|value| value == "msg_lifecycle_v1"));
            }
            ClaudeOutput::Result {
                subtype,
                is_error,
                terminal_reason,
                result,
                ..
            } => {
                saw_auth_error = true;
                assert_eq!(subtype, "success");
                assert!(is_error);
                assert_eq!(terminal_reason.as_deref(), Some("api_error"));
                assert_eq!(result.as_deref(), Some("Not logged in · Please run /login"));
            }
            _ => {}
        }
    }
    assert_eq!(unknown, 3);
    assert!(saw_init);
    assert!(saw_auth_error);
}

#[test]
fn provider_binary_uses_generated_json_line_dispatcher() {
    let mut child = Command::new(provider_executable())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let initialized = binary_request(
        &mut stdin,
        &mut stdout,
        "initialize",
        "provider.initialize",
        json!({
            "hostClientId": "client-binary",
            "hostDeviceId": "device-binary",
            "hostVersion": "test",
            "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
        }),
    );
    assert_eq!(
        initialized.pointer("/result/plugin/pluginId").and_then(Value::as_str),
        Some(CLAUDE_PLUGIN_ID)
    );
    let described = binary_request(
        &mut stdin,
        &mut stdout,
        "describe",
        "provider.describe",
        json!({}),
    );
    assert_eq!(
        described.pointer("/result/plugin/pluginId").and_then(Value::as_str),
        Some(CLAUDE_PLUGIN_ID)
    );
    let unsupported = binary_request(
        &mut stdin,
        &mut stdout,
        "list",
        "conversation.list",
        json!({
            "route": {
                "deviceId": "device-binary",
                "providerPluginId": CLAUDE_PLUGIN_ID,
                "providerInstanceId": "claude"
            }
        }),
    );
    assert_eq!(
        unsupported
            .pointer("/error/data/code")
            .and_then(Value::as_str),
        Some("capability_unsupported")
    );
    binary_request(
        &mut stdin,
        &mut stdout,
        "shutdown",
        "provider.shutdown",
        json!({}),
    );
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn provider_binary_fails_stop_after_an_oversized_host_frame() {
    let mut child = Command::new(provider_executable())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&vec![b'x'; 1024 * 1024 + 1]).unwrap();
    stdin.write_all(b"\n").unwrap();
    serde_json::to_writer(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "must-not-run",
            "method": "provider.describe",
            "params": {}
        }),
    )
    .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Provider did not fail-stop");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!status.success());
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let frames = output.lines().collect::<Vec<_>>();
    assert_eq!(frames.len(), 1);
    let response: Value = serde_json::from_str(frames[0]).unwrap();
    assert_eq!(response["error"]["code"], -32600);
    assert_ne!(response["id"], "must-not-run");
}

#[test]
fn provider_runtime_dependency_boundary_excludes_host_gateway_pet_and_tauri() {
    let crate_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_manifest = crate_directory
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("Cargo.toml");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("metadata")
        .arg("--manifest-path")
        .arg(workspace_manifest)
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "codepet-provider-claude")
        .unwrap();
    let runtime_dependencies = package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|dependency| dependency["kind"].is_null())
        .map(|dependency| dependency["name"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let expected = [
        "codepet-provider-sdk",
        "libc",
        "serde",
        "serde_json",
        "tokio",
        "uuid",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    assert_eq!(runtime_dependencies, expected);
    for forbidden in ["codepet-host", "codepet-gateway-sdk", "codepet-pet-sdk", "tauri"] {
        assert!(!runtime_dependencies.contains(forbidden));
    }
}

struct TerminalTurn {
    status: TurnStatus,
    output: String,
}

fn terminal_turn(events: &mpsc::Receiver<ProtocolEvent>, turn_id: &str) -> TerminalTurn {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = events.recv_timeout(remaining).unwrap();
        match event {
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.turn.native_resource_id == turn_id =>
            {
                output.push_str(&params.delta);
            }
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.resource.native_resource_id == turn_id
                    && matches!(
                        params.turn.status,
                        TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Interrupted
                    ) =>
            {
                return TerminalTurn {
                    status: params.turn.status,
                    output,
                };
            }
            _ => {}
        }
    }
}

fn wait_for_claude_init(events: &mpsc::Receiver<ProtocolEvent>, turn_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = events.recv_timeout(remaining).unwrap();
        if matches!(
            event,
            ProtocolEvent::EventConversationUpserted { params, .. }
                if params.conversation.model.as_deref() == Some("claude-sonnet-5")
                    && params.conversation.active_turn.as_ref().is_some_and(|turn| {
                        turn.resource.native_resource_id == turn_id
                    })
        ) {
            return;
        }
    }
}

fn assert_resource_route(
    resource: &codepet_provider_sdk::RoutedResourceId,
    route: &ProviderInstanceRoute,
) {
    assert_eq!(resource.device_id, route.device_id);
    assert_eq!(resource.provider_plugin_id, route.provider_plugin_id);
    assert_eq!(resource.provider_instance_id, route.provider_instance_id);
    assert!(!resource.native_resource_id.is_empty());
}

fn binary_request(
    stdin: &mut impl Write,
    stdout: &mut impl BufRead,
    id: &str,
    method: &str,
    params: Value,
) -> Value {
    serde_json::to_writer(
        &mut *stdin,
        &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    )
    .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
