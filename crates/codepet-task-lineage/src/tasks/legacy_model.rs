#![allow(dead_code)]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    InProgress,
    ReadyForDelivery,
    Published,
}

// Read existing ledgers without exposing the old six-stage workflow to new clients.
impl<'de> Deserialize<'de> for Phase {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "in_progress" | "investigating" | "planning" | "implementing" | "validating_local" => {
                Ok(Self::InProgress)
            }
            "ready_for_delivery"
            | "ready_to_publish"
            | "awaiting_acceptance"
            | "ready_for_release" => Ok(Self::ReadyForDelivery),
            "published" | "validating_release" => Ok(Self::Published),
            _ => Err(serde::de::Error::custom(
                "任务只能设为进行中、待交付或已发布；结束由用户操作",
            )),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Local,
    Release,
    #[default]
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestResult {
    Passed,
    Failed,
    NotTested,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Criterion {
    /// Omit for a new item; reuse its returned ID when revising.
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub precondition: String,
    pub steps: String,
    pub expected: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default = "yes")]
    pub required: bool,
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanInput {
    pub summary: String,
    #[serde(default)]
    pub affected_areas: Vec<String>,
    pub criteria: Vec<Criterion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckResult {
    pub criterion_id: String,
    pub result: TestResult,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationInput {
    pub plan_revision: u64,
    /// local or release (both is not a test execution stage).
    pub stage: Scope,
    /// The actual tested worktree/commit or installed version. Never just say "latest".
    pub target: String,
    pub environment: String,
    pub results: Vec<CheckResult>,
}

/// A single milestone write. No polling, per-message updates, or repeated full-task submission.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveInput {
    /// Omit to create. Keep the returned taskId for further milestones.
    pub task_id: Option<String>,
    /// Required on updates. Reuse on a retry; conflicts do not overwrite user edits.
    pub expected_revision: Option<u64>,
    /// Optional retry key; otherwise derived from this exact request.
    pub operation_id: Option<String>,
    /// Absolute project/worktree directory. Required on create; omit on updates if unchanged.
    pub workspace: Option<String>,
    pub title: Option<String>,
    pub objective: Option<String>,
    pub description: Option<String>,
    /// bug, feature, or investigation. Defaults to feature.
    pub kind: Option<String>,
    /// Actual Codex UUID if known; omit rather than inventing one.
    pub thread_id: Option<String>,
    /// in_progress (default), ready_for_delivery after implementation, published with an explicit delivery reference. Only a human closes a task.
    pub phase: Option<Phase>,
    /// A short milestone summary, not a transcript.
    pub note: Option<String>,
    /// Explicitly associated commit SHAs only. Never all commits on the branch.
    #[serde(default)]
    pub commits: Vec<String>,
    pub plan: Option<PlanInput>,
    pub verification: Option<VerificationInput>,
    /// Only for a previously closed task. Reason for a new handling round.
    pub reopen_reason: Option<String>,
    /// Optional build/release URL or version. Registration is not proof of deployment.
    pub delivery: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindInput {
    /// Return the current round and checklist for this task; otherwise compact search results.
    pub task_id: Option<String>,
    pub workspace: Option<String>,
    pub query: Option<String>,
    /// Opt in to historical round summaries. Defaults to false.
    #[serde(default)]
    pub history: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub thread_id: String,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeObservation {
    pub workspace: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty: Option<bool>,
    pub upstream: Option<String>,
    pub commits: Vec<CommitRef>,
    pub observed_at: i64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRef {
    pub sha: String,
    /// Only checks the locally cached upstream ref; never implies a live remote query.
    pub in_cached_upstream: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: u64,
    pub created_at: i64,
    #[serde(flatten)]
    pub input: PlanInput,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub id: String,
    pub actor: String,
    pub created_at: i64,
    pub workspace_observation: Option<CodeObservation>,
    #[serde(flatten)]
    pub input: VerificationInput,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Closure {
    pub outcome: String,
    pub reason: String,
    pub actor: String,
    pub closed_at: i64,
    pub verification_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub at: i64,
    pub actor: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    pub id: String,
    pub number: usize,
    pub opened_at: i64,
    pub opened_reason: String,
    pub previous_round_id: Option<String>,
    pub objective: String,
    pub description: String,
    pub phase: Phase,
    pub sources: Vec<SourceRef>,
    pub code_refs: Vec<CodeObservation>,
    pub plans: Vec<Plan>,
    pub verifications: Vec<Verification>,
    pub deliveries: Vec<String>,
    pub closure: Option<Closure>,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub schema_version: u32,
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub repository_path: String,
    pub workspace: String,
    pub title: String,
    pub objective: String,
    pub description: String,
    pub kind: String,
    pub current_round_id: String,
    pub rounds: Vec<Round>,
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub task_id: String,
    pub round_id: String,
    pub revision: u64,
    pub plan_revision: Option<u64>,
    pub criterion_ids: Vec<String>,
    pub source_linked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}
