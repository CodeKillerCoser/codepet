use chrono::Utc;
use code_pet_lib::activity_actions::{activation_strategy_for_event, activation_target_for_event, approval_strategy_for_event, reply_strategy_for_event, resolve_approval_for_event, ActivationStrategy, ActivationTarget, ApprovalStrategy, ReplyStrategy};
use code_pet_lib::events::{ActivitySource, AgentId, PetEvent, PetEventKind, TaskStatus};
use code_pet_lib::state::{ApprovalBehavior, ApprovalDecision, SharedState};
use serde_json::json;

fn event(provider: AgentId, source: Option<ActivitySource>) -> PetEvent {
    PetEvent {
        id: "event-1".to_string(),
        provider,
        kind: PetEventKind::TaskUpdated,
        status: TaskStatus::Running,
        title: "任务".to_string(),
        message: "正在执行".to_string(),
        session_id: Some("session-1".to_string()),
        cwd: Some("/tmp/project".to_string()),
        tool_name: None,
        should_ring: false,
        created_at: Utc::now(),
        raw: json!({}),
        source,
    }
}

#[test]
#[cfg(target_os = "macos")]
fn activation_prefers_source_bundle_id() {
    let target = activation_target_for_event(&event(
        AgentId::Claude,
        Some(ActivitySource {
            pid: Some(1234),
            ppid: None,
            terminal_program: Some("iTerm.app".to_string()),
            term_session_id: None,
            tty_path: None,
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.googlecode.iterm2".to_string()),
        }),
    ));

    assert_eq!(target, ActivationTarget::BundleId("com.googlecode.iterm2".to_string()));
}

#[test]
#[cfg(target_os = "macos")]
fn activation_targets_live_terminal_session_by_tty() {
    let terminal_event = event(
        AgentId::Claude,
        Some(ActivitySource {
            pid: Some(1234),
            ppid: None,
            terminal_program: Some("Apple_Terminal".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys018".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.apple.Terminal".to_string()),
        }),
    );
    let iterm_event = event(
        AgentId::Qoder,
        Some(ActivitySource {
            pid: Some(2234),
            ppid: None,
            terminal_program: Some("iTerm.app".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys019".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.googlecode.iterm2".to_string()),
        }),
    );

    assert_eq!(
        activation_strategy_for_event(&terminal_event),
        ActivationStrategy::TerminalSession("/dev/ttys018".to_string())
    );
    assert_eq!(
        activation_strategy_for_event(&iterm_event),
        ActivationStrategy::ITermSession("/dev/ttys019".to_string())
    );
}

#[test]
fn activation_falls_back_to_provider_app_or_cwd() {
    let mut codex_without_thread = event(AgentId::Codex, None);
    codex_without_thread.session_id = None;

    assert_eq!(
        activation_target_for_event(&codex_without_thread),
        ActivationTarget::AppName("Codex".to_string())
    );
    assert_eq!(
        activation_target_for_event(&event(AgentId::Claude, None)),
        ActivationTarget::Path("/tmp/project".to_string())
    );
    assert_eq!(
        activation_target_for_event(&event(AgentId::Cursor, None)),
        ActivationTarget::AppName("Cursor".to_string())
    );
    assert_eq!(
        activation_target_for_event(&event(AgentId::Qoder, None)),
        ActivationTarget::AppName("Qoder".to_string())
    );
}

