use super::*;

fn row(id: &str, updated: u64) -> gateway::Conversation {
    serde_json::from_value(serde_json::json!({
        "resource": {"providerId": "p", "nativeResourceId": id},
        "title": id, "status": "idle", "updatedAt": updated,
    }))
    .unwrap()
}

fn key(scope: &str, generation: u64) -> ViewKey {
    ViewKey {
        provider_id: "p".into(),
        reader_scope: scope.into(),
        generation,
    }
}

#[test]
fn all_attention_groups_share_one_page_budget_and_old_ids_are_included() {
    let now = RECENT_WINDOW_MS * 3;
    let mut summaries = BTreeMap::new();
    let mut active = BTreeSet::new();
    let mut unread = BTreeMap::new();
    for n in 0..125 {
        let active_row = row(&format!("a-{n:03}"), 1);
        active.insert(identity(&active_row));
        summaries.insert(identity(&active_row), active_row);
        let unread_row = row(&format!("u-{n:03}"), 2);
        unread.insert(
            identity(&unread_row),
            gateway::ConversationReadState {
                unread: true,
                activity_version: "activity-9".into(),
            },
        );
        summaries.insert(identity(&unread_row), unread_row);
    }
    unread.insert(
        ("p".into(), "a-000".into()),
        gateway::ConversationReadState {
            unread: true,
            activity_version: "activity-9".into(),
        },
    );
    let recent = row("recent", now);
    summaries.insert(identity(&recent), recent);
    let (rows, boundary) = aggregate(summaries, &active, &unread, now);
    assert_eq!(rows.len(), 251);
    assert_eq!(rows[0].resource.native_resource_id, "a-000");
    assert_eq!(rows[124].resource.native_resource_id, "a-124");
    assert_eq!(rows[125].resource.native_resource_id, "u-000");
    assert_eq!(rows[250].resource.native_resource_id, "recent");
    let mut store = RecentSnapshots::default();
    store
        .install(key("reader", 1), 0, rows, boundary, "event-88".into(), now)
        .unwrap();
    let mut cursor = None;
    let mut received = Vec::new();
    loop {
        let page = store
            .page(&key("reader", 1), cursor.as_deref(), Some(20), now)
            .unwrap()
            .unwrap();
        assert!(page.conversations.len() <= 20);
        assert_eq!(page.snapshot_cursor, "event-88");
        assert_eq!(page.revision, "recent-0");
        received.extend(page.conversations);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(received.len(), 251);
    assert_eq!(received[125].resource.native_resource_id, "u-000");
}

#[test]
fn cutoff_is_inclusive_and_expires_without_provider_activity() {
    let now = RECENT_WINDOW_MS + 100;
    let rows = [
        row("boundary", 100),
        row("too-old", 99),
        row("active-old", 1),
    ];
    let active = BTreeSet::from([identity(&rows[2])]);
    let summaries = rows.into_iter().map(|r| (identity(&r), r)).collect();
    let (rows, boundary) = aggregate(summaries, &active, &BTreeMap::new(), now);
    assert_eq!(rows.len(), 2);
    assert_eq!(boundary, Some(now + 1));
    let mut store = RecentSnapshots::default();
    store
        .install(key("reader", 1), 0, rows, boundary, "event-1".into(), now)
        .unwrap();
    assert!(store.take_invalidations(now).is_empty());
    assert_eq!(
        store.take_invalidations(now + 1),
        vec![("p".into(), "recent-1".into())]
    );
    assert!(store
        .page(&key("reader", 1), None, None, now + 1)
        .unwrap()
        .is_none());
}

#[test]
fn cursors_bind_reader_provider_generation_revision_and_lifetime() {
    let mut store = RecentSnapshots::default();
    let view = key("reader", 1);
    store
        .install(
            view.clone(),
            0,
            vec![row("a", 10), row("b", 9)],
            None,
            "event-4".into(),
            10,
        )
        .unwrap();
    let cursor = store
        .page(&view, None, Some(1), 10)
        .unwrap()
        .unwrap()
        .next_cursor
        .unwrap();
    let mut foreign_provider = view.clone();
    foreign_provider.provider_id = "other".into();
    for wrong in [key("other-reader", 1), key("reader", 2), foreign_provider] {
        assert_eq!(
            store
                .page(&wrong, Some(&cursor), Some(1), 10)
                .err()
                .unwrap()
                .code,
            "invalid_cursor"
        );
    }
    for wrong in ["ordinary-list-offset-1", "tampered-token"] {
        assert_eq!(
            store.page(&view, Some(wrong), None, 10).err().unwrap().code,
            "invalid_cursor"
        );
    }
    assert_eq!(
        store
            .page(&view, Some(&cursor), None, 10 + SNAPSHOT_TTL_MS)
            .err()
            .unwrap()
            .code,
        "recent_cursor_expired"
    );
    store
        .install(
            view.clone(),
            0,
            vec![row("a", 10), row("b", 9)],
            None,
            "event-4".into(),
            20,
        )
        .unwrap();
    let cursor = store
        .page(&view, None, Some(1), 20)
        .unwrap()
        .unwrap()
        .next_cursor
        .unwrap();
    store.invalidate("p");
    assert_eq!(
        store
            .page(&view, Some(&cursor), None, 20)
            .err()
            .unwrap()
            .code,
        "recent_cursor_expired"
    );
}

#[test]
fn mark_read_or_provider_change_during_build_cannot_install_stale_rows() {
    let mut store = RecentSnapshots::default();
    let started = store.epoch("p");
    store.invalidate("p");
    store.invalidate("p");
    assert_eq!(
        store
            .install(
                key("reader", 1),
                started,
                vec![row("unread", 1)],
                None,
                "event-9".into(),
                10
            )
            .err()
            .unwrap()
            .code,
        "recent_snapshot_changed"
    );
    assert_eq!(
        store.take_invalidations(10),
        vec![("p".into(), "recent-2".into())]
    );
    assert!(store.take_invalidations(10).is_empty());
    store
        .install(
            key("reader", 1),
            store.epoch("p"),
            vec![],
            None,
            "event-100".into(),
            10,
        )
        .unwrap();
    let page = store
        .page(&key("reader", 1), None, None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(page.revision, "recent-2");
    assert_eq!(page.snapshot_cursor, "event-100");
}

#[test]
fn dates_descend_then_full_identity_breaks_ties_and_status_is_unchanged() {
    let summaries = [row("b", 10), row("a", 10), row("c", 11)]
        .into_iter()
        .map(|r| (identity(&r), r))
        .collect();
    let (rows, _) = aggregate(summaries, &BTreeSet::new(), &BTreeMap::new(), 11);
    assert_eq!(
        rows.iter()
            .map(|r| r.resource.native_resource_id.as_str())
            .collect::<Vec<_>>(),
        ["c", "a", "b"]
    );
    assert!(rows
        .iter()
        .all(|r| r.status == gateway::ConversationStatus::Idle));
}

#[test]
fn time_invalidation_survives_cursor_snapshot_eviction() {
    let mut store = RecentSnapshots::default();
    let boundary = SNAPSHOT_TTL_MS * 2;
    store
        .install(
            key("reader", 1),
            0,
            vec![row("recent", 1)],
            Some(boundary),
            "event-2".into(),
            1,
        )
        .unwrap();
    assert!(store.take_invalidations(SNAPSHOT_TTL_MS + 2).is_empty());
    assert_eq!(
        store.take_invalidations(boundary),
        vec![("p".into(), "recent-1".into())]
    );
}

#[test]
fn altering_authenticated_offset_is_rejected() {
    let mut store = RecentSnapshots::default();
    let view = key("reader", 1);
    store
        .install(
            view.clone(),
            0,
            vec![row("a", 1), row("b", 1), row("c", 1)],
            None,
            "event-1".into(),
            1,
        )
        .unwrap();
    let cursor = store
        .page(&view, None, Some(1), 1)
        .unwrap()
        .unwrap()
        .next_cursor
        .unwrap();
    let tampered = cursor.replace(".1.", ".2.");
    assert_ne!(cursor, tampered);
    assert_eq!(
        store
            .page(&view, Some(&tampered), Some(1), 1)
            .err()
            .unwrap()
            .code,
        "invalid_cursor"
    );
}

#[test]
fn cached_first_page_gets_a_new_fence_and_each_cursor_keeps_its_own_fence() {
    let mut store = RecentSnapshots::default();
    let view = key("reader", 1);
    store
        .install(
            view.clone(),
            0,
            vec![row("a", 1), row("b", 1)],
            None,
            "event-1".into(),
            1,
        )
        .unwrap();
    let first = store
        .page_with_fence(&view, None, Some(1), 2, Some("event-10".into()))
        .unwrap()
        .unwrap();
    let refreshed = store
        .page_with_fence(&view, None, Some(1), 3, Some("event-20".into()))
        .unwrap()
        .unwrap();
    assert_eq!(first.revision, refreshed.revision);
    assert_eq!(first.snapshot_cursor, "event-10");
    assert_eq!(refreshed.snapshot_cursor, "event-20");
    let tail = store
        .page_with_fence(
            &view,
            first.next_cursor.as_deref(),
            Some(1),
            4,
            Some("event-30".into()),
        )
        .unwrap()
        .unwrap();
    let refreshed_tail = store
        .page(&view, refreshed.next_cursor.as_deref(), Some(1), 4)
        .unwrap()
        .unwrap();
    assert_eq!(tail.snapshot_cursor, "event-10");
    assert_eq!(refreshed_tail.snapshot_cursor, "event-20");
}
