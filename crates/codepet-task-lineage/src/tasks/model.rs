//! Canonical v6 records in the stable task/run ledger. Transport inputs cannot supply authors or observed Git facts.
pub use super::legacy_model::{CheckResult, Criterion, Plan, Scope, TestResult, VerificationInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Bugfix,
    #[default]
    Feature,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    InProgress,
    ReadyForDelivery,
    Published,
    Closed,
}

#[derive(Clone, Debug)]
pub struct Actor {
    source: String,
    human: bool,
}
impl Actor {
    pub fn agent(source: &str) -> Result<Self, String> {
        if source.is_empty()
            || source.len() > 64
            || !source
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        {
            return Err("X-CodePet-Agent 必须是 1–64 位字母、数字、点、下划线或短横线".into());
        }
        let source = source.to_ascii_lowercase();
        if ["human", "user", "agent", "unknown"].contains(&source.as_str()) {
            return Err("Agent 来源不能使用保留名称".into());
        }
        Ok(Self {
            source,
            human: false,
        })
    }
    pub fn human() -> Self {
        Self {
            source: "human".into(),
            human: true,
        }
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn is_human(&self) -> bool {
        self.human
    }
    pub fn role(&self) -> &str {
        if self.human {
            "human"
        } else {
            "agent"
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeStatus {
    pub has_uncommitted_changes: Option<bool>,
    pub has_unpushed_commits: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub change_type: String,
    pub staged: bool,
    pub original_path: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitChange {
    pub commit: String,
    pub files: Vec<ChangedFile>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspace {
    pub path: String,
    /// Canonical Git common directory, retained per run for cross-project evidence recovery.
    #[serde(default)]
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub change_status: ChangeStatus,
    pub changed_files: Vec<ChangedFile>,
    pub related_commits: Vec<String>,
    #[serde(default)]
    pub commit_changes: Vec<CommitChange>,
    /// Explicit files owned by this run; None means an older record, Some([]) means no code changes.
    #[serde(default)]
    pub run_files: Option<Vec<String>>,
    pub observed_at: i64,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConversation {
    pub agent: String,
    pub conversation_id: String,
    pub verified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub id: String,
    pub actor: String,
    pub agent: String,
    pub created_at: i64,
    pub agent_workspace: Option<AgentWorkspace>,
    #[serde(flatten)]
    pub input: VerificationInput,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationPlan {
    pub revision: u64,
    pub items: Vec<Criterion>,
    pub records: Vec<Verification>,
    // Immutable checklist snapshots support historical verification; hidden in the default UI.
    pub revisions: Vec<Plan>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub id: String,
    pub at: i64,
    pub actor: String,
    pub kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_workspace: Option<AgentWorkspace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_conversation: Vec<AgentConversation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<ActivityChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityChange {
    pub field: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Closure {
    pub accepted: bool,
    pub reason: String,
    pub closed_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub goal: String,
    pub solution: String,
    pub description: String,
    pub status: RunStatus,
    pub validation_plan: ValidationPlan,
    pub agent_workspace: AgentWorkspace,
    pub agent_conversation: Vec<AgentConversation>,
    pub timeline: Vec<TimelineEvent>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub closure: Option<Closure>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub description: String,
    pub project_id: String,
    pub created_by: String,
    pub created_at: i64,
    pub runs: Vec<TaskRun>,
    /// Human Task completion is independent of the statuses of its historical runs.
    #[serde(default)]
    pub completion: Option<Closure>,
    #[serde(default)]
    pub status_aggregation_version: u32,
    pub revision: u64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceInput {
    /// This run's absolute existing workspace; it may belong to another project. Rust reads Git facts.
    pub path: String,
    /// Only commits explicitly associated with this task; never all branch commits.
    #[serde(default)]
    pub related_commits: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChecklistInput {
    pub items: Vec<Criterion>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InitialRunInput {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub solution: String,
    #[serde(default)]
    pub description: String,
    pub validation_plan: Option<ChecklistInput>,
    /// Required actual conversation IDs or codex://threads/<id> links. Never invent an ID.
    #[schemars(length(min = 1, max = 20), inner(length(min = 1, max = 220)))]
    pub agent_conversation: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTaskInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub agent_workspace: WorkspaceInput,
    pub initial_run: InitialRunInput,
    /// Caller-generated retry key. Use the same value with identical arguments to retry.
    pub operation_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTaskRunInput {
    pub title: String,
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub solution: String,
    pub task_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub description: String,
    pub agent_workspace: Option<WorkspaceInput>,
    pub validation_plan: Option<ChecklistInput>,
    #[schemars(length(min = 1, max = 20), inner(length(min = 1, max = 220)))]
    pub agent_conversation: Vec<String>,
    /// Caller-generated retry key. Use the same value with identical arguments to retry.
    pub operation_id: Option<String>,
}
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AgentRunStatus {
    InProgress,
    ReadyForDelivery,
    Published,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateTaskRunInput {
    pub title: Option<String>,
    pub kind: Option<TaskKind>,
    pub goal: Option<String>,
    pub solution: Option<String>,
    pub task_id: String,
    pub run_id: String,
    pub expected_revision: u64,
    pub description: Option<String>,
    /// Development or delivery status. Only the user can close a run in the App.
    #[schemars(with = "Option<AgentRunStatus>")]
    pub status: Option<RunStatus>,
    pub agent_workspace: Option<WorkspaceInput>,
    pub validation_plan: Option<ChecklistInput>,
    pub verification: Option<VerificationInput>,
    #[schemars(length(min = 1, max = 20), inner(length(min = 1, max = 220)))]
    pub agent_conversation: Vec<String>,
    /// Actual build/release reference. Required when first marking published.
    pub delivery: Option<String>,
    pub note: Option<String>,
    /// Caller-generated retry key. Use the same value with identical arguments to retry.
    pub operation_id: Option<String>,
}
/// Complete development in one atomic write; never records human acceptance.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinishTaskRunInput {
    pub task_id: String,
    pub run_id: String,
    pub expected_revision: u64,
    #[schemars(length(min = 1, max = 2000))]
    pub conclusion: String,
    /// Files changed by this run, relative to the repository. Required, including [] for no code changes.
    #[schemars(length(max = 300), inner(length(min = 1, max = 1024)))]
    pub changed_files: Vec<String>,
    #[schemars(length(min = 1, max = 20), inner(length(min = 1, max = 220)))]
    pub agent_conversation: Vec<String>,
    pub verification: VerificationInput,
    pub agent_workspace: Option<WorkspaceInput>,
    /// Caller-generated retry key. Use the same value with identical arguments to retry.
    pub operation_id: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindTaskRunsInput {
    /// Read the current run of this topic when runId is omitted.
    pub task_id: Option<String>,
    /// Read one exact run, including historical runs; taskId is optional.
    pub run_id: Option<String>,
    /// Prefer this workspace in ranked results, without excluding other workspaces.
    pub workspace: Option<String>,
    /// Keywords or a short description of the work to resume.
    pub query: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub history: bool,
    #[schemars(skip)]
    pub activity_after: Option<String>,
    #[schemars(skip)]
    pub activity_before: Option<String>,
    #[schemars(skip)]
    pub activity_limit: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadTaskRunDetailsInput {
    /// Read an exact run. Use taskId only when requesting the current run.
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    #[serde(flatten)]
    pub detail: RunDetail,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "section", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunDetail {
    Context,
    Activity {
        #[schemars(range(min = 1, max = 20))]
        limit: Option<usize>,
        before: Option<String>,
        after: Option<String>,
    },
    Git {
        /// Omit to page through observed workspace files; supply a listed SHA to page its files.
        commit: Option<String>,
        offset: Option<usize>,
    },
    Verification {
        /// Read records newest first in the current plan, or select an older plan explicitly.
        #[serde(rename = "planRevision")]
        plan_revision: Option<u64>,
        offset: Option<usize>,
    },
    History {
        offset: Option<usize>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetTaskOperationInput {
    /// Only needed when the same ID was reused with different write tools.
    pub operation: Option<TaskOperation>,
    pub operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TaskOperation {
    CreateTask,
    CreateTaskRun,
    UpdateTaskRun,
    FinishTaskRun,
    CommentTaskRun,
}

impl TaskOperation {
    pub(super) const ALL: [Self; 5] = [
        Self::CreateTask,
        Self::CreateTaskRun,
        Self::UpdateTaskRun,
        Self::FinishTaskRun,
        Self::CommentTaskRun,
    ];

    pub(super) fn tool_name(&self) -> &'static str {
        match self {
            Self::CreateTask => "create-task",
            Self::CreateTaskRun => "create-task-run",
            Self::UpdateTaskRun => "update-task-run",
            Self::FinishTaskRun => "finish-task-run",
            Self::CommentTaskRun => "comment-task-run",
        }
    }
}

/// Append a comment or reply, including on a historical/closed run. Never changes development state.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommentTaskRunInput {
    pub task_id: String,
    pub run_id: String,
    #[schemars(length(min = 1, max = 4000))]
    pub comment: String,
    pub reply_to: Option<String>,
    #[schemars(length(min = 1, max = 20), inner(length(min = 1, max = 220)))]
    pub agent_conversation: Vec<String>,
    /// Caller-generated retry key. Use the same value with identical arguments to retry.
    pub operation_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub task_id: String,
    pub run_id: String,
    pub revision: u64,
    pub plan_revision: u64,
    pub criterion_ids: Vec<String>,
    pub status: RunStatus,
    pub created_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
}
