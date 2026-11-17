use super::*;
use codepet_gateway_sdk as gateway;
use codepet_provider_data::conversation_state::SharedConversationStateStore;

async fn fetch(
    service: &ProviderGatewayService,
    scope: &str,
    cursor: Option<String>,
) -> Result<gateway::ConversationRecentResponse, String> {
    let request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0", "id": "1", "method":"conversation.recent",
        "params": {"providerId":"recent-instance", "limit":20, "cursor":cursor},
    }))
    .unwrap();
    let response = service.dispatch_for_caller_scope(scope, request).await;
    match response.response {
        JsonRpcResponsePayload::Ok { result } => Ok(serde_json::from_value(result).unwrap()),
        JsonRpcResponsePayload::Error { error } => Err(error
            .data
            .unwrap()
            .get("code")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()),
    }
}

fn host_identity() -> RemoteHostIdentity {
    RemoteHostIdentity {
        device_id: "recent-device".into(),
        descriptor: DeviceDescriptor {
            device_name: "Recent test".into(),
            operating_system: "test".into(),
            system_version: "1".into(),
        },
    }
}

fn setup(
    path: &std::path::Path,
    fail_active: bool,
) -> (Arc<PluginManager>, Arc<ProviderGatewayService>) {
    setup_with_forwarding(path, fail_active, true)
}

fn setup_with_forwarding(
    path: &std::path::Path,
    fail_active: bool,
    forward_events: bool,
) -> (Arc<PluginManager>, Arc<ProviderGatewayService>) {
    let mut descriptor = plugin("recent-plugin", &["recent-instance"]);
    if let Ok(count) = std::fs::read_to_string(path.with_extension("ordered")) {
        descriptor.env.insert("CODEPET_FAKE_RECENT_ORDERED".into(), "1".into());
        descriptor.env.insert("CODEPET_FAKE_RECENT_ATTENTION_COUNT".into(), count);
    }
    descriptor
        .env
        .insert("CODEPET_FAKE_RECENT".into(), "1".into());
    descriptor.env.insert(
        "CODEPET_FAKE_RECENT_ACTIVE_FAIL_FILE".into(),
        path.with_extension("fail").to_string_lossy().into_owned(),
    );
    descriptor.env.insert("CODEPET_FAKE_RECENT_REQUESTS".into(), path.with_extension("requests").to_string_lossy().into_owned());
    descriptor.env.insert("CODEPET_FAKE_RECENT_DATED_MODE".into(), path.with_extension("dated").to_string_lossy().into_owned());
    descriptor.env.insert("CODEPET_FAKE_RECENT_EXPIRED".into(), path.with_extension("expired").to_string_lossy().into_owned());
    if fail_active {
        descriptor
            .env
            .insert("CODEPET_FAKE_RECENT_ACTIVE_FAIL".into(), "1".into());
    }
    let manager = build_manager_at("recent-device",vec![descriptor],PluginManagerConfig::default().event_capacity,PluginProcessOptions::default(),Some(path));
    let gateway = Arc::new(
        ProviderGatewayService::with_remote_identity_and_state_path(
            manager.clone(),
            host_identity(),
            path,
        )
        .unwrap(),
    );
    if forward_events { gateway.start_event_forwarding(); }
    (manager, gateway)
}

