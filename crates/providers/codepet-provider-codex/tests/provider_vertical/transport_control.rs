use super::*;

#[test]
fn provider_binary_dispatches_other_conversations_and_stop_while_resume_hangs() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-concurrent-stop.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);
    let blocked_conversation = conversation_resource_value("thread-resume-pending-a");
    let other_conversation = conversation_resource_value("thread-b");

    provider.send_request(
        "stdio-blocked-a",
        "turn.start",
        turn_start_params(
            blocked_conversation,
            "stdio-blocked-message-a",
            "block A resume",
            &capability_revision,
        ),
    );
    wait_for_file_method(
        &marker,
        "thread/resume",
        "thread-resume-pending-a",
        Duration::from_secs(2),
    );
    provider.send_request(
        "stdio-other-b",
        "turn.start",
        turn_start_params(
            other_conversation,
            "stdio-other-message-b",
            "run B",
            &capability_revision,
        ),
    );
    let other_response = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("stdio-other-b")
    });
    assert_eq!(other_response["id"], "stdio-other-b");
    assert!(other_response.get("error").is_none());
    wait_for_file_method(&marker, "thread/resume", "thread-b", Duration::from_secs(2));
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    provider.send_request(
        "stdio-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    let stopped = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("stdio-stop")
    });
    assert_eq!(stopped["id"], "stdio-stop");
    assert_eq!(
        stopped
            .pointer("/result/instance/status")
            .and_then(Value::as_str),
        Some("stopped")
    );
    let blocked_response = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("stdio-blocked-a")
    });
    assert_eq!(blocked_response["id"], "stdio-blocked-a");
    assert!(blocked_response.get("error").is_some());
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));

    provider.request("stdio-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reserves_dispatch_for_stop_when_all_normal_slots_are_blocked() {
    const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-saturated-stop.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);

    for index in 0..MAX_CONCURRENT_HOST_REQUESTS {
        provider.send_request(
            &format!("saturated-stop-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("saturated-stop-message-{index}"),
                "hold every normal dispatch slot",
                &capability_revision,
            ),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session_method_count(&marker, "thread/resume") < MAX_CONCURRENT_HOST_REQUESTS {
        assert!(
            Instant::now() < deadline,
            "bounded requests did not reach the dispatch limit"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        MAX_CONCURRENT_HOST_REQUESTS
    );
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    provider.send_request(
        "saturated-heartbeat",
        "provider.ping",
        json!({
            "sequence": 1, "hostSessionId": "saturated-host",
            "clients": {"revision": 1, "connections": []}, "instances": []
        }),
    );
    let pong = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("saturated-heartbeat")
    });
    assert_eq!(pong.pointer("/result/sequence"), Some(&json!(1)));
    provider.send_request(
        "saturated-instance-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    let stopped = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("saturated-instance-stop")
    });
    assert_eq!(stopped["id"], "saturated-instance-stop");
    assert_eq!(
        stopped
            .pointer("/result/instance/status")
            .and_then(Value::as_str),
        Some("stopped")
    );
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));

    provider.request(
        "saturated-provider-shutdown",
        "provider.shutdown",
        json!({}),
    );
}

