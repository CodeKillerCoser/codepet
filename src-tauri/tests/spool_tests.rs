use code_pet_lib::agents::{AgentId, AgentView};
use code_pet_lib::collector::{replay_spooled_events, spool_path_for_settings};
use code_pet_lib::events::TaskStatus;
use code_pet_lib::settings::AppSettings;
use code_pet_lib::state::SharedState;
use serde_json::json;

#[test]
fn spool_path_keeps_legacy_default_until_app_data_directory_is_customized() {
    let settings = AppSettings::default();
    let spool_path = spool_path_for_settings(&settings);

    assert!(spool_path.ends_with(std::path::Path::new(".code-pet").join("spool").join("events.jsonl")));
}

#[test]
fn spool_path_follows_custom_app_data_directory() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.data.data_directory = Some(temp.path().join("code-pet-data").to_string_lossy().to_string());

    assert_eq!(spool_path_for_settings(&settings), temp.path().join("code-pet-data").join("spool").join("events.jsonl"));
}

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

#[test]
fn replay_spooled_events_skips_disabled_agents() {
    let temp = tempfile::tempdir().unwrap();
    let spool_path = temp.path().join("events.jsonl");
    std::fs::write(
        &spool_path,
        serde_json::to_string(&json!({
            "agent": "qoder",
            "payload": {
                "hook_event_name": "UserPromptSubmit",
                "session_id": "qoder-session",
                "cwd": "/workspace/project",
                "message": "关闭后不应出现"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let state = SharedState::default();
    state.set_agents(vec![agent_view(AgentId::Qoder, false)]);

    let imported = replay_spooled_events(&state, &spool_path).unwrap();

    assert_eq!(imported, 0);
    assert!(state.recent_events().is_empty());
    assert!(!spool_path.exists());
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
