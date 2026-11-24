use super::*;

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
            "thread/turns/list\tthread-writer-held",
            "thread/read\tthread-writer-held",
            "thread/turns/list\tthread-writer-held"
        ]
    );
    assert!(session_pids(&marker, "process/start", "").is_empty());
    assert!(session_pids(&marker, "thread/resume", "thread-writer-held").is_empty());

    provider.request(
        "writer-held-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("writer-held-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_create_returns_before_conversation_is_materialized() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("create-readiness.txt");
    let request_log = directory.path().join("create-readiness-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) =
        provider.configure_with_request_log("create-read-eventually", &marker, Some(&request_log));

    assert_eq!(
        conversation.get("nativeResourceId").and_then(Value::as_str),
        Some("thread-created")
    );
    let fetched = provider.request(
        "created-readable",
        "conversation.get",
        json!({ "conversation": conversation }),
    );
    assert!(fetched.get("error").is_none(), "{fetched}");

    let requests = std::fs::read_to_string(&request_log).unwrap();
    let creation_requests = requests
        .lines()
        .skip_while(|line| *line != "thread/start")
        .collect::<Vec<_>>();
    assert_eq!(
        creation_requests,
        vec![
            "thread/start",
            "thread/read\tthread-created",
            "thread/turns/list\tthread-created"
        ]
    );

    provider.request(
        "create-readiness-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("create-readiness-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_create_does_not_wait_for_history_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("observer-create-readiness.txt");
    let request_log = directory
        .path()
        .join("observer-create-readiness-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure_with_request_log(
        "observer-create-read-eventually",
        &marker,
        Some(&request_log),
    );

    assert_eq!(
        conversation.get("nativeResourceId").and_then(Value::as_str),
        Some("thread-created")
    );
    let requests = std::fs::read_to_string(&request_log).unwrap();
    let created_reads = requests
        .lines()
        .filter(|line| *line == "thread/read\tthread-created")
        .count();
    assert_eq!(created_reads, 0, "{requests}");
    assert_eq!(
        requests
            .lines()
            .filter(|line| *line == "thread/turns/list\tthread-created")
            .count(),
        0,
        "{requests}"
    );

    provider.request(
        "observer-create-readiness-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request(
        "observer-create-readiness-shutdown",
        "provider.shutdown",
        json!({}),
    );
}

#[test]
fn provider_binary_returns_empty_history_for_unmaterialized_new_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unmaterialized.txt");
    let request_log = directory.path().join("unmaterialized-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure_with_request_log(
        "unmaterialized-before-first-message",
        &marker,
        Some(&request_log),
    );

    let fetched = provider.request(
        "unmaterialized-get",
        "conversation.get",
        json!({ "conversation": conversation }),
    );

    assert!(fetched.get("error").is_none(), "{fetched}");
    assert_eq!(
        fetched
            .pointer("/result/conversation/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("thread-created")
    );
    assert_eq!(
        fetched.pointer("/result/items").and_then(Value::as_array),
        Some(&Vec::new())
    );
    assert!(fetched.pointer("/result/conversation/activeTurn").is_none());
    let log = std::fs::read_to_string(&request_log).unwrap();
    let lines = log.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "initialize");
    let probes = [
        "model/list",
        "project/list",
        "account/read",
        "account/rateLimits/read",
        "account/usage/read",
    ];
    for probe in probes {
        assert_eq!(lines.iter().filter(|line| **line == probe).count(), 1);
    }
    assert_eq!(
        lines
            .into_iter()
            .filter(|line| !probes.contains(line))
            .collect::<Vec<_>>(),
        vec![
            "initialize",
            "thread/start",
            "thread/read\tthread-created",
            "thread/turns/list\tthread-created"
        ]
    );

    provider.request(
        "unmaterialized-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("unmaterialized-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_starts_first_turn_without_reading_unmaterialized_history() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unmaterialized-first-turn.txt");
    let request_log = directory
        .path()
        .join("unmaterialized-first-turn-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure_with_request_log(
        "unmaterialized-before-first-message",
        &marker,
        Some(&request_log),
    );
    std::fs::write(&request_log, "").unwrap();

    let started = provider.request(
        "unmaterialized-first-turn",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "unmaterialized-first-message",
            "start without persisted history",
            &capability_revision,
        ),
    );

    assert!(started.get("error").is_none(), "{started}");
    assert!(started
        .pointer("/result/turn/resource/nativeResourceId")
        .is_some());
    let output = provider.event("event.turnOutputDelta");
    assert_eq!(
        output
            .pointer("/params/conversation/nativeResourceId")
            .and_then(Value::as_str),
        conversation.get("nativeResourceId").and_then(Value::as_str)
    );
    assert_eq!(
        output.pointer("/params/delta").and_then(Value::as_str),
        Some("fixture output")
    );
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["turn/start\tthread-created"]
    );

    provider.request(
        "unmaterialized-first-turn-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request(
        "unmaterialized-first-turn-shutdown",
        "provider.shutdown",
        json!({}),
    );
}

