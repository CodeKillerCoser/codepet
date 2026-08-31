use codepet_provider_codex::{CodexProvider, CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationCreateRequest, ConversationListRequest,
    ConversationGetRequest, ConversationSearchRequest, InstanceCapabilitiesRequest, InstanceCreateRequest,
    InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest, JsonObject, ProtocolEvent,
    ProtocolServer as ProviderProtocolServer, ProviderInitializeRequest,
    FlatModelCatalogKind, FlatModelSelection, ModelSelection, ProviderInstanceRoute,
    ProviderShutdownRequest, TurnInput, TurnInputKind, TurnInterruptRequest, TurnSelection,
    TurnStartRequest, TurnSteerRequest, VersionRange, PROTOCOL_VERSION,
};
use serde_json::json;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn provider_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codepet-provider-codex"))
}

fn app_server_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codex-app-server-fixture"))
}

fn route(device_id: &str) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: device_id.to_string(),
        provider_plugin_id: CODEX_PLUGIN_ID.to_string(),
        provider_instance_id: "codex".to_string(),
    }
}

fn instance_settings(app_server: &Path, approval_mode: &str, marker: &Path) -> JsonObject {
    [
        (
            "appServerExecutable".to_string(),
            json!(app_server.to_string_lossy()),
        ),
        (
            "appServerArgs".to_string(),
            json!([
                "--approval-mode",
                approval_mode,
                "--marker",
                marker.to_string_lossy()
            ]),
        ),
    ]
    .into_iter()
    .collect()
}

