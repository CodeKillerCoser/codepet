use crate::agents::AgentId;
use crate::events::PetEvent;
use crate::state::{ApprovalDecision, SharedState};
#[cfg(target_os = "macos")]
use std::io::Write;
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivationTarget {
    BundleId(String),
    AppName(String),
    Path(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivationStrategy {
    Target(ActivationTarget),
    TerminalSession(String),
    ITermSession(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyStrategy {
    CodexAppServer,
    Terminal,
    ITerm,
    AccessibilityPaste,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalStrategy {
    CollectorWait,
    Unsupported,
}

trait AgentInteractionDriver {
    fn reply_strategy(&self, event: &PetEvent) -> ReplyStrategy;

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        let _ = event;
        ApprovalStrategy::Unsupported
    }

    fn send_reply(&self, event: &PetEvent, message: &str) -> Result<(), String> {
        match self.reply_strategy(event) {
            ReplyStrategy::CodexAppServer => crate::codex_app_server::send_reply(event, message),
            ReplyStrategy::Terminal => send_terminal_reply(event, message),
            ReplyStrategy::ITerm => send_iterm_reply(event, message),
            ReplyStrategy::AccessibilityPaste => {
                activate_event(event)?;
                paste_and_submit(message)
            }
            ReplyStrategy::Unsupported => Err("当前来源不支持可靠回复，请打开原会话输入".to_string()),
        }
    }

    fn resolve_approval(
        &self,
        state: &SharedState,
        event: &PetEvent,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        match self.approval_strategy(event) {
            ApprovalStrategy::CollectorWait => {
                if state.resolve_approval(&event.id, decision) {
                    Ok(())
                } else {
                    Err(format!("approval not found: {}", event.id))
                }
            }
            ApprovalStrategy::Unsupported => Err("当前来源不支持审批".to_string()),
        }
    }
}

#[derive(Clone, Copy)]
struct CodexRemoteDriver;

#[derive(Clone, Copy)]
struct QoderDriver;

#[derive(Clone, Copy)]
struct DefaultDriver;

enum AgentInteraction {
    CodexRemote(CodexRemoteDriver),
    Qoder(QoderDriver),
    Default(DefaultDriver),
}

impl AgentInteractionDriver for CodexRemoteDriver {
    fn reply_strategy(&self, event: &PetEvent) -> ReplyStrategy {
        if event
            .session_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            ReplyStrategy::CodexAppServer
        } else {
            ReplyStrategy::Unsupported
        }
    }

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        collector_approval_strategy(event)
    }
}

impl AgentInteractionDriver for QoderDriver {
    fn reply_strategy(&self, event: &PetEvent) -> ReplyStrategy {
        terminal_reply_strategy(event)
    }

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        collector_approval_strategy(event)
    }
}

impl AgentInteractionDriver for DefaultDriver {
    fn reply_strategy(&self, _event: &PetEvent) -> ReplyStrategy {
        ReplyStrategy::Unsupported
    }

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        collector_approval_strategy(event)
    }
}

impl AgentInteractionDriver for AgentInteraction {
    fn reply_strategy(&self, event: &PetEvent) -> ReplyStrategy {
        match self {
            Self::CodexRemote(driver) => driver.reply_strategy(event),
            Self::Qoder(driver) => driver.reply_strategy(event),
            Self::Default(driver) => driver.reply_strategy(event),
        }
    }

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        match self {
            Self::CodexRemote(driver) => driver.approval_strategy(event),
            Self::Qoder(driver) => driver.approval_strategy(event),
            Self::Default(driver) => driver.approval_strategy(event),
        }
    }

    fn send_reply(&self, event: &PetEvent, message: &str) -> Result<(), String> {
        match self {
            Self::CodexRemote(driver) => driver.send_reply(event, message),
            Self::Qoder(driver) => driver.send_reply(event, message),
            Self::Default(driver) => driver.send_reply(event, message),
        }
    }

    fn resolve_approval(
        &self,
        state: &SharedState,
        event: &PetEvent,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        match self {
            Self::CodexRemote(driver) => driver.resolve_approval(state, event, decision),
            Self::Qoder(driver) => driver.resolve_approval(state, event, decision),
            Self::Default(driver) => driver.resolve_approval(state, event, decision),
        }
    }
}

pub fn activation_strategy_for_event(event: &PetEvent) -> ActivationStrategy {
    #[cfg(target_os = "macos")]
    match event
        .source
        .as_ref()
        .and_then(|source| source.terminal_program.as_deref())
    {
        Some("Apple_Terminal" | "Terminal" | "Terminal.app") => {
            if let Some(tty) = source_tty(event) {
                return ActivationStrategy::TerminalSession(tty.to_string());
            }
        }
        Some("iTerm.app" | "iTerm2" | "iTerm2.app") => {
            if let Some(tty) = source_tty(event) {
                return ActivationStrategy::ITermSession(tty.to_string());
            }
        }
        _ => {}
    }

    ActivationStrategy::Target(activation_target_for_event(event))
}