#[test]
fn provider_binary_exposes_codex_authentication_usage_and_projects() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("provider-runtime-metadata.txt");
    let mut provider = ProviderBinary::spawn();
    provider
        .initialize_and_create_instance(&app_server_executable(), fixture_args("normal", &marker));

    let started = provider.request(
        "runtime-metadata-start",
        "instance.start",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        started.pointer("/result/instance/status"),
        Some(&json!("starting"))
    );
    let event = provider.receive(Duration::from_secs(3), |event| {
        event.get("method").and_then(Value::as_str) == Some("event.instanceStatusChanged")
            && event.pointer("/params/instance/authentication").is_some()
            && event.pointer("/params/instance/usage").is_some()
            && event
                .pointer("/params/instance/capabilities/turnSend")
                .is_some()
    });
    let started = json!({"result":{"instance":event["params"]["instance"]}});
    assert_eq!(
        started
            .pointer("/result/instance/authentication/status")
            .and_then(Value::as_str),
        Some("signed-in")
    );
    assert_eq!(
        started
            .pointer("/result/instance/authentication/displayText")
            .and_then(Value::as_str),
        Some("Signed in · ChatGPT Pro")
    );
    assert_eq!(
        started
            .pointer("/result/instance/usage/displayText")
            .and_then(Value::as_str),
        Some("5h 75% remaining · 7d 60% remaining")
    );
    assert_eq!(
        started
            .pointer("/result/instance/usage/details")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    let projects = provider.request(
        "runtime-metadata-projects",
        "project.list",
        json!({ "route": route_value(), "limit": 40 }),
    );
    assert_eq!(
        projects
            .pointer("/result/projects/0/name")
            .and_then(Value::as_str),
        Some("Fixture Project")
    );

    provider.request(
        "runtime-metadata-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("runtime-metadata-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_acquire_interaction_reuses_created_session_and_returns_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("acquire-interaction.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("normal", &marker);
    clear_session_log(&marker);

    let first = provider.request(
        "acquire-interaction-first",
        "conversation.acquireInteraction",
        json!({ "conversation": conversation.clone() }),
    );
    let second = provider.request(
        "acquire-interaction-second",
        "conversation.acquireInteraction",
        json!({ "conversation": conversation.clone() }),
    );

    for response in [&first, &second] {
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(
            response
                .pointer("/result/selection/accessModeId")
                .and_then(Value::as_str),
            Some("workspace-write")
        );
        assert_eq!(
            response
                .pointer("/result/selection/reasoningEffortId")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            response
                .pointer("/result/selection/model/modelId")
                .and_then(Value::as_str),
            Some("gpt-fixture")
        );
        assert!(response
            .pointer("/result/leaseExpiresAt")
            .and_then(Value::as_u64)
            .is_none());
    }
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-created").len(),
        0
    );

    let started = provider.request(
        "acquire-interaction-turn",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "acquire-interaction-message",
            "keep the leased writer",
            &capability_revision,
        ),
    );
    let turn = started.pointer("/result/turn/resource").cloned().unwrap();
    let approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();
    provider.request(
        "acquire-interaction-approval",
        "approval.resolve",
        json!({ "approval": approval, "decision": "approve" }),
    );
    provider.request(
        "acquire-interaction-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation.clone(), "turn": turn }),
    );
    let restarted = provider.request(
        "acquire-interaction-restart",
        "turn.start",
        turn_start_params(
            conversation,
            "acquire-interaction-message-two",
            "reuse the leased writer",
            &capability_revision,
        ),
    );
    assert!(restarted.get("error").is_none(), "{restarted}");
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-created").len(),
        0
    );

    provider.request(
        "acquire-interaction-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request(
        "acquire-interaction-shutdown",
        "provider.shutdown",
        json!({}),
    );
}

