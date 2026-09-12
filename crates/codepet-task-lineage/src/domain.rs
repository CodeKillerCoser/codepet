use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub event_id: String,
    pub file: String,
    pub byte_offset: u64,
    pub generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub evidence: Evidence,
    pub thread_id: String,
    pub role: String,
    pub text: String,
    pub timestamp: Option<String>,
    pub turn_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub offset: u64,
    pub generation: u64,
    pub prefix_hash: String,
    pub prefix_length: usize,
    #[serde(default)]
    pub current_turn_id: Option<String>,
    #[serde(default)]
    pub observed_length: u64,
    #[serde(default)]
    pub modified_nanos: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub created_by: String,
    pub creation_kind: String,
    pub parent_id: Option<String>,
    pub source_file: String,
    pub timestamp: Option<String>,
    #[serde(default)]
    pub inherited_end_byte_offset: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LineageLink {
    pub parent_id: String,
    pub child_id: String,
    pub kind: String,
    pub evidence: Evidence,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBatch {
    pub thread: Thread,
    pub messages: Vec<Message>,
    pub links: Vec<LineageLink>,
    pub cursor: Cursor,
    pub reset: bool,
    pub last_runtime_event: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub id: String,
    pub thread_id: String,
    pub title: String,
    pub evidence_ids: Vec<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub schema_version: u32,
    pub revision: u64,
    pub id: String,
    pub root_thread_id: String,
    pub title: String,
    pub detail: String,
    pub episodes: Vec<Episode>,
    pub edges: Vec<Edge>,
    pub manual_completion: bool,
    #[serde(default)]
    pub completion_watermarks: std::collections::HashMap<String, Cursor>,
}
