use codepet_provider_data::{
    app_state::open_app_state, conversation_state::SharedConversationStateStore,
};
use serde_json::json;

fn register(db: &rusqlite::Connection) {
    db.execute(
        "INSERT INTO providers(id,display_name,created_at,updated_at) VALUES('p','Provider',1,1)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO provider_instances VALUES('a','p','local','A')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO provider_instances VALUES('b','p','local','B')",
        [],
    )
    .unwrap();
}
fn row(instance: &str, preview: &str) -> codepet_provider_data::Conversation {
    serde_json::from_value(json!({"resource":{"providerId":instance,"nativeResourceId":"same-id"},"title":"title","preview":preview,"status":"idle"})).unwrap()
}
#[test]
fn schema_separates_instances_reads_and_queue_control_without_document_blobs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("app-state.sqlite");
    let db = open_app_state(&path).unwrap();
    register(&db);
    let columns = db
        .prepare("PRAGMA table_info(conversations)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        columns,
        vec![
            "id",
            "provider_instance_id",
            "native_conversation_id",
            "title",
            "activity_scope_id",
            "activity_version",
            "summary_fingerprint",
            "detail_fingerprint",
            "last_activity_event_key",
            "queue_paused",
            "queue_revision",
            "next_queue_sequence",
            "created_at",
            "updated_at"
        ]
    );
    let store = SharedConversationStateStore::open(&path).unwrap();
    store.ensure_client("phone").unwrap();
    store.ensure_client("desktop").unwrap();
    store.observe_summary(&row("a", "first")).unwrap();
    store.observe_summary(&row("b", "first")).unwrap();
    store.observe_summary(&row("a", "second")).unwrap();
    store
        .mark_read("phone", &row("a", "").resource, "activity-1")
        .unwrap();
    assert!(store.unread("phone", "a").unwrap().is_empty());
    assert_eq!(store.unread("desktop", "a").unwrap().len(), 1);
    assert!(store.unread("desktop", "b").unwrap().is_empty());
    db.execute("UPDATE conversations SET queue_paused=1,queue_revision=4,next_queue_sequence=7 WHERE provider_instance_id='a'",[]).unwrap();
    store.observe_summary(&row("a", "third")).unwrap();
    let control:(i64,i64,i64)=db.query_row("SELECT queue_paused,queue_revision,next_queue_sequence FROM conversations WHERE provider_instance_id='a'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(control, (1, 4, 7));
    assert!(db
        .execute("DELETE FROM provider_instances WHERE id='a'", [])
        .is_err());
    drop(store);
    let store = SharedConversationStateStore::open(&path).unwrap();
    assert_eq!(
        store.unread("phone", "a").unwrap()[0].1.activity_version,
        "activity-2"
    );
    assert!(store.observe_summary(&row("unregistered", "bad")).is_err());
    let clock: i64 = db
        .query_row(
            "SELECT last_version FROM activity_scopes WHERE id='host'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(clock, 2);
    assert!(db
        .prepare("SELECT document FROM conversation_state")
        .is_err());
}
#[test]
fn concurrent_handles_commit_activity_and_read_state_without_lost_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("app-state.sqlite");
    register(&open_app_state(&path).unwrap());
    let store = SharedConversationStateStore::open(&path).unwrap();
    store.ensure_client("reader").unwrap();
    for instance in ["a", "b"] {
        store.observe_summary(&row(instance, "first")).unwrap();
    }
    let threads: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|instance| {
            let path = path.clone();
            std::thread::spawn(move || {
                SharedConversationStateStore::open(path)
                    .unwrap()
                    .observe_summary(&row(instance, "changed"))
                    .unwrap()
            })
        })
        .collect();
    let mut versions: Vec<_> = threads
        .into_iter()
        .map(|t| t.join().unwrap().unwrap())
        .collect();
    versions.sort();
    assert_eq!(versions, vec![1, 2]);
    assert_eq!(store.unread("reader", "a").unwrap().len(), 1);
    assert_eq!(store.unread("reader", "b").unwrap().len(), 1);
}
