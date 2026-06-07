use crate::activity_actions::{
    collector_approval_strategy, has_session_id, is_replyable_event, ActivationStrategy,
    ActivationTarget, AgentInteractionDriver, ApprovalStrategy, ReplyStrategy,
};
use crate::app_log;
use crate::events::PetEvent;
use serde_json::{json, Value};
use std::env;
#[cfg(windows)]
use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexReplyAction {
    StartTurn,
    SteerTurn(String),
}

#[derive(Clone, Copy)]
pub(crate) struct CodexAppServerManager;

impl AgentInteractionDriver for CodexAppServerManager {
    fn activation_strategy(&self, event: &PetEvent) -> ActivationStrategy {
        if let Some(thread_id) = event
            .session_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            ActivationStrategy::Target(ActivationTarget::Url(codex_thread_deeplink(thread_id)))
        } else {
            crate::activity_actions::default_activation_strategy_for_event(event)
        }
    }

    fn reply_strategy(&self, event: &PetEvent) -> ReplyStrategy {
        if is_replyable_event(event) && has_session_id(event) {
            ReplyStrategy::CodexAppServer
        } else {
            ReplyStrategy::Unsupported
        }
    }

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        collector_approval_strategy(event)
    }

    fn send_reply(&self, event: &PetEvent, message: &str) -> Result<(), String> {
        send_reply(event, message)
    }
}

pub fn send_reply(event: &PetEvent, message: &str) -> Result<(), String> {
    let thread_id = event
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "codex reply requires a thread id".to_string())?;
    let mut client = CodexAppServerClient::spawn()?;
    client.initialize()?;
    let resume = client.request("thread/resume", json!({ "threadId": thread_id }))?;
    let action = reply_action_for_resume_response(&resume)?;
    let input = text_input(message);
    match action {
        CodexReplyAction::SteerTurn(turn_id) => {
            app_log::info(
                "codex_app_server",
                &format!("reply action=turn/steer thread_id={thread_id} turn_id={turn_id}"),
            );
            client.request(
                "turn/steer",
                json!({
                    "threadId": thread_id,
                    "input": input,
                    "expectedTurnId": turn_id,
                }),
            )?;
        }
        CodexReplyAction::StartTurn => {
            app_log::info(
                "codex_app_server",
                &format!("reply action=turn/start thread_id={thread_id}"),
            );
            client.request(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": input,
                }),
            )?;
        }
    }
    client.wait_for_turn_completion(thread_id)?;
    refresh_codex_thread_view(thread_id);
    Ok(())
}

pub fn reply_action_for_resume_response(resume: &Value) -> Result<CodexReplyAction, String> {
    let thread = resume
        .get("thread")
        .ok_or_else(|| "codex app-server resume response missing thread".to_string())?;
    let Some(turns) = thread.get("turns").and_then(Value::as_array) else {
        return Ok(CodexReplyAction::StartTurn);
    };
    let Some(last_turn) = turns.last() else {
        return Ok(CodexReplyAction::StartTurn);
    };
    if last_turn.get("status").and_then(Value::as_str) == Some("inProgress") {
        let turn_id = last_turn
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "codex active turn is missing id".to_string())?;
        return Ok(CodexReplyAction::SteerTurn(turn_id.to_string()));
    }
    Ok(CodexReplyAction::StartTurn)
}

fn text_input(message: &str) -> Value {
    json!([{ "type": "text", "text": message, "text_elements": [] }])
}

struct CodexAppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    turn_completion: Option<Value>,
}

