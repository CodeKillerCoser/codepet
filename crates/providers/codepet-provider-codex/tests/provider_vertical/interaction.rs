use super::*;

#[test]
fn release_interaction_restarts_clean_server_without_reacquiring_old_conversations() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("release-interaction.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, revision) = provider.configure("none", &marker);
    let mut unrelated = ProviderBinary::spawn();
    let unrelated_marker = directory.path().join("unrelated.txt");
    unrelated.configure("none", &unrelated_marker);
    let unrelated_server = session_pids(&unrelated_marker, "process/start", "");
    let server = session_pids(&marker, "process/start", "");
    for id in ["thread-release-a", "thread-release-b"] {
        let response = provider.request(
            id,
            "turn.start",
            turn_start_params(
                conversation_resource_value(id),
                &format!("message-{id}"),
                "running",
                &revision,
            ),
        );
        assert!(response.get("error").is_none(), "{response}");
    }
    provider
        .buffered
        .retain(|message| message["method"] != "event.instanceStatusChanged");
    let resumed_before = session_method_count(&marker, "thread/resume");
    for index in 0..2 {
        let response = provider.request(
            &format!("release-{index}"),
            "conversation.releaseInteraction",
            json!({
                "conversation": conversation_resource_value("thread-release-a")
            }),
        );
        assert_eq!(
            response["result"],
            json!({"released":true,"scope":"providerInstance"})
        );
    }
    assert!(
        !process_is_running(server[0]),
        "release must wait for the owned harness to exit"
    );
    assert!(
        process_is_running(unrelated_server[0]),
        "release must only kill its own harness"
    );
    wait_for_replacement_server(&marker, 2);
    provider.receive(Duration::from_secs(3), |message| {
        message["method"] == "event.instanceStatusChanged"
            && message.pointer("/params/instance/status") == Some(&json!("ready"))
    });
    assert_eq!(
        session_pids(&marker, "process/start", "").len(),
        2,
        "duplicate release must not restart twice"
    );
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        resumed_before
    );
    let start = provider.request(
        "background-start",
        "instance.start",
        json!({"route":route_value()}),
    );
    assert_eq!(
        start
            .pointer("/result/instance/status")
            .and_then(Value::as_str),
        Some("ready")
    );
    for sequence in 1..=3 {
        provider.request(&format!("heartbeat-{sequence}"), "provider.ping", json!({
            "sequence":sequence,"hostSessionId":"release-host", "clients":{"revision":sequence,
                "connections":[{"clientId":"phone","connectionId":format!("connection-{sequence}")}]}, "instances":[route_value()]
        }));
    }
    for id in ["thread-release-a", "thread-release-b"] {
        let response = provider.request(
            &format!("read-{id}"),
            "conversation.get",
            json!({"conversation":conversation_resource_value(id)}),
        );
        assert!(
            response.get("error").is_none(),
            "history must be available without taking control: {response}"
        );
    }
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(session_pids(&marker, "process/start", "").len(), 2);
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        resumed_before,
        "history must not reacquire writers"
    );
    let resumed = provider.request(
        "explicit-resume",
        "conversation.acquireInteraction",
        json!({"conversation":conversation_resource_value("thread-release-a")}),
    );
    assert!(resumed.get("error").is_none(), "{resumed}");
    let servers = session_pids(&marker, "process/start", "");
    assert_eq!(servers.len(), 2);
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-release-b"),
        server,
        "B must not reacquire automatically"
    );
    unrelated.request("unrelated-shutdown", "provider.shutdown", json!({}));
    provider.request("release-shutdown", "provider.shutdown", json!({}));
    wait_for_processes_to_exit(&servers, Duration::from_secs(2));
}

#[test]
fn release_interaction_allows_host_release_control_on_the_clean_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("release-then-idle.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("none", &marker);
    let released = provider.request(
        "release",
        "conversation.releaseInteraction",
        json!({
            "conversation": conversation_resource_value("thread-created")
        }),
    );
    assert_eq!(released["result"]["released"], true);
    wait_for_replacement_server(&marker, 2);
    let after_release = session_pids(&marker, "process/start", "");
    assert_eq!(after_release.len(), 2);
    provider.wait_for_idle_release();
    wait_for_replacement_server(&marker, 3);
    wait_for_processes_to_exit(&after_release, Duration::from_secs(2));
    assert_eq!(session_method_count(&marker, "thread/resume"), 0);
    provider.request("shutdown", "provider.shutdown", json!({}));
    wait_for_processes_to_exit(
        &session_pids(&marker, "process/start", ""),
        Duration::from_secs(2),
    );
}

