use codepet_provider_codex::{CodexProvider, CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationCreateRequest, ConversationListRequest,
    ConversationGetRequest, ConversationSearchRequest, InstanceCapabilitiesRequest, InstanceCreateRequest,
    InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest, JsonObject, ProtocolEvent,
    ProtocolServer as ProviderProtocolServer, ProviderInitializeRequest,
    FlatModelCatalogKind, FlatModelSelection, ModelSelection, ProviderInstanceRoute,
    ProviderShutdownRequest, RoutedResourceId, TurnInput, TurnInputKind, TurnInterruptRequest,
    TurnSelection, TurnStartRequest, TurnSteerRequest, VersionRange, PROTOCOL_VERSION,
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

async fn configured_direct_provider(
    approval_mode: &str,
    marker: &Path,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    let provider = Arc::new(CodexProvider::new(Arc::new(|_| Ok(()))));
    let route = route("device-provider-direct");
    ProviderProtocolServer::provider_initialize(
        provider.as_ref(),
        ProviderInitializeRequest {
            host_client_id: "client-provider-direct".to_string(),
            host_device_id: route.device_id.clone(),
            host_version: "test".to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::instance_create(
        provider.as_ref(),
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CODEX_INSTANCE_KIND.to_string(),
            display_name: "Codex Direct Fixture".to_string(),
            settings: instance_settings(&app_server_executable(), approval_mode, marker),
        },
    )
    .await
    .unwrap();
    let started = ProviderProtocolServer::instance_start(
        provider.as_ref(),
        InstanceStartRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    (
        provider,
        route,
        started.instance.capabilities.revision,
    )
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
    let reasoning = capabilities
        .capabilities
        .turn_send
        .as_ref()
        .and_then(|turn_send| turn_send.reasoning_effort.as_ref())
        .unwrap();
    assert_eq!(
        reasoning
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect::<Vec<_>>(),
        vec!["high"]
    );

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
    let rejected_reasoning = ProviderProtocolServer::turn_start(
        &provider,
        TurnStartRequest {
            conversation: conversation.resource.clone(),
            client_request_id: "message-rejected".to_string(),
            capability_revision: capabilities.capabilities.revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "must not start".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: Some("workspace-write".to_string()),
                reasoning_effort_id: Some("low".to_string()),
                model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                    kind: FlatModelCatalogKind::Flat,
                    model_id: "gpt-fixture".to_string(),
                })),
            },
        },
    )
    .await
    .unwrap_err();
    assert_eq!(rejected_reasoning.code, "invalid_turn_selection");
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
    assert!(started_turn.user_item.is_none());
    let refreshed = ProviderProtocolServer::conversation_get(
        &provider,
        ConversationGetRequest {
            conversation: conversation.resource.clone(),
        },
    )
    .await
    .unwrap();
    assert!(refreshed
        .items
        .iter()
        .any(|item| item.resource.native_resource_id == "user-one"));
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

    let started = provider.request(
        "turn-unsafe",
        "turn.start",
        turn_start_params(
            conversation,
            "message-unsafe",
            "request unsafe approval",
            &capability_revision,
        ),
    );
    assert_eq!(started.pointer("/result/userItem"), Some(&Value::Null));
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
fn provider_binary_conversation_get_is_pure_read_with_external_writer() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unused-approval.txt");
    let request_log = directory.path().join("app-server-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("none", &marker, Some(&request_log));
    std::fs::write(&request_log, "").unwrap();
    clear_session_log(&marker);
    let conversation = json!({
        "deviceId": "device-provider-binary",
        "providerPluginId": CODEX_PLUGIN_ID,
        "providerInstanceId": "codex",
        "nativeResourceId": "thread-writer-held"
    });

    let first = provider.request(
        "writer-held-first",
        "conversation.get",
        json!({ "conversation": conversation.clone() }),
    );
    let second = provider.request(
        "writer-held-second",
        "conversation.get",
        json!({ "conversation": conversation }),
    );

    for response in [&first, &second] {
        assert!(response.get("error").is_none());
        assert_eq!(
            response
                .pointer("/result/conversation/resource/nativeResourceId")
                .and_then(Value::as_str),
            Some("thread-writer-held")
        );
        assert_eq!(
            response
                .pointer("/result/items")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(8)
        );
    }
    assert_eq!(first.pointer("/result"), second.pointer("/result"));
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-writer-held",
            "thread/read\tthread-writer-held"
        ]
    );
    assert!(session_pids(&marker, "process/start", "").is_empty());
    assert!(session_pids(&marker, "thread/resume", "thread-writer-held").is_empty());

    provider.request("writer-held-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("writer-held-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reuses_one_execution_session_until_authoritative_terminal_state() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("turn-lifecycle.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("normal", &marker);
    let observer_pid = session_pids(&marker, "model/list", "")[0];
    let creation_pid = session_pids(&marker, "thread/start", "")[0];
    assert_ne!(observer_pid, creation_pid);
    clear_session_log(&marker);

    let started = provider.request(
        "lifecycle-turn-start",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "lifecycle-message-one",
            "start lifecycle",
            &capability_revision,
        ),
    );
    let turn = started
        .pointer("/result/turn/resource")
        .cloned()
        .unwrap();
    let approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();

    provider.collect_for(Duration::from_millis(50));
    let resolved = provider.request(
        "lifecycle-approval",
        "approval.resolve",
        json!({ "approval": approval, "decision": "approve" }),
    );
    assert_eq!(
        resolved
            .pointer("/result/approval/status")
            .and_then(Value::as_str),
        Some("approved")
    );
    let steered = provider.request(
        "lifecycle-steer",
        "turn.steer",
        json!({
            "conversation": conversation.clone(),
            "turn": turn.clone(),
            "clientMessageId": "lifecycle-message-two",
            "message": "continue lifecycle"
        }),
    );
    assert!(steered.get("error").is_none());
    let interrupted = provider.request(
        "lifecycle-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation.clone(), "turn": turn }),
    );
    assert_eq!(
        interrupted
            .pointer("/result/turn/status")
            .and_then(Value::as_str),
        Some("interrupted")
    );

    let restarted = provider.request(
        "lifecycle-turn-restart",
        "turn.start",
        turn_start_params(
            conversation,
            "lifecycle-message-three",
            "restart after terminal snapshot",
            &capability_revision,
        ),
    );
    assert!(restarted.get("error").is_none());

    let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
    assert_eq!(resume_pids.len(), 2);
    let first_execution = resume_pids[0];
    assert_ne!(observer_pid, first_execution);
    assert_eq!(
        session_pids(&marker, "approval/response", "thread-created"),
        vec![first_execution]
    );
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-created"),
        vec![first_execution]
    );
    assert_eq!(
        session_pids(&marker, "turn/interrupt", "thread-created"),
        vec![first_execution]
    );
    assert_ne!(resume_pids[0], resume_pids[1]);

    provider.request("lifecycle-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("lifecycle-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_keeps_waiting_user_input_execution_detached_from_remote_reads() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("waiting-user-input.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) =
        provider.configure("waiting-user-input", &marker);
    let observer_pid = session_pids(&marker, "model/list", "")[0];
    clear_session_log(&marker);

    let started = provider.request(
        "waiting-input-start",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "waiting-input-message",
            "wait for input",
            &capability_revision,
        ),
    );
    let turn = started
        .pointer("/result/turn/resource")
        .cloned()
        .unwrap();
    let execution_pid = session_pids(&marker, "thread/resume", "thread-created")[0];
    let fetched = provider.request(
        "waiting-input-read",
        "conversation.get",
        json!({ "conversation": conversation.clone() }),
    );
    assert_eq!(
        fetched
            .pointer("/result/conversation/status")
            .and_then(Value::as_str),
        Some("waiting-user-input")
    );

    provider.collect_for(Duration::from_millis(50));
    let steered = provider.request(
        "waiting-input-steer",
        "turn.steer",
        json!({
            "conversation": conversation.clone(),
            "turn": turn.clone(),
            "clientMessageId": "waiting-input-follow-up",
            "message": "input supplied"
        }),
    );
    assert!(steered.get("error").is_none());
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-created"),
        vec![execution_pid]
    );
    let read_pids = session_pids(&marker, "thread/read", "thread-created");
    assert!(read_pids.contains(&observer_pid));
    assert!(read_pids.contains(&execution_pid));
    assert_eq!(session_pids(&marker, "thread/resume", "thread-created").len(), 1);

    provider.request(
        "waiting-input-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation, "turn": turn }),
    );
    provider.request("waiting-input-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("waiting-input-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_terminal_notification_releases_only_its_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("parallel-terminal.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("complete-on-approval", &marker);
    clear_session_log(&marker);
    let conversation_a = conversation_resource_value("thread-a");
    let conversation_b = conversation_resource_value("thread-b");

    let started_a = provider.request(
        "parallel-start-a",
        "turn.start",
        turn_start_params(
            conversation_a.clone(),
            "parallel-message-a",
            "run A",
            &capability_revision,
        ),
    );
    let turn_a = started_a.pointer("/result/turn/resource").cloned().unwrap();
    let approval_one = provider
        .event("event.approvalRequested")
        .pointer("/params/approval")
        .cloned()
        .unwrap();
    let started_b = provider.request(
        "parallel-start-b",
        "turn.start",
        turn_start_params(
            conversation_b.clone(),
            "parallel-message-b",
            "run B",
            &capability_revision,
        ),
    );
    let turn_b = started_b.pointer("/result/turn/resource").cloned().unwrap();
    let approval_two = provider
        .event("event.approvalRequested")
        .pointer("/params/approval")
        .cloned()
        .unwrap();
    let (approval_a, _approval_b) = if approval_one
        .pointer("/conversation/nativeResourceId")
        .and_then(Value::as_str)
        == Some("thread-a")
    {
        (approval_one, approval_two)
    } else {
        (approval_two, approval_one)
    };

    provider.request(
        "parallel-resolve-a",
        "approval.resolve",
        json!({ "approval": approval_a["resource"].clone(), "decision": "approve" }),
    );
    provider.receive(Duration::from_secs(5), |message| {
        message.get("method").and_then(Value::as_str) == Some("event.turnUpserted")
            && message
                .pointer("/params/turn/conversation/nativeResourceId")
                .and_then(Value::as_str)
                == Some("thread-a")
            && message
                .pointer("/params/turn/status")
                .and_then(Value::as_str)
                == Some("completed")
    });

    let steered_b = provider.request(
        "parallel-steer-b",
        "turn.steer",
        json!({
            "conversation": conversation_b.clone(),
            "turn": turn_b.clone(),
            "clientMessageId": "parallel-message-b-two",
            "message": "B remains active"
        }),
    );
    assert!(steered_b.get("error").is_none());
    let restarted_a = provider.request(
        "parallel-restart-a",
        "turn.start",
        turn_start_params(
            conversation_a,
            "parallel-message-a-two",
            "A starts again",
            &capability_revision,
        ),
    );
    assert!(restarted_a.get("error").is_none());

    let a_pids = session_pids(&marker, "thread/resume", "thread-a");
    let b_pids = session_pids(&marker, "thread/resume", "thread-b");
    assert_eq!(a_pids.len(), 2);
    assert_eq!(b_pids.len(), 1);
    assert_ne!(a_pids[0], a_pids[1]);
    assert_ne!(a_pids[0], b_pids[0]);
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-b"),
        vec![b_pids[0]]
    );

    provider.request("parallel-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("parallel-start-provider", "instance.start", json!({ "route": route_value() }));
    let resumed_b = provider.request(
        "parallel-steer-b-after-stop",
        "turn.steer",
        json!({
            "conversation": conversation_b,
            "turn": turn_b,
            "clientMessageId": "parallel-message-b-three",
            "message": "B resumes after Provider stop"
        }),
    );
    assert!(resumed_b.get("error").is_none());
    let b_pids_after_stop = session_pids(&marker, "thread/resume", "thread-b");
    assert_eq!(b_pids_after_stop.len(), 2);
    assert_ne!(b_pids_after_stop[0], b_pids_after_stop[1]);

    provider.request("parallel-stop-final", "instance.stop", json!({ "route": route_value() }));
    provider.request("parallel-shutdown", "provider.shutdown", json!({}));
    let _ = turn_a;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_first_turn_start_shares_one_resume_and_one_active_turn() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("concurrent-resume.txt");
    let (provider, route, capability_revision) =
        configured_direct_provider("none", &marker).await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-concurrent");
    let request = |client_request_id: &str| TurnStartRequest {
        conversation: conversation.clone(),
        client_request_id: client_request_id.to_string(),
        capability_revision: capability_revision.clone(),
        input: TurnInput {
            kind: TurnInputKind::Text,
            text: "concurrent start".to_string(),
        },
        selection: TurnSelection {
            access_mode_id: None,
            reasoning_effort_id: None,
            model: None,
        },
    };
    let first = ProviderProtocolServer::turn_start(provider.as_ref(), request("concurrent-one"));
    let second = ProviderProtocolServer::turn_start(provider.as_ref(), request("concurrent-two"));
    let (first, second) = tokio::join!(first, second);
    let results = [first, second];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(|error| error.code.as_str())
            .collect::<Vec<_>>(),
        vec!["turn_already_active"]
    );
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-concurrent").len(),
        1
    );
    assert_eq!(
        session_pids(&marker, "turn/start", "thread-concurrent").len(),
        1
    );

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_stop_interrupts_an_execution_still_waiting_for_resume() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stop-during-resume.txt");
    let (provider, route, capability_revision) =
        configured_direct_provider("resume-no-response", &marker).await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-resume-pending");
    let operation_provider = provider.clone();
    let operation = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            operation_provider.as_ref(),
            TurnStartRequest {
                conversation,
                client_request_id: "resume-pending-message".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "wait for resume".to_string(),
                },
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while session_pids(&marker, "thread/resume", "thread-resume-pending").is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let stopped = ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    assert_eq!(stopped.instance.status, codepet_provider_sdk::InstanceStatus::Stopped);
    let operation_error = tokio::time::timeout(Duration::from_secs(3), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(operation_error.code, "provider_unavailable");
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-resume-pending").len(),
        1
    );

    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test]
