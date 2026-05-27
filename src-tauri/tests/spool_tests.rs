use code_pet_lib::collector::replay_spooled_events;
use code_pet_lib::events::TaskStatus;
use code_pet_lib::state::SharedState;
use serde_json::json;

#[test]
fn replay_spooled_events_imports_hook_events_and_clears_file() {
    let temp = tempfile::tempdir().unwrap();
    let spool_path = temp.path().join("events.jsonl");
    std::fs::write(
        &spool_path,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&json!({
                "agent": "codex",
                "payload": {
                    "hook_event_name": "UserPromptSubmit",
                    "session_id": "spooled-session",
                    "cwd": "/workspace/project",
                    "message": "spooled prompt"
                },
                "spooledAt": "2026-05-26T05:43:30.484Z"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "agent": "claude",
                "payload": {
                    "hook_event_name": "PermissionRequest",
                    "session_id": "spooled-approval",
                    "cwd": "/workspace/project",
                    "tool_name": "Bash"
                },
                "spooledAt": "2026-05-26T05:44:30.484Z"
            }))
            .unwrap(),
        ),
    )
    .unwrap();
    let state = SharedState::default();

    let imported = replay_spooled_events(&state, &spool_path).unwrap();

    let events = state.recent_events();
    assert_eq!(imported, 2);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].message, "spooled prompt");
    assert_eq!(events[0].created_at.to_rfc3339(), "2026-05-26T05:43:30.484+00:00");
    assert_eq!(events[1].status, TaskStatus::WaitingApproval);
    assert!(!spool_path.exists());
}
