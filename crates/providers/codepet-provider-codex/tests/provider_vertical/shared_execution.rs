use super::*;

#[test]
fn provider_binary_reuses_one_execution_session_until_authoritative_terminal_state() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("turn-lifecycle.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("normal", &marker);
    let observer_pid = session_pids(&marker, "model/list", "")[0];
    let creation_pid = session_pids(&marker, "thread/start", "")[0];
    assert_eq!(observer_pid, creation_pid);
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
    let turn = started.pointer("/result/turn/resource").cloned().unwrap();
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
    assert!(resume_pids.is_empty());
    let approval_pids = session_pids(&marker, "approval/response", "thread-created");
    assert_eq!(approval_pids.len(), 1);
    let first_execution = approval_pids[0];
    assert_eq!(first_execution, creation_pid);
    assert_eq!(observer_pid, first_execution);
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-created"),
        vec![first_execution]
    );
    assert_eq!(
        session_pids(&marker, "turn/interrupt", "thread-created"),
        vec![first_execution]
    );

    provider.request(
        "lifecycle-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("lifecycle-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_keeps_waiting_user_input_execution_detached_from_remote_reads() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("waiting-user-input.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("waiting-user-input", &marker);
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
    let turn = started.pointer("/result/turn/resource").cloned().unwrap();
    let execution_pid = session_pids(&marker, "turn/start", "thread-created")[0];
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
    assert!(session_pids(&marker, "thread/resume", "thread-created").is_empty());

    provider.request(
        "waiting-input-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation, "turn": turn }),
    );
    provider.request(
        "waiting-input-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("waiting-input-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_terminal_notification_preserves_other_conversations_in_shared_server() {
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
    assert_eq!(a_pids.len(), 1);
    assert_eq!(b_pids.len(), 1);
    assert_eq!(a_pids[0], b_pids[0]);
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-b"),
        vec![b_pids[0]]
    );

    provider.request(
        "parallel-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request(
        "parallel-start-provider",
        "instance.start",
        json!({ "route": route_value() }),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let state = provider.request(
            "parallel-wait-ready",
            "instance.start",
            json!({ "route": route_value() }),
        );
        if state
            .pointer("/result/instance/status")
            .and_then(Value::as_str)
            == Some("ready")
        {
            break;
        }
        assert!(Instant::now() < deadline, "restart probes did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
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

    provider.request(
        "parallel-stop-final",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("parallel-shutdown", "provider.shutdown", json!({}));
    let _ = turn_a;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_publication_precedes_next_operation_without_closing_shared_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("terminal-closing-race.txt");
    let terminal_entered = Arc::new(Barrier::new(2));
    let terminal_release = Arc::new(Barrier::new(2));
    let sink_entered = terminal_entered.clone();
    let sink_release = terminal_release.clone();
    let (event_sender, event_receiver) = mpsc::channel();
    let events = Arc::new(move |event: ProtocolEvent| {
        let is_terminal = matches!(
            &event,
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.status == codepet_provider_sdk::TurnStatus::Completed
                    && params.turn.conversation.native_resource_id == "thread-terminal-race"
        );
        event_sender
            .send(event)
            .map_err(|error| codepet_provider_sdk::ProtocolError {
                code: "test_event_sink_closed".to_string(),
                message: error.to_string(),
                retryable: false,
                details: None,
            })?;
        if is_terminal {
            sink_entered.wait();
            sink_release.wait();
        }
        Ok(())
    });
    let (provider, route, capability_revision) =
        configured_direct_provider_with_events("complete-on-approval", &marker, events).await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-terminal-race");
    ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.clone(),
            client_request_id: "terminal-race-first".to_string(),
            capability_revision: capability_revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "finish the first turn".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        },
    )
    .await
    .unwrap();
    let approval = loop {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if let ProtocolEvent::EventApprovalRequested { params, .. } = event {
            break params.approval.resource;
        }
    };
    ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: conversation_resource(&route, &approval.native_resource_id),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap();
    let entered = terminal_entered.clone();
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();

    let next_provider = provider.clone();
    let next_conversation = conversation.clone();
    let next = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            next_provider.as_ref(),
            TurnStartRequest {
                conversation: next_conversation,
                client_request_id: "terminal-race-second".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "start after terminal cleanup".to_string(),
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!next.is_finished());
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-terminal-race").len(),
        1
    );

    let release = terminal_release.clone();
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), next)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let execution_pids = session_pids(&marker, "thread/resume", "thread-terminal-race");
    assert_eq!(execution_pids.len(), 1);
    assert!(process_is_running(execution_pids[0]));

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_server_keeps_writer_through_delayed_output_and_terminal_forwarding() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("delayed-user-item-output.txt");
    let (event_sender, event_receiver) = mpsc::channel();
    let terminal_forwarded_while_running = Arc::new(AtomicBool::new(false));
    let sink_terminal_forwarded_while_running = terminal_forwarded_while_running.clone();
    let sink_marker = marker.clone();
    let events = Arc::new(move |event: ProtocolEvent| {
        if matches!(
            &event,
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.resource.native_resource_id == "turn-started"
                    && params.turn.status == codepet_provider_sdk::TurnStatus::Completed
        ) {
            let execution_pids =
                session_pids(&sink_marker, "turn/start", "thread-delayed-user-item");
            sink_terminal_forwarded_while_running.store(
                execution_pids.len() == 1 && process_is_running(execution_pids[0]),
                Ordering::SeqCst,
            );
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
    let (provider, route, capability_revision) = configured_direct_provider_with_events_and_hook(
        "delayed-output-after-user-item",
        &marker,
        events,
        None,
    )
    .await;
    let conversation = conversation_resource(&route, "thread-delayed-user-item");
    ProviderProtocolServer::conversation_acquire_interaction(
        provider.as_ref(),
        ConversationAcquireInteractionRequest {
            force: None,
            conversation: conversation.clone(),
        },
    )
    .await
    .unwrap();
    clear_session_log(&marker);

    let started = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.clone(),
            client_request_id: "delayed-user-item".to_string(),
            capability_revision,
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "keep the writer while model output is delayed".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        },
    )
    .await
    .unwrap();
    assert!(started.user_item.is_none());
    let execution_pid = session_pids(&marker, "turn/start", "thread-delayed-user-item")[0];

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_completed_user_item = false;
    let mut saw_delayed_output = false;
    let mut saw_terminal = false;
    while !saw_terminal {
        let event = event_receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        match event {
            ProtocolEvent::EventConversationItemUpserted { params, .. }
                if item_resource_id(params.item.as_ref().unwrap()) == "user-one"
                    && item_status(params.item.as_ref().unwrap())
                        == codepet_provider_sdk::ConversationItemStatus::Completed =>
            {
                saw_completed_user_item = true;
            }
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.turn.native_resource_id == "turn-started"
                    && params.delta == "delayed fixture output" =>
            {
                assert!(saw_completed_user_item);
                assert!(process_is_running(execution_pid));
                saw_delayed_output = true;
            }
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.resource.native_resource_id == "turn-started"
                    && params.turn.status == codepet_provider_sdk::TurnStatus::Completed =>
            {
                assert!(saw_delayed_output);
                saw_terminal = true;
            }
            _ => {}
        }
    }
    assert!(saw_completed_user_item);
    assert!(terminal_forwarded_while_running.load(Ordering::SeqCst));

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefetched_ready_handle_remains_valid_after_terminal_in_shared_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("prefetched-ready-handle.txt");
    let handle_entered = Arc::new(Barrier::new(2));
    let handle_release = Arc::new(Barrier::new(2));
    let hook = Arc::new(BlockHandleOnceHook {
        conversation_id: "thread-prefetched-handle".to_string(),
        operation: "turn.start",
        armed: AtomicBool::new(false),
        entered: handle_entered.clone(),
        release: handle_release.clone(),
    });
    let (event_sender, event_receiver) = mpsc::channel();
    let events = Arc::new(move |event: ProtocolEvent| {
        event_sender
            .send(event)
            .map_err(|error| codepet_provider_sdk::ProtocolError {
                code: "test_event_sink_closed".to_string(),
                message: error.to_string(),
                retryable: false,
                details: None,
            })
    });
    let (provider, route, capability_revision) = configured_direct_provider_with_events_and_hook(
        "complete-on-approval",
        &marker,
        events,
        Some(hook.clone()),
    )
    .await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-prefetched-handle");
    ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.clone(),
            client_request_id: "prefetched-first".to_string(),
            capability_revision: capability_revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "start the first turn".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        },
    )
    .await
    .unwrap();
    let approval = loop {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if let ProtocolEvent::EventApprovalRequested { params, .. } = event {
            break params.approval.resource;
        }
    };
    let first_generation_pids = session_pids(&marker, "thread/resume", "thread-prefetched-handle");
    assert_eq!(first_generation_pids.len(), 1);

    hook.armed.store(true, Ordering::SeqCst);
    let next_provider = provider.clone();
    let next_conversation = conversation.clone();
    let next = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            next_provider.as_ref(),
            TurnStartRequest {
                conversation: next_conversation,
                client_request_id: "prefetched-second".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "start on the next generation".to_string(),
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
    let entered = handle_entered.clone();
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();

    ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: conversation_resource(&route, &approval.native_resource_id),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap();
    loop {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if matches!(
            event,
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.status == codepet_provider_sdk::TurnStatus::Completed
                    && params.turn.conversation.native_resource_id == "thread-prefetched-handle"
        ) {
            break;
        }
    }
    assert!(process_is_running(first_generation_pids[0]));

    let release = handle_release.clone();
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), next)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let execution_pids = session_pids(&marker, "thread/resume", "thread-prefetched-handle");
    assert_eq!(execution_pids.len(), 1);
    assert_eq!(execution_pids, first_generation_pids);

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_cancelled_before_resume_linearization_never_writes_resume_or_start() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory
        .path()
        .join("cancel-before-resume-linearization.txt");
    let resume_entered = Arc::new(Barrier::new(2));
    let resume_release = Arc::new(Barrier::new(2));
    let cancelled_entered = Arc::new(Barrier::new(2));
    let cancelled_release = Arc::new(Barrier::new(2));
    let hook = Arc::new(BlockResumeUntilCancelledHook {
        conversation_id: "thread-resume-pending-linearization".to_string(),
        resume_entered: resume_entered.clone(),
        resume_release: resume_release.clone(),
        cancelled_entered: cancelled_entered.clone(),
        cancelled_release: cancelled_release.clone(),
    });
    let (provider, route, capability_revision) = configured_direct_provider_with_events_and_hook(
        "resume-no-response",
        &marker,
        Arc::new(|_| Ok(())),
        Some(hook),
    )
    .await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-resume-pending-linearization");
    let operation_provider = provider.clone();
    let operation = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            operation_provider.as_ref(),
            TurnStartRequest {
                conversation,
                client_request_id: "cancel-before-resume-message".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "stop before resume is sent".to_string(),
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
    let entered = resume_entered.clone();
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();
    let execution_pids: Vec<u32> = vec![];
    assert!(execution_pids.is_empty());
    assert_eq!(session_method_count(&marker, "initialize"), 0);

    let stop_provider = provider.clone();
    let stop_route = route.clone();
    let stop = tokio::spawn(async move {
        ProviderProtocolServer::instance_stop(
            stop_provider.as_ref(),
            InstanceStopRequest { route: stop_route },
        )
        .await
    });
    let cancelled = cancelled_entered.clone();
    tokio::task::spawn_blocking(move || cancelled.wait())
        .await
        .unwrap();

    let release_resume = resume_release.clone();
    tokio::task::spawn_blocking(move || release_resume.wait())
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while session_pids(
        &marker,
        "thread/resume",
        "thread-resume-pending-linearization",
    )
    .is_empty()
        && !operation.is_finished()
        && Instant::now() < deadline
    {
        tokio::task::yield_now().await;
    }
    let resume_was_written = !session_pids(
        &marker,
        "thread/resume",
        "thread-resume-pending-linearization",
    )
    .is_empty();

    let release_cancelled = cancelled_release.clone();
    tokio::task::spawn_blocking(move || release_cancelled.wait())
        .await
        .unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(2), stop)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        stopped.instance.status,
        codepet_provider_sdk::InstanceStatus::Stopped
    );
    let operation_error = tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(operation_error.code, "provider_unavailable");
    assert!(!resume_was_written);
    assert!(session_pids(
        &marker,
        "thread/resume",
        "thread-resume-pending-linearization",
    )
    .is_empty());
    assert!(session_pids(&marker, "turn/start", "thread-resume-pending-linearization",).is_empty());
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));

    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[test]
