use code_pet_lib::claude_transcript::{claude_outcome_from_transcript_line, ClaudeTranscriptOutcome};

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