impl CodexAppServerClient {
    fn spawn() -> Result<Self, String> {
        let binary = codex_binary();
        app_log::info(
            "codex_app_server",
            &format!(
                "starting codex app-server binary={} args=app-server --listen stdio://",
                binary.display()
            ),
        );
        let mut child = Command::new(binary)
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to start codex app-server: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open codex app-server stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open codex app-server stdout".to_string())?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            turn_completion: None,
        })
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "code-pet",
                    "title": "Code Pet",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false,
                    "optOutNotificationMethods": [],
                },
            }),
        )?;
        self.notify("initialized", json!({}))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        self.read_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_json(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn write_json(&mut self, value: Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.stdin, &value).map_err(|error| error.to_string())?;
        self.stdin
            .write_all(b"\n")
            .map_err(|error| error.to_string())?;
        self.stdin.flush().map_err(|error| error.to_string())
    }

    fn read_response(&mut self, id: u64) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("failed to read codex app-server response: {error}"))?;
            if read == 0 {
                return Err("codex app-server closed before replying".to_string());
            }
            let value: Value = serde_json::from_str(line.trim()).map_err(|error| {
                format!(
                    "invalid codex app-server json response: {error}; line={}",
                    line.trim()
                )
            })?;
            if is_turn_completed_notification(&value) {
                self.turn_completion = Some(value.clone());
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("codex app-server {id} error: {error}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn wait_for_turn_completion(&mut self, thread_id: &str) -> Result<(), String> {
        if let Some(completion) = self.turn_completion.take() {
            return self.validate_turn_completion(thread_id, &completion);
        }

        let mut line = String::new();
        loop {
            line.clear();
            let read = self.stdout.read_line(&mut line).map_err(|error| {
                format!("failed to read codex app-server turn completion: {error}")
            })?;
            if read == 0 {
                return Err("codex app-server closed before turn completed".to_string());
            }
            let value: Value = serde_json::from_str(line.trim()).map_err(|error| {
                format!(
                    "invalid codex app-server json notification: {error}; line={}",
                    line.trim()
                )
            })?;
            if is_turn_completed_notification(&value) {
                app_log::info(
                    "codex_app_server",
                    &format!("turn completed notification thread_id={thread_id}"),
                );
                return self.validate_turn_completion(thread_id, &value);
            }
        }
    }

    fn validate_turn_completion(&self, thread_id: &str, completion: &Value) -> Result<(), String> {
        if turn_completion_thread_id(completion) != Some(thread_id) {
            return Err(format!(
                "codex app-server completed unexpected thread: expected={thread_id} actual={}",
                turn_completion_thread_id(completion).unwrap_or("<missing>")
            ));
        }
        if !turn_completion_succeeded(completion) {
            return Err(format!(
                "codex reply turn did not complete successfully: status={}",
                turn_completion_status(completion).unwrap_or("<missing>")
            ));
        }
        app_log::info(
            "codex_app_server",
            &format!("reply turn completed successfully thread_id={thread_id}"),
        );
        Ok(())
    }
}

fn is_turn_completed_notification(value: &Value) -> bool {
    value.get("method").and_then(Value::as_str) == Some("turn/completed")
}

fn turn_completion_thread_id(value: &Value) -> Option<&str> {
    value
        .get("params")
        .and_then(|params| params.get("threadId"))
        .and_then(Value::as_str)
}

fn turn_completion_status(value: &Value) -> Option<&str> {
    value
        .get("params")
        .and_then(|params| params.get("turn"))
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
}

fn turn_completion_succeeded(value: &Value) -> bool {
    is_turn_completed_notification(value) && turn_completion_status(value) == Some("completed")
}

fn refresh_codex_thread_view(thread_id: &str) {
    let deeplink = codex_thread_deeplink(thread_id);
    match open::that_detached(&deeplink) {
        Ok(()) => app_log::info(
            "codex_app_server",
            &format!("opened Codex thread deeplink thread_id={thread_id}"),
        ),
        Err(error) => app_log::error(
            "codex_app_server",
            &format!("failed to open Codex thread deeplink thread_id={thread_id} error={error}"),
        ),
    }
}

pub(crate) fn codex_thread_deeplink(thread_id: &str) -> String {
    let escaped: String = url::form_urlencoded::byte_serialize(thread_id.as_bytes()).collect();
    format!("codex://threads/{escaped}")
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn codex_binary() -> PathBuf {
    if let Some(path) = env::var_os("CODE_PET_CODEX_BIN") {
        return PathBuf::from(path);
    }
    if let Some(path) = platform_codex_binary_candidates()
        .into_iter()
        .find(|path| path.exists())
    {
        return path;
    }
    PathBuf::from("codex")
}

fn platform_codex_binary_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    append_platform_codex_binary_candidates(&mut candidates);
    append_path_codex_binary_candidates(&mut candidates);
    candidates
}

#[cfg(target_os = "macos")]
fn append_platform_codex_binary_candidates(candidates: &mut Vec<PathBuf>) {
    let app_binary = PathBuf::from("/Applications/Codex.app/Contents/Resources/codex");
    push_candidate(candidates, app_binary);
}

