use crate::events::PetEvent;
use serde_json::{json, Value};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexReplyAction {
    StartTurn,
    SteerTurn(String),
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
            client.request(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": input,
                }),
            )?;
        }
    }
    client.keep_alive_until_turn_finishes();
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
}

impl CodexAppServerClient {
    fn spawn() -> Result<Self, String> {
        let mut child = Command::new(codex_binary())
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
        self.stdin.write_all(b"\n").map_err(|error| error.to_string())?;
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
                format!("invalid codex app-server json response: {error}; line={}", line.trim())
            })?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("codex app-server {id} error: {error}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn keep_alive_until_turn_finishes(self) {
        thread::spawn(move || {
            let mut client = self;
            let mut line = String::new();
            loop {
                line.clear();
                let Ok(read) = client.stdout.read_line(&mut line) else {
                    break;
                };
                if read == 0 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                if value.get("method").and_then(Value::as_str) == Some("turn/completed") {
                    break;
                }
            }
        });
    }
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
    let app_binary = PathBuf::from("/Applications/Codex.app/Contents/Resources/codex");
    if app_binary.exists() {
        return app_binary;
    }
    PathBuf::from("codex")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
