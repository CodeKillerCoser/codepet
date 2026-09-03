use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

pub const OPENCODE_PLUGIN_ID: &str = "dev.codepet.opencode";
pub const OPENCODE_INSTANCE_KIND: &str = "opencode";
pub const OPENCODE_VERIFIED_SERVER_VERSION: &str = "1.18.25";
pub const OPENCODE_PERMISSION_LEVEL: &str = "opencode-default";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenCodeServerError {
    Spawn(String),
    Io(String),
    Timeout(String),
    Protocol(String),
    Http {
        status: u16,
        message: String,
    },
    ProcessExited(String),
    Shutdown,
}

impl fmt::Display for OpenCodeServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "failed to start OpenCode Server: {message}"),
            Self::Io(message) => write!(formatter, "OpenCode Server I/O error: {message}"),
            Self::Timeout(message) => write!(formatter, "OpenCode Server timeout: {message}"),
            Self::Protocol(message) => write!(formatter, "OpenCode Server protocol error: {message}"),
            Self::Http { status, message } => {
                write!(formatter, "OpenCode Server HTTP {status}: {message}")
            }
            Self::ProcessExited(message) => write!(formatter, "OpenCode Server exited: {message}"),
            Self::Shutdown => write!(formatter, "OpenCode Server session is shut down"),
        }
    }
}

impl std::error::Error for OpenCodeServerError {}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeHealth {
    pub healthy: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeSession {
    pub id: String,
    #[serde(rename = "parentID")]
    pub parent_id: Option<String>,
    pub agent: Option<String>,
    pub model: Option<OpenCodeModelRef>,
    pub time: OpenCodeSessionTime,
    pub title: String,
    pub location: OpenCodeLocationRef,
}

impl OpenCodeSession {
    pub fn workspace_root(&self) -> String {
        self.location.directory.clone()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeModelRef {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeLocationRef {
    pub directory: String,
    #[serde(rename = "workspaceID", skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeSessionTime {
    pub created: u64,
    pub updated: u64,
    pub archived: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeDataResponse<T> {
    pub data: T,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeSessionPage {
    pub data: Vec<OpenCodeSession>,
    pub cursor: OpenCodeCursors,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeMessagePage {
    pub data: Vec<OpenCodeMessage>,
    pub cursor: OpenCodeCursors,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub time: OpenCodeMessageTime,
    pub text: Option<String>,
    pub content: Option<Vec<OpenCodeMessageContent>>,
    pub command: Option<String>,
    pub output: Option<String>,
    pub summary: Option<String>,
    pub recent: Option<String>,
    pub finish: Option<String>,
    pub error: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeMessageTime {
    pub created: u64,
    pub completed: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeMessageContent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "callID")]
    pub call_id: Option<String>,
    pub state: Option<Value>,
    pub time: Option<OpenCodeMessageTime>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeCursors {
    pub previous: Option<String>,
    pub next: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum OpenCodeActiveSession {
    #[serde(rename = "running")]
    Running,
}

pub type OpenCodeActiveSessions = HashMap<String, OpenCodeActiveSession>;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeSessionCreate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<OpenCodeModelRef>,
    pub location: OpenCodeLocationRef,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeCatalogResponse<T> {
    pub data: Vec<T>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeAgent {
    pub id: String,
    pub description: Option<String>,
    pub mode: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeModel {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    pub name: String,
    pub status: String,
    pub enabled: bool,
    pub variants: Vec<OpenCodeModelVariant>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeModelVariant {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeProvider {
    pub id: String,
    pub name: String,
    pub disabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct OpenCodeAgentSwitch {
    pub agent: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct OpenCodeModelSwitch {
    pub model: OpenCodeModelRef,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum OpenCodeDelivery {
    #[serde(rename = "queue")]
    Queue,
    #[serde(rename = "steer")]
    Steer,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePromptRequest {
    pub id: String,
    pub prompt: OpenCodePrompt,
    pub delivery: OpenCodeDelivery,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OpenCodePrompt {
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePromptAdmission {
    pub admitted_seq: u64,
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub prompt: OpenCodePrompt,
    pub delivery: String,
    pub time_created: u64,
    pub promoted_seq: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum OpenCodePermissionReply {
    #[serde(rename = "once")]
    Once,
    #[serde(rename = "reject")]
    Reject,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct OpenCodePermissionReplyRequest {
    pub reply: OpenCodePermissionReply,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpenCodeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub data: Value,
    pub location: Option<OpenCodeLocationRef>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePromptAdmittedEventData {
    pub timestamp: u64,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    pub delivery: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeStepStartedEventData {
    pub timestamp: u64,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "assistantMessageID")]
    pub assistant_message_id: String,
    pub agent: String,
    pub model: OpenCodeModelRef,
    pub snapshot: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeDeltaEventData {
    pub timestamp: u64,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "assistantMessageID")]
    pub assistant_message_id: String,
    #[serde(rename = "textID")]
    pub text_id: Option<String>,
    #[serde(rename = "reasoningID")]
    pub reasoning_id: Option<String>,
    pub delta: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeStepEndedEventData {
    pub timestamp: u64,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "assistantMessageID")]
    pub assistant_message_id: String,
    pub finish: String,
    pub cost: f64,
    pub tokens: OpenCodeTokenUsage,
    pub snapshot: Option<String>,
    pub files: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeStepFailedEventData {
    pub timestamp: u64,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "assistantMessageID")]
    pub assistant_message_id: String,
    pub error: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeTokenUsage {
    pub input: f64,
    pub output: f64,
    pub reasoning: f64,
    pub cache: OpenCodeCacheUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeCacheUsage {
    pub read: f64,
    pub write: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePermissionAskedEventData {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub action: String,
    pub resources: Vec<String>,
    pub source: Option<OpenCodePermissionSource>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePermissionSource {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "callID")]
    pub call_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePermissionRepliedEventData {
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "requestID")]
    pub request_id: String,
    pub reply: String,
}
