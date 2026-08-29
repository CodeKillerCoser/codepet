use crate::runtime_gateway::generated::PermissionLevel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

pub const CODEX_PROVIDER_ID: &str = "codex";
pub const CODEX_EXTENSION_NAMESPACE: &str = "codex.app-server";

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
}

impl JsonRpcId {
    pub fn approval_id(&self) -> String {
        match self {
            Self::String(value) => format!("codex-request:string:{value}"),
            Self::Number(value) => format!("codex-request:number:{value}"),
        }
    }
}

impl fmt::Display for JsonRpcId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => write!(formatter, "{value}"),
            Self::Number(value) => write!(formatter, "{value}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexAppServerError {
    Spawn(String),
    Io(String),
    Timeout(String),
    Protocol(String),
    Rpc {
        code: i64,
        message: String,
    },
    ProcessExited,
    Shutdown,
    UnsupportedCapability {
        capability: String,
        message: String,
    },
}

impl fmt::Display for CodexAppServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "failed to start codex app-server: {message}"),
            Self::Io(message) => write!(formatter, "codex app-server I/O error: {message}"),
            Self::Timeout(message) => write!(formatter, "codex app-server timeout: {message}"),
            Self::Protocol(message) => write!(formatter, "codex app-server protocol error: {message}"),
            Self::Rpc { code, message } => {
                write!(formatter, "codex app-server RPC error {code}: {message}")
            }
            Self::ProcessExited => write!(formatter, "codex app-server process exited"),
            Self::Shutdown => write!(formatter, "codex app-server session is shut down"),
            Self::UnsupportedCapability {
                capability,
                message,
            } => write!(formatter, "unsupported codex capability {capability}: {message}"),
        }
    }
}

