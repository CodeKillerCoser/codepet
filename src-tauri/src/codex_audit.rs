use crate::agents::AgentId;
use crate::events::{frontend_event, normalize_hook_payload, PetEvent};
use crate::state::SharedState;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::HashMap;
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
    let raw = serde_json::from_str::<Value>(line).ok()?;
    if raw.get("platform").and_then(Value::as_str) != Some("codex") {
        return None;
    }
    raw.get("hook_event_name").and_then(Value::as_str)?;

    let mut event = normalize_hook_payload(AgentId::Codex, raw.clone()).ok()?;
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
    if !audit_path.exists() {
        return Ok(0);
    }

    let text = fs::read_to_string(audit_path)?;
    let mut lines = text.lines().rev().take(max_lines).collect::<Vec<_>>();
    lines.reverse();

    let mut imported = 0;
    for line in lines {
        if let Some(event) = parse_codex_audit_line(line) {
            let event = apply_session_title(&mut session_titles, event);
            app_state.push_event(event);
            imported += 1;
        }
    }
    Ok(imported)
}

pub async fn watch_default_codex_audit(app_state: SharedState, app_handle: AppHandle) {
    watch_codex_audit(app_state, app_handle, default_codex_audit_path()).await;
}

async fn watch_codex_audit(app_state: SharedState, app_handle: AppHandle, audit_path: PathBuf) {
    let mut offset = file_len(&audit_path).unwrap_or(0);
    let mut session_titles = HashMap::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(len) = file_len(&audit_path) else {
            continue;
        };
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
                let event = apply_session_title(&mut session_titles, event);
                app_state.push_event(event.clone());
                let _ = app_handle.emit("pet-event", &frontend_event(&event));
            }
        }
    }
}

fn apply_session_title(session_titles: &mut HashMap<String, String>, event: PetEvent) -> PetEvent {
    let Some(session_id) = event.session_id.clone() else {
        return event;
    };
    let indexed_title = session_titles.get(&session_id).cloned();

    if !is_transcript_path(&event.message) && matches!(event.kind, crate::events::PetEventKind::TaskStarted) {
        session_titles.entry(session_id).or_insert_with(|| event.message.clone());
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

fn is_transcript_path(value: &str) -> bool {
    value.ends_with(".jsonl") && (value.contains("/.codex/") || value.contains("/.claude/"))
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