#[cfg(windows)]
fn append_platform_codex_binary_candidates(candidates: &mut Vec<PathBuf>) {
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for path in windows_codex_binary_candidates_from_local_app_data(&local_app_data) {
            push_candidate(candidates, path);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn append_platform_codex_binary_candidates(_candidates: &mut Vec<PathBuf>) {}

#[cfg(windows)]
fn windows_codex_binary_candidates_from_local_app_data(local_app_data: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    append_codex_exes_in_child_dirs(
        &mut candidates,
        &local_app_data.join("OpenAI").join("Codex").join("bin"),
    );

    let packages = local_app_data.join("Packages");
    for package_dir in child_dirs_with_prefix(&packages, "OpenAI.Codex_") {
        let package_bin = package_dir
            .join("LocalCache")
            .join("Local")
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("codex.exe");
        push_candidate(&mut candidates, package_bin);
    }

    candidates
}

#[cfg(windows)]
fn append_codex_exes_in_child_dirs(candidates: &mut Vec<PathBuf>, parent: &Path) {
    for child in child_dirs_newest_first(parent) {
        push_candidate(candidates, child.join("codex.exe"));
    }
}

#[cfg(windows)]
fn child_dirs_with_prefix(parent: &Path, prefix: &str) -> Vec<PathBuf> {
    child_dirs_newest_first(parent)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect()
}

#[cfg(windows)]
fn child_dirs_newest_first(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut dirs = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    dirs.sort_by(|left, right| modified_time(right).cmp(&modified_time(left)));
    dirs
}

#[cfg(windows)]
fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn append_path_codex_binary_candidates(candidates: &mut Vec<PathBuf>) {
    let Some(paths) = env::var_os("PATH") else {
        return;
    };
    for dir in env::split_paths(&paths) {
        for name in path_codex_binary_names() {
            push_candidate(candidates, dir.join(name));
        }
    }
}

#[cfg(windows)]
fn path_codex_binary_names() -> &'static [&'static str] {
    &["codex.exe", "codex.cmd", "codex.bat", "codex"]
}

#[cfg(not(windows))]
fn path_codex_binary_names() -> &'static [&'static str] {
    &["codex"]
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !candidates.iter().any(|candidate| candidate == &path) {
        candidates.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity_actions::{AgentInteractionDriver, ApprovalStrategy, ReplyStrategy};
    use crate::agents::AgentId;
    use crate::events::{PetEvent, PetEventKind, TaskStatus};
    use chrono::Utc;

    fn codex_event(status: TaskStatus, session_id: Option<&str>) -> PetEvent {
        PetEvent {
            id: "event-1".to_string(),
            provider: AgentId::Codex,
            kind: PetEventKind::TaskUpdated,
            status,
            title: "任务".to_string(),
            message: "完成".to_string(),
            session_id: session_id.map(str::to_string),
            cwd: Some("/tmp/project".to_string()),
            tool_name: None,
            should_ring: false,
            created_at: Utc::now(),
            raw: json!({}),
            source: None,
        }
    }

    #[test]
    fn chooses_steer_for_in_progress_turn() {
        let resume = json!({
            "thread": {
                "turns": [
                    { "id": "turn-old", "status": "completed" },
                    { "id": "turn-active", "status": "inProgress" }
                ]
            }
        });

        assert_eq!(
            reply_action_for_resume_response(&resume).unwrap(),
            CodexReplyAction::SteerTurn("turn-active".to_string())
        );
    }

    #[test]
    fn chooses_start_for_idle_thread() {
        let resume = json!({
            "thread": {
                "turns": [
                    { "id": "turn-old", "status": "completed" }
                ]
            }
        });

        assert_eq!(
            reply_action_for_resume_response(&resume).unwrap(),
            CodexReplyAction::StartTurn
        );
    }

    #[test]
    fn turn_completion_success_requires_completed_turn_status() {
        let completed = json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": { "id": "turn-1", "status": "completed" }
            }
        });
        let failed = json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": { "id": "turn-1", "status": "failed" }
            }
        });

        assert!(turn_completion_succeeded(&completed));
        assert!(!turn_completion_succeeded(&failed));
    }

    #[test]
    fn app_server_manager_is_the_codex_interaction_driver() {
        let manager = CodexAppServerManager;

        assert_eq!(
            manager.reply_strategy(&codex_event(TaskStatus::Done, Some("thread-1"))),
            ReplyStrategy::CodexAppServer
        );
        assert_eq!(
            manager.reply_strategy(&codex_event(TaskStatus::Running, Some("thread-1"))),
            ReplyStrategy::Unsupported
        );
        assert_eq!(
            manager.reply_strategy(&codex_event(TaskStatus::Done, None)),
            ReplyStrategy::Unsupported
        );
    }

    #[test]
    fn app_server_manager_supports_codex_collector_approval() {
        let manager = CodexAppServerManager;

        assert_eq!(
            manager.approval_strategy(&codex_event(TaskStatus::WaitingApproval, Some("thread-1"))),
            ApprovalStrategy::CollectorWait
        );
    }

    #[test]
    fn codex_thread_deeplink_points_at_the_desktop_thread_route() {
        assert_eq!(
            codex_thread_deeplink("019e8862-0d6c-7150-823f-18d4cd4e2813"),
            "codex://threads/019e8862-0d6c-7150-823f-18d4cd4e2813"
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_codex_binary_candidates_include_local_app_bins() {
        let temp = tempfile::tempdir().unwrap();
        let user_bin = temp
            .path()
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("hash")
            .join("codex.exe");
        std::fs::create_dir_all(user_bin.parent().unwrap()).unwrap();
        std::fs::write(&user_bin, b"").unwrap();

        let package_bin = temp
            .path()
            .join("Packages")
            .join("OpenAI.Codex_test")
            .join("LocalCache")
            .join("Local")
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("codex.exe");
        std::fs::create_dir_all(package_bin.parent().unwrap()).unwrap();
        std::fs::write(&package_bin, b"").unwrap();

        let candidates = windows_codex_binary_candidates_from_local_app_data(temp.path());

        assert!(candidates.contains(&user_bin));
        assert!(candidates.contains(&package_bin));
    }
}