#[tokio::test]
async fn complete_global_recent_pages_preserve_scope_and_ordinary_list() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conversation-state.sqlite");
    let db=codepet_provider_data::app_state::open_app_state(&path).unwrap();
    db.execute("INSERT INTO providers(id,display_name,created_at,updated_at) VALUES('recent-plugin','Fake recent-plugin',0,0)",[]).unwrap();
    db.execute("INSERT INTO provider_instances VALUES('recent-instance','recent-plugin','fake','Instance recent-instance')",[]).unwrap();
    let store = SharedConversationStateStore::open(&path).unwrap();
    store.ensure_client("reader-a").unwrap();
    // Existing read records predate the one unread change, as in an existing database.
    for id in (0..125)
        .map(|n| format!("active-{n:03}"))
        .chain(std::iter::once("recent".into()))
    {
        let row: gateway::Conversation = serde_json::from_value(serde_json::json!({
            "resource":{"providerId":"recent-instance","nativeResourceId":id},
            "title":"seed", "preview":"fixture conversation", "updatedAt":1,
            "status": if id == "recent" { "idle" } else { "running" },
        }))
        .unwrap();
        store.observe_summary(&row).unwrap();
    }
    let mut old: gateway::Conversation = serde_json::from_value(serde_json::json!({
        "resource":{"providerId":"recent-instance","nativeResourceId":"unread-old"},
        "title":"Fake unread-old", "preview":"before", "status":"idle", "updatedAt":1,
    }))
    .unwrap();
    store.observe_summary(&old).unwrap();
    old.preview = Some("fixture conversation".into());
    store.observe_summary(&old).unwrap();
    store.ensure_client("reader-b").unwrap();
    let (manager, service) = setup(&path, false);
    for (_, result) in manager.start_enabled().await {
        result.unwrap();
    }
    let first = fetch(&service, "reader-a", None).await.unwrap();
    assert_eq!(first.conversations.len(), 20);
    assert!(first
        .conversations
        .iter()
        .all(|row| row.status == gateway::ConversationStatus::Running));
    let first_cursor = first.page_info.next_cursor.clone().unwrap();
    assert_eq!(
        fetch(&service, "reader-b", Some(first_cursor.clone()))
            .await
            .err()
            .unwrap(),
        "invalid_cursor"
    );
    let mut rows = first.conversations;
    let mut cursor = Some(first_cursor);
    while let Some(next) = cursor {
        let page = fetch(&service, "reader-a", Some(next)).await.unwrap();
        assert_eq!(page.revision, first.revision);
        assert_eq!(page.snapshot_cursor, first.snapshot_cursor);
        assert!(page.conversations.len() <= 20);
        rows.extend(page.conversations);
        cursor = page.page_info.next_cursor;
    }
    assert_eq!(rows.len(), 127);
    assert!(rows.iter().all(|row| row.read_state.is_some()));
    assert_eq!(rows[124].resource.native_resource_id, "active-124");
    assert_eq!(rows[125].resource.native_resource_id, "unread-old");
    assert!(rows[125].read_state.as_ref().unwrap().unread);
    assert_eq!(rows[126].resource.native_resource_id, "recent");

    let mut reader_b = Vec::new();
    let mut cursor = None;
    loop {
        let page = fetch(&service, "reader-b", cursor).await.unwrap();
        reader_b.extend(page.conversations);
        cursor = page.page_info.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(reader_b.len(), 126);
    assert!(reader_b
        .iter()
        .all(|row| row.resource.native_resource_id != "unread-old"));
    let before_discovery = fetch(&service, "reader-a", None)
        .await
        .unwrap()
        .page_info
        .next_cursor
        .unwrap();
    let ordinary = GatewayProtocolServer::conversation_list(
        &*service,
        GatewayConversationListRequest {
            provider_id: "recent-instance".into(),
            cursor: None,
            limit: Some(20),
            project_filter: serde_json::from_value(serde_json::json!({"kind":"standalone"}))
                .unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        ordinary.conversations[0].resource.native_resource_id,
        "conversation-list"
    );
    assert!(ordinary.conversations[0].updated_at.unwrap() < 14 * 24 * 60 * 60 * 1_000);
    assert_eq!(
        fetch(&service, "reader-a", Some(before_discovery))
            .await
            .err()
            .unwrap(),
        "recent_cursor_expired"
    );
    let before_read = fetch(&service, "reader-a", None).await.unwrap();
    let old_cursor = before_read.page_info.next_cursor.unwrap();
    // A new activity arrives after the client observed its version. Mark only the
    // observed version and leave the newer activity unread in the shared document.
    let event = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0", "method":"event.turnOutputDelta", "params": {
            "conversation":{"deviceId":"recent-device","providerPluginId":"recent-plugin","providerInstanceId":"recent-instance","nativeResourceId":"unread-old"},
            "turn":{"deviceId":"recent-device","providerPluginId":"recent-plugin","providerInstanceId":"recent-instance","nativeResourceId":"turn-race"},
            "itemId":"item-race", "contentId":"content-race", "kind":"text", "delta":"new output",
        },
    })).unwrap();
    store.observe_provider_event(&event).unwrap();
    let request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0", "id":"2", "method":"conversation.markRead", "params": {
            "conversation":rows[125].resource,
            "observedActivityVersion":rows[125].read_state.as_ref().unwrap().activity_version,
        },
    }))
    .unwrap();
    match service
        .dispatch_for_caller_scope("reader-a", request)
        .await
        .response
    {
        JsonRpcResponsePayload::Ok { result } => {
            let result: gateway::ConversationMarkReadResponse =
                serde_json::from_value(result).unwrap();
            assert!(result.read_state.unread);
        }
        result => panic!("markRead failed: {result:?}"),
    }
    assert_eq!(
        fetch(&service, "reader-a", Some(old_cursor))
            .await
            .err()
            .unwrap(),
        "recent_cursor_expired"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn read_baseline_retry_reuses_completed_id_lookups_without_source_changes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite");
    // Isolate baseline invalidation from legitimate startup capability events.
    // Source-event fencing remains enabled in the other recent integration tests.
    let (manager, service) = setup_with_forwarding(&path, false, false);
    for (_, result) in manager.start_enabled().await { result.unwrap(); }
    let first = fetch(&service, "reader", None).await.unwrap();
    assert_eq!(first.conversations.len(), 20);
    let requests = std::fs::read_to_string(path.with_extension("requests")).unwrap();
    let ids_queries = requests.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|request| request["query"]["kind"] == "ids").collect::<Vec<_>>();
    assert_eq!(ids_queries.len(), 2, "baseline invalidation must not repeat the two ID batches");
    let replay = fetch(&service, "reader", None).await.unwrap();
    assert_eq!(serde_json::to_value(first).unwrap()["conversations"], serde_json::to_value(replay).unwrap()["conversations"]);
    assert_eq!(std::fs::read_to_string(path.with_extension("requests")).unwrap(), requests);
    manager.shutdown().await;
}