#[test]
fn provider_binary_conversation_get_pages_history_without_resuming() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("paginated-history.txt");
    let request_log = directory.path().join("paginated-history-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("none", &marker, Some(&request_log));
    std::fs::write(&request_log, "").unwrap();
    clear_session_log(&marker);
    let conversation = conversation_resource_value("thread-paginated");

    let first = provider.request(
        "paginated-history-first",
        "conversation.get",
        json!({ "conversation": conversation.clone(), "limit": 1 }),
    );
    let wider = provider.request(
        "paginated-history-wider",
        "conversation.get",
        json!({ "conversation": conversation.clone(), "limit": 2 }),
    );
    let second = provider.request(
        "paginated-history-second",
        "conversation.get",
        json!({
            "conversation": conversation,
            "cursor": first.pointer("/result/pageInfo/nextCursor").unwrap(),
            "limit": 1
        }),
    );

    assert!(first.get("error").is_none(), "{first}");
    assert!(wider.get("error").is_none(), "{wider}");
    assert!(second.get("error").is_none(), "{second}");
    assert_eq!(
        first
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert_eq!(
        first
            .pointer("/result/items/0/contents/0/contentId")
            .and_then(Value::as_str),
        Some("agent-page-two:text")
    );
    assert_eq!(
        first
            .pointer("/result/pageInfo/nextCursor")
            .and_then(Value::as_str),
        Some("page-two")
    );
    assert_eq!(
        wider
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-one")
    );
    assert_eq!(
        wider
            .pointer("/result/items/1/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert!(wider.pointer("/result/pageInfo/nextCursor").is_none());
    assert_eq!(
        second
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-one")
    );
    assert!(second.pointer("/result/pageInfo/nextCursor").is_none());
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated",
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated",
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated"
        ]
    );
    assert!(session_pids(&marker, "process/start", "").is_empty());
    assert!(session_pids(&marker, "thread/resume", "thread-paginated").is_empty());

    provider.request(
        "paginated-history-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("paginated-history-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reads_full_turns_for_app_server_0_152() {
    assert_provider_reads_full_turns("app-server-0.152-history");
}

#[test]
fn provider_binary_read_errors_preserve_shared_server_and_other_conversations() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("read-error-isolation.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("read-error-isolation", &marker);
    let original_pids = session_pids(&marker, "process/start", "");
    assert_eq!(original_pids.len(), 1);
    for (conversation, error_code) in [
        ("thread-oversized", "provider_protocol_error"),
        ("thread-invalid-json", "provider_protocol_error"),
        ("thread-invalid-envelope", "provider_protocol_error"),
        ("thread-rpc-error", "provider_error"),
    ] {
        let response = provider.request_with_timeout(
            conversation,
            "conversation.get",
            json!({
                "conversation": conversation_resource_value(conversation), "limit": 20,
            }),
            Duration::from_secs(30),
        );
        assert_eq!(
            response.pointer("/error/data/code").and_then(Value::as_str),
            Some(error_code),
            "{response}"
        );
        let healthy = provider.request_with_timeout(
            "healthy-after-error",
            "conversation.get",
            json!({
                "conversation": conversation_resource_value("thread-other"), "limit": 20,
            }),
            Duration::from_secs(30),
        );
        assert!(healthy.get("error").is_none(), "{healthy}");
        assert_eq!(session_pids(&marker, "process/start", ""), original_pids);
    }
    provider.request(
        "isolation-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("isolation-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reads_full_turns_with_host_client_user_agent() {
    assert_provider_reads_full_turns("app-server-code-pet-history");
}

#[test]
fn provider_binary_forwards_turn_limit_and_truncates_only_tool_text_with_item_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("tool-text-policy.txt");
    let requests = directory.path().join("tool-text-policy-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("app-server-code-pet-history", &marker, Some(&requests));
    let pids = session_pids(&marker, "process/start", "");
    std::fs::write(&requests, "").unwrap();
    let timer = Instant::now();
    let response = provider.request_with_timeout(
        "tool-text-policy",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-tool-text-policy"), "limit": 20,
        }),
        Duration::from_secs(30),
    );
    eprintln!("20 MiB history diagnostic: {:?}", timer.elapsed());
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response.pointer("/result/pageInfo/nextCursor"),
        Some(&json!("native-next-page"))
    );
    let items = response
        .pointer("/result/items")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 3);
    let text = items[0]
        .pointer("/tool/outcome/content/0/text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(text.len() <= codepet_provider_sdk::DEFAULT_TOOL_TEXT_BYTES);
    assert_eq!(
        items[0].pointer("/_meta/truncations/0/originalBytes"),
        Some(&json!(20 * 1024 * 1024))
    );
    assert_eq!(
        items[0].pointer("/_meta/truncations/0/retainedBytes"),
        Some(&json!(text.len()))
    );
    assert_eq!(
        items[1]
            .pointer("/contents/0/text")
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        300_000
    );
    assert_eq!(
        items[2]
            .pointer("/tool/outcome/content/0/text")
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        300_000
    );
    assert!(items[1].get("_meta").is_none() && items[2].get("_meta").is_none());
    // A large response can overlap the independent summary poll. Assert the
    // requested thread's operations rather than timing the global request log.
    assert_eq!(
        std::fs::read_to_string(&requests)
            .unwrap()
            .lines()
            .filter(|line| line.split('\t').nth(1) == Some("thread-tool-text-policy"))
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-tool-text-policy",
            "thread/turns/list\tthread-tool-text-policy",
        ]
    );
    assert_eq!(session_pids(&marker, "process/start", ""), pids);
    provider.request("tool-text-policy-shutdown", "provider.shutdown", json!({}));
}

#[tokio::test]
async fn oversized_event_keeps_shared_server_and_subsequent_events_available() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("event-error-isolation.txt");
    let rejected = Arc::new(AtomicBool::new(false));
    let sink_rejected = rejected.clone();
    let (sender, receiver) = mpsc::channel();
    let events = Arc::new(move |event| {
        if matches!(&event, ProtocolEvent::EventTurnUpserted { .. })
            && !sink_rejected.swap(true, Ordering::SeqCst)
        {
            return Err(codepet_provider_sdk::ProtocolError {
                code: "provider_frame_too_large".to_string(),
                message: "fixture event exceeds wire frame limit".to_string(),
                retryable: false,
                details: None,
            });
        }
        sender.send(event).unwrap();
        Ok(())
    });
    let (provider, route, capability_revision) =
        configured_direct_provider_with_events("complete-on-approval", &marker, events).await;
    let pids = session_pids(&marker, "process/start", "");
    assert_eq!(pids.len(), 1);
    ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation_resource(&route, "thread-event-isolation"),
            client_request_id: "event-isolation-start".to_string(),
            capability_revision,
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "fixture turn".to_string(),
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
    loop {
        if matches!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            ProtocolEvent::EventApprovalRequested { .. }
        ) {
            break;
        }
    }
    assert!(rejected.load(Ordering::SeqCst));
    ProviderProtocolServer::project_list(
        provider.as_ref(),
        ProjectListRequest {
            route,
            cursor: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    assert_eq!(session_pids(&marker, "process/start", ""), pids);
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[test]
fn provider_binary_reads_full_turns_without_item_probes_on_app_server_0_151() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("legacy-full-history.txt");
    let request_log = directory.path().join("legacy-full-history-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("app-server-0.151-history", &marker, Some(&request_log));
    std::fs::write(&request_log, "").unwrap();

    let response = provider.request(
        "legacy-full-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-paginated"),
            "limit": 1
        }),
    );

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated"
        ]
    );

    provider.request(
        "legacy-full-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("legacy-full-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_preserves_large_history_content_and_keeps_serving() {
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
    assert!(fetched.get("error").is_none(), "{fetched}");
    let text = fetched
        .pointer("/result/items/1/contents/0/text")
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(text.len(), 1024 * 1024 + 4096);
    assert!(fetched
        .pointer("/result/items/1/contents/0/truncation")
        .is_none());

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
fn provider_binary_pages_large_turns_without_transport_budgeting() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("oversized-history.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("none", &marker);

    let fetched = provider.request_with_timeout(
        "oversized-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-output-too-large")
        }),
        Duration::from_secs(30),
    );
    assert_eq!(fetched["id"], "oversized-history");
    assert!(fetched.get("error").is_none(), "{fetched}");
    assert_eq!(
        fetched
            .pointer("/result/items")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|item| item
                .pointer("/resource/nativeResourceId")
                .and_then(Value::as_str)
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["agent-large-one", "agent-large-two", "agent-large-three"]
    );
    assert!(fetched
        .pointer("/result/items/0/contents/0/truncation")
        .is_none());

    let reduced = provider.request_with_timeout(
        "reduced-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-output-too-large"),
            "limit": 2
        }),
        Duration::from_secs(30),
    );
    assert!(reduced.get("error").is_none(), "{reduced}");
    assert_eq!(
        reduced
            .pointer("/result/items")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|item| item
                .pointer("/resource/nativeResourceId")
                .and_then(Value::as_str)
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["agent-large-two", "agent-large-three"]
    );
    assert_eq!(
        reduced
            .pointer("/result/pageInfo/nextCursor")
            .and_then(Value::as_str),
        Some("large-page-three")
    );

    let remainder = provider.request_with_timeout(
        "remaining-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-output-too-large"),
            "cursor": "large-page-three",
            "limit": 2
        }),
        Duration::from_secs(30),
    );
    assert!(remainder.get("error").is_none(), "{remainder}");
    assert_eq!(
        remainder
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-large-one")
    );
    assert!(remainder.pointer("/result/pageInfo/nextCursor").is_none());

    let described = provider.request("after-oversized-history", "provider.describe", json!({}));
    assert_eq!(
        described
            .pointer("/result/plugin/pluginId")
            .and_then(Value::as_str),
        Some(CODEX_PLUGIN_ID)
    );
    provider.request("oversized-history-shutdown", "provider.shutdown", json!({}));
}
