use codepet_provider_claude::{
    decode_claude_output, ClaudeOutput, ClaudeProvider, ProviderEventSink,
    CLAUDE_INSTANCE_KIND, CLAUDE_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationContentKind, ConversationCreateRequest,
    ConversationGetRequest, ConversationListRequest, ConversationProjectFilter,
    ConversationProjectFilterAll, ConversationProjectFilterAllKind, InstanceCapabilitiesRequest,
    InstanceCreateRequest, InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest,
    JsonLineCodec, JsonObject, ProtocolEvent, ProtocolServer as ProviderProtocolServer,
    ProviderCapability, ProviderInitializeRequest, ProviderInstanceRoute, ProviderResourceId,
    ProviderShutdownRequest, ProviderWireMessage, RoutedResourceId, TurnInput, TurnInputKind, TurnInterruptRequest,
    TurnSelection, TurnStartRequest, TurnStatus, TurnSteerRequest, VersionRange,
    PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn provider_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codepet-provider-claude"))
}

fn all_project_filter() -> ConversationProjectFilter {
    ConversationProjectFilter::ConversationProjectFilterAll(ConversationProjectFilterAll {
        kind: ConversationProjectFilterAllKind::All,
    })
}

fn turn_start_request(
    conversation: ProviderResourceId,
    client_request_id: String,
    text: String,
) -> TurnStartRequest {
    TurnStartRequest {
        conversation,
        client_request_id,
        capability_revision: "claude-cli-stream-json-controls-v1".to_string(),
        input: TurnInput {
            kind: TurnInputKind::Text,
            text,
        },
        selection: TurnSelection {
            access_mode_id: None,
            reasoning_effort_id: None,
            model: None,
        },
    }
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
    codepet_provider_sdk::Conversation,
) {
    ready_provider_with_permission(workspace_root, "workspace-write").await
}

async fn ready_provider_with_permission(
    workspace_root: &Path,
    permission_level: &str,
) -> (
    Arc<ClaudeProvider>,
    mpsc::Receiver<ProtocolEvent>,
    ProviderInstanceRoute,
    codepet_provider_sdk::Conversation,
) {
    let (event_sender, event_receiver) = mpsc::channel();
    let events: Arc<dyn ProviderEventSink> = Arc::new(move |event: ProtocolEvent| {
        JsonLineCodec::default()
            .encode_message(&ProviderWireMessage::Event(event.clone()))?;
        event_sender
            .send(event)
            .map_err(|error| codepet_provider_sdk::ProtocolError {
                code: "test_event_sink_closed".to_string(),
                message: error.to_string(),
                retryable: false,
                details: None,
            })
    });
    let (provider, route, conversation) = configured_provider(
        workspace_root,
        permission_level,
        &fixture_executable(),
        events,
    )
    .await;
    (provider, event_receiver, route, conversation)
}

