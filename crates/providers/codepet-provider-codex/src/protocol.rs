use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

pub const CODEX_PLUGIN_ID: &str = "dev.codepet.codex";
pub const CODEX_INSTANCE_KIND: &str = "codex";
pub const CODEX_EXTENSION_NAMESPACE: &str = "codex.app-server";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexPermissionLevel {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
}

impl JsonRpcId {
    fn resource_component(&self) -> String {
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
        }
    }
}

impl std::error::Error for CodexAppServerError {}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexThread {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub cwd: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: CodexThreadStatus,
    pub turns: Vec<CodexTurn>,
    pub cli_version: String,
    pub ephemeral: bool,
    pub model_provider: String,
    pub project_id: Value,
    pub session_id: String,
    pub source: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CodexThreadStatus {
    NotLoaded,
    Idle,
    SystemError,
    Active {
        #[serde(rename = "activeFlags")]
        active_flags: Vec<CodexThreadActiveFlag>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum CodexThreadActiveFlag {
    #[serde(rename = "waitingOnApproval")]
    WaitingOnApproval,
    #[serde(rename = "waitingOnUserInput")]
    WaitingOnUserInput,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexTurn {
    pub id: String,
    pub status: CodexTurnStatus,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub items: Vec<Value>,
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
    pub permission_level: Option<CodexPermissionLevel>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

impl CodexConversationSnapshot {
    pub fn from_thread(thread: CodexThread) -> Self {
        let workspace_root = Some(thread.cwd.clone());
        Self {
            thread,
            workspace_root,
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
    pub permission_level: CodexPermissionLevel,
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
    pub session_generation: String,
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
        approval_resource_id(&self.session_generation, &self.request_id)
    }

    pub const fn native_method(&self) -> &'static str {
        match self.kind {
            CodexApprovalKind::CommandExecution => "item/commandExecution/requestApproval",
            CodexApprovalKind::FileChange => "item/fileChange/requestApproval",
        }
    }
}

pub fn approval_resource_id(session_generation: &str, request_id: &JsonRpcId) -> String {
    format!(
        "codex-approval:{session_generation}:{}",
        request_id.resource_component()
    )
}

pub fn approval_generation(resource_id: &str) -> Option<&str> {
    resource_id
        .strip_prefix("codex-approval:")
        .and_then(|value| value.split_once(':'))
        .map(|(generation, _)| generation)
        .filter(|generation| !generation.is_empty())
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
        session_generation: String,
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
    pub next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadReadResponse {
    pub thread: CodexThread,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadConfiguredResponse {
    pub thread: CodexThread,
    pub model: String,
    #[serde(rename = "modelProvider")]
    pub _model_provider: String,
    pub cwd: String,
    pub reasoning_effort: Option<String>,
    pub sandbox: CodexSandboxPolicy,
    #[serde(rename = "approvalPolicy")]
    pub _approval_policy: Value,
    #[serde(rename = "approvalsReviewer")]
    pub _approvals_reviewer: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum CodexSandboxPolicy {
    DangerFullAccess,
    ReadOnly,
    ExternalSandbox,
    WorkspaceWrite,
}

impl CodexSandboxPolicy {
    fn policy_type(&self) -> &str {
        match self {
            Self::DangerFullAccess => "dangerFullAccess",
            Self::ReadOnly => "readOnly",
            Self::ExternalSandbox => "externalSandbox",
            Self::WorkspaceWrite => "workspaceWrite",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(crate) struct InitializeResponse {
    pub codex_home: String,
    pub platform_family: String,
    pub platform_os: String,
    pub user_agent: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub started_at_ms: i64,
    pub command: Option<String>,
    #[serde(rename = "cwd")]
    pub _cwd: Option<String>,
    pub reason: Option<String>,
    #[serde(rename = "approvalId")]
    pub _approval_id: Option<String>,
    pub available_decisions: Option<Vec<Value>>,
    #[serde(rename = "commandActions")]
    pub _command_actions: Option<Vec<Value>>,
    #[serde(rename = "environmentId")]
    pub _environment_id: Option<String>,
    pub kind: Option<String>,
    pub additional_permissions: Option<Value>,
    pub network_approval_context: Option<Value>,
    pub proposed_execpolicy_amendment: Option<Vec<String>>,
    pub proposed_network_policy_amendments: Option<Vec<Value>>,
}

impl CommandApprovalParams {
    pub(crate) fn has_unsupported_semantics(&self) -> bool {
        self.additional_permissions.is_some()
            || self.network_approval_context.is_some()
            || self.proposed_execpolicy_amendment.is_some()
            || self.proposed_network_policy_amendments.is_some()
            || matches!(self.kind.as_deref(), Some(kind) if kind != "command")
            || matches!(
                self.available_decisions.as_deref(),
                Some(decisions)
                    if decisions.len() != 2
                        || !decisions.iter().any(|decision| decision == "accept")
                        || !decisions.iter().any(|decision| decision == "decline")
            )
    }

    pub(crate) fn binary_decisions(&self) -> Vec<String> {
        self.available_decisions
            .as_ref()
            .map(|decisions| {
                decisions
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_else(|| vec!["accept".to_string(), "decline".to_string()])
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(crate) struct FileApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub started_at_ms: i64,
    pub reason: Option<String>,
    pub grant_root: Option<String>,
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

pub(crate) fn permission_settings(permission: CodexPermissionLevel) -> (&'static str, &'static str) {
    match permission {
        CodexPermissionLevel::ReadOnly => ("read-only", "on-request"),
        CodexPermissionLevel::WorkspaceWrite => ("workspace-write", "on-request"),
        CodexPermissionLevel::FullAccess => ("danger-full-access", "never"),
    }
}

pub(crate) fn permission_from_sandbox(
    sandbox: &CodexSandboxPolicy,
) -> Option<CodexPermissionLevel> {
    match sandbox.policy_type() {
        "readOnly" => Some(CodexPermissionLevel::ReadOnly),
        "workspaceWrite" => Some(CodexPermissionLevel::WorkspaceWrite),
        "dangerFullAccess" => Some(CodexPermissionLevel::FullAccess),
        _ => None,
    }
}
