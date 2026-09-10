use codepet_task_lineage::{
    domain::*,
    extraction::*,
    service,
    sources::codex::CodexSource,
    store::Store,
    watch::{self, WatchConfig},
    Result,
};
use serde_json::{json, Value};
use std::path::Path;
const PARENT: &str = "00000000-0000-0000-0000-000000000001";
const CHILD: &str = "00000000-0000-0000-0000-000000000002";
fn time(minute: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::hours(1) + chrono::Duration::minutes(minute))
        .to_rfc3339()
}
fn meta(id: &str, time: &str) -> Value {
    json!({"timestamp":time,"type":"session_meta","payload":{"id":id,"thread_source":"user","cwd":"/repo","timestamp":time}})
}
fn message(role: &str, text: &str, time: &str) -> Value {
    json!({"timestamp":time,"type":"response_item","payload":{"type":"message","role":role,"content":[{"type":"output_text","text":text}]}})
}
fn transfer(name: &str, parent: &str, time: &str) -> Value {
    json!({"timestamp":time,"type":"response_item","payload":{"type":"function_call_output","name":name,"output":format!("<codex_delegation><source_thread_id>{parent}</source_thread_id><input>TOOL TEXT MUST NOT ENTER INFERENCE</input></codex_delegation>")}})
}
fn write(path: &Path, rows: &[Value]) {
    std::fs::write(
        path,
        rows.iter()
            .map(|row| row.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
}
struct Extractor {
    multiple: bool,
}
impl TaskExtractor for Extractor {
    fn version(&self) -> String {
        "fixture".into()
    }
    fn extract(&self, messages: &[Message], candidates: &[Task]) -> Result<ExtractionResult> {
        assert!(messages.iter().all(|message| matches!(
            message.role.as_str(),
            "user" | "assistant"
        ) && !message.text.contains("TOOL TEXT")));
        let mut tasks = vec![];
        for title in if self.multiple {
            vec!["焦点修复", "导出发票"]
        } else {
            vec!["焦点修复"]
        } {
            tasks.push(TaskDelta {
                title: title.into(),
                detail: "验证".into(),
                existing_task_id: candidates.first().map(|task| task.id.clone()),
                episodes: vec![EpisodeDelta {
                    title: "执行".into(),
                    evidence_ids: messages
                        .iter()
                        .map(|message| message.evidence.event_id.clone())
                        .collect(),
                }],
            });
        }
        Ok(ExtractionResult {
            extraction: Extraction { tasks },
            requested_model: "fixture".into(),
            model_usage: json!({}),
            reported_cost_usd: None,
        })
    }
}
fn fixture() -> (tempfile::TempDir, CodexSource, Store) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("codex");
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    (dir, CodexSource { home }, store)
}
#[test]
fn one_user_message_can_contain_two_independent_tasks() {
    let (_dir, source, store) = fixture();
    write(
        &source.home.join("sessions/a.jsonl"),
        &[
            meta(PARENT, &time(0)),
            message("user", "修复焦点，另一个需求是导出发票。", &time(1)),
        ],
    );
    service::scan(&store, &source).unwrap();
    let result = service::extract_next(&store, &Extractor { multiple: true }, None).unwrap();
    assert_eq!(result.tasks.len(), 2);
    assert_ne!(result.tasks[0].id, result.tasks[1].id);
}
#[test]
fn native_delegation_and_return_form_three_episodes_without_extracting_tools() {
    let (_dir, source, store) = fixture();
    write(
        &source.home.join("sessions/parent.jsonl"),
        &[
            meta(PARENT, &time(0)),
            message("user", "修复焦点", &time(1)),
            transfer("send_message_to_thread", CHILD, &time(4)),
            message("assistant", "已整合结果", &time(5)),
        ],
    );
    let mut child = meta(CHILD, &time(2));
    child["payload"]["thread_source"] = json!("agent_created_thread");
    write(
        &source.home.join("sessions/child.jsonl"),
        &[
            child,
            transfer("create_thread", PARENT, &time(2)),
            message("assistant", "焦点修复完成", &time(3)),
        ],
    );
    let view = service::scan(&store, &source).unwrap();
    assert!(view
        .threads
        .iter()
        .find(|thread| thread.id == PARENT)
        .unwrap()
        .parent_id
        .is_none());
    assert_eq!(
        view.threads
            .iter()
            .find(|thread| thread.id == CHILD)
            .unwrap()
            .parent_id
            .as_deref(),
        Some(PARENT)
    );
    service::extract_next(&store, &Extractor { multiple: false }, Some(PARENT)).unwrap();
    let result =
        service::extract_next(&store, &Extractor { multiple: false }, Some(PARENT)).unwrap();
    assert_eq!(result.tasks.len(), 1);
    let task = &result.tasks[0];
    assert_eq!(task.episodes.len(), 3);
    let child = task
        .episodes
        .iter()
        .find(|episode| episode.thread_id == CHILD)
        .unwrap();
    assert!(task.edges.iter().any(|edge| edge.to == child.id));
    assert!(task.edges.iter().any(|edge| edge.from == child.id));
}
#[test]
fn fork_history_is_visible_but_not_reextracted() {
    let (_dir, source, store) = fixture();
    let prefix = vec![
        meta(PARENT, &time(0)),
        message("user", "继承的消息", &time(1)),
    ];
    let bytes = prefix
        .iter()
        .map(|row| row.to_string().len() + 1)
        .sum::<usize>();
    let mut rows = prefix;
    rows.push(message("assistant", "Fork 之后的父线程消息", &time(3)));
    write(&source.home.join("sessions/parent.jsonl"), &rows);
    let mut child = meta(CHILD, &time(2));
    child["payload"]["forked_from_id"] = json!(PARENT);
    child["payload"]["history_base"] = json!({"thread_id":PARENT,"end_byte_offset":bytes});
    write(
        &source.home.join("sessions/child.jsonl"),
        &[child, message("user", "Fork 新增消息", &time(4))],
    );
    service::scan(&store, &source).unwrap();
    assert_eq!(service::messages(&store, CHILD).unwrap().len(), 1);
    let history = service::conversation_messages(&store, CHILD).unwrap();
    assert_eq!(
        history
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["继承的消息", "Fork 新增消息"]
    );
}
#[test]
fn idle_watch_does_not_consume_budget_and_user_pause_wins_over_inflight_result() {
    let (dir, source, store) = fixture();
    write(
        &source.home.join("sessions/parent.jsonl"),
        &[
            meta(PARENT, &time(0)),
            message("user", "修复焦点", &time(1)),
        ],
    );
    watch::configure(
        &store,
        WatchConfig {
            enabled: true,
            extract: true,
            thread_id: Some(PARENT.into()),
            ..WatchConfig::default()
        },
    )
    .unwrap();
    struct Pause {
        root: std::path::PathBuf,
    }
    impl TaskExtractor for Pause {
        fn version(&self) -> String {
            "pause".into()
        }
        fn extract(&self, messages: &[Message], candidates: &[Task]) -> Result<ExtractionResult> {
            let store = Store::open(&self.root)?;
            let mut config = watch::read(&store)?;
            assert_eq!(config.remaining_jobs, 4);
            config.extract = false;
            watch::configure(&store, config)?;
            Extractor { multiple: false }.extract(messages, candidates)
        }
    }
    watch::tick(
        &store,
        &source,
        Some(&Pause {
            root: dir.path().join("store"),
        }),
    )
    .unwrap();
    let mut config = watch::read(&store).unwrap();
    assert!(!config.extract);
    config.extract = true;
    watch::configure(&store, config).unwrap();
    watch::tick(&store, &source, Some(&Extractor { multiple: false })).unwrap();
    assert_eq!(watch::read(&store).unwrap().remaining_jobs, 4);
}

#[test]
fn extraction_skips_old_and_undated_history_but_includes_recent_append() {
    let (_dir, source, store) = fixture();
    let path = source.home.join("sessions/a.jsonl");
    let old = (chrono::Utc::now() - chrono::Duration::hours(49)).to_rfc3339();
    let mut rows = vec![meta(PARENT, &old)];
    for _ in 0..500 {
        rows.push(message("user", "OLD HISTORY", &old));
    }
    rows.push(message("assistant", "UNDATED", "invalid"));
    write(&path, &rows);
    watch::configure(
        &store,
        WatchConfig {
            enabled: true,
            extract: true,
            thread_id: Some(PARENT.into()),
            ..WatchConfig::default()
        },
    )
    .unwrap();
    let view = watch::tick(&store, &source, Some(&Extractor { multiple: false }))
        .unwrap()
        .unwrap();
    assert_eq!(view.pending_messages, 0);
    assert!(view.tasks.is_empty());
    assert_eq!(watch::read(&store).unwrap().remaining_jobs, 5);
    rows.push(message("user", "RECENT REQUEST", &time(1)));
    rows.push(message("assistant", "RECENT RESPONSE", &time(2)));
    write(&path, &rows);
    assert_eq!(service::scan(&store, &source).unwrap().pending_messages, 2);
    let view = service::extract_next(&store, &Extractor { multiple: false }, None).unwrap();
    assert_eq!(view.pending_messages, 0);
    assert_eq!(
        view.tasks[0]
            .episodes
            .iter()
            .map(|e| e.evidence_ids.len())
            .sum::<usize>(),
        2
    );
    assert_eq!(service::messages(&store, PARENT).unwrap().len(), 503);
}