async fn configured_provider(
    workspace_root: &Path,
    permission_level: &str,
    executable: &Path,
    events: Arc<dyn ProviderEventSink>,
) -> (
    Arc<ClaudeProvider>,
    ProviderInstanceRoute,
    codepet_provider_sdk::Conversation,
) {
    let provider = Arc::new(ClaudeProvider::new(events));
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
    assert!(initialized.plugin.default_workspace_root.is_some());

    let described = ProviderProtocolServer::provider_describe(
        provider.as_ref(),
        codepet_provider_sdk::ProviderDescribeRequest {},
    )
    .await
    .unwrap();
    assert_eq!(described.plugin, initialized.plugin);

    let mut settings = instance_settings(executable);
    settings.insert(
        "claudeConfigDir".to_string(),
        json!(workspace_root.join(".claude-test").to_string_lossy()),
    );
    let created = ProviderProtocolServer::instance_create(
        provider.as_ref(),
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CLAUDE_INSTANCE_KIND.to_string(),
            display_name: "Claude Fixture".to_string(),
            settings,
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
            project: None,
            title: Some("Fixture conversation".to_string()),
            permission_level: permission_level.to_string(),
            model: Some("sonnet".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            workspace_mode: None,
            extension: None,
        },
    )
    .await
    .unwrap()
    .conversation;
    (provider, route, conversation)
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
    assert!(capabilities.methods.contains(&ProviderCapability::ConversationList));
    assert!(capabilities.methods.contains(&ProviderCapability::ConversationGet));
    assert!(capabilities.methods.contains(&ProviderCapability::TurnStart));
    #[cfg(unix)]
    assert!(capabilities.methods.contains(&ProviderCapability::TurnInterrupt));
    assert!(!capabilities.methods.contains(&ProviderCapability::ConversationSearch));
    assert!(!capabilities.methods.contains(&ProviderCapability::TurnSteer));
    assert!(capabilities.methods.contains(&ProviderCapability::ApprovalResolve));
    let controls = capabilities.turn_send.as_ref().unwrap();
    assert!(controls.access_mode.as_ref().is_some_and(|choices| choices.options.len() >= 4));
    assert!(controls.reasoning_effort.as_ref().is_some_and(|choices| choices.options.len() == 5));
    assert!(controls.model_catalog.is_some());

    let first_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "message-first".to_string(), "run fixture".to_string()),
    )
    .await
    .unwrap()
    .turn;
    assert_resource_route(&first_turn.resource, &route);
    assert_eq!(first_turn.conversation, conversation.resource);
    let first = terminal_turn(&events, &first_turn.resource.native_resource_id);
    assert_eq!(first.status, TurnStatus::Completed);
    assert_eq!(first.output, "fixture output");
    assert_eq!(first.deltas.len(), 2);
    for (index, delta) in first.deltas.iter().enumerate() {
        let item_id = format!("{}:text:{index}", first_turn.resource.native_resource_id);
        assert_eq!(delta.item_id, item_id);
        assert_eq!(delta.content_id, format!("{item_id}:text"));
        assert_eq!(delta.kind, ConversationContentKind::Text);
    }

    let second_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "message-second".to_string(), "run fixture again".to_string()),
    )
    .await
    .unwrap()
    .turn;
    let second = terminal_turn(&events, &second_turn.resource.native_resource_id);
    assert_eq!(second.status, TurnStatus::Completed);
    assert_eq!(second.output, "fixture resumed");

    let approval_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(
            provider_resource(&route, &conversation.resource),
            "message-approval".to_string(),
            "needs approval".to_string(),
        ),
    )
    .await
    .unwrap()
    .turn;
    let approval = loop {
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            ProtocolEvent::EventApprovalRequested { params, .. }
                if params.approval.turn == approval_turn.resource =>
            {
                break params.approval
            }
            _ => {}
        }
    };
    assert_eq!(approval.kind, "Bash");
    assert_eq!(approval.title, "Run a shell command");
    assert_eq!(approval.description.as_deref(), Some("touch approved.txt"));
    let resolved = ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: provider_resource(&route, &approval.resource),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap()
    .approval;
    assert_eq!(resolved.status, codepet_provider_sdk::ApprovalStatus::Approved);
    assert_eq!(resolved.decision, Some(ApprovalDecision::Approve));
    let approval_terminal = terminal_turn(&events, &approval_turn.resource.native_resource_id);
    assert_eq!(approval_terminal.status, TurnStatus::Completed);
    assert_eq!(approval_terminal.output, "fixture approved");

    let denied_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(
            provider_resource(&route, &conversation.resource),
            "message-denied".to_string(),
            "needs approval".to_string(),
        ),
    )
    .await
    .unwrap()
    .turn;
    let denied_approval = loop {
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            ProtocolEvent::EventApprovalRequested { params, .. }
                if params.approval.turn == denied_turn.resource =>
            {
                break params.approval
            }
            _ => {}
        }
    };
    let denied = ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: provider_resource(&route, &denied_approval.resource),
            decision: ApprovalDecision::Deny,
        },
    )
    .await
    .unwrap()
    .approval;
    assert_eq!(denied.status, codepet_provider_sdk::ApprovalStatus::Denied);
    let denied_terminal = terminal_turn(&events, &denied_turn.resource.native_resource_id);
    assert_eq!(denied_terminal.status, TurnStatus::Completed);
    assert_eq!(denied_terminal.output, "fixture denied");

    let failed_turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "message-failed".to_string(), "fail".to_string()),
    )
    .await
    .unwrap()
    .turn;
    let failed = terminal_turn(&events, &failed_turn.resource.native_resource_id);
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(failed.output, "Not logged in");
    assert_eq!(failed.deltas.len(), 1);
    let failed_item_id = format!("{}:result", failed_turn.resource.native_resource_id);
    assert_eq!(failed.deltas[0].item_id, failed_item_id);
    assert_eq!(failed.deltas[0].content_id, format!("{failed_item_id}:summary"));
    assert_eq!(failed.deltas[0].kind, ConversationContentKind::ActivitySummary);

    let listed = ProviderProtocolServer::conversation_list(
        provider.as_ref(),
        ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: None,
            project_filter: all_project_filter(),
        },
    )
    .await
    .unwrap();
    assert!(listed
        .conversations
        .iter()
        .any(|item| item.resource == conversation.resource));
    let fetched = ProviderProtocolServer::conversation_get(
        provider.as_ref(),
        ConversationGetRequest {
            conversation: provider_resource(&route, &conversation.resource),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(fetched.conversation.resource, conversation.resource);
    let steer_error = ProviderProtocolServer::turn_steer(
        provider.as_ref(),
        TurnSteerRequest {
            conversation: provider_resource(&route, &conversation.resource),
            turn: provider_resource(&route, &failed_turn.resource),
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
            approval: provider_resource(&route, &failed_turn.resource),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(approval_error.code, "approval_not_found");

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
        turn_start_request(provider_resource(&route, &conversation.resource), "message-interrupt".to_string(), "wait for interrupt".to_string()),
    )
    .await
    .unwrap()
    .turn;
    wait_for_claude_init(&events, &turn.resource.native_resource_id);

    let interrupted = ProviderProtocolServer::turn_interrupt(
        provider.as_ref(),
        TurnInterruptRequest {
            conversation: provider_resource(&route, &conversation.resource),
            turn: provider_resource(&route, &turn.resource),
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

#[tokio::test]
async fn provider_inherits_claude_project_configuration_and_rejects_strong_access_modes() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers": {
                "fixture-observed": {
                    "type": "http",
                    "url": "http://127.0.0.1:9/not-contacted"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
    assert_eq!(conversation.resource.provider_id, route.provider_instance_id);

    let inherited = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "inherit-project-config".to_string(), "inherit project config".to_string()),
    )
    .await
    .unwrap()
    .turn;
    let inherited = terminal_turn(&events, &inherited.resource.native_resource_id);
    assert_eq!(inherited.status, TurnStatus::Completed);
    assert_eq!(inherited.output, "fixture inherited project MCP");

    for permission_level in ["read-only", "full-access"] {
        let error = ProviderProtocolServer::conversation_create(
            provider.as_ref(),
            ConversationCreateRequest {
                route: route.clone(),
                project: None,
                title: Some("Unsupported access mode".to_string()),
                permission_level: permission_level.to_string(),
                model: Some("sonnet".to_string()),
                reasoning_effort: Some("high".to_string()),
                workspace_root: Some(workspace.path().to_string_lossy().to_string()),
                workspace_mode: None,
                extension: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "invalid_permission_level");
    }

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

#[tokio::test]
async fn provider_chunks_two_mib_result_before_the_provider_frame_limit() {
    let workspace = tempfile::tempdir().unwrap();
    let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
    let turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "large-result".to_string(), "two mib result".to_string()),
    )
    .await
    .unwrap()
    .turn;
    let terminal = terminal_turn(&events, &turn.resource.native_resource_id);
    assert_eq!(terminal.status, TurnStatus::Completed);
    assert_eq!(terminal.output.len(), 2 * 1024 * 1024);
    assert!(terminal.output.bytes().all(|byte| byte == b'x'));
    assert!(terminal.max_chunk_bytes <= 64 * 1024);
    let item_id = format!("{}:result", turn.resource.native_resource_id);
    assert!(!terminal.deltas.is_empty());
    assert!(terminal.deltas.iter().all(|delta| {
        delta.item_id == item_id
            && delta.content_id == format!("{item_id}:text")
            && delta.kind == ConversationContentKind::Text
    }));

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

#[tokio::test]
async fn provider_event_failure_still_publishes_a_small_failed_terminal() {
    let workspace = tempfile::tempdir().unwrap();
    let (event_sender, events) = mpsc::channel();
    let fail_first_delta = Arc::new(AtomicBool::new(true));
    let failure = fail_first_delta.clone();
    let sink: Arc<dyn ProviderEventSink> = Arc::new(move |event: ProtocolEvent| {
        JsonLineCodec::default()
            .encode_message(&ProviderWireMessage::Event(event.clone()))?;
        if matches!(event, ProtocolEvent::EventTurnOutputDelta { .. })
            && failure.swap(false, Ordering::SeqCst)
        {
            return Err(codepet_provider_sdk::ProtocolError {
                code: "test_output_rejected".to_string(),
                message: "reject one output event".to_string(),
                retryable: false,
                details: None,
            });
        }
        event_sender
            .send(event)
            .map_err(|error| codepet_provider_sdk::ProtocolError {
                code: "test_event_sink_closed".to_string(),
                message: error.to_string(),
                retryable: false,
                details: None,
            })
    });
    let (provider, route, conversation) = configured_provider(
        workspace.path(),
        "workspace-write",
        &fixture_executable(),
        sink,
    )
    .await;
    let turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "event-failure".to_string(), "run fixture".to_string()),
    )
    .await
    .unwrap()
    .turn;
    let terminal = terminal_turn(&events, &turn.resource.native_resource_id);
    assert_eq!(terminal.status, TurnStatus::Failed);
    assert!(terminal.output.is_empty());

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

#[cfg(unix)]
#[tokio::test]
async fn provider_reaps_result_interrupt_stdout_and_oversize_process_trees() {
    let workspace = tempfile::tempdir().unwrap();
    let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
    let turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "result-then-sleep".to_string(), "result then sleep".to_string()),
    )
    .await
    .unwrap()
    .turn;
    let pids = wait_for_probe_pids(workspace.path());
    assert_no_terminal(&events, &turn.resource.native_resource_id, Duration::from_millis(150));
    let active_error = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "must-stay-blocked".to_string(), "run fixture".to_string()),
    )
    .await
    .unwrap_err();
    assert_eq!(active_error.code, "turn_already_active");
    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route },
    )
    .await
    .unwrap();
    assert_eq!(
        terminal_turn(&events, &turn.resource.native_resource_id).status,
        TurnStatus::Interrupted
    );
    assert_pids_gone(pids);
    ProviderProtocolServer::provider_shutdown(
        provider.as_ref(),
        ProviderShutdownRequest {},
    )
    .await
    .unwrap();

    let workspace = tempfile::tempdir().unwrap();
    let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
    let turn = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        turn_start_request(provider_resource(&route, &conversation.resource), "ignore-sigint".to_string(), "ignore sigint".to_string()),
    )
    .await
    .unwrap()
    .turn;
    wait_for_claude_init(&events, &turn.resource.native_resource_id);
    let pids = wait_for_probe_pids(workspace.path());
    let interrupted = ProviderProtocolServer::turn_interrupt(
        provider.as_ref(),
        TurnInterruptRequest {
            conversation: provider_resource(&route, &conversation.resource),
            turn: provider_resource(&route, &turn.resource),
        },
    )
    .await
    .unwrap()
    .turn;
    assert_eq!(interrupted.status, TurnStatus::Interrupted);
    assert_eq!(
        terminal_turn(&events, &turn.resource.native_resource_id).status,
        TurnStatus::Interrupted
    );
    assert_pids_gone(pids);
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

    for message in ["stdout close then sleep", "oversized no newline"] {
        let workspace = tempfile::tempdir().unwrap();
        let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
        let turn = ProviderProtocolServer::turn_start(
            provider.as_ref(),
            turn_start_request(provider_resource(&route, &conversation.resource), message.to_string(), message.to_string()),
        )
        .await
        .unwrap()
        .turn;
        let pids = wait_for_probe_pids(workspace.path());
        let terminal = terminal_turn(&events, &turn.resource.native_resource_id);
        assert_eq!(terminal.status, TurnStatus::Failed);
        if message == "oversized no newline" {
            assert!(terminal.output.contains("exceeds 4194304 bytes"));
        }
        assert_pids_gone(pids);
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

    for lifecycle in ["destroy", "shutdown"] {
        let workspace = tempfile::tempdir().unwrap();
        let (provider, events, route, conversation) = ready_provider(workspace.path()).await;
        let turn = ProviderProtocolServer::turn_start(
            provider.as_ref(),
            turn_start_request(provider_resource(&route, &conversation.resource), format!("active-{lifecycle}"), "result then sleep".to_string()),
        )
        .await
        .unwrap()
        .turn;
        let pids = wait_for_probe_pids(workspace.path());
        if lifecycle == "destroy" {
            assert!(ProviderProtocolServer::instance_destroy(
                provider.as_ref(),
                InstanceDestroyRequest {
                    route: route.clone(),
                },
            )
            .await
            .unwrap()
            .destroyed);
        } else {
            assert!(ProviderProtocolServer::provider_shutdown(
                provider.as_ref(),
                ProviderShutdownRequest {},
            )
            .await
            .unwrap()
            .accepted);
        }
        assert_eq!(
            terminal_turn(&events, &turn.resource.native_resource_id).status,
            TurnStatus::Interrupted
        );
        assert_pids_gone(pids);
        if lifecycle == "destroy" {
            ProviderProtocolServer::provider_shutdown(
                provider.as_ref(),
                ProviderShutdownRequest {},
            )
            .await
            .unwrap();
        }
    }
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
    let missing_instance = binary_request(
        &mut stdin,
        &mut stdout,
        "list",
        "conversation.list",
        json!({
            "route": {
                "deviceId": "device-binary",
                "providerPluginId": CLAUDE_PLUGIN_ID,
                "providerInstanceId": "claude"
            },
            "projectFilter": { "kind": "all" }
        }),
    );
    assert_eq!(
        missing_instance
            .pointer("/error/data/code")
            .and_then(Value::as_str),
        Some("unknown_provider_instance")
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

#[cfg(unix)]
#[test]
fn provider_binary_reaps_active_tree_after_response_pipe_breaks() {
    let workspace = tempfile::tempdir().unwrap();
    let ActiveProviderBinary {
        mut child,
        mut stdin,
        stdout,
        stdout_backpressure: _,
        pids,
    } = active_provider_binary(workspace.path());
    drop(stdout);
    serde_json::to_writer(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "broken-response-pipe",
            "method": "provider.describe",
            "params": {}
        }),
    )
    .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();

    let status = wait_for_provider_exit(&mut child);
    drop(stdin);
    assert!(!status.success());
    assert_pids_gone(pids);
}

