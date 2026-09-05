use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
        data: Option<Value>,
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
            Self::Rpc { code, message, .. } => {
                write!(formatter, "codex app-server RPC error {code}: {message}")
            }
            Self::ProcessExited => write!(formatter, "codex app-server process exited"),
            Self::Shutdown => write!(formatter, "codex app-server session is shut down"),
        }
    }
}

impl std::error::Error for CodexAppServerError {}

impl CodexAppServerError {
    pub(crate) fn is_method_not_found(&self) -> bool {
        matches!(self, Self::Rpc { code: -32601, .. })
    }

    pub(crate) fn is_thread_not_loaded(&self, thread_id: &str) -> bool {
        matches!(
            self,
            Self::Rpc {
                code: -32600,
                message,
                ..
            } if message == &format!("thread not loaded: {thread_id}")
        )
    }

    pub(crate) fn is_thread_turns_unavailable_before_first_user_message(
        &self,
        thread_id: &str,
    ) -> bool {
        matches!(
            self,
            Self::Rpc {
                code: -32600,
                message,
                ..
            } if message == &format!(
                "thread {thread_id} is not materialized yet; thread/turns/list is unavailable before first user message"
            )
        )
    }
}

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
    pub project_id: Option<String>,
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
    #[serde(default)]
    pub items_view: CodexTurnItemsView,
    pub items: Vec<CodexThreadItem>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexTurnItemsView {
    NotLoaded,
    Summary,
    #[default]
    Full,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodexThreadItem {
    UserMessage {
        id: String,
        text_inputs: Vec<Option<String>>,
    },
    AgentMessage {
        id: String,
        text: String,
    },
    Plan {
        id: String,
        text: String,
    },
    Reasoning {
        id: String,
        summary: Vec<String>,
    },
    CommandExecution {
        id: String,
        command: String,
        status: String,
        aggregated_output: Option<String>,
        cwd: Option<String>,
        duration_ms: Option<u64>,
        exit_code: Option<i64>,
        process_id: Option<String>,
        command_actions: Vec<CodexCommandAction>,
    },
    FileChange {
        id: String,
        status: String,
        change_count: usize,
    },
    ToolActivity {
        id: String,
        title: String,
        status: Option<String>,
        details: Option<CodexToolDetails>,
    },
    Unknown {
        id: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexCommandAction {
    pub kind: String,
    pub command: String,
    pub name: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexToolDetails {
    pub name: String,
    pub namespace: Option<String>,
    pub origin_kind: String,
    pub origin_name: Option<String>,
    pub input: Value,
    pub result: Option<Value>,
}

impl CodexThreadItem {
    pub fn id(&self) -> &str {
        match self {
            Self::UserMessage { id, .. }
            | Self::AgentMessage { id, .. }
            | Self::Plan { id, .. }
            | Self::Reasoning { id, .. }
            | Self::CommandExecution { id, .. }
            | Self::FileChange { id, .. }
            | Self::ToolActivity { id, .. }
            | Self::Unknown { id } => id,
        }
    }
}

impl<'de> Deserialize<'de> for CodexThreadItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        parse_thread_item(&value).map_err(D::Error::custom)
    }
}

fn parse_thread_item(value: &Value) -> Result<CodexThreadItem, String> {
    let id = thread_item_string(value, "id")?;
    let item_type = thread_item_string(value, "type")?;
    match item_type.as_str() {
        "userMessage" => {
            let content = value
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| "userMessage item is missing content".to_string())?;
            let mut text_inputs = Vec::with_capacity(content.len());
            for input in content {
                let text = match input.get("type").and_then(Value::as_str) {
                    Some("text") => Some(
                        input
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                "userMessage text input is missing text".to_string()
                            })?
                            .to_string(),
                    ),
                    _ => None,
                };
                text_inputs.push(text);
            }
            Ok(CodexThreadItem::UserMessage { id, text_inputs })
        }
        "agentMessage" => Ok(CodexThreadItem::AgentMessage {
            id,
            text: thread_item_text(value, "text")?,
        }),
        "plan" => Ok(CodexThreadItem::Plan {
            id,
            text: thread_item_string(value, "text")?,
        }),
        "reasoning" => {
            let summary = value
                .get("summary")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .map(|part| {
                            part.as_str().map(str::to_string).ok_or_else(|| {
                                "reasoning summary contains a non-string value".to_string()
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(CodexThreadItem::Reasoning { id, summary })
        }
        "commandExecution" => Ok(CodexThreadItem::CommandExecution {
            id,
            command: thread_item_string(value, "command")?,
            status: thread_item_string(value, "status")?,
            aggregated_output: optional_thread_item_string(value, "aggregatedOutput")?,
            cwd: optional_thread_item_string(value, "cwd")?,
            duration_ms: value.get("durationMs").and_then(Value::as_u64),
            exit_code: value.get("exitCode").and_then(Value::as_i64),
            process_id: value
                .get("processId")
                .and_then(|value| value.as_str().map(str::to_string).or_else(|| value.as_i64().map(|value| value.to_string()))),
            command_actions: value
                .get("commandActions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(parse_command_action)
                .collect(),
        }),
        "fileChange" => Ok(CodexThreadItem::FileChange {
            id,
            status: thread_item_string(value, "status")?,
            change_count: value
                .get("changes")
                .and_then(Value::as_array)
                .ok_or_else(|| "fileChange item is missing changes".to_string())?
                .len(),
        }),
        "mcpToolCall" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: format!(
                "{}/{}",
                thread_item_string(value, "server")?,
                thread_item_string(value, "tool")?
            ),
            status: Some(thread_item_string(value, "status")?),
            details: Some(CodexToolDetails {
                name: thread_item_string(value, "tool")?,
                namespace: None,
                origin_kind: "mcp".to_string(),
                origin_name: Some(thread_item_string(value, "server")?),
                input: value.get("arguments").cloned().unwrap_or_else(|| Value::Object(Default::default())),
                result: value.get("result").cloned(),
            }),
        }),
        "dynamicToolCall" => {
            let tool = thread_item_string(value, "tool")?;
            let title = optional_thread_item_string(value, "namespace")?
                .map(|namespace| format!("{namespace}/{tool}"))
                .unwrap_or_else(|| tool.clone());
            Ok(CodexThreadItem::ToolActivity {
                id,
                title,
                status: Some(thread_item_string(value, "status")?),
                details: Some(CodexToolDetails {
                    name: tool,
                    namespace: optional_thread_item_string(value, "namespace")?,
                    origin_kind: "custom".to_string(),
                    origin_name: None,
                    input: value.get("arguments").cloned().unwrap_or_else(|| Value::Object(Default::default())),
                    result: value.get("result").cloned(),
                }),
            })
        }
        "functionCallOutput" => {
            let name = thread_item_string(value, "name")?;
            let title = optional_thread_item_string(value, "namespace")?
                .map(|namespace| format!("{namespace}/{name}"))
                .unwrap_or_else(|| name.clone());
            Ok(CodexThreadItem::ToolActivity {
                id,
                title,
                status: Some("completed".to_string()),
                details: Some(CodexToolDetails {
                    name,
                    namespace: optional_thread_item_string(value, "namespace")?,
                    origin_kind: "custom".to_string(),
                    origin_name: None,
                    input: Value::Object(Default::default()),
                    result: value.get("output").cloned(),
                }),
            })
        }
        "collabAgentToolCall" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: thread_item_string(value, "tool")?,
            status: Some(thread_item_string(value, "status")?),
            details: None,
        }),
        "subAgentActivity" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Sub-agent activity".to_string(),
            status: Some("completed".to_string()),
            details: None,
        }),
        "webSearch" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Web search".to_string(),
            status: Some("completed".to_string()),
            details: None,
        }),
        "imageView" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Image view".to_string(),
            status: Some("completed".to_string()),
            details: None,
        }),
        "sleep" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Wait".to_string(),
            status: optional_thread_item_string(value, "status")?
                .or_else(|| Some("completed".to_string())),
            details: None,
        }),
        "imageGeneration" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Image generation".to_string(),
            status: optional_thread_item_string(value, "status")?
                .or_else(|| Some("completed".to_string())),
            details: None,
        }),
        "enteredReviewMode" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Entered review mode".to_string(),
            status: Some("completed".to_string()),
            details: None,
        }),
        "exitedReviewMode" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Exited review mode".to_string(),
            status: Some("completed".to_string()),
            details: None,
        }),
        "contextCompaction" => Ok(CodexThreadItem::ToolActivity {
            id,
            title: "Context compaction".to_string(),
            status: Some("completed".to_string()),
            details: None,
        }),
        _ => Ok(CodexThreadItem::Unknown { id }),
    }
}

