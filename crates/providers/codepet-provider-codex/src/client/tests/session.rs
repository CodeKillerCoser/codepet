use super::*;

#[test]
fn extracts_cli_and_desktop_harness_versions_without_using_shell_product_versions() {
    assert_eq!(
        harness_version_from_user_agent(
            "code-pet/0.153.1 (Mac OS 26.3.2; arm64) iTerm.app/3.6.9 (code-pet; 0.1.0)"
        ),
        Some("0.153.1".to_string())
    );
    assert_eq!(
        harness_version_from_user_agent("iTerm.app/3.6.9 (code-pet; 0.1.0)"),
        None
    );
    assert_eq!(
        harness_version_from_user_agent("codex-cli/0.151.0"),
        Some("0.151.0".to_string())
    );
    assert_eq!(
        harness_version_from_user_agent(
            "Codex Desktop/0.152.1 (Mac OS 26.3.2; arm64) iTerm.app/3.6.9"
        ),
        Some("0.152.1".to_string())
    );
}

#[test]
fn release_shutdown_failure_is_not_reported_as_success_on_retry() {
    struct FailOnce(Arc<std::sync::atomic::AtomicUsize>);
    impl SessionControl for FailOnce {
        fn shutdown(&mut self) -> Result<(), CodexAppServerError> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(CodexAppServerError::Io("mock kill denied".into()))
            } else {
                Ok(())
            }
        }
    }
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (outgoing, _receiver) = mpsc::channel();
    let (_sender, incoming) = mpsc::channel();
    let session = CodexAppServerSession::from_parts(
        Box::new(MockReader { receiver: incoming }),
        Box::new(MockWriter { sender: outgoing }),
        Some(Box::new(FailOnce(count.clone()))),
    );
    assert!(session.shutdown().is_err());
    assert!(!session.is_running());
    assert!(session.shutdown().is_ok());
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "retry must invoke process control even though RPC admission is closed"
    );
}

#[test]
fn app_server_command_uses_the_resolved_executable_path() {
    let executable = Path::new("/resolved/runtime/codex");
    let args = vec![
        "app-server".to_string(),
        "--listen".to_string(),
        "stdio://".to_string(),
    ];
    let command = codex_app_server_command(executable, &args);

    assert_eq!(command.get_program(), executable.as_os_str());
    let marker = command
        .get_envs()
        .find(|(key, _)| *key == crate::desktop_takeover::HARNESS_MARKER)
        .and_then(|(_, value)| value)
        .unwrap();
    assert_eq!(
        marker.to_str(),
        Some(std::process::id().to_string().as_str())
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["app-server", "--listen", "stdio://"]
            .iter()
            .map(std::ffi::OsStr::new)
            .collect::<Vec<_>>()
    );
}

#[test]
fn initialize_timeout_fails_closed_instead_of_blocking_forever() {
    let (outgoing_sender, _outgoing_receiver) = mpsc::channel::<Value>();
    let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();

    let error = CodexAppServerSession::connect_with_initialize_timeout(
        Box::new(MockReader {
            receiver: incoming_receiver,
        }),
        Box::new(MockWriter {
            sender: outgoing_sender,
        }),
        Duration::from_millis(20),
    )
    .err()
    .expect("a silent App Server must time out");
    drop(incoming_sender);

    assert!(matches!(error, CodexAppServerError::Timeout(_)));
}

#[test]
fn empty_initialize_result_is_rejected() {
    let (outgoing_sender, outgoing_receiver) = mpsc::channel::<Value>();
    let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();
    let peer = thread::spawn(move || {
        let initialize = outgoing_receiver.recv().unwrap();
        incoming_sender
            .send(json!({
                "id": initialize["id"],
                "result": {}
            }))
            .unwrap();
    });

    let error = CodexAppServerSession::connect(
        Box::new(MockReader {
            receiver: incoming_receiver,
        }),
        Box::new(MockWriter {
            sender: outgoing_sender,
        }),
    )
    .err()
    .expect("an empty initialize result must fail closed");
    peer.join().unwrap();

    assert!(matches!(error, CodexAppServerError::Protocol(_)));
}