impl std::error::Error for CodexAppServerError {}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexThread {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub status: CodexThreadStatus,
    #[serde(default)]
    pub turns: Vec<CodexTurn>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CodexThreadStatus {
    NotLoaded,
    #[default]
    Idle,
    SystemError,
    Active {
        #[serde(default)]
        active_flags: Vec<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexTurn {
    pub id: String,
    pub status: CodexTurnStatus,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum CodexTurnStatus {
    #[serde(rename = "inProgress")]
    InProgress,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "interrupted")]
    Interrupted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexConversationSnapshot {
    pub thread: CodexThread,
    pub workspace_root: Option<String>,
    pub permission_level: Option<PermissionLevel>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

impl CodexConversationSnapshot {
    pub fn from_thread(thread: CodexThread) -> Self {
        Self {
            thread,
            workspace_root: None,
            permission_level: None,
            model: None,
            reasoning_effort: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexThreadListRequest {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub workspace_root: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexThreadPage {
    pub data: Vec<CodexConversationSnapshot>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexThreadStartRequest {
    pub workspace_root: Option<String>,
    pub permission_level: PermissionLevel,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexTurnStartRequest {
    pub thread_id: String,
    pub message: String,
    pub client_message_id: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexTurnSteerRequest {
    pub thread_id: String,
    pub expected_turn_id: String,
    pub message: String,
    pub client_message_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexApprovalKind {
    CommandExecution,
    FileChange,
}

impl CodexApprovalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommandExecution => "command-execution",
            Self::FileChange => "file-change",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexApprovalRequest {
    pub request_id: JsonRpcId,
    pub kind: CodexApprovalKind,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub title: String,
    pub description: Option<String>,
    pub requested_at_ms: u64,
    pub available_decisions: Vec<String>,
}

impl CodexApprovalRequest {
    pub fn approval_id(&self) -> String {
        self.request_id.approval_id()
    }

    pub const fn native_method(&self) -> &'static str {
        match self.kind {
            CodexApprovalKind::CommandExecution => "item/commandExecution/requestApproval",
            CodexApprovalKind::FileChange => "item/fileChange/requestApproval",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodexNotification {
    ThreadStarted {
        thread: CodexThread,
    },
    TurnStarted {
        thread_id: String,
        turn: CodexTurn,
    },
    TurnCompleted {
        thread_id: String,
        turn: CodexTurn,
    },
    OutputDelta {
        native_method: String,
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: String,
        delta: String,
    },
    ServerRequestResolved {
        request_id: JsonRpcId,
        thread_id: String,
    },
    Unknown {
        method: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodexIncoming {
    Notification(CodexNotification),
    ApprovalRequested(CodexApprovalRequest),
    UnsupportedServerRequest {
        request_id: JsonRpcId,
        method: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadListResponse {
    pub data: Vec<CodexThread>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResponse {
    pub thread: CodexThread,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub sandbox: Option<CodexSandboxPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum CodexSandboxPolicy {
    Structured {
        #[serde(rename = "type")]
        policy_type: String,
    },
    Legacy(String),
}

impl CodexSandboxPolicy {
    fn policy_type(&self) -> &str {
        match self {
            Self::Structured { policy_type } | Self::Legacy(policy_type) => policy_type,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct TurnResponse {
    pub turn: CodexTurn,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnSteerResponse {
    pub turn_id: String,
}

pub(crate) fn thread_list_params(request: &CodexThreadListRequest) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(cursor) = &request.cursor {
        params.insert("cursor".to_string(), json!(cursor));
    }
    if let Some(limit) = request.limit {
        params.insert("limit".to_string(), json!(limit));
    }
    if let Some(cwd) = &request.workspace_root {
        params.insert("cwd".to_string(), json!(cwd));
    }
    Value::Object(params)
}

pub(crate) fn thread_start_params(request: &CodexThreadStartRequest) -> Value {
    let (sandbox, approval_policy) = permission_settings(request.permission_level);
    let mut params = serde_json::Map::new();
    params.insert("sandbox".to_string(), json!(sandbox));
    params.insert("approvalPolicy".to_string(), json!(approval_policy));
    if let Some(cwd) = &request.workspace_root {
        params.insert("cwd".to_string(), json!(cwd));
    }
    if let Some(model) = &request.model {
        params.insert("model".to_string(), json!(model));
    }
    if let Some(effort) = &request.reasoning_effort {
        params.insert(
            "config".to_string(),
            json!({ "model_reasoning_effort": effort }),
        );
    }
    Value::Object(params)
}

pub(crate) fn turn_start_params(request: &CodexTurnStartRequest) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("threadId".to_string(), json!(request.thread_id));
    params.insert("input".to_string(), text_input(&request.message));
    if let Some(client_message_id) = &request.client_message_id {
        params.insert("clientUserMessageId".to_string(), json!(client_message_id));
    }
    if let Some(model) = &request.model {
        params.insert("model".to_string(), json!(model));
    }
    if let Some(effort) = &request.reasoning_effort {
        params.insert("effort".to_string(), json!(effort));
    }
    Value::Object(params)
}

pub(crate) fn turn_steer_params(request: &CodexTurnSteerRequest) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("threadId".to_string(), json!(request.thread_id));
    params.insert(
        "expectedTurnId".to_string(),
        json!(request.expected_turn_id),
    );
    params.insert("input".to_string(), text_input(&request.message));
    if let Some(client_message_id) = &request.client_message_id {
        params.insert("clientUserMessageId".to_string(), json!(client_message_id));
    }
    Value::Object(params)
}

pub(crate) fn text_input(message: &str) -> Value {
    json!([{ "type": "text", "text": message, "text_elements": [] }])
}

pub(crate) fn permission_settings(permission: PermissionLevel) -> (&'static str, &'static str) {
    match permission {
        PermissionLevel::ReadOnly => ("read-only", "on-request"),
        PermissionLevel::WorkspaceWrite => ("workspace-write", "on-request"),
        PermissionLevel::FullAccess => ("danger-full-access", "never"),
    }
}

pub(crate) fn permission_from_sandbox(
    sandbox: Option<&CodexSandboxPolicy>,
) -> Option<PermissionLevel> {
    match sandbox.map(CodexSandboxPolicy::policy_type) {
        Some("readOnly" | "read-only") => Some(PermissionLevel::ReadOnly),
        Some("workspaceWrite" | "workspace-write") => Some(PermissionLevel::WorkspaceWrite),
        Some("dangerFullAccess" | "danger-full-access") => Some(PermissionLevel::FullAccess),
        _ => None,
    }
}
