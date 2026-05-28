use crate::agents::AgentId;
use crate::events::{clean_title, PetEvent};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn enrich_event_title(mut event: PetEvent) -> PetEvent {
    let Some(session_id) = event.session_id.as_deref() else {
        return event;
    };
    let Some(title) = resolve_external_title(event.provider, session_id) else {
        return event;
    };
    event.title = title;
    event
}

pub fn resolve_external_title(provider: AgentId, session_id: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    match provider {
        AgentId::Claude => resolve_claude_title_from_projects(&home.join(".claude").join("projects"), session_id),
        AgentId::Qoder => resolve_qoder_title_from_projects(&home.join(".qoder").join("projects"), session_id),
        AgentId::Codex | AgentId::Cursor => None,
    }
}

pub fn resolve_claude_title_from_projects(projects_root: &Path, session_id: &str) -> Option<String> {
    for entry in fs::read_dir(projects_root).ok()?.filter_map(Result::ok) {
        let index_path = entry.path().join("sessions-index.json");
        if !index_path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(index_path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(entries) = json
            .as_array()
            .or_else(|| json.get("sessions").and_then(Value::as_array))
            .or_else(|| json.get("entries").and_then(Value::as_array))
        else {
            continue;
        };
        for item in entries {
            if item.get("sessionId").and_then(Value::as_str) == Some(session_id) {
                return item
                    .get("firstPrompt")
                    .and_then(Value::as_str)
                    .and_then(clean_title)
                    .or_else(|| item.get("title").and_then(Value::as_str).and_then(clean_title));
            }
        }
    }
    None
}

pub fn resolve_qoder_title_from_projects(projects_root: &Path, session_id: &str) -> Option<String> {
    let target_name = format!("{session_id}-session.json");
    find_qoder_session_file(projects_root, &target_name, 0).and_then(|path| {
        let text = fs::read_to_string(path).ok()?;
        let json: Value = serde_json::from_str(&text).ok()?;
        json.get("title")
            .and_then(Value::as_str)
            .and_then(clean_title)
            .or_else(|| json.get("name").and_then(Value::as_str).and_then(clean_title))
    })
}

fn find_qoder_session_file(root: &Path, target_name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 6 || !root.is_dir() {
        return None;
    }

    for entry in fs::read_dir(root).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|name| name.to_str()) == Some(target_name) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_qoder_session_file(&path, target_name, depth + 1) {
                return Some(found);
            }
        }
    }

    None
}
