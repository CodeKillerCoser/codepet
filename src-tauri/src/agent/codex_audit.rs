use crate::agents::AgentId;
use crate::app_log::PerfSpan;
use crate::events::{frontend_event, normalize_hook_payload, PetEvent, PetEventKind, TaskStatus};
use crate::state::SharedState;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

const RECENT_AUDIT_LINES: usize = 500;

pub fn default_codex_audit_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("audit")
        .join("audit.jsonl")
}

pub fn default_codex_session_index_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("session_index.jsonl")
}

pub fn parse_codex_audit_line(line: &str) -> Option<PetEvent> {
    parse_codex_audit_line_with_transcript_cache(line, None)
}

fn parse_codex_audit_line_with_transcript_cache(
    line: &str,
    transcript_cache: Option<&mut HashMap<PathBuf, String>>,
) -> Option<PetEvent> {
    let raw = serde_json::from_str::<Value>(line).ok()?;
    if raw.get("platform").and_then(Value::as_str) != Some("codex") {
        return None;
    }
    raw.get("hook_event_name").and_then(Value::as_str)?;

    let mut event = normalize_hook_payload(AgentId::Codex, raw.clone()).ok()?;
    if audit_line_waits_for_escalated_approval(&raw, transcript_cache) {
        event.kind = PetEventKind::PermissionRequested;
        event.status = TaskStatus::WaitingApproval;
        event.should_ring = true;
        event.title = "等待授权".to_string();
    }
    if let Some(created_at) = parse_audit_timestamp(&raw) {
        event.created_at = created_at;
    }
    Some(event)
}

pub fn replay_default_codex_audit_events(app_state: &SharedState) -> io::Result<usize> {
    replay_recent_codex_audit_events_with_session_index(
        app_state,
        &default_codex_audit_path(),
        &default_codex_session_index_path(),
        RECENT_AUDIT_LINES,
    )
}

pub fn replay_recent_codex_audit_events(
    app_state: &SharedState,
    audit_path: &Path,
    max_lines: usize,
) -> io::Result<usize> {
    replay_recent_codex_audit_events_with_titles(app_state, audit_path, max_lines, HashMap::new())
}

pub fn replay_recent_codex_audit_events_with_session_index(
    app_state: &SharedState,
    audit_path: &Path,
    session_index_path: &Path,
    max_lines: usize,
) -> io::Result<usize> {
    replay_recent_codex_audit_events_with_titles(app_state, audit_path, max_lines, load_session_index_titles(session_index_path)?)
}

fn replay_recent_codex_audit_events_with_titles(
    app_state: &SharedState,
    audit_path: &Path,
    max_lines: usize,
    mut session_titles: HashMap<String, String>,
) -> io::Result<usize> {
    if !app_state.agent_enabled(AgentId::Codex) {
        return Ok(0);
    }
    if !audit_path.exists() {
        return Ok(0);
    }

    let read_span = PerfSpan::start("startup.codex_audit.read_audit");
    let text = fs::read_to_string(audit_path)?;
    let audit_bytes = text.len();
    let audit_lines = text.lines().count();
    read_span.finish_ok(&[
        ("audit_bytes", audit_bytes.to_string()),
        ("audit_lines", audit_lines.to_string()),
    ]);
    let recent_span = PerfSpan::start("startup.codex_audit.collect_recent");
    let mut lines = text.lines().rev().take(max_lines).collect::<Vec<_>>();
    lines.reverse();
    let recent_lines = lines.len();
    recent_span.finish_ok(&[
        ("max_lines", max_lines.to_string()),
        ("recent_lines", recent_lines.to_string()),
    ]);

    let parse_span = PerfSpan::start("startup.codex_audit.parse_recent");
    let mut imported = 0;
    let mut hidden_internal_sessions = HashSet::new();
    let mut transcript_cache = HashMap::new();
    for line in lines {
        if let Some(event) = parse_codex_audit_line_with_transcript_cache(line, Some(&mut transcript_cache)) {
            if should_skip_internal_session(&mut hidden_internal_sessions, &event) {
                continue;
            }
            let event = apply_session_title(&mut session_titles, event);
            app_state.push_event(event);
            imported += 1;
        }
    }
    let transcript_files = transcript_cache.len();
    let transcript_bytes = transcript_cache.values().map(|text| text.len()).sum::<usize>();
    parse_span.finish_ok(&[
        ("imported", imported.to_string()),
        ("recent_lines", recent_lines.to_string()),
        ("transcript_files", transcript_files.to_string()),
        ("transcript_bytes", transcript_bytes.to_string()),
    ]);
    Ok(imported)
}

pub async fn watch_default_codex_audit(app_state: SharedState, app_handle: AppHandle) {
    watch_codex_audit(app_state, app_handle, default_codex_audit_path()).await;
}

async fn watch_codex_audit(app_state: SharedState, app_handle: AppHandle, audit_path: PathBuf) {
    let mut offset = file_len(&audit_path).unwrap_or(0);
    let mut session_titles = HashMap::new();
    let mut hidden_internal_sessions = HashSet::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(len) = file_len(&audit_path) else {
            continue;
        };
        if !app_state.agent_enabled(AgentId::Codex) {
            offset = len;
            continue;
        }
        if len < offset {
            offset = 0;
        }
        if len == offset {
            continue;
        }

        let Ok(mut file) = File::open(&audit_path) else {
            continue;
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            continue;
        }
        let mut chunk = String::new();
        if file.read_to_string(&mut chunk).is_err() {
            continue;
        }
        offset = len;

        if let Ok(index_titles) = load_session_index_titles(&default_codex_session_index_path()) {
            session_titles.extend(index_titles);
        }
        for line in chunk.lines() {
            if let Some(event) = parse_codex_audit_line(line) {
                if should_skip_internal_session(&mut hidden_internal_sessions, &event) {
                    continue;
                }
                let event = apply_session_title(&mut session_titles, event);
                app_state.push_event(event.clone());
                let _ = app_handle.emit("pet-event", &frontend_event(&event));
                refresh_token_usage_if_needed(&app_handle, event);
            }
        }
    }
}