#[cfg(unix)]
#[test]
fn provider_binary_reaps_active_tree_after_invalid_json_under_stdout_backpressure() {
    let workspace = tempfile::tempdir().unwrap();
    let ActiveProviderBinary {
        mut child,
        mut stdin,
        stdout: _stdout,
        mut stdout_backpressure,
        pids,
    } = active_provider_binary(workspace.path());
    saturate_provider_stdout(&mut stdout_backpressure);
    stdin.write_all(b"{not-json}\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let status = wait_for_provider_exit(&mut child);
    assert!(!status.success());
    assert_pids_gone(pids);
}

#[cfg(unix)]
#[test]
fn provider_binary_reaps_active_tree_after_an_oversized_host_frame_under_stdout_backpressure() {
    let workspace = tempfile::tempdir().unwrap();
    let ActiveProviderBinary {
        mut child,
        mut stdin,
        stdout: _stdout,
        mut stdout_backpressure,
        pids,
    } = active_provider_binary(workspace.path());
    saturate_provider_stdout(&mut stdout_backpressure);
    stdin
        .write_all(&vec![
            b'x';
            codepet_provider_sdk::MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES + 1
        ])
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let status = wait_for_provider_exit(&mut child);
    assert!(!status.success());
    assert_pids_gone(pids);
}

#[test]
fn provider_binary_returns_a_standard_error_then_fail_stops_after_an_oversized_host_frame() {
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
    let responses = output.lines().collect::<Vec<_>>();
    assert_eq!(responses.len(), 1, "{output}");
    let response: Value = serde_json::from_str(responses[0]).unwrap();
    assert_eq!(response["jsonrpc"], "2.0");
    assert!(response["id"].is_null());
    assert_eq!(response["error"]["code"], -32600);
    assert!(!output.contains("must-not-run"));
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
    max_chunk_bytes: usize,
    deltas: Vec<CapturedDelta>,
}

struct CapturedDelta {
    item_id: String,
    content_id: String,
    kind: ConversationContentKind,
}

fn terminal_turn(events: &mpsc::Receiver<ProtocolEvent>, turn_id: &str) -> TerminalTurn {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = String::new();
    let mut max_chunk_bytes = 0;
    let mut deltas = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = events.recv_timeout(remaining).unwrap();
        match event {
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.turn.native_resource_id == turn_id =>
            {
                max_chunk_bytes = max_chunk_bytes.max(params.delta.len());
                output.push_str(&params.delta);
                deltas.push(CapturedDelta {
                    item_id: params.item_id,
                    content_id: params.content_id,
                    kind: params.kind,
                });
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
                    max_chunk_bytes,
                    deltas,
                };
            }
            _ => {}
        }
    }
}