fn parse_command_action(value: &Value) -> Option<CodexCommandAction> {
    let command = value.get("command")?.as_str()?.to_string();
    Some(CodexCommandAction {
        kind: value
            .get("type")
            .or_else(|| value.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        command,
        name: value.get("name").and_then(Value::as_str).map(str::to_string),
        path: value.get("path").and_then(Value::as_str).map(str::to_string),
        query: value.get("query").and_then(Value::as_str).map(str::to_string),
    })
}

fn thread_item_string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("thread item is missing {key}"))
}

fn thread_item_text(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("thread item is missing {key}"))
}

fn optional_thread_item_string(value: &Value, key: &str) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("thread item {key} is not a string or null")),
    }
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

    pub(crate) fn from_threads(threads: Vec<CodexThread>) -> Vec<Self> {
        threads
            .into_iter()
            .map(|thread| {
                let workspace_root = Some(thread.cwd.clone());
                Self {
                    thread,
                    workspace_root,
                    permission_level: None,
                    model: None,
                    reasoning_effort: None,
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexThreadListRequest {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub project_id: Option<Option<String>>,
    pub workspace_root: Option<String>,
    pub search_term: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexThreadPage {
    pub data: Vec<CodexConversationSnapshot>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexProject {
    pub id: String,
    pub name: String,
    pub roots: Vec<CodexProjectRoot>,
    pub metadata: BTreeMap<String, String>,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodexProjectRoot {
    pub path: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexProjectCreateRequest {
    pub idempotency_key: String,
    pub name: String,
    pub roots: Vec<CodexProjectRoot>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexProjectUpdateRequest {
    pub project_id: String,
    pub name: Option<String>,
    pub roots: Option<Vec<CodexProjectRoot>>,
    pub metadata: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexProjectPage {
    pub data: Vec<CodexProject>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexProjectChangeType {
    Created,
    Updated,
    Deleted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexTurnPage {
    pub data: Vec<CodexTurn>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexThreadItemEntry {
    pub turn_id: String,
    pub item: CodexThreadItem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexThreadItemPage {
    pub data: Vec<CodexThreadItemEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexModel {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub is_default: bool,
    pub default_reasoning_effort: String,
    pub supported_reasoning_efforts: Vec<CodexReasoningEffortOption>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexReasoningEffortOption {
    pub reasoning_effort: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexModelListResponse {
    pub data: Vec<CodexModel>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexThreadStartRequest {
    pub workspace_root: Option<String>,
    pub project_id: Option<String>,
    pub permission_level: CodexPermissionLevel,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexTurnStartRequest {
    pub thread_id: String,
    pub message: String,
    pub client_message_id: Option<String>,
    pub permission_level: Option<CodexPermissionLevel>,
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
    ProjectChanged {
        project_id: String,
        change_type: CodexProjectChangeType,
    },
    ThreadStarted {
        snapshot: CodexConversationSnapshot,
    },
    ThreadNameUpdated {
        thread_id: String,
        thread_name: Option<String>,
    },
    ThreadStatusChanged {
        thread_id: String,
        status: CodexThreadStatus,
    },
    TurnStarted {
        thread_id: String,
        turn: CodexTurn,
    },
    TurnCompleted {
        thread_id: String,
        turn: CodexTurn,
    },
    ItemUpserted {
        thread_id: String,
        turn_id: String,
        turn_status: CodexTurnStatus,
        item: CodexThreadItem,
    },
    OutputDelta {
        native_method: String,
        thread_id: String,
        turn_id: String,
        item_id: String,
        content_id: String,
        kind: CodexContentKind,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexContentKind {
    Text,
    ReasoningSummary,
    Output,
}

pub fn user_input_content_id(item_id: &str, index: usize) -> String {
    format!("{item_id}:input:{index}")
}

pub fn text_content_id(item_id: &str) -> String {
    format!("{item_id}:text")
}

pub fn reasoning_summary_content_id(item_id: &str, index: usize) -> String {
    format!("{item_id}:summary:{index}")
}

pub fn output_content_id(item_id: &str) -> String {
    format!("{item_id}:output")
}

pub fn activity_summary_content_id(item_id: &str) -> String {
    format!("{item_id}:summary")
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
    #[serde(rename = "backwardsCursor")]
    pub _backwards_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectListResponse {
    pub data: Vec<CodexProject>,
    pub next_cursor: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ProjectResponse {
    pub project: CodexProject,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadReadResponse {
    pub thread: CodexThread,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadTurnsListResponse {
    pub data: Vec<CodexTurn>,
    pub next_cursor: Option<String>,
    #[serde(default, rename = "backwardsCursor")]
    pub _backwards_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemsListResponse {
    pub data: Vec<CodexThreadItemEntry>,
    pub next_cursor: Option<String>,
    #[serde(default, rename = "backwardsCursor")]
    pub _backwards_cursor: Option<String>,
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

#[derive(Clone, Deserialize)]
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
    if let Some(project_id) = &request.project_id {
        params.insert("projectId".to_string(), json!(project_id));
    }
    if let Some(cwd) = &request.workspace_root {
        params.insert("cwd".to_string(), json!(cwd));
    }
    if let Some(search_term) = &request.search_term {
        params.insert("searchTerm".to_string(), json!(search_term));
    }
    params.insert("sortKey".to_string(), json!("updated_at"));
    params.insert("sortDirection".to_string(), json!("desc"));
    params.insert("useStateDbOnly".to_string(), json!(true));
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
    if let Some(project_id) = &request.project_id {
        params.insert("projectId".to_string(), json!(project_id));
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
    if let Some(permission_level) = request.permission_level {
        let (_, approval_policy) = permission_settings(permission_level);
        let sandbox_policy = match permission_level {
            CodexPermissionLevel::ReadOnly => json!({ "type": "readOnly" }),
            CodexPermissionLevel::WorkspaceWrite => json!({ "type": "workspaceWrite" }),
            CodexPermissionLevel::FullAccess => json!({ "type": "dangerFullAccess" }),
        };
        params.insert("sandboxPolicy".to_string(), sandbox_policy);
        params.insert("approvalPolicy".to_string(), json!(approval_policy));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_only_the_exact_unmaterialized_thread_turns_error() {
        let error = CodexAppServerError::Rpc {
            code: -32600,
            message: "thread thread-new is not materialized yet; thread/turns/list is unavailable before first user message".to_string(),
            data: None,
        };

        assert!(error.is_thread_turns_unavailable_before_first_user_message("thread-new"));
        assert!(!error.is_thread_turns_unavailable_before_first_user_message("thread-other"));
        assert!(!CodexAppServerError::Rpc {
            code: -32603,
            message: "thread thread-new is not materialized yet; thread/turns/list is unavailable before first user message".to_string(),
            data: None,
        }
        .is_thread_turns_unavailable_before_first_user_message("thread-new"));
    }

    #[test]
    fn classifies_only_the_exact_thread_not_loaded_error() {
        let error = CodexAppServerError::Rpc {
            code: -32600,
            message: "thread not loaded: thread-new".to_string(),
            data: None,
        };

        assert!(error.is_thread_not_loaded("thread-new"));
        assert!(!error.is_thread_not_loaded("thread-other"));
        assert!(!CodexAppServerError::Rpc {
            code: -32603,
            message: "thread not loaded: thread-new".to_string(),
            data: None,
        }
        .is_thread_not_loaded("thread-new"));
    }

    #[test]
    fn thread_list_params_use_state_db_and_updated_at_order_for_list_and_search() {
        let list = thread_list_params(&CodexThreadListRequest::default());
        assert_eq!(list["sortKey"], "updated_at");
        assert_eq!(list["sortDirection"], "desc");
        assert_eq!(list["useStateDbOnly"], true);
        assert!(list.get("searchTerm").is_none());
        assert!(list.get("projectId").is_none());

        let standalone = thread_list_params(&CodexThreadListRequest {
            project_id: Some(None),
            ..CodexThreadListRequest::default()
        });
        assert_eq!(standalone["projectId"], Value::Null);

        let project = thread_list_params(&CodexThreadListRequest {
            project_id: Some(Some("project-one".to_string())),
            ..CodexThreadListRequest::default()
        });
        assert_eq!(project["projectId"], "project-one");

        let search = thread_list_params(&CodexThreadListRequest {
            search_term: Some("gateway protocol".to_string()),
            ..CodexThreadListRequest::default()
        });
        assert_eq!(search["searchTerm"], "gateway protocol");
        assert_eq!(search["sortKey"], "updated_at");
        assert_eq!(search["sortDirection"], "desc");
        assert_eq!(search["useStateDbOnly"], true);
    }

    #[test]
    fn thread_item_decoder_keeps_safe_history_fields_and_drops_raw_reasoning() {
        let turn: CodexTurn = serde_json::from_value(json!({
            "id": "turn-one",
            "itemsView": "full",
            "status": "completed",
            "startedAt": 1,
            "completedAt": 2,
            "items": [
                {
                    "type": "userMessage",
                    "id": "user-one",
                    "content": [
                        { "type": "text", "text": "hello" },
                        { "type": "image", "imageUrl": "private" }
                    ]
                },
                {
                    "type": "reasoning",
                    "id": "reasoning-one",
                    "summary": ["safe summary"],
                    "content": ["private raw reasoning"]
                },
                {
                    "type": "futureItem",
                    "id": "future-one",
                    "privatePayload": "private"
                }
            ]
        }))
        .unwrap();

        assert_eq!(turn.items_view, CodexTurnItemsView::Full);
        assert_eq!(
            turn.items[0],
            CodexThreadItem::UserMessage {
                id: "user-one".to_string(),
                text_inputs: vec![Some("hello".to_string()), None]
            }
        );
        assert_eq!(
            turn.items[1],
            CodexThreadItem::Reasoning {
                id: "reasoning-one".to_string(),
                summary: vec!["safe summary".to_string()]
            }
        );
        assert_eq!(
            turn.items[2],
            CodexThreadItem::Unknown {
                id: "future-one".to_string()
            }
        );
    }

    #[test]
    fn content_ids_are_stable_and_channel_specific() {
        assert_eq!(user_input_content_id("item-one", 2), "item-one:input:2");
        assert_eq!(text_content_id("item-one"), "item-one:text");
        assert_eq!(
            reasoning_summary_content_id("item-one", 1),
            "item-one:summary:1"
        );
        assert_eq!(output_content_id("item-one"), "item-one:output");
        assert_ne!(
            reasoning_summary_content_id("item-one", 0),
            output_content_id("item-one")
        );
    }
}