#[tokio::test]
async fn ordered_attention_is_paged_without_losing_old_unread_or_scope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite");
    std::fs::write(path.with_extension("ordered"), "1000").unwrap();
    let (manager, service) = setup_with_forwarding(&path, false, false);
    for (_, result) in manager.start_enabled().await { result.unwrap(); }
    let store = SharedConversationStateStore::open(&path).unwrap();
    store.ensure_client("reader").unwrap();
    let mut baseline = (0..1000).map(|i| serde_json::from_value::<gateway::Conversation>(serde_json::json!({
        "resource":{"providerId":"recent-instance","nativeResourceId":format!("active-{i:03}")},
        "title":format!("Fake active-{i:03}"),"preview":"fixture conversation","status":"running","updatedAt":1,
    })).unwrap()).collect::<Vec<_>>();
    baseline.push(serde_json::from_value(serde_json::json!({"resource":{"providerId":"recent-instance","nativeResourceId":"recent"},"title":"Fake recent","preview":"fixture conversation","status":"idle","updatedAt":1})).unwrap());
    for row in baseline.iter_mut().take(20) { row.preview = Some("before".into()); }
    store.observe_and_decorate_summaries("reader", &mut baseline).unwrap();
    let mut old: gateway::Conversation = serde_json::from_value(serde_json::json!({"resource":{"providerId":"recent-instance","nativeResourceId":"unread-old"},"title":"Fake unread-old","preview":"before","status":"idle","updatedAt":1})).unwrap();
    store.observe_summary(&old).unwrap();
    old.preview = Some("fixture conversation".into());
    store.observe_summary(&old).unwrap();
    store.ensure_client("other-reader").unwrap();
    let first = fetch(&service, "reader", None).await.unwrap();
    assert_eq!(first.conversations.len(), 20);
    let requests = std::fs::read_to_string(path.with_extension("requests")).unwrap();
    let first_queries = requests.lines().map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()).collect::<Vec<_>>();
    assert_eq!(first_queries.len(), 1, "1000 active candidates require only 20 objects, including a read-state baseline retry");
    assert_eq!(first_queries[0]["query"]["orderByUpdatedAt"], true);
    assert_eq!(first_queries[0]["limit"], 20);
    let first_cursor = first.page_info.next_cursor.clone().unwrap();
    assert_eq!(fetch(&service,"other-reader",Some(first_cursor.clone())).await.unwrap_err(), "invalid_cursor");
    let second = fetch(&service, "reader", Some(first_cursor.clone())).await.unwrap();
    let reads = std::fs::read_to_string(path.with_extension("requests")).unwrap();
    assert_eq!(serde_json::to_value(&second).unwrap(), serde_json::to_value(fetch(&service, "reader", Some(first_cursor)).await.unwrap()).unwrap());
    assert_eq!(std::fs::read_to_string(path.with_extension("requests")).unwrap(), reads);
    let mut rows = first.conversations;
    rows.extend(second.conversations);
    let mut cursor = second.page_info.next_cursor;
    while let Some(next) = cursor {
        let page = fetch(&service, "reader", Some(next)).await.unwrap();
        rows.extend(page.conversations);
        cursor = page.page_info.next_cursor;
    }
    assert_eq!(rows.len(), 1002);
    assert_eq!(rows[999].resource.native_resource_id, "active-999");
    assert_eq!(rows[1000].resource.native_resource_id, "unread-old");
    assert!(rows[1000].read_state.as_ref().unwrap().unread);
    assert_eq!(rows[1001].resource.native_resource_id, "recent");
    manager.shutdown().await;
}

#[tokio::test]
async fn ordered_attention_expired_source_requires_a_new_recent_first_page() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite");
    std::fs::write(path.with_extension("ordered"), "125").unwrap();
    let (manager, service) = setup_with_forwarding(&path, false, false);
    for (_, result) in manager.start_enabled().await { result.unwrap(); }
    let first = fetch(&service, "reader", None).await.unwrap();
    std::fs::write(path.with_extension("expired"), "1").unwrap();
    assert_eq!(fetch(&service, "reader", first.page_info.next_cursor).await.unwrap_err(), "recent_cursor_expired");
    std::fs::remove_file(path.with_extension("expired")).unwrap();
    assert_eq!(fetch(&service, "reader", None).await.unwrap().conversations.len(), 20);
    manager.shutdown().await;
}