#[test]
fn provider_binary_keeps_eof_visible_after_at_least_forty_nine_saturated_frames() {
    const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;
    const MAX_PENDING_HOST_MESSAGES: usize = 32;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-saturated-eof.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);

    for index in 0..MAX_CONCURRENT_HOST_REQUESTS {
        provider.send_request(
            &format!("saturated-eof-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("saturated-eof-message-{index}"),
                "saturate dispatch before EOF",
                &capability_revision,
            ),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session_method_count(&marker, "thread/resume") < MAX_CONCURRENT_HOST_REQUESTS {
        assert!(
            Instant::now() < deadline,
            "saturated requests did not reach the dispatch limit"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    for index in
        MAX_CONCURRENT_HOST_REQUESTS..(MAX_CONCURRENT_HOST_REQUESTS + MAX_PENDING_HOST_MESSAGES + 1)
    {
        provider.send_request(
            &format!("saturated-eof-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("saturated-eof-message-{index}"),
                "saturate dispatch before EOF",
                &capability_revision,
            ),
        );
    }
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    let status = provider.close_input_and_wait(Duration::from_secs(2));
    assert!(status.success());
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));
}

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
fn provider_binary_stop_linearizes_repeated_shared_server_initialization() {
    const ROUNDS: usize = 5;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("observer-initialize-stop.txt");
    let mut provider = ProviderBinary::spawn();
    provider.initialize_and_create_instance(
        &app_server_executable(),
        fixture_args("observer-initialize-no-response", &marker),
    );

    for round in 0..ROUNDS {
        let start_id = format!("observer-start-{round}");
        let stop_id = format!("observer-stop-{round}");
        provider.send_request(
            &start_id,
            "instance.start",
            json!({ "route": route_value() }),
        );
        wait_for_session_count(&marker, "initialize", round + 1, Duration::from_secs(2));
        let process_id = *session_pids(&marker, "process/start", "")
            .last()
            .expect("observer fixture process was not recorded");
        provider.send_request(&stop_id, "instance.stop", json!({ "route": route_value() }));
        let stopped = provider.receive(Duration::from_secs(2), |message| {
            message.get("id").and_then(Value::as_str) == Some(stop_id.as_str())
        });
        let process_still_running = process_is_running(process_id);
        if process_still_running {
            terminate_processes(&[process_id]);
        }
        assert_eq!(stopped["id"], stop_id);
        assert_eq!(
            stopped
                .pointer("/result/instance/status")
                .and_then(Value::as_str),
            Some("stopped")
        );
        assert!(
            !process_still_running,
            "round {round}: stop returned before observer {process_id} exited"
        );
        let start_response = provider.receive(Duration::from_secs(2), |message| {
            message.get("id").and_then(Value::as_str) == Some(start_id.as_str())
        });
        assert_eq!(start_response["id"], start_id);
        assert!(start_response.get("error").is_some());
    }
    provider.collect_for(Duration::from_millis(100));
    assert!(provider.buffered.iter().all(|message| {
        message.get("method").and_then(Value::as_str) != Some("event.instanceStatusChanged")
            || message
                .pointer("/params/instance/status")
                .and_then(Value::as_str)
                != Some("ready")
    }));

    let destroyed = provider.request(
        "observer-destroy",
        "instance.destroy",
        json!({ "route": route_value() }),
    );
    assert_eq!(destroyed["id"], "observer-destroy");
    provider.request("observer-shutdown", "provider.shutdown", json!({}));
}

#[cfg(unix)]
#[test]
fn provider_binary_async_event_broken_pipe_triggers_global_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("event-broken-pipe.txt");
    let mut provider = ProviderBinary::spawn_ignoring_sigpipe();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    let observer_pids = session_pids(&marker, "model/list", "");
    assert_eq!(observer_pids.len(), 1);
    clear_session_log(&marker);

    provider.send_request(
        "broken-pipe-turn",
        "turn.start",
        turn_start_params(
            conversation_resource_value("thread-broken-pipe"),
            "broken-pipe-message",
            "hold execution while stdout closes",
            &capability_revision,
        ),
    );
    wait_for_file_method(
        &marker,
        "thread/resume",
        "thread-broken-pipe",
        Duration::from_secs(2),
    );
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    provider.close_stdout_after_probe();
    terminate_processes(&observer_pids);
    let exit_status = provider.wait_for_exit(Duration::from_secs(2));
    let execution_processes_still_running = execution_pids
        .iter()
        .copied()
        .filter(|process_id| process_is_running(*process_id))
        .collect::<Vec<_>>();
    if exit_status.is_none() {
        let _ = provider.child.kill();
        let _ = provider.child.wait();
    }
    if !execution_processes_still_running.is_empty() {
        terminate_processes(&execution_processes_still_running);
    }
    assert!(
        exit_status.is_some(),
        "Provider ignored an async event stdout failure"
    );
    assert!(
        execution_processes_still_running.is_empty(),
        "stdout failure returned before execution sessions exited: {execution_processes_still_running:?}"
    );
}

