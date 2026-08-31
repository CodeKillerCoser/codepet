use codepet_provider_opencode::{
    OpenCodeProvider, OPENCODE_INSTANCE_KIND, OPENCODE_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationContentKind, ConversationCreateRequest,
    ConversationGetRequest, ConversationListRequest, InstanceCreateRequest,
    InstanceStartRequest, InstanceStatus, InstanceStopRequest, ProtocolEvent,
    ProtocolServer, ProviderInitializeRequest, ProviderInstanceRoute, RoutedResourceId,
    TurnInterruptRequest, TurnStartRequest, TurnStatus, TurnSteerRequest, VersionRange,
    PROTOCOL_VERSION,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[tokio::test]
async fn official_v2_shapes_map_through_the_provider_protocol() {
    let process_directory = tempfile::tempdir().unwrap();
    let first_pid_file = process_directory.path().join("opencode-first.pid");
    std::env::set_var("OPENCODE_FIXTURE_PID_FILE", &first_pid_file);
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
        ("serverVersion".to_string(), json!("1.18.25")),
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
    std::env::remove_var("OPENCODE_FIXTURE_PID_FILE");
    assert_eq!(started.instance.status, InstanceStatus::Ready);
    let first_server_pid = read_pid(&first_pid_file);
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

    let successful_turn = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "shared-client-id".to_string(),
            message: "complete normally".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(successful_turn.turn.conversation, fixture_conversation);
    assert_eq!(successful_turn.turn.started_at, None);
    let completed = wait_for_turn_status(
        &event_receiver,
        &successful_turn.turn.resource,
        TurnStatus::Completed,
    );
    assert_eq!(completed.started_at, Some(1_700_000_002_001));
    assert_eq!(completed.completed_at, Some(1_700_000_002_003));
    assert_no_duplicate_turn_status(
        &event_receiver,
        &successful_turn.turn.resource,
        TurnStatus::Completed,
    );

    let created_conversation = new_conversation.conversation.resource.clone();
    let same_client_other_session = provider
        .turn_start(TurnStartRequest {
            conversation: created_conversation.clone(),
            client_message_id: "shared-client-id".to_string(),
            message: "complete normally".to_string(),
        })
        .await
        .unwrap();
    assert_ne!(
        successful_turn.turn.resource,
        same_client_other_session.turn.resource
    );
    wait_for_turn_status(
        &event_receiver,
        &same_client_other_session.turn.resource,
        TurnStatus::Completed,
    );

    let multi_step = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-multiple-steps".to_string(),
            message: "multiple steps".to_string(),
        })
        .await
        .unwrap();
    let multi_step_completed = wait_for_turn_status(
        &event_receiver,
        &multi_step.turn.resource,
        TurnStatus::Completed,
    );
    assert_eq!(multi_step_completed.completed_at, Some(1_700_000_002_005));
    assert_no_duplicate_turn_status(
        &event_receiver,
        &multi_step.turn.resource,
        TurnStatus::Completed,
    );

    let reordered = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-response-after-completion".to_string(),
            message: "response after completion".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(reordered.code, "turn_not_active");
    let after_reordered = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-after-reordered".to_string(),
            message: "complete normally".to_string(),
        })
        .await
        .unwrap();
    wait_for_turn_status(
        &event_receiver,
        &after_reordered.turn.resource,
        TurnStatus::Completed,
    );

    let delayed = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-delayed-old".to_string(),
            message: "delay previous step".to_string(),
        })
        .await
        .unwrap();
    wait_for_turn_status(&event_receiver, &delayed.turn.resource, TurnStatus::Running);
    std::thread::sleep(Duration::from_millis(20));
    provider
        .turn_interrupt(TurnInterruptRequest {
            conversation: fixture_conversation.clone(),
            turn: delayed.turn.resource,
        })
        .await
        .unwrap();
    let after_delayed = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-after-delayed".to_string(),
            message: "complete after delayed".to_string(),
        })
        .await
        .unwrap();
    let after_delayed_completed = wait_for_turn_status(
        &event_receiver,
        &after_delayed.turn.resource,
        TurnStatus::Completed,
    );
    assert_eq!(after_delayed_completed.completed_at, Some(1_700_000_002_003));
    assert_no_duplicate_turn_status(
        &event_receiver,
        &after_delayed.turn.resource,
        TurnStatus::Completed,
    );

    let started_turn = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "client-start-approval".to_string(),
            message: "needs approval".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        started_turn.turn.status,
        TurnStatus::Queued | TurnStatus::Running | TurnStatus::WaitingApproval
    ));

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_delta = false;
    let mut saw_reasoning = false;
    let mut approval = None;
    while Instant::now() < deadline && (!saw_delta || !saw_reasoning || approval.is_none()) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(event) = event_receiver.recv_timeout(remaining) else {
            break;
        };
        match event {
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.delta == "fixture output"
                    && params.kind == ConversationContentKind::Text => {
                assert!(params.item_id.starts_with("txt_"));
                assert_eq!(params.content_id, format!("{}:text", params.item_id));
                saw_delta = true;
            }
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.delta == "fixture reasoning"
                    && params.kind == ConversationContentKind::ReasoningSummary => {
                assert!(params.item_id.starts_with("reasoning_"));
                assert_eq!(params.content_id, format!("{}:summary:0", params.item_id));
                saw_reasoning = true;
            }
            ProtocolEvent::EventApprovalRequested { params, .. } => {
                approval = Some(params.approval);
            }
            _ => {}
        }
    }
    assert!(saw_delta, "official text delta shape was not mapped");
    assert!(saw_reasoning, "official reasoning delta shape was not mapped");
    let approval = approval.expect("official permission.v2.asked shape was not mapped");
    assert_eq!(approval.kind, "bash");
    assert_eq!(approval.turn, started_turn.turn.resource);
    assert_eq!(approval.requested_at, None);

    let approval_resource = approval.resource.clone();
    let resolved = provider
        .approval_resolve(ApprovalResolveRequest {
            approval: approval_resource.clone(),
            decision: ApprovalDecision::Approve,
        })
        .await
        .unwrap();
    assert_eq!(resolved.approval.decision, Some(ApprovalDecision::Approve));
    let repeated = provider
        .approval_resolve(ApprovalResolveRequest {
            approval: approval_resource,
            decision: ApprovalDecision::Approve,
        })
        .await
        .unwrap_err();
    assert_eq!(repeated.code, "approval_not_found");

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
            conversation: fixture_conversation.clone(),
            turn: started_turn.turn.resource,
        })
        .await
        .unwrap();
    assert_eq!(interrupted.turn.status, TurnStatus::Interrupted);

    let idle_turn = provider
        .turn_start(TurnStartRequest {
            conversation: created_conversation.clone(),
            client_message_id: "idle-interrupt".to_string(),
            message: "idle interrupt".to_string(),
        })
        .await
        .unwrap();
    wait_for_turn_status(&event_receiver, &idle_turn.turn.resource, TurnStatus::Running);
    let idle_interrupt = provider
        .turn_interrupt(TurnInterruptRequest {
            conversation: created_conversation,
            turn: idle_turn.turn.resource,
        })
        .await
        .unwrap();
    assert_eq!(idle_interrupt.turn.status, TurnStatus::Running);
    assert_eq!(idle_interrupt.turn.completed_at, None);

    let stale_turn = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "stale-after-restart".to_string(),
            message: "needs approval".to_string(),
        })
        .await
        .unwrap();
    let stale_approval = loop {
        let event = event_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("timed out waiting for stale approval fixture");
        if let ProtocolEvent::EventApprovalRequested { params, .. } = event {
            break params.approval.resource;
        }
    };
    provider
        .instance_stop(InstanceStopRequest {
            route: route.clone(),
        })
        .await
        .unwrap();
    assert_process_exited(first_server_pid);

    let attacker = TcpAttackFixture::start();
    let second_pid_file = process_directory.path().join("opencode-second.pid");
    std::env::set_var("OPENCODE_FIXTURE_PID_FILE", &second_pid_file);
    provider
        .instance_start(InstanceStartRequest {
            route: route.clone(),
        })
        .await
        .unwrap();
    std::env::remove_var("OPENCODE_FIXTURE_PID_FILE");
    let second_server_pid = read_pid(&second_pid_file);
    attacker.assert_never_contacted();

    let current_turn = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation.clone(),
            client_message_id: "stale-after-restart".to_string(),
            message: "needs approval".to_string(),
        })
        .await
        .unwrap();
    assert_ne!(stale_turn.turn.resource, current_turn.turn.resource);
    let stale_approval_error = provider
        .approval_resolve(ApprovalResolveRequest {
            approval: stale_approval,
            decision: ApprovalDecision::Deny,
        })
        .await
        .unwrap_err();
    assert_eq!(stale_approval_error.code, "approval_not_found");
    let stale_turn_error = provider
        .turn_interrupt(TurnInterruptRequest {
            conversation: fixture_conversation.clone(),
            turn: stale_turn.turn.resource,
        })
        .await
        .unwrap_err();
    assert_eq!(stale_turn_error.code, "stale_turn");
    provider
        .turn_interrupt(TurnInterruptRequest {
            conversation: fixture_conversation.clone(),
            turn: current_turn.turn.resource,
        })
        .await
        .unwrap();

    let long_wait = provider
        .turn_start(TurnStartRequest {
            conversation: fixture_conversation,
            client_message_id: "long-wait-cancel".to_string(),
            message: "wait until cancelled".to_string(),
        })
        .await
        .unwrap();
    wait_for_turn_status(&event_receiver, &long_wait.turn.resource, TurnStatus::Running);
    std::thread::sleep(Duration::from_millis(50));

    let stop_started = Instant::now();
    let stopped = provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
    assert_eq!(stopped.instance.status, InstanceStatus::Stopped);
    assert!(stop_started.elapsed() < Duration::from_secs(1));
    assert_process_exited(second_server_pid);
}

