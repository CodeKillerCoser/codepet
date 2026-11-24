use super::*;

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
            "protocol": {"version": codepet_provider_sdk::PROTOCOL_VERSION, "minVersion": codepet_provider_sdk::MIN_PROTOCOL_VERSION}
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
        started
            .pointer("/result/instance/status")
            .and_then(Value::as_str),
        Some("ready")
    );
    let capability_revision = started
        .pointer("/result/instance/capabilities/revision")
        .and_then(Value::as_str)
        .expect("real Provider must advertise a capability revision")
        .to_string();
    let listed = provider.request(
        "real-list",
        "conversation.list",
        json!({
            "route": route_value(),
            "projectFilter": { "kind": "all" },
            "limit": 1
        }),
    );
    assert!(listed.pointer("/result/conversations").is_some());
    let projects_response = provider.request(
        "real-project-list",
        "project.list",
        json!({
            "route": route_value(),
            "limit": 100
        }),
    );
    let projects = projects_response
        .pointer("/result/projects")
        .and_then(Value::as_array)
        .expect("Codex 0.152 project.list must return projects");
    let standalone = provider.request(
        "real-standalone-list",
        "conversation.list",
        json!({
            "route": route_value(),
            "projectFilter": { "kind": "standalone" },
            "limit": 100
        }),
    );
    eprintln!(
        "real Codex projects={} standalone conversations={}",
        projects.len(),
        standalone
            .pointer("/result/conversations")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default()
    );
    assert!(
        standalone
            .pointer("/result/conversations")
            .and_then(Value::as_array)
            .is_some_and(|conversations| conversations.iter().all(|conversation| {
                conversation.get("project").is_none()
                    || conversation.get("project").is_some_and(Value::is_null)
            })),
        "standalone list returned a project-owned conversation: {standalone}"
    );
    for (index, project) in projects.iter().enumerate() {
        let resource = project
            .get("resource")
            .cloned()
            .expect("project must have a routed resource");
        let conversations = provider.request(
            &format!("real-project-conversations-{index}"),
            "conversation.list",
            json!({
                "route": route_value(),
                "projectFilter": { "kind": "project", "project": resource.clone() },
                "limit": 100
            }),
        );
        eprintln!(
            "project {:?} conversations={} response_error={:?}",
            project.get("name").and_then(Value::as_str),
            conversations
                .pointer("/result/conversations")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
            conversations.get("error")
        );
        assert!(conversations.get("error").is_none(), "{conversations}");
        assert!(
            conversations
                .pointer("/result/conversations")
                .and_then(Value::as_array)
                .is_some_and(|values| values
                    .iter()
                    .all(|conversation| { conversation.get("project") == Some(&resource) })),
            "project list returned a conversation with the wrong membership: {conversations}"
        );
    }
    if let Some(workspace) = std::env::var_os("CODEPET_REAL_WORKSPACE") {
        let created = provider.request(
            "real-conversation-create",
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "workspaceRoot": workspace.to_string_lossy(),
                "workspaceMode": "worktree"
            }),
        );
        assert!(created
            .pointer("/result/conversation/resource/nativeResourceId")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty()));
        assert!(created
            .pointer("/result/conversation/workspaceRoot")
            .and_then(Value::as_str)
            .is_some_and(|path| path == workspace.to_string_lossy()));
    }
    if let Some(workspace) = std::env::var_os("CODEPET_REAL_UNMATERIALIZED_WORKSPACE") {
        let created = provider.request(
            "real-unmaterialized-create",
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "workspaceRoot": workspace.to_string_lossy(),
                "workspaceMode": "main"
            }),
        );
        assert!(created.get("error").is_none(), "{created}");
        let resource = created
            .pointer("/result/conversation/resource")
            .cloned()
            .expect("real conversation.create must return a resource");
        let fetched = provider.request(
            "real-unmaterialized-get",
            "conversation.get",
            json!({ "conversation": resource.clone() }),
        );
        assert!(fetched.get("error").is_none(), "{fetched}");
        assert_eq!(
            fetched.pointer("/result/conversation/resource"),
            Some(&resource)
        );
        assert_eq!(
            fetched
                .pointer("/result/items")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        const USER_MARKER: &str = "CODEPET_PROVIDER_USER_MARKER_0152";
        const ASSISTANT_MARKER: &str = "CODEPET_PROVIDER_ASSISTANT_OK_0152";
        provider.send_request(
            "real-unmaterialized-turn-start",
            "turn.start",
            turn_start_params(
                resource.clone(),
                "real-unmaterialized-client-message",
                &format!("{USER_MARKER}. Reply with exactly {ASSISTANT_MARKER}"),
                &capability_revision,
            ),
        );
        let turn_started = provider.receive(Duration::from_secs(30), |message| {
            message.get("id").and_then(Value::as_str) == Some("real-unmaterialized-turn-start")
        });
        assert!(turn_started.get("error").is_none(), "{turn_started}");
        let conversation_id = resource
            .get("nativeResourceId")
            .and_then(Value::as_str)
            .expect("created resource must have a native id");
        provider.receive(Duration::from_secs(120), |message| {
            message.get("method").and_then(Value::as_str) == Some("event.turnUpserted")
                && message
                    .pointer("/params/turn/conversation/nativeResourceId")
                    .and_then(Value::as_str)
                    == Some(conversation_id)
                && message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    == Some("completed")
        });
        let materialized = provider.request(
            "real-materialized-get",
            "conversation.get",
            json!({ "conversation": resource, "limit": 20 }),
        );
        assert!(materialized.get("error").is_none(), "{materialized}");
        let serialized = materialized.to_string();
        assert!(
            serialized.contains(USER_MARKER),
            "user message was not materialized"
        );
        assert!(
            serialized.contains(ASSISTANT_MARKER),
            "assistant response was not exposed by conversation.get: {materialized}"
        );
    }
    if let Some(conversation_id) = std::env::var_os("CODEPET_REAL_CONVERSATION_ID") {
        let conversation_id = conversation_id.to_string_lossy();
        let fetched = provider.request(
            "real-existing-conversation-get",
            "conversation.get",
            json!({
                "conversation": conversation_resource_value(&conversation_id),
                "limit": 1
            }),
        );
        assert!(fetched.get("error").is_none(), "{fetched}");
        assert_eq!(
            fetched
                .pointer("/result/conversation/resource/nativeResourceId")
                .and_then(Value::as_str),
            Some(conversation_id.as_ref())
        );
        assert!(fetched
            .pointer("/result/items")
            .is_some_and(Value::is_array));
        assert!(fetched
            .pointer("/result/pageInfo")
            .is_some_and(Value::is_object));
    }
    let stopped = provider.request(
        "real-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        stopped
            .pointer("/result/instance/status")
            .and_then(Value::as_str),
        Some("stopped")
    );
    provider.request("real-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn automatic_title_is_saved_once_without_blocking_execution_or_publishing_ephemeral_events() {
    for mode in ["auto-title", "auto-title-failure"] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("title.marker");
        let requests = directory.path().join("requests.log");
        let mut provider = ProviderBinary::spawn();
        let (conversation, revision) =
            provider.configure_with_request_log(mode, &marker, Some(&requests));
        let started = provider.request(
            "first-title-turn",
            "turn.start",
            turn_start_params(conversation.clone(), "first", "需要一个简短标题", &revision),
        );
        assert_eq!(started["result"]["accepted"], true, "{started}");
        let output = provider.event("event.turnOutputDelta");
        assert_eq!(output["params"]["delta"], "fixture output");
        wait_for_session_count(&marker, "thread/unsubscribe", 1, Duration::from_secs(3));
        if mode == "auto-title" {
            let named = provider.receive(Duration::from_secs(3), |event| {
                event["method"] == "event.conversationUpserted"
                    && event["params"]["conversation"]["title"] == "测试自动会话标题"
            });
            assert_eq!(
                named["params"]["conversation"]["resource"]["nativeResourceId"],
                "thread-created"
            );
        }
        let interrupted = provider.request(
            "interrupt-title-turn",
            "turn.interrupt",
            json!({
                "conversation": conversation, "turn": started["result"]["turn"]["resource"]
            }),
        );
        assert!(interrupted.get("error").is_none(), "{interrupted}");
        let second = provider.request(
            "second-title-turn",
            "turn.start",
            turn_start_params(conversation, "second", "继续对话", &revision),
        );
        assert_eq!(second["result"]["accepted"], true, "{second}");
        provider.collect_for(Duration::from_millis(150));
        let log = std::fs::read_to_string(&requests).unwrap();
        assert_eq!(
            log.lines()
                .filter(|line| *line == "turn/start\ttitle-job")
                .count(),
            1,
            "{log}"
        );
        assert!(provider
            .buffered
            .iter()
            .all(|event| !event.to_string().contains("title-job")));
        provider.request(
            "title-stop",
            "instance.stop",
            json!({"route": route_value()}),
        );
        provider.request("title-shutdown", "provider.shutdown", json!({}));
    }
}