#[test]
fn release_interaction_reports_restart_failure_after_terminating_the_old_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("release-restart-failure.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("execution-initialize-reject", &marker);
    let released = provider.request(
        "release",
        "conversation.releaseInteraction",
        json!({
            "conversation": conversation_resource_value("thread-created")
        }),
    );
    assert_eq!(
        released.pointer("/error/data/code").and_then(Value::as_str),
        Some("interaction_restart_failed")
    );
    let pids = session_pids(&marker, "process/start", "");
    assert_eq!(pids.len(), 2);
    wait_for_processes_to_exit(&pids, Duration::from_secs(2));
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(
        session_pids(&marker, "process/start", ""),
        pids,
        "failure must not loop restart"
    );
    provider.request("shutdown", "provider.shutdown", json!({}));
}

#[test]
fn release_interaction_restart_is_cancelled_by_provider_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("release-restart-shutdown.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("execution-initialize-no-response", &marker);
    provider.send_request(
        "release",
        "conversation.releaseInteraction",
        json!({
            "conversation": conversation_resource_value("thread-created")
        }),
    );
    wait_for_session_count(&marker, "initialize", 2, Duration::from_secs(3));
    let shutdown = provider.request("shutdown", "provider.shutdown", json!({}));
    assert_eq!(shutdown["result"]["accepted"], true);
    let pids = session_pids(&marker, "process/start", "");
    assert_eq!(pids.len(), 2);
    wait_for_processes_to_exit(&pids, Duration::from_secs(2));
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(session_pids(&marker, "process/start", ""), pids);
}