#[tokio::test]
async fn ordered_ids_are_never_sent_to_a_provider_without_the_capability() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite");
    let (manager, _service) = setup_with_forwarding(&path, false, false);
    for (_, result) in manager.start_enabled().await { result.unwrap(); }
    let request = serde_json::from_value(serde_json::json!({
        "route":{"deviceId":"recent-device","providerPluginId":"recent-plugin","providerInstanceId":"recent-instance"},
        "projectFilter":{"kind":"all"},"limit":20,
        "query":{"kind":"ids","ids":["active-000"],"orderByUpdatedAt":true},
    })).unwrap();
    assert_eq!(manager.conversation_list(request).await.unwrap_err().code, "unsupported");
    assert!(!path.with_extension("requests").exists(), "unsupported mode must be rejected before any plugin RPC");
    manager.shutdown().await;
}

#[tokio::test]
async fn atomic_failure_is_not_a_partial_recent_page() {
    let directory = tempfile::tempdir().unwrap();
    let (manager, service) = setup(&directory.path().join("conversation-state.sqlite"), true);
    for (_, result) in manager.start_enabled().await {
        result.unwrap();
    }
    assert_eq!(
        fetch(&service, "reader", None).await.err().unwrap(),
        "conversation_query_incomplete"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn memory_authority_never_advertises_recent() {
    let mut descriptor = plugin("recent-plugin", &["recent-instance"]);
    descriptor
        .env
        .insert("CODEPET_FAKE_RECENT".into(), "1".into());
    let manager = build_manager("recent-device",vec![descriptor]);
    let service = ProviderGatewayService::new(manager.clone()).unwrap();
    for (_, result) in manager.start_enabled().await {
        result.unwrap();
    }
    let described = GatewayProtocolServer::provider_describe(
        &service,
        ProviderDescribeRequest {
            provider_id: "recent-instance".into(),
        },
    )
    .await
    .unwrap();
    assert!(!described
        .capabilities
        .methods
        .contains(&gateway::GatewayCapability::ConversationRecent));
    assert_eq!(
        fetch(&service, "reader", None).await.err().unwrap(),
        "unsupported"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn completeness_failure_invalidates_other_reader_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conversation-state.sqlite");
    let (manager, service) = setup(&path, false);
    for (_, result) in manager.start_enabled().await {
        result.unwrap();
    }
    let first = fetch(&service, "reader-a", None).await.unwrap();
    std::fs::write(path.with_extension("fail"), "fail active query").unwrap();
    assert_eq!(
        fetch(&service, "reader-b", None).await.err().unwrap(),
        "conversation_query_incomplete"
    );
    assert_eq!(
        fetch(&service, "reader-a", first.page_info.next_cursor)
            .await
            .err()
            .unwrap(),
        "recent_cursor_expired"
    );
    manager.shutdown().await;
}


#[tokio::test]
async fn recent_time_window_is_fetched_on_demand_and_replayed_without_reads() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite");
    std::fs::write(path.with_extension("dated"), "1").unwrap();
    let (manager, service) = setup(&path, false);
    for (_, result) in manager.start_enabled().await { result.unwrap(); }
    let first = fetch(&service, "reader", None).await.unwrap();
    assert_eq!(first.conversations.len(), 20);
    assert_eq!(first.conversations[0].resource.native_resource_id, "dated-0000");
    let log = path.with_extension("requests");
    let first_reads = std::fs::read_to_string(&log).unwrap().lines().count();
    assert!(first_reads <= 2, "one page, with at most the initial read-state baseline retry");
    let cursor = first.page_info.next_cursor.unwrap();
    let second = fetch(&service, "reader", Some(cursor.clone())).await.unwrap();
    assert_eq!(second.conversations[0].resource.native_resource_id, "dated-0020");
    assert_eq!(second.conversations.len(), 20);
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), first_reads + 1);
    let replay = fetch(&service, "reader", Some(cursor)).await.unwrap();
    assert_eq!(serde_json::to_value(second).unwrap(), serde_json::to_value(&replay).unwrap());
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), first_reads + 1);
    std::fs::write(path.with_extension("expired"), "1").unwrap();
    assert_eq!(fetch(&service, "reader", replay.page_info.next_cursor).await.unwrap_err(), "recent_cursor_expired");
    std::fs::remove_file(path.with_extension("expired")).unwrap();
    assert!(fetch(&service, "reader", None).await.is_ok());
    manager.shutdown().await;
}