fn shared_server_and_event_routing_are_stable_across_sixty_four_conversations() {
    const ITERATIONS: usize = 64;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("terminal-stress.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("complete-on-approval", &marker);
    clear_session_log(&marker);

    for index in 0..ITERATIONS {
        let thread_id = format!("thread-terminal-stress-{index}");
        let conversation = conversation_resource_value(&thread_id);
        let started = provider.request(
            &format!("terminal-stress-start-{index}"),
            "turn.start",
            turn_start_params(
                conversation,
                &format!("terminal-stress-message-{index}"),
                "complete this turn",
                &capability_revision,
            ),
        );
        assert!(started.get("error").is_none());
        let approval = provider
            .event("event.approvalRequested")
            .pointer("/params/approval/resource")
            .cloned()
            .unwrap();
        let resolved = provider.request(
            &format!("terminal-stress-resolve-{index}"),
            "approval.resolve",
            json!({ "approval": approval, "decision": "approve" }),
        );
        assert!(resolved.get("error").is_none());
        provider.receive(Duration::from_secs(2), |message| {
            message.get("method").and_then(Value::as_str) == Some("event.turnUpserted")
                && message
                    .pointer("/params/turn/conversation/nativeResourceId")
                    .and_then(Value::as_str)
                    == Some(thread_id.as_str())
                && message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    == Some("completed")
        });
        let resume_pids = session_pids(&marker, "thread/resume", &thread_id);
        assert_eq!(
            resume_pids.len(),
            1,
            "iteration {index} lost resume PID evidence"
        );
        assert_eq!(
            session_pids(&marker, "approval/response", &thread_id),
            resume_pids,
            "iteration {index} crossed execution process evidence"
        );
    }
    assert_eq!(session_pids(&marker, "thread/resume", "*").len(), 1);

    provider.request(
        "terminal-stress-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("terminal-stress-shutdown", "provider.shutdown", json!({}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_first_turn_start_shares_one_resume_and_one_active_turn() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("concurrent-resume.txt");
    let (provider, route, capability_revision) = configured_direct_provider("none", &marker).await;
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
        InstanceStopRequest {
            route: route.clone(),
        },
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
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        stopped.instance.status,
        codepet_provider_sdk::InstanceStatus::Stopped
    );
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