#[test]
fn release_interaction_invalidates_pending_approval_in_previous_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("release-approval.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, revision) = provider.configure("complete-on-approval", &marker);
    let started = provider.request(
        "start-approval",
        "turn.start",
        turn_start_params(
            conversation_resource_value("thread-created"),
            "approval-message",
            "approval",
            &revision,
        ),
    );
    assert!(started.get("error").is_none());
    let approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();
    let released = provider.request(
        "release-approval",
        "conversation.releaseInteraction",
        json!({"conversation":conversation_resource_value("thread-created")}),
    );
    assert_eq!(released["result"]["released"], true);
    let resumed = provider.request(
        "resume-approval",
        "conversation.acquireInteraction",
        json!({"conversation":conversation_resource_value("thread-created")}),
    );
    assert!(resumed.get("error").is_none(), "{resumed}");
    let stale = provider.request(
        "old-approval",
        "approval.resolve",
        json!({"approval":approval,"decision":"approve"}),
    );
    assert!(
        stale.get("error").is_some(),
        "old approval must not resolve against a new server"
    );
    provider.request("approval-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn release_interaction_cancels_hung_acquire_through_control_lane() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("release-pending.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("resume-no-response", &marker);
    provider.send_request(
        "hung-acquire",
        "conversation.acquireInteraction",
        json!({"conversation":conversation_resource_value("thread-resume-pending-release")}),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while session_method_count(&marker, "thread/resume") == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let released = provider.request(
        "release-hung",
        "conversation.releaseInteraction",
        json!({"conversation":conversation_resource_value("thread-resume-pending-release")}),
    );
    assert_eq!(
        released
            .pointer("/result/released")
            .and_then(Value::as_bool),
        Some(true)
    );
    let pending = provider.receive(Duration::from_secs(2), |v| v["id"] == "hung-acquire");
    assert!(pending.get("error").is_some());
    assert_eq!(session_pids(&marker, "process/start", "").len(), 2);
    provider.request("hung-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_has_no_autonomous_idle_timer_and_duplicate_host_release_is_a_noop() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("host-owned-release.txt");
    let mut command = Command::new(provider_executable());
    command.env("CODEPET_PROVIDER_IDLE_TIMEOUT_SECONDS", "1"); // obsolete Provider timer configuration
    let mut provider = ProviderBinary::spawn_command(&mut command);
    provider.configure("none", &marker);
    let original = session_pids(&marker, "process/start", "");
    provider.collect_for(Duration::from_millis(2500));
    assert_eq!(
        session_pids(&marker, "process/start", ""),
        original,
        "only the Host can trigger recycling"
    );
    assert!(process_is_running(original[0]));
    assert_eq!(provider.release_control(), "released");
    wait_for_replacement_server(&marker, 2);
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(provider.release_control(), "notIdle");
    assert_eq!(session_pids(&marker, "process/start", "").len(), 2);
    provider.request(
        "explicit-stop",
        "instance.stop",
        json!({"route":route_value()}),
    );
    assert_eq!(provider.release_control(), "unavailable");
    provider.request("shutdown", "provider.shutdown", json!({}));
}

#[test]
fn host_release_control_hung_restart_is_cancelled_by_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("host-release-shutdown.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("execution-initialize-no-response", &marker);
    provider.collect_for(Duration::from_millis(2100));
    provider.send_request(
        "release",
        "instance.releaseControl",
        json!({"route":route_value(),"idleTimeoutSeconds":2}),
    );
    wait_for_session_count(&marker, "initialize", 2, Duration::from_secs(3));
    let shutdown = provider.request("shutdown", "provider.shutdown", json!({}));
    assert_eq!(shutdown["result"]["accepted"], true);
    let pids = session_pids(&marker, "process/start", "");
    assert_eq!(pids.len(), 2);
    wait_for_processes_to_exit(&pids, Duration::from_secs(2));
}

#[test]
fn host_release_control_restart_failure_leaves_no_stale_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("host-release-failed.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("execution-initialize-reject", &marker);
    provider.collect_for(Duration::from_millis(2100));
    let result = provider.request(
        "release",
        "instance.releaseControl",
        json!({"route":route_value(),"idleTimeoutSeconds":2}),
    );
    assert_eq!(
        result.pointer("/error/data/code"),
        Some(&json!("release_control_restart_failed"))
    );
    wait_for_processes_to_exit(
        &session_pids(&marker, "process/start", ""),
        Duration::from_secs(2),
    );
    assert_eq!(provider.release_control(), "unavailable");
    provider.request("shutdown", "provider.shutdown", json!({}));
}

#[test]
fn host_release_control_restarts_without_resuming_previous_conversations() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("online-idle.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("none", &marker);
    for id in ["thread-a", "thread-b"] {
        let acquired = provider.request(
            id,
            "conversation.acquireInteraction",
            json!({"conversation":conversation_resource_value(id)}),
        );
        assert!(acquired.get("error").is_none(), "{acquired}");
    }
    provider
        .buffered
        .retain(|message| message["method"] != "event.instanceStatusChanged");
    let old = session_pids(&marker, "process/start", "");
    assert_eq!(old.len(), 1);
    assert!(session_method_count(&marker, "thread/resume") >= 2);
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut sequence = 0;
    while process_is_running(old[0]) {
        assert!(
            Instant::now() < deadline,
            "healthy heartbeats and reads must not keep idle locks forever"
        );
        sequence += 1;
        provider.request(&format!("ping-{sequence}"), "provider.ping", json!({
            "sequence":sequence,"hostSessionId":"online-host",
            "clients":{"revision":1,"connections":[{"clientId":"phone","connectionId":"socket"}]},
            "instances":[route_value()]
        }));
        let read = provider.request(
            &format!("read-{sequence}"),
            "conversation.get",
            json!({"conversation":conversation_resource_value("thread-listed"),"limit":1}),
        );
        // A read racing the idle transition can report unavailable, but cannot restart or resume itself.
        if !process_is_running(old[0]) {
            break;
        }
        assert!(read.get("error").is_none(), "{read}");
        let outcome = provider.release_control();
        assert!(matches!(outcome.as_str(), Some("released" | "notIdle")));
        provider.collect_for(Duration::from_millis(100));
    }
    let resumed_before = session_method_count(&marker, "thread/resume");
    wait_for_replacement_server(&marker, 2);
    provider.receive(Duration::from_secs(3), |message| {
        message["method"] == "event.instanceStatusChanged"
            && message.pointer("/params/instance/status") == Some(&json!("ready"))
    });
    let all = session_pids(&marker, "process/start", "");
    assert_eq!(all.len(), 2);
    assert_ne!(all[0], all[1]);
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        resumed_before,
        "fresh startup must not reload A/B"
    );
    let read = provider.request(
        "fresh-history",
        "conversation.get",
        json!({"conversation":conversation_resource_value("thread-listed"),"limit":1}),
    );
    assert!(read.get("error").is_none(), "{read}");
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        resumed_before,
        "history must not restore old writers"
    );
    let acquired = provider.request(
        "new-interaction",
        "conversation.acquireInteraction",
        json!({"conversation":conversation_resource_value("thread-c")}),
    );
    assert!(acquired.get("error").is_none(), "{acquired}");
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        resumed_before + 1
    );
    provider
        .buffered
        .retain(|message| message["method"] != "event.instanceStatusChanged");
    provider.wait_for_idle_release();
    wait_for_processes_to_exit(&all, Duration::from_secs(5));
    provider.receive(Duration::from_secs(3), |message| {
        message["method"] == "event.instanceStatusChanged"
            && message.pointer("/params/instance/status") == Some(&json!("ready"))
    });
    let resumed = provider.request(
        "acquire-after-idle",
        "conversation.acquireInteraction",
        json!({"conversation":conversation_resource_value("thread-c")}),
    );
    assert!(
        resumed.get("error").is_none(),
        "idle stop must remain recoverable: {resumed}"
    );
    assert_eq!(session_pids(&marker, "process/start", "").len(), 3);
    provider.request("shutdown", "provider.shutdown", json!({}));
}

