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
    let mut descriptor = plugin("recent-plugin", &["recent-instance"]);
    descriptor
        .env
        .insert("CODEPET_FAKE_RECENT".into(), "1".into());
    descriptor.env.insert(
        "CODEPET_FAKE_RECENT_ACTIVE_FAIL_FILE".into(),
        path.with_extension("fail").to_string_lossy().into_owned(),
    );
    if fail_active {
        descriptor
            .env
            .insert("CODEPET_FAKE_RECENT_ACTIVE_FAIL".into(), "1".into());
    }
    let manager = build_manager("recent-device", vec![descriptor]);
    let gateway = Arc::new(
        ProviderGatewayService::with_remote_identity_and_state_path(
            manager.clone(),
            host_identity(),
            path,
        )
        .unwrap(),
    );
    gateway.start_event_forwarding();
    (manager, gateway)
}

#[tokio::test]
async fn complete_global_recent_pages_preserve_scope_and_ordinary_list() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conversation-state.sqlite");
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
    let manager = build_manager("recent-device", vec![descriptor]);
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
