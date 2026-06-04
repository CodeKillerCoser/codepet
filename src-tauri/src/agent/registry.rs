use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    Codex,
    Claude,
    Qoder,
    Cursor,
}

impl AgentId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Qoder => "qoder",
            Self::Cursor => "cursor",
        }
    }
}

impl FromStr for AgentId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "qoder" => Ok(Self::Qoder),
            "cursor" => Ok(Self::Cursor),
            other => Err(format!("unsupported agent id: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFormat {
    JsonHooks,
}

#[derive(Clone, Debug)]
pub struct AgentSpec {
    pub id: AgentId,
    pub name: &'static str,
    pub description: &'static str,
    pub config_format: ConfigFormat,
    pub hook_events: &'static [&'static str],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: AgentId,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub config_path: String,
    pub hook_events: Vec<String>,
}

impl AgentView {
    pub fn from_spec(spec: AgentSpec, config_path: PathBuf, enabled: bool) -> Self {
        Self {
            id: spec.id,
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            enabled,
            config_path: config_path.display().to_string(),
            hook_events: spec.hook_events.iter().map(|event| event.to_string()).collect(),
        }
    }
}

pub fn agent_specs() -> Vec<AgentSpec> {
    vec![
        AgentSpec {
            id: AgentId::Codex,
            name: "Codex",
            description: "接入本机 Codex hooks.json 的 prompt、tool、failure、session 与 stop 事件。",
            config_format: ConfigFormat::JsonHooks,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "PermissionRequest",
                "Stop",
            ],
        },
        AgentSpec {
            id: AgentId::Claude,
            name: "Claude Code",
            description: "接入 Session、Prompt、Tool、Permission、Stop 等 Claude Code hooks。",
            config_format: ConfigFormat::JsonHooks,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "PermissionRequest",
                "Stop",
            ],
        },
        AgentSpec {
            id: AgentId::Qoder,
            name: "Qoder",
            description: "接入 Qoder prompt、tool、failure、stop 与 notification hooks。",
            config_format: ConfigFormat::JsonHooks,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "PermissionRequest",
                "Notification",
                "Stop",
            ],
        },
        AgentSpec {
            id: AgentId::Cursor,
            name: "Cursor",
            description: "接入 Cursor hooks.json 的 session、prompt、tool、file edit、shell、MCP 与 stop 事件。",
            config_format: ConfigFormat::JsonHooks,
            hook_events: &[
                "sessionStart",
                "beforeSubmitPrompt",
                "preToolUse",
                "postToolUse",
                "beforeShellExecution",
                "afterShellExecution",
                "beforeMCPExecution",
                "afterMCPExecution",
                "afterFileEdit",
                "stop",
            ],
        },
    ]
}

pub fn resolve_agent_config_path(agent_id: AgentId) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match agent_id {
        AgentId::Codex => home.join(".codex").join("hooks.json"),
        AgentId::Claude => home.join(".claude").join("settings.json"),
        AgentId::Qoder => home.join(".qoder").join("settings.json"),
        AgentId::Cursor => home.join(".cursor").join("hooks.json"),
    }
}
