use crate::agents::AgentId;
use crate::events::{frontend_event, PetEvent, PetEventKind, TaskStatus};
use crate::state::SharedState;
use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaudeTranscriptOutcome {
    Failed {
        session_id: String,
        cwd: Option<String>,
        message: String,
    },
    Done {
        session_id: String,
        cwd: Option<String>,
        message: String,
    },
}

pub fn claude_outcome_from_transcript_line(line: &str) -> Option<ClaudeTranscriptOutcome> {
    let raw: Value = serde_json::from_str(line).ok()?;
    if raw.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }

    let session_id = raw.get("sessionId").and_then(Value::as_str)?.to_string();
    let cwd = raw.get("cwd").and_then(Value::as_str).map(ToString::to_string);
    let message = raw.get("message")?;
    let stop_reason = message.get("stop_reason").and_then(Value::as_str);
    let text = assistant_text(message)?;
    let is_api_error = raw
        .get("isApiErrorMessage")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || raw.get("apiErrorStatus").and_then(Value::as_u64).is_some()
        || text.trim_start().starts_with("API Error:");

    if is_api_error {
        return Some(ClaudeTranscriptOutcome::Failed {
            session_id,
            cwd,
            message: text,
        });
    }

    if stop_reason == Some("end_turn") && !text.trim().is_empty() {
        return Some(ClaudeTranscriptOutcome::Done {
            session_id,
            cwd,
            message: text,
        });
    }

    None
}

pub fn event_from_claude_outcome(fallback: &PetEvent, outcome: ClaudeTranscriptOutcome) -> Option<PetEvent> {
    let (kind, status, session_id, cwd, message, title) = match outcome {
        ClaudeTranscriptOutcome::Failed { session_id, cwd, message } => (
            PetEventKind::TaskFailed,
            TaskStatus::Failed,
            session_id,
            cwd,
            message,
            "任务失败".to_string(),
        ),
        ClaudeTranscriptOutcome::Done { session_id, cwd, message } => (
            PetEventKind::TaskCompleted,
            TaskStatus::Done,
            session_id,
            cwd,
            message,
            "任务完成".to_string(),
        ),
    };

    if fallback.session_id.as_deref() != Some(session_id.as_str()) {
        return None;
    }

    let should_ring = matches!(status, TaskStatus::Failed | TaskStatus::Done);

    Some(PetEvent {
        id: Uuid::new_v4().to_string(),
        provider: AgentId::Claude,
        kind,
        status,
        title: fallback_title(fallback, &title),
        message,
        session_id: Some(session_id),
        cwd: cwd.or_else(|| fallback.cwd.clone()),
        tool_name: None,
        should_ring,
        created_at: Utc::now(),
        raw: fallback.raw.clone(),
        source: fallback.source.clone(),
    })
}

pub async fn watch_claude_transcript_for_outcome(
    transcript_path: PathBuf,
    fallback: PetEvent,
    state: SharedState,
    app_handle: AppHandle,
) {
    let mut offset = fs::metadata(&transcript_path).map(|metadata| metadata.len() as usize).unwrap_or(0);

    for _ in 0..600 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let Ok(text) = fs::read_to_string(&transcript_path) else {
            continue;
        };
        if text.len() <= offset {
            continue;
        }

        let next_text = &text[offset..];
        offset = text.len();
        for line in next_text.lines().filter(|line| !line.trim().is_empty()) {
            if let Some(outcome) = claude_outcome_from_transcript_line(line) {
                if let Some(event) = event_from_claude_outcome(&fallback, outcome) {
                    state.push_event(event.clone());
                    let _ = app_handle.emit("pet-event", &frontend_event(&event));
                    return;
                }
            }
        }
    }
}

pub fn transcript_path_from_event(event: &PetEvent) -> Option<PathBuf> {
    let value = event.raw.get("transcript_path").and_then(Value::as_str)?;
    let path = Path::new(value);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn assistant_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.trim().to_string()).filter(|value| !value.is_empty());
    }

    let mut parts = Vec::new();
    for item in content.as_array()? {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn fallback_title(fallback: &PetEvent, default_title: &str) -> String {
    if matches!(
        fallback.title.as_str(),
        "任务开始" | "收到消息" | "正在执行工具" | "工具执行完成" | "任务完成" | "任务失败"
    ) && !fallback.message.trim().is_empty()
    {
        fallback.message.clone()
    } else if fallback.title.trim().is_empty() {
        default_title.to_string()
    } else {
        fallback.title.clone()
    }
}