fn refresh_token_usage_if_needed(app_handle: &AppHandle, event: PetEvent) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(Some(summary)) = crate::token_usage::refresh_usage_for_event(&event) {
            let _ = app_handle.emit("token-usage-updated", summary);
        }
    });
}

fn apply_session_title(session_titles: &mut HashMap<String, String>, event: PetEvent) -> PetEvent {
    let Some(session_id) = event.session_id.clone() else {
        return event;
    };
    let indexed_title = session_titles.get(&session_id).cloned();

    if is_storable_session_title(&event) {
        session_titles.entry(session_id).or_insert_with(|| event.message.clone());
        if let Some(title) = indexed_title {
            return PetEvent { title, ..event };
        }
        return event;
    }

    if matches!(event.kind, crate::events::PetEventKind::TaskStarted) {
        if let Some(title) = indexed_title {
            return PetEvent { title, ..event };
        }
        return event;
    }

    if is_transcript_path(&event.message) {
        if let Some(title) = session_titles.get(&session_id) {
            return PetEvent {
                title: title.clone(),
                message: title.clone(),
                ..event
            };
        }
    }
    if let Some(title) = indexed_title {
        return PetEvent { title, ..event };
    }
    event
}

fn should_skip_internal_session(hidden_internal_sessions: &mut HashSet<String>, event: &PetEvent) -> bool {
    let Some(session_id) = event.session_id.as_deref() else {
        return false;
    };
    if hidden_internal_sessions.contains(session_id) {
        return true;
    }
    if is_codex_internal_background_event(event) {
        hidden_internal_sessions.insert(session_id.to_string());
        return true;
    }
    false
}

fn is_storable_session_title(event: &PetEvent) -> bool {
    matches!(event.kind, crate::events::PetEventKind::TaskStarted)
        && !is_transcript_path(&event.message)
        && event.message.trim() != "SessionStart"
        && !is_codex_internal_background_event(event)
}

fn is_codex_internal_background_event(event: &PetEvent) -> bool {
    let text = format!("{}\n{}", event.title, event.message);
    text.contains("Generate 0 to 3 hyperpersonalized suggestions for what this user can do with Codex")
        || text.contains("Recent Codex threads in this project:")
        || text.contains("Avoid repeating these previously dismissed suggestions:")
        || text.contains("Each suggestion must include: title, description, prompt, appId")
        || text.contains("You will be presented with a user prompt, and your job is to provide a short title for a task")
}

fn load_session_index_titles(path: &Path) -> io::Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let text = fs::read_to_string(path)?;
    let mut titles = HashMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(raw) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = raw.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(thread_name) = raw.get("thread_name").and_then(Value::as_str) else {
            continue;
        };
        if !thread_name.is_empty() {
            titles.insert(id.to_string(), thread_name.to_string());
        }
    }
    Ok(titles)
}

fn audit_line_waits_for_escalated_approval(raw: &Value, transcript_cache: Option<&mut HashMap<PathBuf, String>>) -> bool {
    if raw.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse") {
        return false;
    }
    let Some(call_id) = raw.get("tool_use_id").and_then(Value::as_str) else {
        return false;
    };
    let Some(transcript_path) = raw.get("transcript_path").and_then(Value::as_str) else {
        return false;
    };

    if let Some(cache) = transcript_cache {
        let text = cache
            .entry(PathBuf::from(transcript_path))
            .or_insert_with(|| fs::read_to_string(transcript_path).unwrap_or_default());
        return transcript_text_waits_for_escalated_approval(text, call_id);
    };

    let Ok(text) = fs::read_to_string(transcript_path) else {
        return false;
    };
    transcript_text_waits_for_escalated_approval(&text, call_id)
}

fn transcript_text_waits_for_escalated_approval(text: &str, call_id: &str) -> bool {
    let mut requires_approval = false;
    let mut has_output = false;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(raw_line) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(payload) = raw_line.get("payload") else {
            continue;
        };
        if payload.get("call_id").and_then(Value::as_str) != Some(call_id) {
            continue;
        }
        match payload.get("type").and_then(Value::as_str) {
            Some("function_call") | Some("custom_tool_call") => {
                requires_approval = tool_call_requires_escalation(payload);
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                has_output = true;
            }
            _ => {}
        }
    }
    requires_approval && !has_output
}

fn tool_call_requires_escalation(payload: &Value) -> bool {
    argument_value(payload.get("arguments"))
        .or_else(|| argument_value(payload.get("input")))
        .and_then(|arguments| {
            arguments
                .get("sandbox_permissions")
                .or_else(|| arguments.get("sandboxPermissions"))
                .and_then(Value::as_str)
                .map(|value| value == "require_escalated")
        })
        .unwrap_or(false)
}

fn argument_value(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        Value::Object(_) => value.cloned(),
        _ => None,
    }
}

fn is_transcript_path(value: &str) -> bool {
    value.ends_with(".jsonl")
        && (value.contains("/.codex/")
            || value.contains("/.claude/")
            || value.contains("\\.codex\\")
            || value.contains("\\.claude\\"))
}

fn file_len(path: &Path) -> io::Result<u64> {
    fs::metadata(path).map(|metadata| metadata.len())
}

fn parse_audit_timestamp(raw: &Value) -> Option<DateTime<Utc>> {
    let value = raw.get("_timestamp").and_then(Value::as_str)?;
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }

    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|datetime| datetime.with_timezone(&Utc))
}