#[test]
fn provider_binary_backpressures_excess_mux_requests_and_keeps_control_available() {
    const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;
    const MAX_PENDING_HOST_MESSAGES: usize = 32;
    const OVERLOADED_INDEX: usize = MAX_CONCURRENT_HOST_REQUESTS + MAX_PENDING_HOST_MESSAGES;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-overload-response.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);

    for index in 0..MAX_CONCURRENT_HOST_REQUESTS {
        provider.send_request(
            &format!("overload-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("overload-message-{index}"),
                "verify explicit overload",
                &capability_revision,
            ),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session_method_count(&marker, "thread/resume") < MAX_CONCURRENT_HOST_REQUESTS {
        assert!(
            Instant::now() < deadline,
            "overload requests did not fill dispatch slots"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    for index in MAX_CONCURRENT_HOST_REQUESTS..=OVERLOADED_INDEX {
        provider.send_request(
            &format!("overload-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("overload-message-{index}"),
                "verify explicit overload",
                &capability_revision,
            ),
        );
    }
    // Mux holds excess senders before opening streams; it does not dispatch them
    // and then synthesize the old flat-queue provider_overloaded response.
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        MAX_CONCURRENT_HOST_REQUESTS
    );
    assert!(session_pids(
        &marker,
        "thread/resume",
        &format!("thread-resume-pending-{OVERLOADED_INDEX}")
    )
    .is_empty());

    provider.send_request(
        "overload-instance-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    let stopped = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("overload-instance-stop")
    });
    assert_eq!(
        stopped
            .pointer("/result/instance/status")
            .and_then(Value::as_str),
        Some("stopped")
    );
    assert!(session_pids(
        &marker,
        "thread/resume",
        &format!("thread-resume-pending-{OVERLOADED_INDEX}")
    )
    .is_empty());
    provider.request("overload-provider-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_standardizes_writer_conflict_and_removes_failed_execution() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("writer-conflict.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("none", &marker);
    clear_session_log(&marker);
    let conversation = conversation_resource_value("thread-active-writer");

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
        assert!(!encoded.contains("thread-active-writer already has an active writer"));
    }
    let resume_pids = session_pids(&marker, "thread/resume", "thread-active-writer");
    assert_eq!(resume_pids.len(), 1);
    assert_eq!(session_method_count(&marker, "thread/resume"), 2);
    assert!(session_pids(&marker, "turn/start", "thread-active-writer").is_empty());

    provider.request(
        "writer-conflict-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("writer-conflict-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_keeps_rpc_reject_local_and_never_retries_unknown_outcome() {
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
        if mode == "turn-sent-unknown" {
            assert_eq!(
                first.pointer("/error/data/details/outcome"),
                Some(&json!("unknown"))
            );
        }
        assert_eq!(
            session_pids(&marker, "thread/resume", "thread-created").len(),
            0
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
        assert!(resume_pids.is_empty());
        assert_eq!(
            session_method_count(&marker, "turn/start"),
            if mode == "turn-reject" { 2 } else { 1 }
        );

        provider.request(
            "failed-turn-stop",
            "instance.stop",
            json!({ "route": route_value() }),
        );
        provider.request("failed-turn-shutdown", "provider.shutdown", json!({}));
    }
}

#[test]
fn provider_binary_shared_server_crash_requires_instance_restart() {
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
    let turn = started.pointer("/result/turn/resource").cloned().unwrap();
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
    assert!(steered.get("error").is_some());
    let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
    assert!(resume_pids.is_empty());

    provider.request(
        "crash-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("crash-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_rejects_legacy_framing_before_initialization() {
    let mut child = Command::new(provider_executable())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut oversized_header = Vec::from(codepet_provider_sdk::PROVIDER_FRAME_MAGIC);
    oversized_header.push(codepet_provider_sdk::PROVIDER_FRAME_VERSION);
    oversized_header.push(codepet_provider_sdk::ProviderFrameEncoding::RawJson as u8);
    oversized_header
        .extend_from_slice(&(codepet_provider_sdk::MAX_PROVIDER_FRAME_BYTES as u32).to_be_bytes());
    stdin.write_all(&oversized_header).unwrap();
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
    let mut stdout = child.stdout.take().unwrap();
    let mut output = Vec::new();
    stdout.read_to_end(&mut output).unwrap();
    assert!(
        output.is_empty(),
        "Unnegotiated input must not dispatch or emit business messages"
    );
}

#[test]
fn provider_binary_public_mux_smoke() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure("normal", &directory.path().join("smoke.txt"));
    let described = provider.request("smoke-describe", "provider.describe", json!({}));
    assert_eq!(described["result"]["plugin"]["pluginId"], CODEX_PLUGIN_ID);
    let listed = provider.request(
        "smoke-list",
        "conversation.list",
        json!({
            "route": route_value(), "limit": 10, "projectFilter": { "kind": "all" }
        }),
    );
    assert!(listed["result"]["conversations"].is_array());
    let fetched = provider.request(
        "smoke-get",
        "conversation.get",
        json!({ "conversation": conversation }),
    );
    assert_eq!(fetched["result"]["conversation"]["resource"], conversation);
    assert!(fetched["result"]["items"].is_array());
    let stopped = provider.request(
        "smoke-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    assert_eq!(stopped["result"]["instance"]["status"], "stopped");
    let shutdown = provider.request("smoke-shutdown", "provider.shutdown", json!({}));
    assert_eq!(shutdown["result"]["accepted"], true);
    assert!(provider
        .close_input_and_wait(Duration::from_secs(3))
        .success());
}