#[test]
#[cfg(not(target_os = "macos"))]
fn activation_uses_cross_platform_fallback_when_macos_source_metadata_is_present() {
    let claude_event = event(
        AgentId::Claude,
        Some(ActivitySource {
            pid: Some(1234),
            ppid: None,
            terminal_program: Some("Apple_Terminal".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys018".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.apple.Terminal".to_string()),
        }),
    );
    let qoder_event = event(
        AgentId::Qoder,
        Some(ActivitySource {
            pid: None,
            ppid: None,
            terminal_program: Some("WarpTerminal".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys018".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("dev.warp.Warp-Stable".to_string()),
        }),
    );

    assert_eq!(
        activation_target_for_event(&claude_event),
        ActivationTarget::Path("/tmp/project".to_string())
    );
    assert_eq!(
        activation_strategy_for_event(&claude_event),
        ActivationStrategy::Target(ActivationTarget::Path("/tmp/project".to_string()))
    );
    assert_eq!(
        activation_target_for_event(&qoder_event),
        ActivationTarget::AppName("Qoder".to_string())
    );
}

#[test]
fn activation_uses_codex_thread_deeplink_when_session_id_available() {
    let mut codex_event = event(AgentId::Codex, None);
    codex_event.session_id = Some("019e8862-0d6c-7150-823f-18d4cd4e2813".to_string());

    assert_eq!(
        activation_target_for_event(&codex_event),
        ActivationTarget::Url(
            "codex://threads/019e8862-0d6c-7150-823f-18d4cd4e2813".to_string()
        )
    );
}

#[test]
fn reply_strategy_does_not_send_qoder_messages_without_verified_existing_session_api() {
    let mut qoder_event = event(
        AgentId::Qoder,
        Some(ActivitySource {
            pid: None,
            ppid: None,
            terminal_program: Some("Apple_Terminal".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys018".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.apple.Terminal".to_string()),
        }),
    );
    qoder_event.status = TaskStatus::Done;
    let strategy = reply_strategy_for_event(&qoder_event);

    assert_eq!(strategy, ReplyStrategy::Unsupported);
}

#[test]
fn reply_strategy_does_not_support_claude_terminal_automation() {
    let strategy = reply_strategy_for_event(&event(
        AgentId::Claude,
        Some(ActivitySource {
            pid: None,
            ppid: None,
            terminal_program: Some("Apple_Terminal".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys018".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("com.apple.Terminal".to_string()),
        }),
    ));

    assert_eq!(strategy, ReplyStrategy::Unsupported);
}

#[test]
#[cfg(target_os = "macos")]
fn activation_targets_warp_by_bundle_id() {
    let target = activation_target_for_event(&event(
        AgentId::Qoder,
        Some(ActivitySource {
            pid: None,
            ppid: None,
            terminal_program: Some("WarpTerminal".to_string()),
            term_session_id: None,
            tty_path: Some("/dev/ttys018".to_string()),
            tmux_pane: None,
            wezterm_pane: None,
            kitty_window_id: None,
            app_bundle_id: Some("dev.warp.Warp-Stable".to_string()),
        }),
    ));

    assert_eq!(target, ActivationTarget::BundleId("dev.warp.Warp-Stable".to_string()));
}

#[test]
fn reply_strategy_uses_codex_app_server_for_desktop_threads() {
    let mut codex_event = event(AgentId::Codex, None);
    codex_event.status = TaskStatus::Done;

    assert_eq!(
        reply_strategy_for_event(&codex_event),
        ReplyStrategy::CodexAppServer
    );
}

#[test]
fn reply_strategy_rejects_running_events() {
    let mut codex_event = event(AgentId::Codex, None);
    codex_event.status = TaskStatus::Running;
    let mut qoder_event = event(AgentId::Qoder, None);
    qoder_event.status = TaskStatus::Running;

    assert_eq!(reply_strategy_for_event(&codex_event), ReplyStrategy::Unsupported);
    assert_eq!(reply_strategy_for_event(&qoder_event), ReplyStrategy::Unsupported);
}

#[test]
fn reply_strategy_requires_codex_thread_id() {
    let mut codex_event = event(AgentId::Codex, None);
    codex_event.status = TaskStatus::Done;
    codex_event.session_id = None;

    assert_eq!(
        reply_strategy_for_event(&codex_event),
        ReplyStrategy::Unsupported
    );
}

#[test]
fn reply_strategy_requires_qoder_session_id() {
    let mut qoder_event = event(AgentId::Qoder, None);
    qoder_event.status = TaskStatus::Done;
    qoder_event.session_id = None;

    assert_eq!(
        reply_strategy_for_event(&qoder_event),
        ReplyStrategy::Unsupported
    );
}

#[test]
fn approval_strategy_uses_collector_wait_for_waiting_approval_events() {
    let mut codex_event = event(AgentId::Codex, None);
    codex_event.status = TaskStatus::WaitingApproval;
    let mut qoder_event = event(AgentId::Qoder, None);
    qoder_event.status = TaskStatus::WaitingApproval;

    assert_eq!(approval_strategy_for_event(&codex_event), ApprovalStrategy::CollectorWait);
    assert_eq!(approval_strategy_for_event(&qoder_event), ApprovalStrategy::CollectorWait);
}

#[test]
fn approval_strategy_rejects_non_approval_events() {
    assert_eq!(
        approval_strategy_for_event(&event(AgentId::Codex, None)),
        ApprovalStrategy::Unsupported
    );
}

#[test]
fn approval_driver_can_resolve_from_pending_event_snapshot() {
    let state = SharedState::default();
    let mut approval = event(AgentId::Codex, None);
    approval.id = "approval-driver".to_string();
    approval.status = TaskStatus::WaitingApproval;
    state.push_event(approval.clone());

    resolve_approval_for_event(
        &state,
        &approval,
        ApprovalDecision {
            behavior: ApprovalBehavior::Allow,
            message: None,
        },
    )
    .unwrap();
}

#[test]
fn reply_strategy_requires_a_targetable_terminal_session() {
    assert_eq!(
        reply_strategy_for_event(&event(
            AgentId::Claude,
            Some(ActivitySource {
                pid: None,
                ppid: None,
                terminal_program: Some("Apple_Terminal".to_string()),
                term_session_id: None,
                tty_path: None,
                tmux_pane: None,
                wezterm_pane: None,
                kitty_window_id: None,
                app_bundle_id: Some("com.apple.Terminal".to_string()),
            }),
        )),
        ReplyStrategy::Unsupported
    );
}
