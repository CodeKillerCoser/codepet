use code_pet_lib::events::{normalize_hook_payload, AgentId, PetEventKind, TaskStatus};
use serde_json::json;

#[test]
fn permission_request_becomes_waiting_status_and_alert() {
    let payload = json!({
        "hook_event_name": "PermissionRequest",
        "session_id": "session-1",
        "cwd": "/tmp/project",
        "tool_name": "Bash",
        "message": "Approve shell command?"
    });

    let event = normalize_hook_payload(AgentId::Claude, payload).unwrap();

    assert_eq!(event.kind, PetEventKind::PermissionRequested);
    assert_eq!(event.status, TaskStatus::WaitingApproval);
    assert!(event.should_ring);
    assert_eq!(event.session_id.as_deref(), Some("session-1"));
    assert_eq!(event.cwd.as_deref(), Some("/tmp/project"));
}

#[test]
fn post_tool_failure_becomes_failed_status_and_alert() {
    let payload = json!({
        "hook_event_name": "PostToolUseFailure",
        "session_id": "session-2",
        "tool_name": "Read",
        "error": "file not found"
    });

    let event = normalize_hook_payload(AgentId::Qoder, payload).unwrap();

    assert_eq!(event.kind, PetEventKind::TaskFailed);
    assert_eq!(event.status, TaskStatus::Failed);
    assert!(event.should_ring);
    assert!(event.message.contains("file not found"));
}

#[test]
fn codex_stop_hook_payload_becomes_completed_update() {
    let payload = json!({
        "hook_event_name": "Stop",
        "message": "turn-ended",
        "cwd": "/tmp/codex"
    });

    let event = normalize_hook_payload(AgentId::Codex, payload).unwrap();

    assert_eq!(event.kind, PetEventKind::TaskCompleted);
    assert_eq!(event.status, TaskStatus::Done);
    assert!(event.should_ring);
    assert_eq!(event.provider, AgentId::Codex);
}

#[test]
fn codex_stop_uses_last_assistant_message_as_completion_summary() {
    let payload = json!({
        "hook_event_name": "Stop",
        "last_assistant_message": "已修复并跑完真实验证。",
        "cwd": "/tmp/codex"
    });

    let event = normalize_hook_payload(AgentId::Codex, payload).unwrap();

    assert_eq!(event.status, TaskStatus::Done);
    assert_eq!(event.message, "已修复并跑完真实验证。");
}

#[test]
fn stop_prefers_completion_summary_over_transcript_path() {
    let payload = json!({
        "hook_event_name": "Stop",
        "transcript_path": "/Users/wangxin/.claude/projects/session.jsonl",
        "last_assistant_message": "已完成标题解析和窗口验证。",
        "cwd": "/tmp/claude"
    });

    let event = normalize_hook_payload(AgentId::Claude, payload).unwrap();

    assert_eq!(event.status, TaskStatus::Done);
    assert_eq!(event.message, "已完成标题解析和窗口验证。");
}

#[test]
fn stop_with_api_error_message_becomes_failed_status() {
    let payload = json!({
        "hook_event_name": "Stop",
        "session_id": "session-api-error",
        "message": "API Error: 400 Team API AKday消费金额已达上限"
    });

    let event = normalize_hook_payload(AgentId::Claude, payload).unwrap();

    assert_eq!(event.kind, PetEventKind::TaskFailed);
    assert_eq!(event.status, TaskStatus::Failed);
    assert!(event.should_ring);
    assert_eq!(event.message, "API Error: 400 Team API AKday消费金额已达上限");
}

#[test]
fn qoder_payload_name_can_be_used_as_task_title() {
    let payload = json!({
        "hook_event_name": "Stop",
        "session_id": "qoder-1",
        "parent_business_info": {
            "name": "无法获取实时天气信息"
        },
        "last_assistant_message": "没有可用实时天气工具。"
    });

    let event = normalize_hook_payload(AgentId::Qoder, payload).unwrap();

    assert_eq!(event.title, "无法获取实时天气信息");
    assert_eq!(event.message, "没有可用实时天气工具。");
}

#[test]
fn hook_source_context_is_preserved_for_window_activation() {
    let payload = json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": "session-1",
        "cwd": "/tmp/project",
        "prompt": "继续实现授权",
        "code_pet": {
            "pid": 1234,
            "ppid": 1200,
            "terminalProgram": "iTerm.app",
            "termSessionId": "w0t1p0",
            "ttyPath": "/dev/ttys007",
            "tmuxPane": "%7",
            "appBundleId": "com.googlecode.iterm2"
        }
    });

    let event = normalize_hook_payload(AgentId::Claude, payload).unwrap();

    let source = event.source.expect("source context should be captured");
    assert_eq!(source.pid, Some(1234));
    assert_eq!(source.ppid, Some(1200));
    assert_eq!(source.terminal_program.as_deref(), Some("iTerm.app"));
    assert_eq!(source.term_session_id.as_deref(), Some("w0t1p0"));
    assert_eq!(source.tty_path.as_deref(), Some("/dev/ttys007"));
    assert_eq!(source.tmux_pane.as_deref(), Some("%7"));
    assert_eq!(source.app_bundle_id.as_deref(), Some("com.googlecode.iterm2"));
}

#[test]
fn tool_event_uses_bash_command_as_message() {
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "session_id": "session-1",
        "tool_name": "Bash",
        "tool_input": {
            "command": "npm run build",
            "description": "构建前端资源"
        }
    });

    let event = normalize_hook_payload(AgentId::Claude, payload).unwrap();

    assert_eq!(event.message, "构建前端资源 · npm run build");
}

#[test]
fn tool_event_uses_file_path_as_message_when_present() {
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "session_id": "session-1",
        "tool_name": "Read",
        "input": {
            "file_path": "/Users/wangxin/Documents/code-pet/src/styles.css"
        }
    });

    let event = normalize_hook_payload(AgentId::Claude, payload).unwrap();

    assert_eq!(event.message, "Read · src/styles.css");
}
