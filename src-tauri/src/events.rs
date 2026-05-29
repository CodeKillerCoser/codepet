pub use crate::agents::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PetEventKind {
    TaskStarted,
    TaskUpdated,
    ToolStarted,
    PermissionRequested,
    Message,
    TaskFailed,
    TaskCompleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Idle,
    Thinking,
    Running,
    WaitingApproval,
    Failed,
    Done,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySource {
    pub pid: Option<u32>,
    pub ppid: Option<u32>,
    pub terminal_program: Option<String>,
    pub term_session_id: Option<String>,
    pub tty_path: Option<String>,
    pub tmux_pane: Option<String>,
    pub wezterm_pane: Option<String>,
    pub kitty_window_id: Option<String>,
    pub app_bundle_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetEvent {
    pub id: String,
    pub provider: AgentId,
    pub kind: PetEventKind,
    pub status: TaskStatus,
    pub title: String,
    pub message: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub tool_name: Option<String>,
    pub should_ring: bool,
    pub created_at: DateTime<Utc>,
    pub raw: Value,
    pub source: Option<ActivitySource>,
}

pub fn frontend_event(event: &PetEvent) -> PetEvent {
    PetEvent {
        id: event.id.clone(),
        provider: event.provider.clone(),
        kind: event.kind.clone(),
        status: event.status.clone(),
        title: event.title.clone(),
        message: event.message.clone(),
        session_id: event.session_id.clone(),
        cwd: event.cwd.clone(),
        tool_name: event.tool_name.clone(),
        should_ring: event.should_ring,
        created_at: event.created_at,
        raw: Value::Null,
        source: event.source.clone(),
    }
}

pub fn normalize_hook_payload(provider: AgentId, raw: Value) -> Result<PetEvent, String> {
    let raw_hook_event = first_string(
        &raw,
        &[
            "hook_event_name",
            "hookEventName",
            "event_name",
            "eventName",
            "event",
        ],
    )
    .or_else(|| {
        if provider == AgentId::Codex {
            Some("Notification".to_string())
        } else {
            None
        }
    })
    .ok_or_else(|| "hook payload missing event name".to_string())?;
    let hook_event = canonical_hook_event(provider, &raw_hook_event).to_string();
    let tool_name = tool_name_from_payload(provider, &raw, &raw_hook_event, &hook_event);
    let session_id = first_string(&raw, &["session_id", "sessionId", "conversation_id", "generation_id"]);
    let cwd = first_string(&raw, &["cwd", "workspace", "project_dir", "projectDir"]).or_else(|| workspace_root_from_payload(&raw));
    let message = message_from_payload(&raw, &hook_event, tool_name.as_deref());
    let payload_title = title_from_payload(&raw);
    let source = source_from_payload(&raw);

    let terminal_failure = matches!(hook_event.as_str(), "Stop" | "SessionEnd" | "Notification")
        && payload_indicates_failure(&raw, &message);
    let (kind, status, should_ring, title) = if terminal_failure {
        (
            PetEventKind::TaskFailed,
            TaskStatus::Failed,
            true,
            "任务失败",
        )
    } else {
        match hook_event.as_str() {
        "SessionStart" | "UserPromptSubmit" => (
            PetEventKind::TaskStarted,
            TaskStatus::Thinking,
            false,
            "任务开始",
        ),
        "PreToolUse" => (
            PetEventKind::ToolStarted,
            TaskStatus::Running,
            false,
            "正在执行工具",
        ),
        "PostToolUse" => (
            PetEventKind::TaskUpdated,
            TaskStatus::Running,
            false,
            "工具执行完成",
        ),
        "PermissionRequest" => (
            PetEventKind::PermissionRequested,
            TaskStatus::WaitingApproval,
            true,
            "等待授权",
        ),
        "PostToolUseFailure" => (
            PetEventKind::TaskFailed,
            TaskStatus::Failed,
            true,
            "任务失败",
        ),
        "Stop" | "SessionEnd" => (
            PetEventKind::TaskCompleted,
            TaskStatus::Done,
            true,
            "任务完成",
        ),
        "Notification" => (
            PetEventKind::Message,
            TaskStatus::Running,
            false,
            "收到消息",
        ),
        _ => (
            PetEventKind::Message,
            TaskStatus::Running,
            false,
            "收到消息",
        ),
        }
    };

    Ok(PetEvent {
        id: Uuid::new_v4().to_string(),
        provider,
        kind,
        status,
        title: payload_title.unwrap_or_else(|| title.to_string()),
        message,
        session_id,
        cwd,
        tool_name,
        should_ring,
        created_at: Utc::now(),
        raw,
        source,
    })
}

fn canonical_hook_event(provider: AgentId, hook_event: &str) -> &str {
    if provider != AgentId::Cursor {
        return hook_event;
    }

    match hook_event {
        "sessionStart" | "beforeSubmitPrompt" => "UserPromptSubmit",
        "preToolUse" | "beforeShellExecution" | "beforeMCPExecution" => "PreToolUse",
        "postToolUse" | "afterShellExecution" | "afterMCPExecution" | "afterFileEdit" => "PostToolUse",
        "stop" | "sessionEnd" => "Stop",
        _ => hook_event,
    }
}

fn tool_name_from_payload(provider: AgentId, raw: &Value, raw_hook_event: &str, hook_event: &str) -> Option<String> {
    first_string(raw, &["tool_name", "toolName", "tool", "matcher"]).or_else(|| {
        if provider != AgentId::Cursor {
            return None;
        }
        match raw_hook_event {
            "beforeShellExecution" | "afterShellExecution" => Some("Shell".to_string()),
            "beforeMCPExecution" | "afterMCPExecution" => Some("MCP".to_string()),
            "afterFileEdit" => Some("Edit".to_string()),
            "preToolUse" | "postToolUse" if matches!(hook_event, "PreToolUse" | "PostToolUse") => Some("Tool".to_string()),
            _ => None,
        }
    })
}

fn workspace_root_from_payload(raw: &Value) -> Option<String> {
    raw.get("workspace_roots")
        .or_else(|| raw.get("workspaceRoots"))
        .and_then(Value::as_array)
        .and_then(|roots| roots.iter().find_map(Value::as_str))
        .map(ToString::to_string)
}

fn payload_indicates_failure(raw: &Value, message: &str) -> bool {
    raw.get("isApiErrorMessage").and_then(Value::as_bool).unwrap_or(false)
        || raw.get("apiErrorStatus").and_then(Value::as_u64).is_some()
        || message.trim_start().starts_with("API Error:")
        || first_string(raw, &["error", "error_message", "errorMessage"])
            .is_some_and(|value| !value.trim().is_empty())
}

fn source_from_payload(raw: &Value) -> Option<ActivitySource> {
    let source = raw.get("code_pet").or_else(|| raw.get("codePet"))?;
    Some(ActivitySource {
        pid: number(source, "pid"),
        ppid: number(source, "ppid"),
        terminal_program: string(source, "terminalProgram").or_else(|| string(source, "terminal_program")),
        term_session_id: string(source, "termSessionId").or_else(|| string(source, "term_session_id")),
        tty_path: string(source, "ttyPath").or_else(|| string(source, "tty_path")),
        tmux_pane: string(source, "tmuxPane").or_else(|| string(source, "tmux_pane")),
        wezterm_pane: string(source, "weztermPane").or_else(|| string(source, "wezterm_pane")),
        kitty_window_id: string(source, "kittyWindowId").or_else(|| string(source, "kitty_window_id")),
        app_bundle_id: string(source, "appBundleId").or_else(|| string(source, "app_bundle_id")),
    })
}

fn string(raw: &Value, key: &str) -> Option<String> {
    raw.get(key).and_then(Value::as_str).map(ToString::to_string)
}

fn number(raw: &Value, key: &str) -> Option<u32> {
    raw.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn first_string(raw: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| raw.get(*key))
        .find_map(|value| value.as_str().map(ToString::to_string))
}

fn message_from_payload(raw: &Value, hook_event: &str, tool_name: Option<&str>) -> String {
    let keys = if matches!(hook_event, "Stop" | "SessionEnd" | "Notification") {
        [
            "last_assistant_message",
            "summary",
            "message",
            "reason",
            "error",
            "prompt",
            "transcript_path",
        ]
    } else {
        [
            "message",
            "summary",
            "prompt",
            "error",
            "reason",
            "last_assistant_message",
            "transcript_path",
        ]
    };

    for key in keys {
        if let Some(value) = raw.get(key).and_then(Value::as_str) {
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }

    if matches!(hook_event, "PreToolUse" | "PostToolUse") {
        if let Some(message) = tool_message_from_payload(raw, tool_name) {
            return message;
        }
    }

    if let Some(tool) = tool_name {
        return format!("{hook_event}: {tool}");
    }

    hook_event.to_string()
}

fn tool_message_from_payload(raw: &Value, tool_name: Option<&str>) -> Option<String> {
    if let Some(command) = first_string(raw, &["command", "cmd", "script"]) {
        return Some(compact_line(&command));
    }
    if let Some(path) = first_string(raw, &["file_path", "filePath", "path"]) {
        let label = tool_name.unwrap_or("文件");
        return Some(format!("{label} · {}", compact_path(&path)));
    }

    let input = ["tool_input", "toolInput", "input", "parameters", "args"]
        .iter()
        .find_map(|key| raw.get(*key).filter(|value| value.is_object()))?;
    let command = first_string(input, &["command", "cmd", "script"]);
    let description = first_string(input, &["description", "summary"]);
    if let Some(command) = command {
        return Some(match description {
            Some(description) if !description.trim().is_empty() => format!("{} · {}", description.trim(), compact_line(&command)),
            _ => compact_line(&command),
        });
    }

    if let Some(path) = first_string(input, &["file_path", "filePath", "path"]) {
        let label = tool_name.unwrap_or("文件");
        return Some(format!("{label} · {}", compact_path(&path)));
    }

    first_string(input, &["pattern", "query", "text"])
        .map(|value| format!("{} · {}", tool_name.unwrap_or("工具"), compact_line(&value)))
}

fn compact_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_path(value: &str) -> String {
    let path = std::path::Path::new(value);
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or(value);
    let parent = path.parent().and_then(|parent| parent.file_name()).and_then(|name| name.to_str());
    match parent {
        Some(parent) if parent != "." => format!("{parent}/{file_name}"),
        _ => file_name.to_string(),
    }
}

fn title_from_payload(raw: &Value) -> Option<String> {
    for key in [
        "title",
        "thread_name",
        "threadName",
        "conversation_title",
        "conversationTitle",
        "task_name",
        "taskName",
    ] {
        if let Some(title) = raw.get(key).and_then(Value::as_str).and_then(clean_title) {
            return Some(title);
        }
    }

    for path in [
        &["parent_business_info", "name"][..],
        &["parentBusinessInfo", "name"][..],
        &["conversation", "title"][..],
        &["session", "title"][..],
    ] {
        if let Some(title) = nested_string(raw, path).and_then(clean_title) {
            return Some(title);
        }
    }

    None
}

fn nested_string<'a>(raw: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut value = raw;
    for key in path {
        value = value.get(*key)?;
    }
    value.as_str()
}

pub fn clean_title(value: &str) -> Option<String> {
    let title = value.trim();
    if title.is_empty() || matches!(title, "New Session" | "New Conversation Started") {
        return None;
    }
    Some(title.to_string())
}