#[test]
fn oversized_stderr_keeps_session_and_pending_requests_alive() {
    let (outgoing_sender, _outgoing_receiver) = mpsc::channel::<Value>();
    let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();
    let terminated = Arc::new(AtomicBool::new(false));
    let session = CodexAppServerSession::from_parts(
        Box::new(MockReader {
            receiver: incoming_receiver,
        }),
        Box::new(MockWriter {
            sender: outgoing_sender,
        }),
        Some(Box::new(RecordingControl {
            terminated: terminated.clone(),
        })),
    );
    let (pending_sender, pending_receiver) = mpsc::channel();
    session
        .inner
        .pending
        .lock()
        .unwrap()
        .insert(JsonRpcId::Number(41), pending_sender);
    let mut stderr = vec![b'x'; MAX_APP_SERVER_STDERR_LINE_BYTES + 1];
    stderr.extend_from_slice(b"\nnext line\n");
    session
        .start_stderr_monitor(std::io::Cursor::new(stderr))
        .join()
        .unwrap();
    assert!(session.is_running());
    assert!(!terminated.load(Ordering::SeqCst));
    assert!(matches!(
        pending_receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(incoming_sender);
}

#[test]
fn rejected_frames_preserve_concurrent_requests_events_and_server_request_id_namespace() {
    let (outgoing_sender, outgoing_receiver) = mpsc::channel();
    let (incoming_sender, incoming_receiver) = mpsc::channel();
    let terminated = Arc::new(AtomicBool::new(false));
    let session = CodexAppServerSession::from_parts(
        Box::new(JsonLineReader::new(MockByteReader {
            receiver: incoming_receiver,
            current: std::io::Cursor::new(Vec::new()),
        })),
        Box::new(MockWriter {
            sender: outgoing_sender,
        }),
        Some(Box::new(RecordingControl {
            terminated: terminated.clone(),
        })),
    );
    let notifications = session.subscribe().unwrap();
    let session_a = session.clone();
    let request_a = thread::spawn(move || {
        session_a.request_value_with_timeout(
            "thread/read",
            json!({ "threadId": "thread-a" }),
            Duration::from_secs(2),
        )
    });
    let id_a = outgoing_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()["id"]
        .clone();
    let session_b = session.clone();
    let request_b = thread::spawn(move || {
        session_b.request_value_with_timeout(
            "thread/read",
            json!({ "threadId": "thread-b" }),
            Duration::from_secs(2),
        )
    });
    let id_b = outgoing_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()["id"]
        .clone();

    // An incoming server request may legitimately reuse a pending client id.
    let server_request = format!("{{\"id\":{id_b},\"method\":\"approval\",\"params\":!}}\n");
    let invalid = format!("{{\"id\":{id_a},\"result\":!}}\n");
    let healthy = json!({ "id": id_b, "result": { "ok": true } });
    let event = json!({ "method": "thread/name/updated", "params": {
            "threadId": "thread-b", "threadName": "Still receiving",
        } });
    incoming_sender
        .send(format!("{server_request}{invalid}{healthy}\n{event}\n").into_bytes())
        .unwrap();

    let rejected = outgoing_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(rejected["id"], id_b);
    assert_eq!(rejected["error"]["code"], -32600);
    assert!(matches!(
        request_a.join().unwrap().unwrap_err(),
        CodexAppServerError::Protocol(_)
    ));
    assert_eq!(request_b.join().unwrap().unwrap(), json!({ "ok": true }));
    assert!(
        matches!(notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(),
            CodexIncoming::Notification(CodexNotification::ThreadNameUpdated { thread_id, .. }) if thread_id == "thread-b")
    );
    assert!(session.is_running());
    assert!(!terminated.load(Ordering::SeqCst));
    session.shutdown().unwrap();
    drop(incoming_sender);
}

#[test]
fn persistent_session_initializes_once_and_matches_out_of_order_responses() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let first_session = session.clone();
    let first = thread::spawn(move || first_session.thread_read("thread-one").unwrap().thread.id);
    let second_session = session.clone();
    let second = thread::spawn(move || second_session.thread_read("thread-two").unwrap().thread.id);
    let request_a = peer_receiver.recv().unwrap();
    let request_b = peer_receiver.recv().unwrap();
    assert_eq!(request_a["method"], "thread/read");
    assert_eq!(request_b["method"], "thread/read");
    for request in [&request_b, &request_a] {
        let thread_id = request["params"]["threadId"].as_str().unwrap();
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {
                    "thread": thread_fixture(thread_id, "/tmp/project", "idle", vec![])
                }
            }))
            .unwrap();
    }
    let mut ids = vec![first.join().unwrap(), second.join().unwrap()];
    ids.sort();
    assert_eq!(ids, vec!["thread-one", "thread-two"]);
    assert!(session.is_running());
    session.shutdown().unwrap();
    assert!(!session.is_running());
}

