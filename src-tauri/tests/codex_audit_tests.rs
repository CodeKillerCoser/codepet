use code_pet_lib::codex_audit::{parse_codex_audit_line, replay_recent_codex_audit_events, replay_recent_codex_audit_events_with_session_index};
use code_pet_lib::events::TaskStatus;
use code_pet_lib::state::SharedState;
use serde_json::json;

#[test]
fn parse_codex_audit_line_normalizes_user_prompt_submit() {
    let line = serde_json::to_string(&json!({
        "session_id": "codex-session",
        "turn_id": "codex-turn",
        "cwd": "/workspace/project",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "当前正在运行",
        "_timestamp": "2026-05-26 17:42:37",
        "platform": "codex"
    }))
    .unwrap();

    let event = parse_codex_audit_line(&line).unwrap();

    assert_eq!(event.status, TaskStatus::Thinking);
    assert_eq!(event.session_id.as_deref(), Some("codex-session"));
    assert_eq!(event.cwd.as_deref(), Some("/workspace/project"));
    assert_eq!(event.message, "当前正在运行");
}

#[test]
fn replay_recent_codex_audit_events_imports_only_codex_hook_lines() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    std::fs::write(
        &audit_path,
        [
            "not json".to_string(),
            serde_json::to_string(&json!({
                "platform": "claude",
                "hook_event_name": "UserPromptSubmit",
                "session_id": "claude-session"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "PreToolUse",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "tool_name": "Bash",
                "_timestamp": "2026-05-26 17:43:00"
            }))
            .unwrap(),
        ]
        .join("\n"),
    )
    .unwrap();
    let state = SharedState::default();

    let imported = replay_recent_codex_audit_events(&state, &audit_path, 20).unwrap();

    let events = state.recent_events();
    assert_eq!(imported, 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, TaskStatus::Running);
    assert_eq!(events[0].tool_name.as_deref(), Some("Bash"));
}

#[test]
fn replay_codex_audit_keeps_prompt_title_for_later_tool_events() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    let mut lines = vec![serde_json::to_string(&json!({
        "platform": "codex",
        "hook_event_name": "UserPromptSubmit",
        "session_id": "codex-session",
        "cwd": "/workspace/project",
        "prompt": "排查桌宠刷新",
        "_timestamp": "2026-05-26 17:42:00"
    }))
    .unwrap()];
    for index in 0..150 {
        lines.push(
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "PreToolUse",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "transcript_path": "/Users/wangxin/.codex/sessions/2026/05/26/rollout.jsonl",
                "tool_name": "Bash",
                "_timestamp": format!("2026-05-26 17:43:{:02}", index % 60)
            }))
            .unwrap(),
        );
    }
    std::fs::write(&audit_path, lines.join("\n")).unwrap();
    let state = SharedState::default();

    replay_recent_codex_audit_events(&state, &audit_path, 200).unwrap();

    let latest = state.recent_events().last().unwrap().clone();
    assert_eq!(latest.status, TaskStatus::Running);
    assert_eq!(latest.message, "排查桌宠刷新");
}

#[test]
fn replay_codex_audit_uses_session_index_thread_name_as_title() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    let session_index_path = temp.path().join("session_index.jsonl");
    std::fs::write(
        &audit_path,
        serde_json::to_string(&json!({
            "platform": "codex",
            "hook_event_name": "UserPromptSubmit",
            "session_id": "codex-session",
            "cwd": "/workspace/project",
            "prompt": "先改观测台，任务状态按automation.status判定",
            "_timestamp": "2026-05-26 17:42:00"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &session_index_path,
        serde_json::to_string(&json!({
            "id": "codex-session",
            "thread_name": "评审 agent token统计实现",
            "updated_at": "2026-05-26T09:31:07.654791Z"
        }))
        .unwrap(),
    )
    .unwrap();
    let state = SharedState::default();

    replay_recent_codex_audit_events_with_session_index(&state, &audit_path, &session_index_path, 20).unwrap();

    let latest = state.recent_events().last().unwrap().clone();
    assert_eq!(latest.title, "评审 agent token统计实现");
    assert_eq!(latest.message, "先改观测台，任务状态按automation.status判定");
}
