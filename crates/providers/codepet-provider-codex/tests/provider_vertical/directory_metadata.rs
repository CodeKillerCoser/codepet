use super::*;

#[tokio::test]
async fn start_returns_starting_and_stop_discards_pending_probes() {
    for mode in ["metadata-no-response", "metadata-error"] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("metadata.txt");
        let (sender, events) = mpsc::channel();
        let sink: Arc<dyn ProviderEventSink> = Arc::new(move |event| {
            let _ = sender.send(event);
            Ok(())
        });
        let (provider, route, _) = tokio::time::timeout(
            Duration::from_secs(3),
            configured_direct_provider_with_events(mode, &marker, sink),
        )
        .await
        .unwrap();
        // The fixture records all requests but answers none of the five probes.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if [
                    "model/list",
                    "project/list",
                    "account/read",
                    "account/rateLimits/read",
                    "account/usage/read",
                ]
                .iter()
                .all(|method| !session_pids(&marker, method, "").is_empty())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // Receiving probe requests does not mean their replies have been
        // applied. Only the error mode completes startup; no-response stays pending.
        let snapshot = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let snapshot = ProviderProtocolServer::instance_start(
                    provider.as_ref(),
                    InstanceStartRequest {
                        route: route.clone(),
                    },
                )
                .await
                .unwrap();
                if mode == "metadata-no-response"
                    || snapshot.instance.status == codepet_provider_sdk::InstanceStatus::Ready
                {
                    break snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            snapshot.instance.status,
            if mode == "metadata-no-response" {
                codepet_provider_sdk::InstanceStatus::Starting
            } else {
                codepet_provider_sdk::InstanceStatus::Ready
            }
        );
        assert!(snapshot.instance.authentication.is_none());
        assert!(snapshot.instance.capabilities.turn_send.is_none());
        ProviderProtocolServer::instance_stop(provider.as_ref(), InstanceStopRequest { route })
            .await
            .unwrap();
        let _ = events.try_iter().collect::<Vec<_>>();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!events.try_iter().any(|event| matches!(event, ProtocolEvent::EventInstanceStatusChanged {params,..} if params.instance.status == codepet_provider_sdk::InstanceStatus::Ready)));
        ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn startup_publishes_one_complete_ready_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("catalog-revision.txt");
    let (sender, events) = mpsc::channel();
    let sink: Arc<dyn ProviderEventSink> = Arc::new(move |event| {
        let _ = sender.send(event);
        Ok(())
    });
    let (provider, route, _) =
        configured_direct_provider_with_events("normal", &marker, sink).await;
    let snapshots = tokio::time::timeout(Duration::from_secs(3), async {
        let mut snapshots = Vec::new();
        loop {
            for event in events.try_iter() {
                if let ProtocolEvent::EventInstanceStatusChanged { params, .. } = event {
                    if params.instance.status == codepet_provider_sdk::InstanceStatus::Ready {
                        snapshots.push(params.instance.capabilities);
                    }
                }
            }
            if snapshots.iter().any(|caps| {
                caps.methods
                    .contains(&codepet_provider_sdk::ProviderCapability::ProjectList)
            }) {
                return snapshots;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("catalog change notification");
    assert_eq!(
        snapshots.len(),
        1,
        "publish the complete startup catalog once"
    );
    let discovered = &snapshots[0];
    assert!(discovered.turn_send.is_some());
    assert!(
        !discovered.revision.contains(":catalog:"),
        "keep the existing revision scheme"
    );
    ProviderProtocolServer::instance_stop(provider.as_ref(), InstanceStopRequest { route })
        .await
        .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn union_directory_refreshes_first_page_but_freezes_cursor_membership_and_fields() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("union-marker");
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let source = marker.with_extension("directory.json");
    let native = (0..205)
        .map(|i| json!({"id":format!("native-{i:03}")}))
        .collect::<Vec<_>>();
    std::fs::write(&source, json!({"rows":native}).to_string()).unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER, updated_at INTEGER, history_mode TEXT);
        INSERT INTO threads VALUES ('native-000',0,20,'paginated'),('native-204',1,20,'paginated');").unwrap();
    for i in 0..5 {
        db.execute(
            "INSERT INTO threads VALUES (?1,0,30,'legacy')",
            [format!("db-{i:03}")],
        )
        .unwrap();
    }
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let request = ConversationListRequest {
        route: route.clone(),
        project_filter: all_project_filter(),
        cursor: None,
        limit: Some(100),
        query: None,
        reader_scope: None,
    };
    let first = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(first.conversations.len(), 100);
    assert_eq!(first.conversations[0].resource.native_resource_id, "db-000");
    assert_eq!(first.conversations[0].updated_at, Some(30_000));
    let cursor = first.page_info.next_cursor.clone().unwrap();
    let mut continuation = request.clone();
    continuation.cursor = Some(cursor.clone());
    let second = provider
        .conversation_list(continuation.clone())
        .await
        .unwrap();
    // A new first page must observe committed DB changes. Old cursors must not.
    db.execute_batch("INSERT INTO threads VALUES ('db-new',0,90,'legacy'); UPDATE threads SET archived=1 WHERE id='db-004';").unwrap();
    let mut changed = native.clone();
    for row in &mut changed {
        row["name"] = json!("changed after first page");
    }
    std::fs::write(&source, json!({"rows":changed}).to_string()).unwrap();
    let fresh = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(fresh.conversations[0].resource.native_resource_id, "db-new");
    assert_eq!(fresh.conversations[0].updated_at, Some(90_000));
    let replay = provider
        .conversation_list(continuation.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&second).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    let mut old_rows = first.conversations;
    old_rows.extend(second.conversations);
    continuation.cursor = second.page_info.next_cursor;
    let last = provider
        .conversation_list(continuation.clone())
        .await
        .unwrap();
    assert!(last.page_info.next_cursor.is_none());
    old_rows.extend(last.conversations);
    let ids = old_rows
        .iter()
        .map(|row| row.resource.native_resource_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(old_rows.len(), 209);
    assert_eq!(ids.len(), 209);
    assert!(ids.contains("db-004"));
    assert!(!ids.contains("db-new"));
    assert!(!ids.contains("native-204"));
    assert!(old_rows
        .iter()
        .filter(|row| row.resource.native_resource_id.starts_with("native-"))
        .all(|row| row.title.starts_with("Fixture")));
    // Changing the query or tampering with an offset cannot reuse this cursor.
    continuation.cursor = Some(cursor.clone());
    continuation.query =
        Some(serde_json::from_value(json!({"kind":"updatedAfter","updatedAfter":30000})).unwrap());
    assert_eq!(
        provider
            .conversation_list(continuation.clone())
            .await
            .unwrap_err()
            .code,
        "invalid_cursor"
    );
    continuation.cursor = None;
    let dated = provider.conversation_list(continuation).await.unwrap();
    assert_eq!(dated.conversations.len(), 5);
    assert!(dated
        .conversations
        .iter()
        .all(|row| row.updated_at.unwrap() >= 30000));
    // Existing snapshot pages do not contact failed sources; new queries do.
    std::fs::write(&source, json!({"error":true}).to_string()).unwrap();
    let mut replay_request = request.clone();
    replay_request.cursor = Some(cursor);
    assert!(provider.conversation_list(replay_request).await.is_ok());
    assert!(provider.conversation_list(request.clone()).await.is_err());
    std::fs::write(&source, json!({"rows":native,"cycle":true}).to_string()).unwrap();
    assert_eq!(
        provider
            .conversation_list(request.clone())
            .await
            .unwrap_err()
            .code,
        "conversation_query_incomplete"
    );
    std::fs::write(&source, json!({"rows":native}).to_string()).unwrap();
    db.execute("INSERT INTO threads VALUES ('broken',0,100,'legacy')", [])
        .unwrap();
    assert!(
        provider.conversation_list(request).await.is_err(),
        "unreadable candidate cannot silently disappear"
    );
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn union_directory_applies_desktop_membership_before_project_and_standalone_paging() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("membership-marker");
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({"rows":[]}).to_string(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT, archived INTEGER, updated_at INTEGER, history_mode TEXT);
        INSERT INTO threads VALUES ('mapped',0,30,'legacy'),('unmapped',0,30,'legacy'),('standalone',0,30,'legacy');").unwrap();
    std::fs::write(home.join(".codex-global-state.json"), json!({
        "app-server-project-id-by-legacy-project-id-by-host":{format!("local:{}",home.display()):{"legacy":"project-fixture"}},
        "thread-project-assignments":{
            "mapped":{"projectKind":"local","projectId":"legacy"},
            "unmapped":{"projectKind":"local","projectId":"missing"}
        }
    }).to_string()).unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let mut request: ConversationListRequest = serde_json::from_value(
        json!({"route":route,"projectFilter":{"kind":"standalone"},"limit":1}),
    )
    .unwrap();
    let standalone = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(standalone.conversations.len(), 1);
    assert_eq!(
        standalone.conversations[0].resource.native_resource_id,
        "standalone"
    );
    assert!(standalone.page_info.next_cursor.is_none());
    request.project_filter = serde_json::from_value(
        json!({"kind":"project","project":conversation_resource(&route,"project-fixture")}),
    )
    .unwrap();
    let mapped = provider.conversation_list(request).await.unwrap();
    assert_eq!(mapped.conversations.len(), 1);
    assert_eq!(
        mapped.conversations[0].resource.native_resource_id,
        "mapped"
    );
    assert!(mapped.page_info.next_cursor.is_none());
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn union_directory_resolves_hook_ids_explicitly_without_enumerating_loaded_history() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("event-marker");
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({"rows":[],"loadedUnsupported":true}).to_string(),
    )
    .unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let request = ConversationListRequest {
        route: route.clone(),
        project_filter: all_project_filter(),
        cursor: None,
        limit: Some(20),
        query: Some(serde_json::from_value(json!({"kind":"ids","ids":["event-only"]})).unwrap()),
        reader_scope: None,
    };
    let page = provider.conversation_list(request).await.unwrap();
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(
        page.conversations[0].resource.native_resource_id,
        "event-only"
    );
    assert_eq!(page.conversations[0].title, "event-only");
    assert!(
        !marker.with_extension("directory-requests.jsonl").exists(),
        "ID repair must not enumerate thread/list"
    );
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn union_directory_reads_only_requested_pages_and_preserves_leftovers() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("bounded-marker");
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let native = (0..1000)
        .map(|i| json!({"id":format!("native-{i:04}"),"updatedAt":10000-i*2}))
        .collect::<Vec<_>>();
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({"rows":native}).to_string(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER, updated_at INTEGER, updated_at_ms INTEGER, history_mode TEXT); CREATE INDEX idx_threads_updated_at_ms ON threads(updated_at_ms DESC,id DESC);").unwrap();
    for i in 0..1000 {
        let updated = 9999 - i * 2;
        db.execute(
            "INSERT INTO threads VALUES (?1,0,?2,?3,'legacy')",
            rusqlite::params![format!("db-{i:04}"), updated, updated * 1000],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO threads VALUES ('broken',0,1,1000,'legacy')",
        [],
    )
    .unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let log = marker.with_extension("directory-requests.jsonl");
    // Starting the provider no longer performs directory discovery.
    assert!(!log.exists());
    let mut request = ConversationListRequest {
        route: route.clone(),
        project_filter: all_project_filter(),
        cursor: None,
        limit: Some(20),
        query: None,
        reader_scope: None,
    };
    let first = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(first.conversations.len(), 20);
    let expected = (0..10)
        .flat_map(|i| [format!("native-{i:04}"), format!("db-{i:04}")])
        .collect::<Vec<_>>();
    assert_eq!(
        first
            .conversations
            .iter()
            .map(|row| row.resource.native_resource_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        1,
        "first page must not enumerate 1000 native rows"
    );
    request.cursor = first.page_info.next_cursor;
    let second = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(
        second.conversations[0].resource.native_resource_id,
        "native-0010"
    );
    assert_eq!(second.conversations.len(), 20);
    let before = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        serde_json::to_value(&second).unwrap(),
        serde_json::to_value(provider.conversation_list(request.clone()).await.unwrap()).unwrap()
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), before);
    let token = request.cursor.as_ref().unwrap().rsplit_once(':').unwrap().0;
    request.cursor = Some(format!("{token}:1"));
    assert_eq!(
        provider.conversation_list(request).await.unwrap_err().code,
        "invalid_cursor"
    );
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordered_ids_hydrates_only_the_requested_modern_page_and_replays_it() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("ordered-marker");
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER, updated_at INTEGER, updated_at_ms INTEGER, history_mode TEXT);").unwrap();
    let mut times = serde_json::Map::new();
    for i in 0..1000 {
        let id = format!("ordered-{i:04}");
        db.execute(
            "INSERT INTO threads VALUES(?1,0,?2,?3,'paginated')",
            rusqlite::params![id, 1000 + i, (1000 + i) * 1000 + 123],
        )
        .unwrap();
        times.insert(id, json!(1000 + i));
    }
    let source = marker.with_extension("directory.json");
    std::fs::write(
        &source,
        json!({"rows":[],"metadataUpdatedAt":times}).to_string(),
    )
    .unwrap();
    std::fs::write(home.join(".codex-global-state.json"), json!({
        "app-server-project-id-by-legacy-project-id-by-host":{format!("local:{}",home.display()):{"legacy":"project-fixture"}},
        "thread-project-assignments":{"ordered-0999":{"projectKind":"local","projectId":"legacy"}}
    }).to_string()).unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let mut request = ids_list_request(
        &route,
        (0..1000).map(|i| format!("ordered-{i:04}")).collect(),
    );
    request.limit = Some(20);
    request.query = Some(serde_json::from_value(json!({"kind":"ids","ids":(0..1000).map(|i|format!("ordered-{i:04}")).collect::<Vec<_>>(),"orderByUpdatedAt":true})).unwrap());
    let first = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(
        first.conversations[0].resource.native_resource_id,
        "ordered-0999"
    );
    assert_eq!(first.conversations[0].updated_at, Some(1999123));
    assert_eq!(
        first.conversations[0]
            .project
            .as_ref()
            .unwrap()
            .native_resource_id,
        "project-fixture"
    );
    assert_eq!(
        metadata_read_count(&marker),
        20,
        "1000 rank keys require only 20 native objects for the first page"
    );
    request.cursor = first.page_info.next_cursor;
    let second = provider.conversation_list(request.clone()).await.unwrap();
    assert_eq!(
        second.conversations[0].resource.native_resource_id,
        "ordered-0979"
    );
    assert_eq!(metadata_read_count(&marker), 40);
    let mut wrong_scope = request.clone();
    wrong_scope.reader_scope = Some("other-reader".into());
    assert_eq!(
        provider
            .conversation_list(wrong_scope)
            .await
            .unwrap_err()
            .code,
        "invalid_cursor"
    );
    assert_eq!(
        serde_json::to_value(&second).unwrap(),
        serde_json::to_value(provider.conversation_list(request.clone()).await.unwrap()).unwrap()
    );
    assert_eq!(metadata_read_count(&marker), 40);
    // An unvisited object's native time changed beyond the frozen rank: expire
    // rather than silently returning a misplaced object or skipping it forever.
    times.insert("ordered-0959".into(), json!(3000));
    std::fs::write(
        &source,
        json!({"rows":[],"metadataUpdatedAt":times}).to_string(),
    )
    .unwrap();
    request.cursor = second.page_info.next_cursor;
    assert_eq!(
        provider
            .conversation_list(request.clone())
            .await
            .unwrap_err()
            .code,
        "invalid_cursor"
    );
    db.execute(
        "UPDATE threads SET updated_at=3000,updated_at_ms=3000123 WHERE id='ordered-0959'",
        [],
    )
    .unwrap();
    request.cursor = None;
    assert_eq!(
        provider
            .conversation_list(request)
            .await
            .unwrap()
            .conversations[0]
            .resource
            .native_resource_id,
        "ordered-0959"
    );
    assert!(
        !marker.with_extension("directory-requests.jsonl").exists(),
        "explicit IDs must not enumerate native history"
    );
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordered_ids_certifies_legacy_and_unpersisted_candidates_natively() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("ordered-legacy-marker");
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER, updated_at INTEGER, history_mode TEXT);
        INSERT INTO threads VALUES('legacy-repaired',0,40,'legacy'),('legacy-corrected',0,90,'legacy');").unwrap();
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({"rows":[],"metadataDelayMs":0,"metadataUpdatedAt":{"legacy-corrected":11}})
            .to_string(),
    )
    .unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let mut request = ids_list_request(&route, vec![]);
    request.query = Some(serde_json::from_value(json!({"kind":"ids","ids":["legacy-repaired","legacy-corrected","event-only","missing-000"],"orderByUpdatedAt":true})).unwrap());
    let page = provider.conversation_list(request).await.unwrap();
    assert_eq!(
        page.conversations
            .iter()
            .map(|row| row.resource.native_resource_id.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy-repaired", "legacy-corrected", "event-only"]
    );
    assert_eq!(page.conversations[0].updated_at, Some(40000));
    assert_eq!(
        page.conversations[1].updated_at,
        Some(11000),
        "a valid native correction must not be replaced by max(DB,native)"
    );
    assert_eq!(metadata_read_count(&marker), 4);
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordered_rank_mismatch_does_not_rescan_missing_native_threads() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("rank-mismatch");
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER, updated_at INTEGER, updated_at_ms INTEGER, history_mode TEXT);
        INSERT INTO threads VALUES('first',0,40,40000,'paginated'),('second',0,30,30000,'paginated')").unwrap();
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({
            "rows":[], "metadataUpdatedAt":{"first":10,"second":30,"event-only":12}
        })
        .to_string(),
    )
    .unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let mut ids: Vec<_> = (0..128).map(|i| format!("missing-{i:03}")).collect();
    ids.extend(["first", "second", "event-only"].map(str::to_owned));
    let mut request = ids_list_request(&route, vec![]);
    request.limit = Some(1);
    request.query = Some(
        serde_json::from_value(json!({"kind":"ids","ids":ids,"orderByUpdatedAt":true})).unwrap(),
    );
    let page = provider.conversation_list(request).await.unwrap();
    assert_eq!(page.conversations[0].resource.native_resource_id, "second");
    assert_eq!(metadata_read_count(&marker), ids.len());
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn union_directory_metadata_reads_are_bounded_shared_and_not_cached() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("parallel-marker");
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({"rows":[],"metadataGate":true}).to_string(),
    )
    .unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let request = ids_list_request(
        &route,
        (0..100).map(|i| format!("parallel-{i:03}")).collect(),
    );
    let first = tokio::spawn({
        let provider = provider.clone();
        let request = request.clone();
        async move { provider.conversation_list(request).await }
    });
    wait_for_metadata_reads(&marker, 8).await;
    let second = tokio::spawn({
        let provider = provider.clone();
        let request = request.clone();
        async move { provider.conversation_list(request).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        metadata_read_count(&marker),
        8,
        "both queries share the same eight in-flight native reads"
    );
    // A control request uses the same native process while metadata is blocked.
    let projects = tokio::time::timeout(
        Duration::from_secs(2),
        provider.project_list(ProjectListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(20),
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!projects.projects.is_empty());
    std::fs::write(marker.with_extension("metadata-release"), "1").unwrap();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first.conversations.len(), 100);
    assert_eq!(
        serde_json::to_value(first).unwrap()["conversations"],
        serde_json::to_value(second).unwrap()["conversations"]
    );
    assert_eq!(
        metadata_read_count(&marker),
        100,
        "overlapping queries must not duplicate native reads"
    );
    assert_eq!(
        provider
            .conversation_list(request)
            .await
            .unwrap()
            .conversations
            .len(),
        100
    );
    assert_eq!(
        metadata_read_count(&marker),
        200,
        "fresh queries must observe external metadata changes"
    );
    let missing = ids_list_request(&route, vec!["missing-000".into()]);
    assert!(provider
        .conversation_list(missing.clone())
        .await
        .unwrap()
        .conversations
        .is_empty());
    assert!(provider
        .conversation_list(missing)
        .await
        .unwrap()
        .conversations
        .is_empty());
    assert_eq!(
        metadata_read_count(&marker),
        202,
        "absence must not become a persistent negative cache"
    );
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn union_directory_cancellation_keeps_capacity_until_native_reads_finish() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("cancel-marker");
    std::fs::write(
        marker.with_extension("directory.json"),
        json!({"rows":[],"metadataGate":true}).to_string(),
    )
    .unwrap();
    let (provider, route, _) = configured_direct_provider("union-directory", &marker).await;
    let first = tokio::spawn({
        let provider = provider.clone();
        let request =
            ids_list_request(&route, (0..100).map(|i| format!("cancel-{i:03}")).collect());
        async move { provider.conversation_list(request).await }
    });
    wait_for_metadata_reads(&marker, 8).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let second = tokio::spawn({
        let provider = provider.clone();
        let request = ids_list_request(&route, vec!["after-cancel".into()]);
        async move { provider.conversation_list(request).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        metadata_read_count(&marker),
        8,
        "cancellation cannot free slots still used by native workers"
    );
    std::fs::write(marker.with_extension("metadata-release"), "1").unwrap();
    assert_eq!(second.await.unwrap().unwrap().conversations.len(), 1);
    assert_eq!(metadata_read_count(&marker), 9);
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
}