#[tokio::test]
#[ignore = "requires CODEPET_OPENCODE_EXECUTABLE pointing to OpenCode 1.18.25 exactly"]
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
                ("serverVersion".to_string(), json!("1.18.25")),
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

struct TcpAttackFixture {
    stop: Arc<AtomicBool>,
    saw_connection: Arc<AtomicBool>,
    saw_basic_auth: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

impl TcpAttackFixture {
    fn start() -> Self {
        let listener = first_available_opencode_listener();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let saw_connection = Arc::new(AtomicBool::new(false));
        let saw_basic_auth = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread_connection = saw_connection.clone();
        let thread_auth = saw_basic_auth.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        thread_connection.store(true, Ordering::SeqCst);
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                        let mut request = [0u8; 4096];
                        let read = stream.read(&mut request).unwrap_or(0);
                        if String::from_utf8_lossy(&request[..read])
                            .to_ascii_lowercase()
                            .contains("authorization: basic ")
                        {
                            thread_auth.store(true, Ordering::SeqCst);
                        }
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"healthy\":true}",
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            }
        });
        Self {
            stop,
            saw_connection,
            saw_basic_auth,
            thread,
        }
    }

    fn assert_never_contacted(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().unwrap();
        assert!(!self.saw_connection.load(Ordering::SeqCst));
        assert!(!self.saw_basic_auth.load(Ordering::SeqCst));
    }
}