#[test]
fn sdk_heartbeat_retains_offline_turns_until_finished_and_restarts_once() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("client-presence.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, revision) = provider.configure("none", &marker);
    let server = session_pids(&marker, "process/start", "");
    assert_eq!(server.len(), 1);
    let ping = |sequence: u64, clients: &[&str]| {
        json!({
            "sequence": sequence, "hostSessionId": "test-host",
            "clients": { "revision": sequence, "connections": clients.iter().map(|id|
                json!({ "clientId": id, "connectionId": format!("connection-{id}") })).collect::<Vec<_>>() },
            "instances": [route_value()]
        })
    };
    provider.request(
        "two-clients",
        "provider.ping",
        ping(1, &["phone-a", "phone-b"]),
    );
    for id in ["thread-a", "thread-b"] {
        let response = provider.request(
            id,
            "turn.start",
            turn_start_params(
                conversation_resource_value(id),
                &format!("message-{id}"),
                "keep running",
                &revision,
            ),
        );
        assert!(response.get("error").is_none(), "{response}");
    }
    provider.request("one-client", "provider.ping", ping(2, &["phone-b"]));
    provider.collect_for(Duration::from_millis(80));
    assert!(process_is_running(server[0]));
    assert_eq!(session_pids(&marker, "process/start", ""), server);
    provider.request("no-clients", "provider.ping", ping(3, &[]));
    provider.collect_for(Duration::from_millis(2500));
    assert_eq!(provider.release_control(), "busy");
    assert!(
        process_is_running(server[0]),
        "offline running turns must keep the Server alive"
    );
    for id in ["thread-a", "thread-b"] {
        let interrupted = provider.request(
            &format!("finish-{id}"),
            "turn.interrupt",
            json!({
                "conversation": conversation_resource_value(id),
                "turn": conversation_resource_value("turn-started")
            }),
        );
        assert!(interrupted.get("error").is_none(), "{interrupted}");
        if id == "thread-a" {
            provider.collect_for(Duration::from_millis(2500));
            assert_eq!(provider.release_control(), "busy");
            assert!(
                process_is_running(server[0]),
                "another conversation is still running"
            );
        }
    }
    provider.wait_for_idle_release();
    wait_for_processes_to_exit(&server, Duration::from_secs(5));
    let alive = provider.request("provider-still-alive", "provider.describe", json!({}));
    assert!(alive.get("error").is_none());
    wait_for_replacement_server(&marker, 2);
    provider.request("reconnected", "provider.ping", ping(4, &["phone-a"]));
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(session_pids(&marker, "process/start", "").len(), 2);
    provider.request("presence-stop", "provider.ping", ping(5, &[]));
    let all_servers = session_pids(&marker, "process/start", "");
    provider.wait_for_idle_release();
    wait_for_processes_to_exit(&all_servers, Duration::from_secs(5));
    provider.request("presence-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn sdk_heartbeat_offline_turn_finishes_naturally_without_a_phone() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("offline-natural-completion.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, revision) = provider.configure("offline-natural-completion", &marker);
    let server = session_pids(&marker, "process/start", "");
    assert_eq!(server.len(), 1);
    let ping = |sequence: u64, online: bool| {
        json!({
            "sequence": sequence, "hostSessionId": "test-host",
            "clients": {"revision": sequence, "connections": if online {
                vec![json!({"clientId":"phone", "connectionId":"connection"})]
            } else { vec![] }}, "instances":[route_value()]
        })
    };
    provider.request("online", "provider.ping", ping(1, true));
    let started = provider.request(
        "start",
        "turn.start",
        turn_start_params(
            conversation_resource_value("thread-created"),
            "offline-natural",
            "finish while phone is gone",
            &revision,
        ),
    );
    assert_eq!(
        started.pointer("/result/turn/status"),
        Some(&json!("running")),
        "{started}"
    );
    provider.request("phone-removed", "provider.ping", ping(2, false));
    provider.collect_for(Duration::from_millis(100));
    assert!(
        process_is_running(server[0]),
        "disconnect must not kill the executing Server"
    );
    let terminal = provider.receive(Duration::from_secs(5), |message| {
        message["method"] == "event.turnUpserted"
            && message.pointer("/params/turn/resource/nativeResourceId")
                == Some(&json!("turn-started"))
            && matches!(
                message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str),
                Some("completed" | "failed" | "interrupted")
            )
    });
    assert_eq!(
        terminal.pointer("/params/turn/status"),
        Some(&json!("completed")),
        "{terminal}"
    );
    assert_eq!(session_method_count(&marker, "turn/interrupt"), 0);
    provider.wait_for_idle_release();
    wait_for_processes_to_exit(&server, Duration::from_secs(5));
    wait_for_replacement_server(&marker, 2);
    assert_eq!(
        session_method_count(&marker, "turn/interrupt"),
        0,
        "idle replacement must wait for natural completion"
    );
    provider.request("shutdown", "provider.shutdown", json!({}));
}

