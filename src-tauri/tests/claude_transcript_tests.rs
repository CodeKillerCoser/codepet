use code_pet_lib::agents::AgentId;
use code_pet_lib::claude_transcript::{claude_outcome_from_transcript_line, event_from_claude_outcome, ClaudeTranscriptOutcome};
use code_pet_lib::events::{PetEvent, PetEventKind, TaskStatus};
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;
use tempfile::tempdir;

fn fallback_event(overrides: serde_json::Value) -> PetEvent {
    PetEvent {
        id: "fallback-event".to_string(),
        provider: AgentId::Claude,
        kind: PetEventKind::TaskStarted,
        status: TaskStatus::Thinking,
        title: overrides.get("title").and_then(|value| value.as_str()).unwrap_or("任务开始").to_string(),
        message: overrides.get("message").and_then(|value| value.as_str()).unwrap_or("SessionStart").to_string(),
        session_id: Some(overrides.get("sessionId").and_then(|value| value.as_str()).unwrap_or("session-1").to_string()),
        cwd: Some("/tmp/project".to_string()),
        tool_name: None,
        should_ring: false,
        created_at: Utc::now(),
        raw: overrides.get("raw").cloned().unwrap_or(Value::Null),
        source: None,
    }
}

#[test]
fn api_error_transcript_line_becomes_failed_outcome() {
    let line = r#"{"type":"assistant","sessionId":"cffbb162-4680-43e7-bb7a-b356c9db3ad1","cwd":"/Users/wangxin/Developer/Work/wukong-studio","message":{"role":"assistant","content":[{"type":"text","text":"API Error: 400 Team API AKday消费金额已达上限"}],"stop_reason":"stop_sequence"},"isApiErrorMessage":true,"apiErrorStatus":400}"#;

    let outcome = claude_outcome_from_transcript_line(line).unwrap();

    assert_eq!(outcome, ClaudeTranscriptOutcome::Failed {
        session_id: "cffbb162-4680-43e7-bb7a-b356c9db3ad1".to_string(),
        cwd: Some("/Users/wangxin/Developer/Work/wukong-studio".to_string()),
        message: "API Error: 400 Team API AKday消费金额已达上限".to_string(),
    });
}

#[test]
fn normal_end_turn_transcript_line_becomes_done_outcome() {
    let line = r#"{"type":"assistant","sessionId":"session-1","cwd":"/tmp/project","message":{"role":"assistant","content":[{"type":"text","text":"已完成标题解析和窗口验证。"}],"stop_reason":"end_turn"},"isApiErrorMessage":false}"#;

    let outcome = claude_outcome_from_transcript_line(line).unwrap();

    assert_eq!(outcome, ClaudeTranscriptOutcome::Done {
        session_id: "session-1".to_string(),
        cwd: Some("/tmp/project".to_string()),
        message: "已完成标题解析和窗口验证。".to_string(),
    });
}

#[test]
fn tool_use_assistant_line_is_not_a_terminal_outcome() {
    let line = r#"{"type":"assistant","sessionId":"session-1","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}],"stop_reason":"tool_use"}}"#;

    assert!(claude_outcome_from_transcript_line(line).is_none());
}

#[test]
fn claude_done_title_does_not_use_transcript_path_fallback_message() {
    let fallback = fallback_event(json!({
        "message": "/Users/wangxin/.claude/projects/-Users-wangxin/session-1.jsonl",
        "raw": {
            "transcript_path": "/Users/wangxin/.claude/projects/-Users-wangxin/session-1.jsonl"
        }
    }));
    let event = event_from_claude_outcome(&fallback, ClaudeTranscriptOutcome::Done {
        session_id: "session-1".to_string(),
        cwd: Some("/tmp/project".to_string()),
        message: "我没有获取实时天气数据的能力。".to_string(),
    })
    .unwrap();

    assert_eq!(event.title, "任务完成");
}

#[test]
fn claude_done_title_does_not_use_windows_transcript_path_fallback_message() {
    let fallback = fallback_event(json!({
        "message": r"C:\Users\wangxin\.claude\projects\-Users-wangxin\session-1.jsonl",
        "raw": {
            "transcript_path": r"C:\Users\wangxin\.claude\projects\-Users-wangxin\session-1.jsonl"
        }
    }));
    let event = event_from_claude_outcome(&fallback, ClaudeTranscriptOutcome::Done {
        session_id: "session-1".to_string(),
        cwd: Some("/tmp/project".to_string()),
        message: "我没有获取实时天气数据的能力。".to_string(),
    })
    .unwrap();

    assert_eq!(event.title, "任务完成");
}

#[test]
fn claude_done_title_uses_first_user_message_from_transcript() {
    let dir = tempdir().unwrap();
    let transcript = dir.path().join("session-1.jsonl");
    fs::write(
        &transcript,
        r#"{"type":"user","sessionId":"session-1","message":{"role":"user","content":"今天天气怎么样"}}
{"type":"assistant","sessionId":"session-1","message":{"role":"assistant","content":[{"type":"text","text":"我没有获取实时天气数据的能力。"}],"stop_reason":"end_turn"}}"#,
    )
    .unwrap();

    let transcript_path = transcript.to_string_lossy().to_string();
    let fallback = fallback_event(json!({
        "message": transcript_path,
        "raw": {
            "transcript_path": transcript_path
        }
    }));
    let event = event_from_claude_outcome(&fallback, ClaudeTranscriptOutcome::Done {
        session_id: "session-1".to_string(),
        cwd: Some("/tmp/project".to_string()),
        message: "我没有获取实时天气数据的能力。".to_string(),
    })
    .unwrap();

    assert_eq!(event.title, "今天天气怎么样");
}