fn first_available_opencode_listener() -> TcpListener {
    for port in 4096..=u16::MAX {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return listener,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => panic!("bind attacker fixture: {error}"),
        }
    }
    panic!("no loopback port available for attacker fixture")
}

fn read_pid(path: &std::path::Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[cfg(unix)]
fn assert_process_exited(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
}

#[cfg(not(unix))]
fn assert_process_exited(_pid: i32) {}

fn resource(route: &ProviderInstanceRoute, native_resource_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_resource_id.to_string(),
    }
}

fn wait_for_turn_status(
    receiver: &mpsc::Receiver<ProtocolEvent>,
    resource: &RoutedResourceId,
    status: TurnStatus,
) -> codepet_provider_sdk::ProviderTurn {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = receiver
            .recv_timeout(remaining)
            .expect("timed out waiting for Provider turn status");
        if let ProtocolEvent::EventTurnUpserted { params, .. } = event {
            if params.turn.resource == *resource && params.turn.status == status {
                return params.turn;
            }
        }
    }
    panic!("Provider turn did not reach {status:?}")
}

fn assert_no_duplicate_turn_status(
    receiver: &mpsc::Receiver<ProtocolEvent>,
    resource: &RoutedResourceId,
    status: TurnStatus,
) {
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(event) = receiver.recv_timeout(remaining) else {
            return;
        };
        if let ProtocolEvent::EventTurnUpserted { params, .. } = event {
            assert!(
                params.turn.resource != *resource || params.turn.status != status,
                "Provider emitted a duplicate terminal turn event"
            );
        }
    }
}