pub fn activation_target_for_event(event: &PetEvent) -> ActivationTarget {
    #[cfg(target_os = "macos")]
    if let Some(source) = &event.source {
        if let Some(bundle_id) = source.app_bundle_id.as_ref().filter(|value| !value.is_empty()) {
            return ActivationTarget::BundleId(bundle_id.clone());
        }
        if let Some(program) = source.terminal_program.as_deref() {
            if let Some(target) = target_for_terminal_program(program) {
                return target;
            }
        }
    }

    match event.provider {
        AgentId::Codex => ActivationTarget::AppName("Codex".to_string()),
        AgentId::Qoder => ActivationTarget::AppName("Qoder".to_string()),
        AgentId::Cursor => ActivationTarget::AppName("Cursor".to_string()),
        AgentId::Claude => event
            .cwd
            .clone()
            .map(ActivationTarget::Path)
            .unwrap_or_else(|| ActivationTarget::AppName("Terminal".to_string())),
    }
}

pub fn reply_strategy_for_event(event: &PetEvent) -> ReplyStrategy {
    interaction_for_event(event).reply_strategy(event)
}

pub fn approval_strategy_for_event(event: &PetEvent) -> ApprovalStrategy {
    interaction_for_event(event).approval_strategy(event)
}

fn terminal_reply_strategy(event: &PetEvent) -> ReplyStrategy {
    #[cfg(target_os = "macos")]
    match event
        .source
        .as_ref()
        .and_then(|source| source.terminal_program.as_deref())
    {
        Some("Apple_Terminal" | "Terminal" | "Terminal.app") if source_tty(event).is_some() => ReplyStrategy::Terminal,
        Some("iTerm.app" | "iTerm2" | "iTerm2.app") if source_tty(event).is_some() => ReplyStrategy::ITerm,
        _ => ReplyStrategy::Unsupported,
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = event;
        ReplyStrategy::Unsupported
    }
}

pub fn activate_event(event: &PetEvent) -> Result<(), String> {
    match activation_strategy_for_event(event) {
        ActivationStrategy::Target(target) => activate_target(&target),
        ActivationStrategy::TerminalSession(tty) => activate_terminal_session(&tty),
        ActivationStrategy::ITermSession(tty) => activate_iterm_session(&tty),
    }
}

pub fn send_reply_to_event(event: &PetEvent, message: &str) -> Result<(), String> {
    interaction_for_event(event).send_reply(event, message)
}

pub fn resolve_approval_for_event(
    state: &SharedState,
    event: &PetEvent,
    decision: ApprovalDecision,
) -> Result<(), String> {
    interaction_for_event(event).resolve_approval(state, event, decision)
}

fn interaction_for_event(event: &PetEvent) -> AgentInteraction {
    match event.provider {
        AgentId::Codex => AgentInteraction::CodexRemote(CodexRemoteDriver),
        AgentId::Qoder => AgentInteraction::Qoder(QoderDriver),
        AgentId::Claude | AgentId::Cursor => AgentInteraction::Default(DefaultDriver),
    }
}

fn collector_approval_strategy(event: &PetEvent) -> ApprovalStrategy {
    if event.status == crate::events::TaskStatus::WaitingApproval {
        ApprovalStrategy::CollectorWait
    } else {
        ApprovalStrategy::Unsupported
    }
}

#[cfg(target_os = "macos")]
fn target_for_terminal_program(program: &str) -> Option<ActivationTarget> {
    match program {
        "Apple_Terminal" | "Terminal" | "Terminal.app" => {
            Some(ActivationTarget::BundleId("com.apple.Terminal".to_string()))
        }
        "iTerm.app" | "iTerm2" | "iTerm2.app" => {
            Some(ActivationTarget::BundleId("com.googlecode.iterm2".to_string()))
        }
        "WarpTerminal" | "Warp" | "Warp.app" => {
            Some(ActivationTarget::BundleId("dev.warp.Warp-Stable".to_string()))
        }
        "WezTerm" | "WezTerm.app" => Some(ActivationTarget::AppName("WezTerm".to_string())),
        "kitty" | "kitty.app" => Some(ActivationTarget::AppName("kitty".to_string())),
        "vscode" | "Visual Studio Code" => Some(ActivationTarget::AppName("Visual Studio Code".to_string())),
        _ => None,
    }
}

