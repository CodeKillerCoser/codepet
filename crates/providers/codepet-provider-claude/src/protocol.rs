use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClaudeUserMessage<'a> {
    #[serde(rename = "type")]
    pub message_type: &'static str,
    pub uuid: &'a str,
    pub message: ClaudeUserContent<'a>,
    pub parent_tool_use_id: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClaudeUserContent<'a> {
    pub role: &'static str,
    pub content: &'a str,
}

impl<'a> ClaudeUserMessage<'a> {
    pub fn new(uuid: &'a str, content: &'a str) -> Self {
        Self {
            message_type: "user",
            uuid,
            message: ClaudeUserContent {
                role: "user",
                content,
            },
            parent_tool_use_id: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum ClaudeOutput {
    #[serde(rename = "system")]
    System {
        subtype: String,
        #[serde(default)]
        uuid: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        mcp_servers: Vec<Value>,
        #[serde(default)]
        capabilities: Vec<String>,
    },
    #[serde(rename = "stream_event")]
    StreamEvent {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
        event: ClaudeStreamEvent,
    },
    #[serde(rename = "assistant")]
    Assistant {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        aborted: Option<bool>,
    },
    #[serde(rename = "result")]
    Result {
        subtype: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        stop_reason: Option<String>,
        #[serde(default)]
        terminal_reason: Option<String>,
        #[serde(default)]
        usage: Option<Value>,
        #[serde(default)]
        total_cost_usd: Option<f64>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum ClaudeStreamEvent {
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: u64,
        delta: ClaudeStreamDelta,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum ClaudeStreamDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(other)]
    Unknown,
}

pub fn decode_claude_output(line: &[u8]) -> Result<ClaudeOutput, serde_json::Error> {
    serde_json::from_slice(line)
}