#[tokio::test]
async fn provider_v1_round_trips_fixture_app_server_lifecycle_and_approval() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("approval.txt");
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
            settings: instance_settings(&app_server_executable(), "normal", &marker),
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
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ConversationSearch));
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::TurnStart));
    assert!(!capabilities.capabilities.revision.trim().is_empty());

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
    let searched = ProviderProtocolServer::conversation_search(
        &provider,
        ConversationSearchRequest {
            route: route.clone(),
            search_term: "gateway protocol".to_string(),
            cursor: Some("search-cursor".to_string()),
            limit: Some(7),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        searched.conversations[0].resource.native_resource_id,
        "thread-search-result"
    );
    assert_eq!(searched.page_info.next_cursor.as_deref(), Some("search-next"));

    let empty_search = ProviderProtocolServer::conversation_search(
        &provider,
        ConversationSearchRequest {
            route: route.clone(),
            search_term: " ".to_string(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(empty_search.code, "invalid_request");
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
    assert_eq!(
        fetched
            .items
            .iter()
            .map(|item| item.resource.native_resource_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "user-one",
            "agent-one",
            "reasoning-one",
            "command-one",
            "file-one",
            "mcp-one",
            "dynamic-one",
            "unknown-one"
        ]
    );
    assert_eq!(fetched.items[0].contents[0].content_id, "user-one:input:0");
    assert_eq!(
        fetched.items[2].contents[0].content_id,
        "reasoning-one:summary:0"
    );
    assert_eq!(
        fetched.items[7].kind,
        codepet_provider_sdk::ConversationItemKind::Unknown
    );
    assert!(fetched.items.iter().all(|item| {
        item.kind != codepet_provider_sdk::ConversationItemKind::Approval
    }));
    let fetched_json = serde_json::to_string(&fetched).unwrap();
    assert!(!fetched_json.contains("private raw reasoning"));
    assert!(!fetched_json.contains("must not escape"));
    assert!(!fetched_json.contains("data:image/png"));
    assert!(!fetched_json.contains("private-a"));

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
    let started_turn = ProviderProtocolServer::turn_start(
        &provider,
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_request_id: "message-one".to_string(),
            capability_revision: capabilities.capabilities.revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "run fixture".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: Some("workspace-write".to_string()),
                reasoning_effort_id: Some("high".to_string()),
                model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                    kind: FlatModelCatalogKind::Flat,
                    model_id: "gpt-fixture".to_string(),
                })),
            },
        },
    )
    .await
    .unwrap();
    assert!(started_turn.accepted);
    assert_eq!(
        started_turn.user_item.resource.native_resource_id,
        "user-one"
    );
    let turn = started_turn.turn;
    assert_eq!(turn.resource.native_resource_id, "turn-started");

    let mut approval = None;
    let mut saw_delta = false;
    for _ in 0..8 {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        match event {
            ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                saw_delta = params.delta == "fixture output"
                    && params.item_id == "agent-one"
                    && params.content_id == "agent-one:text";
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

    let active_history = ProviderProtocolServer::conversation_get(
        &provider,
        ConversationGetRequest {
            conversation: conversation.resource.clone(),
        },
    )
    .await
    .unwrap();
    let agent = active_history
        .items
        .iter()
        .find(|item| item.resource.native_resource_id == "agent-one")
        .unwrap();
    let reasoning = active_history
        .items
        .iter()
        .find(|item| item.resource.native_resource_id == "reasoning-one")
        .unwrap();
    let command = active_history
        .items
        .iter()
        .find(|item| item.resource.native_resource_id == "command-one")
        .unwrap();
    let approval_item = active_history
        .items
        .iter()
        .find(|item| item.kind == codepet_provider_sdk::ConversationItemKind::Approval)
        .unwrap();
    assert!(agent.contents.is_empty());
    assert!(reasoning.contents.is_empty());
    assert_eq!(command.contents.len(), 1);
    assert_eq!(
        approval_item.status,
        codepet_provider_sdk::ConversationItemStatus::Approved
    );
    assert_eq!(
        approval_item
            .related_item
            .as_ref()
            .unwrap()
            .native_resource_id,
        "command-one"
    );
    assert_eq!(
        approval_item
            .approval
            .as_ref()
            .unwrap()
            .resource
            .native_resource_id,
        resolved_approval_id
    );

    let steered = ProviderProtocolServer::turn_steer(
        &provider,
        TurnSteerRequest {
            conversation: conversation.resource.clone(),
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
            conversation: conversation.resource,
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
}

#[test]
fn provider_binary_rejects_additional_network_permission_without_publishing_approval() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unsupported-approval.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("additional-network", &marker);

    provider.request(
        "turn-unsafe",
        "turn.start",
        turn_start_params(
            conversation,
            "message-unsafe",
            "request unsafe approval",
            &capability_revision,
        ),
    );
    wait_for_file_blocking(&marker);
    provider.collect_for(Duration::from_millis(100));

    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "error:-32601");
    assert!(provider.buffered.iter().all(|message| {
        message.get("method").and_then(Value::as_str) != Some("event.approvalRequested")
    }));

    provider.request("stop-unsafe", "instance.stop", json!({ "route": route_value() }));
    provider.request("shutdown-unsafe", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_rejects_stale_approval_when_app_server_request_id_is_reused() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("approval-generation.txt");
    let mut provider = ProviderBinary::spawn();
    let (first_conversation, first_capability_revision) = provider.configure("normal", &marker);

    provider.request(
        "turn-first",
        "turn.start",
        turn_start_params(
            first_conversation,
            "message-first",
            "first session",
            &first_capability_revision,
        ),
    );
    let first_approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();

    provider.request("stop-first", "instance.stop", json!({ "route": route_value() }));
    let second_start = provider.request(
        "start-second",
        "instance.start",
        json!({ "route": route_value() }),
    );
    let second_capability_revision = second_start
        .pointer("/result/instance/capabilities/revision")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let second_conversation = provider.create_conversation("conversation-second");
    provider.request(
        "turn-second",
        "turn.start",
        turn_start_params(
            second_conversation.clone(),
            "message-second",
            "second session",
            &second_capability_revision,
        ),
    );
    let second_approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();

    assert_ne!(
        first_approval["nativeResourceId"],
        second_approval["nativeResourceId"]
    );
    let history = provider.request(
        "history-second",
        "conversation.get",
        json!({ "conversation": second_conversation }),
    );
    let approval_items = history
        .pointer("/result/items")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter(|item| item.get("kind").and_then(Value::as_str) == Some("approval"))
        .collect::<Vec<_>>();
    assert_eq!(approval_items.len(), 1);
    assert_eq!(
        approval_items[0].pointer("/resource/nativeResourceId"),
        second_approval.get("nativeResourceId")
    );
    let stale = provider.request(
        "resolve-stale",
        "approval.resolve",
        json!({ "approval": first_approval, "decision": "approve" }),
    );
    assert_eq!(
        stale.pointer("/error/data/code").and_then(Value::as_str),
        Some("stale_approval_session")
    );
    assert!(!marker.exists());

    let resolved = provider.request(
        "resolve-current",
        "approval.resolve",
        json!({ "approval": second_approval, "decision": "approve" }),
    );
    assert_eq!(
        resolved
            .pointer("/result/approval/status")
            .and_then(Value::as_str),
        Some("approved")
    );
    wait_for_file_blocking(&marker);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "accept");

    provider.request("stop-second", "instance.stop", json!({ "route": route_value() }));
    provider.request("shutdown-second", "provider.shutdown", json!({}));
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
    stdin
        .write_all(&vec![
            b'x';
            codepet_provider_sdk::MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES + 1
        ])
        .unwrap();
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
fn provider_binary_transports_a_complete_history_larger_than_one_mebibyte() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unused.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("none", &marker);

    let fetched = provider.request(
        "large-history",
        "conversation.get",
        json!({
            "conversation": {
                "deviceId": "device-provider-binary",
                "providerPluginId": CODEX_PLUGIN_ID,
                "providerInstanceId": "codex",
                "nativeResourceId": "thread-large"
            }
        }),
    );
    assert!(serde_json::to_vec(&fetched).unwrap().len() > 1024 * 1024);

    let described = provider.request("after-large-history", "provider.describe", json!({}));
    assert_eq!(
        described
            .pointer("/result/plugin/pluginId")
            .and_then(Value::as_str),
        Some(CODEX_PLUGIN_ID)
    );
    provider.request("large-history-shutdown", "provider.shutdown", json!({}));
}

#[test]
#[ignore = "requires CODEPET_CODEX_EXECUTABLE pointing to a real Codex CLI"]
fn provider_real_codex_app_server_smoke() {
    let executable = std::env::var_os("CODEPET_CODEX_EXECUTABLE")
        .map(PathBuf::from)
        .expect("CODEPET_CODEX_EXECUTABLE must point to the resolved Codex executable");
    assert!(executable.is_absolute());
    assert!(executable.is_file());
    let mut provider = ProviderBinary::spawn();

    provider.request(
        "real-initialize",
        "provider.initialize",
        json!({
            "hostClientId": "provider-real-codex-smoke",
            "hostDeviceId": "device-provider-binary",
            "hostVersion": "test",
            "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
        }),
    );
    provider.request(
        "real-create",
        "instance.create",
        json!({
            "route": route_value(),
            "instanceKind": CODEX_INSTANCE_KIND,
            "displayName": "Codex Real Smoke",
            "settings": {
                "appServerExecutable": executable.to_string_lossy(),
                "appServerArgs": ["app-server", "--listen", "stdio://"]
            }
        }),
    );
    let started = provider.request(
        "real-start",
        "instance.start",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        started.pointer("/result/instance/status").and_then(Value::as_str),
        Some("ready")
    );
    let listed = provider.request(
        "real-list",
        "conversation.list",
        json!({ "route": route_value(), "limit": 1 }),
    );
    assert!(listed.pointer("/result/conversations").is_some());
    let stopped = provider.request(
        "real-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        stopped.pointer("/result/instance/status").and_then(Value::as_str),
        Some("stopped")
    );
    provider.request("real-shutdown", "provider.shutdown", json!({}));
}

struct ProviderBinary {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    messages: mpsc::Receiver<Value>,
    buffered: VecDeque<Value>,
}

impl ProviderBinary {
    fn spawn() -> Self {
        let mut child = Command::new(provider_executable())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                sender.send(serde_json::from_str(&line).unwrap()).unwrap();
            }
        });
        Self {
            child,
            stdin,
            messages,
            buffered: VecDeque::new(),
        }
    }

    fn configure(&mut self, approval_mode: &str, marker: &Path) -> (Value, String) {
        self.request(
            "initialize",
            "provider.initialize",
            json!({
                "hostClientId": "provider-binary-test",
                "hostDeviceId": "device-provider-binary",
                "hostVersion": "test",
                "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
            }),
        );
        self.request(
            "create",
            "instance.create",
            json!({
                "route": route_value(),
                "instanceKind": CODEX_INSTANCE_KIND,
                "displayName": "Codex Binary Fixture",
                "settings": {
                    "appServerExecutable": app_server_executable(),
                    "appServerArgs": [
                        "--approval-mode",
                        approval_mode,
                        "--marker",
                        marker
                    ]
                }
            }),
        );
        let started = self.request("start", "instance.start", json!({ "route": route_value() }));
        let capability_revision = started
            .pointer("/result/instance/capabilities/revision")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        (
            self.create_conversation("conversation-first"),
            capability_revision,
        )
    }

    fn create_conversation(&mut self, id: &str) -> Value {
        self.request(
            id,
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "model": "gpt-fixture",
                "reasoningEffort": "high",
                "workspaceRoot": "/fixture/workspace"
            }),
        )
        .pointer("/result/conversation/resource")
        .cloned()
        .unwrap()
    }

    fn request(&mut self, id: &str, method: &str, params: Value) -> Value {
        serde_json::to_writer(
            &mut self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
        self.receive(Duration::from_secs(5), |message| {
            message.get("id").and_then(Value::as_str) == Some(id)
        })
    }

    fn event(&mut self, method: &str) -> Value {
        self.receive(Duration::from_secs(5), |message| {
            message.get("method").and_then(Value::as_str) == Some(method)
        })
    }

    fn receive<F>(&mut self, timeout: Duration, predicate: F) -> Value
    where
        F: Fn(&Value) -> bool,
    {
        if let Some(index) = self.buffered.iter().position(&predicate) {
            return self.buffered.remove(index).unwrap();
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self.messages.recv_timeout(remaining).unwrap();
            if predicate(&message) {
                return message;
            }
            self.buffered.push_back(message);
        }
    }

    fn collect_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            match self.messages.recv_timeout(remaining) {
                Ok(message) => self.buffered.push_back(message),
                Err(mpsc::RecvTimeoutError::Timeout) => return,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

impl Drop for ProviderBinary {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn route_value() -> Value {
    json!({
        "deviceId": "device-provider-binary",
        "providerPluginId": CODEX_PLUGIN_ID,
        "providerInstanceId": "codex"
    })
}

fn turn_start_params(
    conversation: Value,
    client_request_id: &str,
    text: &str,
    capability_revision: &str,
) -> Value {
    json!({
        "conversation": conversation,
        "clientRequestId": client_request_id,
        "capabilityRevision": capability_revision,
        "input": { "kind": "text", "text": text },
        "selection": {}
    })
}

fn wait_for_file_blocking(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture marker was not written");
        std::thread::sleep(Duration::from_millis(5));
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