#[test]
fn peer_exit_is_reported_to_subscribers_and_stops_the_session() {
    let (session, _, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    drop(peer_sender);
    let error = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap_err();
    assert_eq!(error, CodexAppServerError::ProcessExited);
    assert!(!session.is_running());
}

#[test]
fn terminal_fault_is_replayed_to_late_subscriber() {
    let (session, _, peer_sender) = mock_session();
    drop(peer_sender);
    for _ in 0..100 {
        if !session.is_running() {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(!session.is_running());

    let error = match session.subscribe() {
        Err(error) => error,
        Ok(_) => panic!("late subscriber did not receive the terminal fault"),
    };
    assert_eq!(error, CodexAppServerError::ProcessExited);
}

#[test]
fn subscribe_racing_terminal_fault_always_observes_the_fault() {
    let (session, _peer_receiver, peer_sender) = mock_session();
    let subscriber_count = 32;
    let barrier = Arc::new(Barrier::new(subscriber_count + 1));
    let expected = CodexAppServerError::Protocol("forced concurrent fault".to_string());
    let subscribers = (0..subscriber_count)
        .map(|_| {
            let session = session.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                match session.subscribe() {
                    Ok(receiver) => receiver
                        .recv_timeout(Duration::from_secs(1))
                        .unwrap()
                        .unwrap_err(),
                    Err(error) => error,
                }
            })
        })
        .collect::<Vec<_>>();

    barrier.wait();
    session.inner.fail(expected.clone());

    for subscriber in subscribers {
        assert_eq!(subscriber.join().unwrap(), expected);
    }
    let late_error = match session.subscribe() {
        Err(error) => error,
        Ok(_) => panic!("terminal fault was not retained after the race"),
    };
    assert_eq!(late_error, expected);
    drop(peer_sender);
}

#[test]
fn thread_read_rejects_non_full_turn_items() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let request_session = session.clone();
    let request = thread::spawn(move || request_session.thread_read("thread-one"));
    let read = peer_receiver.recv().unwrap();
    let mut turn = turn_fixture("turn-one", "completed");
    turn["itemsView"] = json!("summary");
    peer_sender
        .send(json!({
            "id": read["id"],
            "result": {
                "thread": thread_fixture(
                    "thread-one",
                    "/tmp/project",
                    "idle",
                    vec![turn]
                )
            }
        }))
        .unwrap();

    let error = request.join().unwrap().unwrap_err();
    assert!(matches!(
        error,
        CodexAppServerError::Protocol(message)
            if message.contains("non-full items for turn turn-one")
    ));
    session.shutdown().unwrap();
}

#[test]
fn stopped_session_reports_not_sent_without_allocating_or_writing() {
    let (session, peer_receiver, peer_sender) = mock_session();
    session.shutdown().unwrap();
    let next_id = session.inner.next_id.load(Ordering::SeqCst);

    let outcome = session.ensure_thread_loaded_outcome("thread-stopped");

    assert_eq!(
        outcome,
        CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown)
    );
    assert_eq!(session.inner.next_id.load(Ordering::SeqCst), next_id);
    assert!(session.inner.pending.lock().unwrap().is_empty());
    assert!(peer_receiver.try_recv().is_err());
    drop(peer_sender);
}

#[test]
fn unmatched_and_late_responses_do_not_stop_the_session() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let timed =
        session.request_value_with_timeout("project/read", json!({}), Duration::from_millis(20));
    assert!(matches!(timed, Err(CodexAppServerError::Timeout(_))));
    assert!(session.is_running());
    let old = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    peer_sender
        .send(json!({ "id": old["id"], "result": {} }))
        .unwrap();
    peer_sender
        .send(json!({ "id": 999, "result": {} }))
        .unwrap();
    let operation_session = session.clone();
    let operation = thread::spawn(move || {
        operation_session.request_value_with_timeout(
            "project/read",
            json!({}),
            Duration::from_secs(1),
        )
    });
    let current = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    peer_sender
        .send(json!({ "id": current["id"], "result": { "ok": true } }))
        .unwrap();
    assert_eq!(operation.join().unwrap().unwrap(), json!({ "ok": true }));
    assert!(session.is_running());
    session.shutdown().unwrap();
}