fn activate_target(target: &ActivationTarget) -> Result<(), String> {
    match target {
        #[cfg(target_os = "macos")]
        ActivationTarget::BundleId(bundle_id) => run_command("open", &["-b", bundle_id]),
        #[cfg(not(target_os = "macos"))]
        ActivationTarget::BundleId(_) => Err("当前平台不支持按 macOS bundle id 打开应用".to_string()),
        #[cfg(target_os = "macos")]
        ActivationTarget::AppName(app_name) => run_osascript(&[
            format!("tell application \"{}\" to activate", escape_applescript(app_name)),
        ]),
        #[cfg(not(target_os = "macos"))]
        ActivationTarget::AppName(_) => Err("当前平台不支持按应用名称激活会话".to_string()),
        ActivationTarget::Path(path) => open::that_detached(path).map_err(|error| error.to_string()),
    }
}

fn activate_terminal_session(tty: &str) -> Result<(), String> {
    let script = format!(
        r#"tell application "Terminal"
  activate
  repeat with terminalWindow in windows
    repeat with terminalTab in tabs of terminalWindow
      if tty of terminalTab is "{}" then
        set selected tab of terminalWindow to terminalTab
        set index of terminalWindow to 1
        return
      end if
    end repeat
  end repeat
end tell
error "Terminal tab not found for tty {}""#,
        escape_applescript(tty),
        escape_applescript(tty)
    );
    run_osascript(&[script])
}

fn activate_iterm_session(tty: &str) -> Result<(), String> {
    let script = format!(
        r#"tell application "iTerm2"
  activate
  repeat with terminalWindow in windows
    repeat with terminalTab in tabs of terminalWindow
      repeat with terminalSession in sessions of terminalTab
        if tty of terminalSession is "{}" then
          select terminalWindow
          select terminalTab
          select terminalSession
          return
        end if
      end repeat
    end repeat
  end repeat
end tell
error "iTerm session not found for tty {}""#,
        escape_applescript(tty),
        escape_applescript(tty)
    );
    run_osascript(&[script])
}

fn paste_and_submit(message: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return paste_and_submit_macos(message);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = message;
        Err("当前平台不支持自动粘贴回复".to_string())
    }
}

#[cfg(target_os = "macos")]
fn paste_and_submit_macos(message: &str) -> Result<(), String> {
    let previous_clipboard = read_clipboard().unwrap_or_default();
    write_clipboard(message)?;
    std::thread::sleep(std::time::Duration::from_millis(80));
    let send_result = run_osascript(&[
        "tell application \"System Events\" to keystroke \"v\" using command down".to_string(),
        "tell application \"System Events\" to key code 36".to_string(),
    ]);
    std::thread::sleep(std::time::Duration::from_millis(80));
    let _ = write_clipboard(&previous_clipboard);
    send_result
}

#[cfg(target_os = "macos")]
fn read_clipboard() -> Result<String, String> {
    let output = Command::new("pbpaste").output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(target_os = "macos")]
fn write_clipboard(value: &str) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(value.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn send_terminal_reply(event: &PetEvent, message: &str) -> Result<(), String> {
    if let Some(tty) = source_tty(event) {
        let script = format!(
            r#"tell application "Terminal"
  activate
  repeat with terminalWindow in windows
    repeat with terminalTab in tabs of terminalWindow
      if tty of terminalTab is "{}" then
        set selected tab of terminalWindow to terminalTab
        set index of terminalWindow to 1
        return
      end if
    end repeat
  end repeat
end tell
error "Terminal tab not found for tty {}""#,
            escape_applescript(tty),
            escape_applescript(tty)
        );
        run_osascript(&[script])?;
        return paste_and_submit(message);
    }

    run_osascript(&[
        "tell application \"Terminal\" to activate".to_string(),
    ])?;
    paste_and_submit(message)
}

fn send_iterm_reply(event: &PetEvent, message: &str) -> Result<(), String> {
    if let Some(tty) = source_tty(event) {
        let script = format!(
            r#"tell application "iTerm2"
  activate
  repeat with terminalWindow in windows
    repeat with terminalTab in tabs of terminalWindow
      repeat with terminalSession in sessions of terminalTab
        if tty of terminalSession is "{}" then
          tell terminalSession to write text "{}"
          return
        end if
      end repeat
    end repeat
  end repeat
end tell
error "iTerm session not found for tty {}""#,
            escape_applescript(tty),
            escape_applescript(message),
            escape_applescript(tty)
        );
        return run_osascript(&[script]);
    }

    run_osascript(&[
        "tell application \"iTerm2\" to activate".to_string(),
        format!(
            "tell application \"iTerm2\" to tell current session of current window to write text \"{}\"",
            escape_applescript(message)
        ),
    ])
}

fn source_tty(event: &PetEvent) -> Option<&str> {
    event
        .source
        .as_ref()
        .and_then(|source| source.tty_path.as_deref())
        .filter(|value| value.starts_with("/dev/tty"))
}

fn run_osascript(lines: &[String]) -> Result<(), String> {
    let mut command = Command::new("osascript");
    for line in lines {
        command.arg("-e").arg(line);
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(target_os = "macos")]
fn run_command(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