#[test]
fn sdk_heartbeat_retains_offline_approval_until_completion() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("offline-approval.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, revision) = provider.configure("complete-on-approval", &marker);
    let server = session_pids(&marker, "process/start", "");
    let ping = |sequence: u64, online: bool| {
        json!({
            "sequence": sequence, "hostSessionId": "test-host",
            "clients": {"revision": sequence, "connections": if online {
                vec![json!({"clientId":"phone", "connectionId":"connection"})]
            } else { vec![] }}, "instances":[route_value()]
        })
    };
    provider.request("online", "provider.ping", ping(1, true));
    let started = provider.request(
        "start",
        "turn.start",
        turn_start_params(
            conversation_resource_value("thread-created"),
            "offline-approval-turn",
            "approval",
            &revision,
        ),
    );
    assert!(started.get("error").is_none(), "{started}");
    let approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();
    provider.request("offline", "provider.ping", ping(2, false));
    provider.collect_for(Duration::from_millis(2500));
    assert_eq!(provider.release_control(), "busy");
    assert!(process_is_running(server[0]));
    // A real phone reconnects to resolve this; the binary fixture sends the decision directly
    // while presence remains empty, allowing terminal-event-driven cleanup to be verified.
    let resolved = provider.request(
        "approve",
        "approval.resolve",
        json!({"approval":approval, "decision":"approve"}),
    );
    assert!(resolved.get("error").is_none(), "{resolved}");
    provider.wait_for_idle_release();
    wait_for_processes_to_exit(&server, Duration::from_secs(5));
    provider.request("shutdown", "provider.shutdown", json!({}));
}

