use code_pet_lib::codex_audit::{parse_codex_audit_line, replay_recent_codex_audit_events, replay_recent_codex_audit_events_with_session_index};
use code_pet_lib::agents::{AgentView, AgentId};
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
fn replay_codex_audit_skips_events_when_codex_is_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    std::fs::write(
        &audit_path,
        serde_json::to_string(&json!({
            "platform": "codex",
            "hook_event_name": "UserPromptSubmit",
            "session_id": "codex-session",
            "cwd": "/workspace/project",
            "prompt": "关闭后不应出现",
            "_timestamp": "2026-05-26 17:43:00"
        }))
        .unwrap(),
    )
    .unwrap();
    let state = SharedState::default();
    state.set_agents(vec![agent_view(AgentId::Codex, false)]);

    let imported = replay_recent_codex_audit_events(&state, &audit_path, 20).unwrap();

    assert_eq!(imported, 0);
    assert!(state.recent_events().is_empty());
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
fn replay_codex_audit_keeps_prompt_title_for_later_windows_tool_events() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    std::fs::write(
        &audit_path,
        [
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "UserPromptSubmit",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "prompt": "继续修复标题",
                "_timestamp": "2026-05-26 17:42:00"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "PreToolUse",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "transcript_path": r"\\?\E:\.codex\sessions\2026\06\06\rollout.jsonl",
                "tool_name": "Bash",
                "_timestamp": "2026-05-26 17:43:00"
            }))
            .unwrap(),
        ]
        .join("\n"),
    )
    .unwrap();
    let state = SharedState::default();

    replay_recent_codex_audit_events(&state, &audit_path, 20).unwrap();

    let latest = state.recent_events().last().unwrap().clone();
    assert_eq!(latest.status, TaskStatus::Running);
    assert_eq!(latest.message, "继续修复标题");
}

#[test]
fn replay_codex_audit_does_not_store_session_start_as_title() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    std::fs::write(
        &audit_path,
        [
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "SessionStart",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "_timestamp": "2026-05-26 17:41:59"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "UserPromptSubmit",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "prompt": "当前活跃的对话列表上还有一个SessionStart，这是从哪里来的？",
                "_timestamp": "2026-05-26 17:42:00"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "PreToolUse",
                "session_id": "codex-session",
                "cwd": "/workspace/project",
                "transcript_path": "/Users/wangxin/.codex/sessions/2026/05/26/rollout.jsonl",
                "tool_name": "Bash",
                "_timestamp": "2026-05-26 17:43:00"
            }))
            .unwrap(),
        ]
        .join("\n"),
    )
    .unwrap();
    let state = SharedState::default();

    replay_recent_codex_audit_events(&state, &audit_path, 20).unwrap();

    let events = state.recent_events();
    assert_eq!(events[1].title, "任务开始");
    assert_eq!(
        events[1].message,
        "当前活跃的对话列表上还有一个SessionStart，这是从哪里来的？"
    );
    assert_eq!(
        events[2].message,
        "当前活跃的对话列表上还有一个SessionStart，这是从哪里来的？"
    );
}

#[test]
fn replay_codex_audit_skips_internal_suggestion_session_after_marker() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    std::fs::write(
        &audit_path,
        [
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "SessionStart",
                "session_id": "suggestion-session",
                "cwd": "/workspace/project",
                "_timestamp": "2026-05-26 17:41:59"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "UserPromptSubmit",
                "session_id": "suggestion-session",
                "cwd": "/workspace/project",
                "prompt": "# Overview\n\nGenerate 0 to 3 hyperpersonalized suggestions for what this user can do with Codex in this local project: /workspace/project\n\nRecent Codex threads in this project:\n- 评审 agent token统计实现\n\n# Response format\nEach suggestion must include: title, description, prompt, appId",
                "_timestamp": "2026-05-26 17:42:00"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "PreToolUse",
                "session_id": "suggestion-session",
                "cwd": "/workspace/project",
                "tool_name": "Bash",
                "_timestamp": "2026-05-26 17:43:00"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "platform": "codex",
                "hook_event_name": "Stop",
                "session_id": "suggestion-session",
                "cwd": "/workspace/project",
                "_timestamp": "2026-05-26 17:44:00"
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
    assert_eq!(events[0].message, "SessionStart");
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

#[test]
fn replay_codex_audit_marks_escalated_tool_call_without_output_as_waiting_approval() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    let transcript_path = temp.path().join("rollout.jsonl");
    std::fs::write(
        &transcript_path,
        serde_json::to_string(&json!({
            "timestamp": "2026-05-28T06:27:02.015Z",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "exec_command",
                "call_id": "call-approval",
                "arguments": serde_json::to_string(&json!({
                    "cmd": "python3 organize_downloads.py --apply",
                    "sandbox_permissions": "require_escalated",
                    "justification": "是否允许移动 Downloads 文件？"
                })).unwrap()
            }
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &audit_path,
        serde_json::to_string(&json!({
            "platform": "codex",
            "hook_event_name": "PreToolUse",
            "session_id": "codex-session",
            "transcript_path": transcript_path,
            "tool_name": "Bash",
            "tool_input": {
                "command": "python3 organize_downloads.py --apply"
            },
            "tool_use_id": "call-approval",
            "_timestamp": "2026-05-28 14:27:02"
        }))
        .unwrap(),
    )
    .unwrap();
    let state = SharedState::default();

    replay_recent_codex_audit_events(&state, &audit_path, 20).unwrap();

    let latest = state.recent_events().last().unwrap().clone();
    assert_eq!(latest.status, TaskStatus::WaitingApproval);
}

fn agent_view(id: AgentId, enabled: bool) -> AgentView {
    AgentView {
        id,
        name: id.as_str().to_string(),
        description: String::new(),
        enabled,
        config_path: String::new(),
        hook_events: Vec::new(),
        selected_hook_events: Vec::new(),
    }
}
