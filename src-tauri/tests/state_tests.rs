use code_pet_lib::events::{ActivitySource, AgentId, PetEvent, PetEventKind, TaskStatus};
use code_pet_lib::state::{ApprovalBehavior, ApprovalDecision, SharedState};
use chrono::Utc;
use serde_json::json;
use std::time::Duration;

fn permission_event(id: &str) -> PetEvent {
    PetEvent {
        id: id.to_string(),
        provider: AgentId::Claude,
        kind: PetEventKind::PermissionRequested,
        status: TaskStatus::WaitingApproval,
        title: "等待授权".to_string(),
        message: "Bash 需要授权".to_string(),
        session_id: Some("session-1".to_string()),
        cwd: Some("/tmp/project".to_string()),
        tool_name: Some("Bash".to_string()),
        should_ring: true,
        created_at: Utc::now(),
        raw: json!({}),
        source: Some(ActivitySource {
            pid: Some(1234),
            ppid: Some(1200),
            terminal_program: Some("iTerm.app".to_string()),
            term_session_id: Some("w0t1p0".to_string()),
            tty_path: Some("/dev/ttys007".to_string()),
            tmux_pane: Some("%7".to_string()),
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.googlecode.iterm2".to_string()),
        }),
    }
}

#[test]
fn recent_events_strips_raw_payload_but_state_keeps_full_event() {
    let state = SharedState::default();
    let mut event = permission_event("raw-heavy");
    event.raw = json!({
        "tool_response": "x".repeat(1024),
        "transcript_path": "/tmp/session.jsonl"
    });

    state.push_event(event);

    let frontend_event = state.recent_events().pop().unwrap();
    let stored_event = state.event_by_id("raw-heavy").unwrap();
    assert!(frontend_event.raw.is_null());
    assert_eq!(stored_event.raw["tool_response"].as_str().unwrap().len(), 1024);
}

#[test]
fn recent_events_returns_a_bounded_frontend_snapshot() {
    let state = SharedState::default();
    for index in 0..130 {
        state.push_event(PetEvent {
            id: format!("event-{index}"),
            raw: json!({ "tool_response": "x".repeat(4096) }),
            ..permission_event(&format!("event-{index}"))
        });
    }

    let frontend_events = state.recent_events();
    assert_eq!(frontend_events.len(), 120);
    assert_eq!(frontend_events.first().unwrap().id, "event-10");
    assert!(frontend_events.iter().all(|event| event.raw.is_null()));
    assert_eq!(
        state
            .event_by_id("event-0")
            .unwrap()
            .raw["tool_response"]
            .as_str()
            .unwrap()
            .len(),
        4096,
    );
}

#[tokio::test]
async fn approval_waiter_resolves_when_user_decides() {
    let state = SharedState::default();
    state.push_event(permission_event("approval-1"));

    let waiter = {
        let state = state.clone();
        tokio::spawn(async move { state.wait_for_approval("approval-1", Duration::from_secs(1)).await })
    };

    state.resolve_approval(
        "approval-1",
        ApprovalDecision {
            behavior: ApprovalBehavior::Allow,
            message: None,
        },
    );

    let decision = waiter.await.unwrap().expect("decision should resolve");
    assert_eq!(decision.behavior, ApprovalBehavior::Allow);
}

#[tokio::test]
async fn approval_waiter_times_out_without_decision() {
    let state = SharedState::default();
    state.push_event(permission_event("approval-2"));

    let decision = state
        .wait_for_approval("approval-2", Duration::from_millis(1))
        .await;

    assert!(decision.is_none());
}