#[cfg(unix)]
fn assert_no_terminal(
    events: &mpsc::Receiver<ProtocolEvent>,
    turn_id: &str,
    duration: Duration,
) {
    let deadline = Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        match events.recv_timeout(remaining) {
            Ok(ProtocolEvent::EventTurnUpserted { params, .. })
                if params.turn.resource.native_resource_id == turn_id
                    && matches!(
                        params.turn.status,
                        TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Interrupted
                    ) =>
            {
                panic!("turn reached terminal before the Claude process exited")
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return,
            Err(error) => panic!("event channel closed: {error}"),
        }
    }
}

#[cfg(unix)]
fn wait_for_probe_pids(workspace: &Path) -> [u32; 2] {
    let deadline = Instant::now() + Duration::from_secs(5);
    let root_path = workspace.join("fixture-root.pid");
    let child_path = workspace.join("fixture-child.pid");
    loop {
        if let (Ok(root), Ok(child)) = (
            std::fs::read_to_string(&root_path),
            std::fs::read_to_string(&child_path),
        ) {
            return [root.trim().parse().unwrap(), child.trim().parse().unwrap()];
        }
        assert!(Instant::now() < deadline, "fixture PID probes were not written");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn assert_pids_gone(pids: [u32; 2]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    for pid in pids {
        loop {
            let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Claude process tree PID {pid} still exists"
            );
            std::thread::sleep(Duration::from_millis(10));
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
    assert_eq!(resource.provider_id, route.provider_instance_id);
    assert!(!resource.native_resource_id.is_empty());
}

fn provider_resource(
    route: &ProviderInstanceRoute,
    resource: &RoutedResourceId,
) -> ProviderResourceId {
    ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: resource.native_resource_id.clone(),
    }
}

#[cfg(unix)]
struct ActiveProviderBinary {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<UnixStream>,
    stdout_backpressure: UnixStream,
    pids: [u32; 2],
}

#[cfg(unix)]
fn active_provider_binary(workspace: &Path) -> ActiveProviderBinary {
    let (provider_stdout, host_stdout) = UnixStream::pair().unwrap();
    let stdout_backpressure = provider_stdout.try_clone().unwrap();
    let provider_stdout: OwnedFd = provider_stdout.into();
    let mut child = Command::new(provider_executable())
        .stdin(Stdio::piped())
        .stdout(Stdio::from(provider_stdout))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(host_stdout);
    let mut pending = Vec::new();
    binary_request_collecting_events(
        &mut stdin,
        &mut stdout,
        "active-initialize",
        "provider.initialize",
        json!({
            "hostClientId": "active-binary",
            "hostDeviceId": "active-device",
            "hostVersion": "test",
            "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
        }),
        &mut pending,
    );
    let route = json!({
        "deviceId": "active-device",
        "providerPluginId": CLAUDE_PLUGIN_ID,
        "providerInstanceId": "claude"
    });
    binary_request_collecting_events(
        &mut stdin,
        &mut stdout,
        "active-create-instance",
        "instance.create",
        json!({
            "route": route,
            "instanceKind": "claude",
            "displayName": "Claude active fixture",
            "settings": { "claudeExecutable": fixture_executable() }
        }),
        &mut pending,
    );
    binary_request_collecting_events(
        &mut stdin,
        &mut stdout,
        "active-start-instance",
        "instance.start",
        json!({ "route": route }),
        &mut pending,
    );
    let created = binary_request_collecting_events(
        &mut stdin,
        &mut stdout,
        "active-create-conversation",
        "conversation.create",
        json!({
            "route": route,
            "title": "Fixture conversation",
            "permissionLevel": "workspace-write",
            "model": "sonnet",
            "reasoningEffort": "high",
            "workspaceRoot": workspace
        }),
        &mut pending,
    );
    let conversation = json!({
        "deviceId": "active-device",
        "providerPluginId": CLAUDE_PLUGIN_ID,
        "providerInstanceId": "claude",
        "nativeResourceId": created["result"]["conversation"]["resource"]["nativeResourceId"]
    });
    binary_request_collecting_events(
        &mut stdin,
        &mut stdout,
        "active-start-turn",
        "turn.start",
        json!({
            "conversation": conversation,
            "clientRequestId": "active-ignore-sigint",
            "capabilityRevision": "claude-cli-stream-json-controls-v1",
            "input": { "kind": "text", "text": "ignore sigint" },
            "selection": {}
        }),
        &mut pending,
    );
    let pids = wait_for_probe_pids(workspace);
    ActiveProviderBinary {
        child,
        stdin,
        stdout,
        stdout_backpressure,
        pids,
    }
}

#[cfg(unix)]
fn saturate_provider_stdout(stdout: &mut UnixStream) {
    stdout.set_nonblocking(true).unwrap();
    let chunk = [b'x'; 16 * 1024];
    let mut filled = 0usize;
    loop {
        match stdout.write(&chunk) {
            Ok(0) => panic!("Provider stdout backpressure socket closed while filling"),
            Ok(written) => filled += written,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("fill Provider stdout backpressure socket: {error}"),
        }
    }
    stdout.set_nonblocking(false).unwrap();
    assert!(filled > 0, "Provider stdout socket did not accept test data");
}

#[cfg(unix)]
fn wait_for_provider_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "Provider did not exit after fatal stdio failure");
        std::thread::sleep(Duration::from_millis(5));
    }
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

fn binary_request_collecting_events(
    stdin: &mut impl Write,
    stdout: &mut impl BufRead,
    id: &str,
    method: &str,
    params: Value,
    pending: &mut Vec<Value>,
) -> Value {
    serde_json::to_writer(
        &mut *stdin,
        &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    )
    .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0);
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["id"] == id {
            return message;
        }
        pending.push(message);
    }
}