async fn execution_initialize_failure_removes_the_conversation_slot() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("execution-initialize-reject.txt");
    let (provider, route, capability_revision) =
        configured_direct_provider("execution-initialize-reject", &marker).await;
    let conversation = conversation_resource(&route, "thread-initialize-reject");

    for client_request_id in ["initialize-reject-one", "initialize-reject-two"] {
        let error = ProviderProtocolServer::turn_start(
            provider.as_ref(),
            TurnStartRequest {
                conversation: conversation.clone(),
                client_request_id: client_request_id.to_string(),
                capability_revision: capability_revision.clone(),
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "initialize failure".to_string(),
                },
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "provider_error");
    }
    assert_eq!(session_pids(&marker, "initialize", "").len(), 3);
    assert!(session_pids(&marker, "thread/resume", "thread-initialize-reject").is_empty());

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[test]
fn provider_binary_standardizes_writer_conflict_and_removes_failed_execution() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("writer-conflict.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("none", &marker);
    clear_session_log(&marker);
    let conversation = conversation_resource_value("thread-writer-held");

    for (id, client_request_id) in [
        ("writer-conflict-first", "writer-conflict-message-one"),
        ("writer-conflict-second", "writer-conflict-message-two"),
    ] {
        let response = provider.request(
            id,
            "turn.start",
            turn_start_params(
                conversation.clone(),
                client_request_id,
                "writer conflict",
                &capability_revision,
            ),
        );
        assert_eq!(
            response.pointer("/error/data/code").and_then(Value::as_str),
            Some("conversation_write_conflict")
        );
        assert_eq!(
            response
                .pointer("/error/data/retryable")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            response
                .pointer("/error/data/details/operation")
                .and_then(Value::as_str),
            Some("thread/resume")
        );
        assert_eq!(
            response
                .pointer("/error/data/details/reason")
                .and_then(Value::as_str),
            Some("owned-by-other-runtime")
        );
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(!encoded.contains("writer is held"));
        assert!(!encoded.contains("another process"));
        assert!(!encoded.contains("another client"));
    }
    let resume_pids = session_pids(&marker, "thread/resume", "thread-writer-held");
    assert_eq!(resume_pids.len(), 2);
    assert_ne!(resume_pids[0], resume_pids[1]);
    assert!(session_pids(&marker, "turn/start", "thread-writer-held").is_empty());

    provider.request("writer-conflict-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("writer-conflict-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_cleans_explicit_reject_and_sent_unknown_without_automatic_retry() {
    for (mode, expected_code) in [
        ("turn-reject", "provider_error"),
        ("turn-sent-unknown", "provider_unavailable"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join(format!("{mode}.txt"));
        let mut provider = ProviderBinary::spawn();
        let (conversation, capability_revision) = provider.configure(mode, &marker);
        clear_session_log(&marker);

        let first = provider.request(
            "failed-turn-first",
            "turn.start",
            turn_start_params(
                conversation.clone(),
                "failed-message-one",
                "fail once",
                &capability_revision,
            ),
        );
        assert_eq!(
            first.pointer("/error/data/code").and_then(Value::as_str),
            Some(expected_code)
        );
        assert_eq!(
            session_pids(&marker, "thread/resume", "thread-created").len(),
            1
        );
        assert_eq!(
            session_pids(&marker, "turn/start", "thread-created").len(),
            1
        );

        let second = provider.request(
            "failed-turn-second",
            "turn.start",
            turn_start_params(
                conversation,
                "failed-message-two",
                "explicit caller retry",
                &capability_revision,
            ),
        );
        assert_eq!(
            second.pointer("/error/data/code").and_then(Value::as_str),
            Some(expected_code)
        );
        let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
        assert_eq!(resume_pids.len(), 2);
        assert_ne!(resume_pids[0], resume_pids[1]);
        assert_eq!(
            session_pids(&marker, "turn/start", "thread-created").len(),
            2
        );

        provider.request("failed-turn-stop", "instance.stop", json!({ "route": route_value() }));
        provider.request("failed-turn-shutdown", "provider.shutdown", json!({}));
    }
}

#[test]
fn provider_binary_async_execution_crash_drops_the_mapped_session() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("execution-crash.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("crash-after-start", &marker);
    clear_session_log(&marker);

    let started = provider.request(
        "crash-start",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "crash-message-one",
            "start before crash",
            &capability_revision,
        ),
    );
    assert!(started.get("error").is_none());
    let turn = started
        .pointer("/result/turn/resource")
        .cloned()
        .unwrap();
    provider.collect_for(Duration::from_millis(100));

    let steered = provider.request(
        "crash-steer",
        "turn.steer",
        json!({
            "conversation": conversation,
            "turn": turn,
            "clientMessageId": "crash-message-two",
            "message": "resume after execution crash"
        }),
    );
    assert!(steered.get("error").is_none());
    let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
    assert_eq!(resume_pids.len(), 2);
    assert_ne!(resume_pids[0], resume_pids[1]);

    provider.request("crash-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("crash-shutdown", "provider.shutdown", json!({}));
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
        self.configure_with_request_log(approval_mode, marker, None)
    }

    fn configure_with_request_log(
        &mut self,
        approval_mode: &str,
        marker: &Path,
        request_log: Option<&Path>,
    ) -> (Value, String) {
        let mut app_server_args = vec![
            "--approval-mode".to_string(),
            approval_mode.to_string(),
            "--marker".to_string(),
            marker.to_string_lossy().into_owned(),
        ];
        if let Some(request_log) = request_log {
            app_server_args.push("--request-log".to_string());
            app_server_args.push(request_log.to_string_lossy().into_owned());
        }
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
                    "appServerArgs": app_server_args
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

fn conversation_resource_value(native_resource_id: &str) -> Value {
    let mut resource = route_value();
    resource["nativeResourceId"] = json!(native_resource_id);
    resource
}

fn conversation_resource(route: &ProviderInstanceRoute, native_resource_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_resource_id.to_string(),
    }
}

fn clear_session_log(marker: &Path) {
    std::fs::write(marker.with_extension("sessions"), "").unwrap();
}

fn session_pids(marker: &Path, method: &str, thread_id: &str) -> Vec<u32> {
    let contents = std::fs::read_to_string(marker.with_extension("sessions")).unwrap();
    let mut pids = Vec::new();
    for line in contents.lines() {
        let mut fields = line.splitn(3, '\t');
        let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        if fields.next() != Some(method) || fields.next() != Some(thread_id) {
            continue;
        }
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
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