#[tokio::test]
async fn force_takeover_releases_writer_only_after_explicit_request_and_keeps_provider_session() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("takeover.txt");
    let hook = Arc::new(MockDesktopShutdown {
        marker: marker.clone(),
        calls: Default::default(),
        fail: false,
        activate_after_close: false,
    });
    let (provider, route, _) = configured_direct_provider_with_events_and_hook(
        "force-takeover",
        &marker,
        Arc::new(|_| Ok(())),
        Some(hook.clone()),
    )
    .await;
    let conversation = conversation_resource(&route, "thread-active-writer");
    for force in [None, Some(false)] {
        let error = ProviderProtocolServer::conversation_acquire_interaction(
            provider.as_ref(),
            ConversationAcquireInteractionRequest {
                conversation: conversation.clone(),
                force,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "conversation_write_conflict");
        assert_eq!(hook.calls.load(Ordering::SeqCst), 0);
    }
    let response = ProviderProtocolServer::conversation_acquire_interaction(
        provider.as_ref(),
        ConversationAcquireInteractionRequest {
            conversation: conversation.clone(),
            force: Some(true),
        },
    )
    .await
    .unwrap();
    assert!(response.selection.model.is_some());
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-active-writer").len(),
        1
    );
    ProviderProtocolServer::conversation_acquire_interaction(
        provider.as_ref(),
        ConversationAcquireInteractionRequest {
            conversation,
            force: Some(true),
        },
    )
    .await
    .unwrap();
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test]
async fn force_takeover_rejects_activity_failures_and_activity_after_shutdown() {
    for (active, fail, after_close, expected, calls) in [
        (Some("inProgress"), false, false, "conversation_active", 0),
        (
            Some("waitingApproval"),
            false,
            false,
            "conversation_active",
            0,
        ),
        (
            Some("waitingUserInput"),
            false,
            false,
            "conversation_active",
            0,
        ),
        (None, true, false, "force_takeover_failed", 1),
        (None, false, true, "conversation_active", 1),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("takeover.txt");
        if let Some(active) = active {
            std::fs::write(
                marker.with_file_name("thread-active-writer.turn-state"),
                active,
            )
            .unwrap();
        }
        let hook = Arc::new(MockDesktopShutdown {
            marker: marker.clone(),
            calls: Default::default(),
            fail,
            activate_after_close: after_close,
        });
        let (provider, route, _) = configured_direct_provider_with_events_and_hook(
            "force-takeover",
            &marker,
            Arc::new(|_| Ok(())),
            Some(hook.clone()),
        )
        .await;
        let error = ProviderProtocolServer::conversation_acquire_interaction(
            provider.as_ref(),
            ConversationAcquireInteractionRequest {
                conversation: conversation_resource(&route, "thread-active-writer"),
                force: Some(true),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(hook.calls.load(Ordering::SeqCst), calls);
        if calls == 0 {
            assert!(session_pids(&marker, "thread/resume", "thread-active-writer").is_empty());
        }
        ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
            .await
            .unwrap();
    }
}

#[test]
fn provider_binary_does_not_guess_writer_conflicts_from_near_match_errors() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("writer-conflict-near-matches.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("none", &marker);
    clear_session_log(&marker);

    for (index, thread_id) in [
        "thread-active-writer-wrong-code",
        "thread-active-writer-wrong-message",
        "thread-active-writer-with-data",
        "thread-active-writer-other-thread",
    ]
    .into_iter()
    .enumerate()
    {
        let response = provider.request(
            &format!("writer-near-match-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(thread_id),
                &format!("writer-near-match-message-{index}"),
                "writer near match",
                &capability_revision,
            ),
        );
        assert_eq!(
            response.pointer("/error/data/code").and_then(Value::as_str),
            Some("provider_error"),
            "near match {thread_id} must retain the ordinary Provider error"
        );
    }

    provider.request(
        "writer-near-match-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("writer-near-match-shutdown", "provider.shutdown", json!({}));
}
