//! Archived v2 reader and validation engine; writes are retained for migration regression fixtures.
#![allow(dead_code, unused_imports)]
pub use super::delivery::{delivery_facts, task_view};
pub use super::legacy_model::*;
use super::source;

use crate::{stable_id, store::Store, workbench, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::InProgress => "进行中",
        Phase::ReadyForDelivery => "待交付",
        Phase::Published => "已发布",
    }
}
pub fn task_status(task: &Task) -> &'static str {
    let round = task.rounds.last().expect("task has a current round");
    if round.closure.is_some() {
        "closed"
    } else {
        match round.phase {
            Phase::InProgress => "in_progress",
            Phase::ReadyForDelivery => "ready_for_delivery",
            Phase::Published => "published",
        }
    }
}
fn value(input: &impl Serialize) -> Result<Value> {
    serde_json::to_value(input).map_err(|e| e.to_string())
}
fn valid_id(input: &str) -> Result<()> {
    uuid::Uuid::parse_str(input)
        .map(|_| ())
        .map_err(|_| "无效的任务或对话 ID".into())
}
fn text(input: &str, name: &str, max: usize) -> Result<()> {
    if input.trim().is_empty() || input.chars().count() > max {
        return Err(format!("{name}不能为空，且不能超过 {max} 字"));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Agent,
    Human,
}
impl Actor {
    fn name(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
        }
    }
}

#[derive(Clone)]
pub struct Ledger {
    root: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    key: String,
    name: String,
    path: String,
}
#[derive(Serialize, Deserialize)]
struct SavedReceipt {
    fingerprint: String,
    receipt: Receipt,
}

struct Context {
    key: String,
    name: String,
    repository: String,
    workspace: String,
    observation: CodeObservation,
}

fn context(path: &str, commits: &[String]) -> Result<Context> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("工作目录必须是绝对路径".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("工作目录不可用：{e}"))?;
    if !canonical.is_dir() {
        return Err("工作目录不是文件夹".into());
    }
    let common = workbench::text_git(
        &canonical,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok();
    let repository = common
        .as_ref()
        .and_then(|s| Path::new(s).canonicalize().ok())
        .unwrap_or(canonical.clone());
    let display_root = if repository.file_name().is_some_and(|n| n == ".git") {
        repository.parent().unwrap_or(&repository)
    } else {
        &repository
    };
    let name = display_root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut observation = CodeObservation {
        workspace: canonical.display().to_string(),
        branch: None,
        head: None,
        dirty: None,
        upstream: None,
        commits: vec![],
        observed_at: now(),
        error: None,
    };
    if common.is_some() {
        observation.branch =
            workbench::text_git(&canonical, &["symbolic-ref", "--short", "-q", "HEAD"]).ok();
        observation.head = workbench::text_git(&canonical, &["rev-parse", "--verify", "HEAD"]).ok();
        observation.upstream =
            workbench::text_git(&canonical, &["rev-parse", "--abbrev-ref", "@{upstream}"]).ok();
        match workbench::text_git(
            &canonical,
            &["status", "--porcelain", "--untracked-files=normal"],
        ) {
            Ok(s) => observation.dirty = Some(!s.is_empty()),
            Err(e) => observation.error = Some(e),
        }
    } else {
        observation.error = Some("目录不是可读取的 Git 仓库，交付状态未知".into());
    }
    if commits.len() > 30 {
        return Err("一次最多登记 30 个关联提交".into());
    }
    for commit in commits {
        if !(7..=64).contains(&commit.len()) || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("关联提交必须是 SHA，不能是分支名或命令参数".into());
        }
        let sha = workbench::text_git(
            &canonical,
            &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
        )
        .map_err(|_| format!("提交 {commit} 在当前仓库中不存在"))?;
        let in_cached_upstream = observation.upstream.as_ref().and_then(|upstream| {
            workbench::text_git(&canonical, &["merge-base", &sha, upstream])
                .ok()
                .map(|base| base == sha)
        });
        if !observation.commits.iter().any(|c| c.sha == sha) {
            observation.commits.push(CommitRef {
                sha,
                in_cached_upstream,
            });
        }
    }
    Ok(Context {
        key: stable_id(repository.to_string_lossy().as_bytes()),
        name,
        repository: repository.display().to_string(),
        workspace: canonical.display().to_string(),
        observation,
    })
}

impl Ledger {
    pub fn new(data: &Path) -> Self {
        Self {
            root: data.join("tasks/v2"),
        }
    }
    pub fn discover() -> Result<Self> {
        Ok(Self::new(
            &codepet_paths::PathManager::discover()
                .map_err(|e| e.to_string())?
                .data,
        ))
    }
    fn store(&self) -> Result<Store> {
        let start = Instant::now();
        loop {
            match Store::open(&self.root) {
                Ok(s) => return Ok(s),
                Err(e) if e.contains("busy") && start.elapsed() < Duration::from_secs(2) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(e),
            }
        }
    }
    fn projects(store: &Store) -> Result<Vec<Project>> {
        Ok(store.read("projects.json")?.unwrap_or_default())
    }
    fn key(project: &str, task: &str) -> String {
        format!("projects/{project}/tasks/{task}.json")
    }
    fn load(store: &Store, task_id: &str) -> Result<Task> {
        valid_id(task_id)?;
        for p in Self::projects(store)? {
            if let Some(t) = store.read::<Task>(&Self::key(&p.id, task_id))? {
                if t.schema_version != 2 {
                    return Err("任务格式版本不受支持".into());
                }
                return Ok(t);
            }
        }
        Err("任务不存在".into())
    }
    pub fn get(&self, task_id: &str) -> Result<Task> {
        Self::load(&self.store()?, task_id)
    }
    pub fn list(&self) -> Result<Vec<Task>> {
        let store = self.store()?;
        let mut tasks = vec![];
        for p in Self::projects(&store)? {
            tasks.extend(store.list::<Task>(&format!("projects/{}/tasks", p.id))?);
        }
        if tasks.iter().any(|t| t.schema_version != 2) {
            return Err("任务格式版本不受支持".into());
        }
        tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        Ok(tasks)
    }
    pub fn find(&self, input: FindInput) -> Result<Value> {
        let scope = input
            .workspace
            .as_deref()
            .map(|w| context(w, &[]))
            .transpose()?;
        if let Some(task_id) = &input.task_id {
            let task = self.get(task_id)?;
            if scope
                .as_ref()
                .is_some_and(|c| c.repository != task.repository_path)
            {
                return Err("任务不属于此项目".into());
            }
            let r = task.rounds.last().ok_or("任务缺少轮次")?;
            // Detailed transcripts and full historical aggregates never enter normal MCP replies.
            let mut result = json!({"taskId":task.id,"revision":task.revision,"title":task.title,"objective":task.objective,"description":task.description,"kind":task.kind,"roundId":r.id,"roundNumber":r.number,"status":task_status(&task),"closure":r.closure,"plan":r.plans.last(),"latestVerification":r.verifications.last(),"latestCode":r.code_refs.last(),"latestProgress":r.events.last(),"deliveryFacts":delivery_facts(r),"sources":r.sources});
            if input.history {
                result["history"] = value(&task.rounds.iter().rev().take(10).map(|r| json!({"round":r.number,"reason":r.opened_reason,"closure":r.closure,"events":r.events.iter().rev().take(8).collect::<Vec<_>>()})).collect::<Vec<_>>())?;
            }
            return Ok(result);
        }
        let query = input.query.unwrap_or_default().to_lowercase();
        let all = self.list()?;
        let matches = all
            .iter()
            .filter(|t| {
                scope
                    .as_ref()
                    .is_none_or(|c| c.repository == t.repository_path)
                    && (query.is_empty()
                        || format!(
                            "{} {} {} {}",
                            t.title,
                            t.objective,
                            t.description,
                            t.rounds
                                .iter()
                                .map(|r| r.opened_reason.as_str())
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                        .to_lowercase()
                        .contains(&query))
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"total":matches.len(),"tasks":matches.iter().take(20).map(|t| {let r=t.rounds.last().unwrap(); json!({"taskId":t.id,"revision":t.revision,"title":t.title,"project":t.project_name,"status":task_status(t),"round":r.number,"updatedAt":t.updated_at})}).collect::<Vec<_>>(),"truncated":matches.len()>20}),
        )
    }
    pub fn save(&self, input: SaveInput, actor: Actor) -> Result<Receipt> {
        if serde_json::to_vec(&input).map_err(|e| e.to_string())?.len() > 64 * 1024 {
            return Err("任务登记过大，请只保留摘要与证据引用".into());
        }
        if let Some(t) = &input.thread_id {
            valid_id(t)?;
        }
        if let Some(v) = &input.title {
            text(v, "标题", 160)?;
        }
        if let Some(v) = &input.objective {
            text(v, "任务内容", 4000)?;
        }
        if input
            .description
            .as_ref()
            .is_some_and(|v| v.chars().count() > 8000)
        {
            return Err("背景描述不能超过 8000 字".into());
        }
        if let Some(v) = &input.note {
            text(v, "进展摘要", 2000)?;
        }
        if let Some(v) = &input.delivery {
            text(v, "交付引用", 2000)?;
        }
        if input
            .kind
            .as_ref()
            .is_some_and(|v| !["bug", "feature", "investigation"].contains(&v.as_str()))
        {
            return Err("未知任务类型".into());
        }
        let fingerprint =
            stable_id(serde_json::to_vec(&(actor.name(), &input)).map_err(|e| e.to_string())?);
        let retry_key = stable_id(format!(
            "{}:{}",
            actor.name(),
            input.operation_id.as_deref().unwrap_or(&fingerprint)
        ));
        let receipt_key = format!("receipts/{retry_key}.json");
        // Check persisted retries before consulting a worktree that may since have been removed.
        {
            let store = self.store()?;
            if let Some(saved) = store.read::<SavedReceipt>(&receipt_key)? {
                if saved.fingerprint != fingerprint {
                    return Err("同一操作 ID 不能用于不同参数".into());
                }
                return Ok(saved.receipt);
            }
        }
        let previous = input.task_id.as_deref().map(|i| self.get(i)).transpose()?;
        let workspace = input
            .workspace
            .as_deref()
            .or_else(|| previous.as_ref().map(|t| t.workspace.as_str()))
            .ok_or("新任务需要提供工作目录")?;
        let ctx = match context(workspace, &input.commits) {
            Ok(c) => Some(c),
            Err(_)
                if previous.is_some() && input.workspace.is_none() && input.commits.is_empty() =>
            {
                None
            }
            Err(e) => return Err(e),
        };
        let source_verified = if let Some(thread) = &input.thread_id {
            if let Some(_repository) = ctx
                .as_ref()
                .map(|c| c.repository.as_str())
                .or_else(|| previous.as_ref().map(|t| t.repository_path.as_str()))
            {
                crate::sources::local::directory(crate::sources::local::CODEX_ID)
                    .ok()
                    .map(|home| source::verify(&home, thread))
                    .transpose()?
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };
        let store = self.store()?;
        if let Some(saved) = store.read::<SavedReceipt>(&receipt_key)? {
            if saved.fingerprint != fingerprint {
                return Err("同一操作 ID 不能用于不同参数".into());
            }
            return Ok(saved.receipt);
        }
        let mut projects = Self::projects(&store)?;
        let at = now();
        let mut task = if let Some(task_id) = &input.task_id {
            let t = Self::load(&store, task_id)?;
            if input.expected_revision != Some(t.revision) {
                return Err(format!(
                    "任务已更新：当前 revision={}；请读取当前任务后合并变更",
                    t.revision
                ));
            }
            if ctx
                .as_ref()
                .is_some_and(|c| c.repository != t.repository_path)
            {
                return Err("工作目录不属于任务的仓库，请新建独立任务".into());
            }
            t
        } else {
            if input.expected_revision.is_some() || input.reopen_reason.is_some() {
                return Err("新任务不能指定修订号或重新开启".into());
            }
            let ctx = ctx.as_ref().ok_or("缺少项目上下文")?;
            let title = input.title.as_ref().ok_or("新任务需要标题")?;
            let objective = input.objective.as_ref().ok_or("新任务需要任务内容")?;
            let project = if let Some(p) = projects.iter().find(|p| p.key == ctx.key) {
                p.clone()
            } else {
                let p = Project {
                    id: id(),
                    key: ctx.key.clone(),
                    name: ctx.name.clone(),
                    path: ctx.repository.clone(),
                };
                projects.push(p.clone());
                p
            };
            let round = new_round(
                1,
                None,
                "首次处理".into(),
                objective.clone(),
                input.description.clone().unwrap_or_default(),
                at,
            );
            Task {
                schema_version: 2,
                id: id(),
                project_id: project.id,
                project_name: project.name,
                repository_path: ctx.repository.clone(),
                workspace: ctx.workspace.clone(),
                title: title.clone(),
                objective: objective.clone(),
                description: input.description.clone().unwrap_or_default(),
                kind: input.kind.clone().unwrap_or("feature".into()),
                current_round_id: round.id.clone(),
                rounds: vec![round],
                revision: 0,
                created_at: at,
                updated_at: at,
            }
        };
        if let Some(reason) = &input.reopen_reason {
            text(reason, "重新开启原因", 2000)?;
            if task.rounds.last().unwrap().closure.is_none() {
                return Err("任务仍在处理中，请继续当前轮次".into());
            }
            let previous_plan = task
                .rounds
                .last()
                .and_then(|r| r.plans.last())
                .map(|p| p.input.clone());
            let mut round = new_round(
                task.rounds.len() + 1,
                Some(task.current_round_id.clone()),
                reason.clone(),
                task.objective.clone(),
                task.description.clone(),
                at,
            );
            // Reuse the checklist, never the results. No extra model call is needed to rebuild it.
            if input.plan.is_none() {
                if let Some(mut plan) = previous_plan {
                    for c in &mut plan.criteria {
                        c.id.clear();
                    }
                    append_plan(&mut round, plan, at)?;
                }
            }
            task.current_round_id = round.id.clone();
            task.rounds.push(round);
        }
        if task.rounds.last().unwrap().closure.is_some() {
            return Err("本轮已结束；需要继续处理时请填写重新开启原因".into());
        }
        if let Some(v) = input.title {
            task.title = v;
        }
        if let Some(v) = input.objective {
            task.objective = v;
        }
        if let Some(v) = input.description {
            task.description = v;
        }
        if let Some(v) = input.kind {
            task.kind = v;
        }
        if let Some(c) = &ctx {
            task.workspace = c.workspace.clone();
        }
        let round = task.rounds.last_mut().unwrap();
        round.objective = task.objective.clone();
        round.description = task.description.clone();
        if input.phase == Some(Phase::Published)
            && round.phase != Phase::Published
            && input.delivery.as_ref().is_none_or(|s| s.trim().is_empty())
        {
            return Err("标为已发布时，请同时登记发布版本或发布记录".into());
        }
        if let Some(v) = input.phase {
            round.phase = v;
        }
        if let Some(thread_id) = input.thread_id {
            if let Some(existing) = round.sources.iter_mut().find(|s| s.thread_id == thread_id) {
                existing.verified |= source_verified;
            } else {
                round.sources.push(SourceRef {
                    thread_id,
                    verified: source_verified,
                });
            }
        }
        if let Some(plan) = input.plan {
            append_plan(round, plan, at)?;
        }
        if let Some(c) = &ctx {
            round.code_refs.push(c.observation.clone());
        }
        if let Some(v) = input.verification {
            validate_verification(round, &v)?;
            if v.results.iter().any(|r| r.result == TestResult::Failed) {
                round.phase = Phase::InProgress;
            }
            round.verifications.push(Verification {
                id: id(),
                actor: actor.name().into(),
                created_at: at,
                workspace_observation: ctx.as_ref().map(|c| c.observation.clone()),
                input: v,
            });
        }
        if let Some(v) = input.delivery {
            if !round.deliveries.contains(&v) {
                round.deliveries.push(v);
            }
        }
        let mut kinds = vec![];
        if input.reopen_reason.is_some() {
            kinds.push("重新开启");
        }
        if round.plans.last().is_some_and(|p| p.created_at == at) {
            kinds.push("验收计划更新");
        }
        if round
            .verifications
            .last()
            .is_some_and(|v| v.created_at == at)
        {
            kinds.push("记录验收");
        }
        let mut summary = input.note.unwrap_or_else(|| {
            if kinds.is_empty() {
                "登记任务进展".into()
            } else {
                kinds.join(" · ")
            }
        });
        summary.push_str(&format!(" · 阶段：{}", phase_label(&round.phase)));
        if !input.commits.is_empty() {
            summary.push_str(&format!(" · 关联 {} 个提交", input.commits.len()));
        }
        round.events.push(Event {
            id: id(),
            at,
            actor: actor.name().into(),
            kind: if input.reopen_reason.is_some() {
                "reopened"
            } else {
                "milestone"
            }
            .into(),
            summary,
        });
        task.revision += 1;
        task.updated_at = at;
        let receipt = receipt(&task);
        store.commit(vec![
            ("projects.json".into(), value(&projects)?),
            (Self::key(&task.project_id, &task.id), value(&task)?),
            (
                receipt_key,
                value(&SavedReceipt {
                    fingerprint,
                    receipt: receipt.clone(),
                })?,
            ),
        ])?;
        Ok(receipt)
    }
    /// Only the desktop's human-action adapter calls this. It is deliberately absent from MCP.
    pub fn close(
        &self,
        task_id: &str,
        expected_revision: u64,
        outcome: &str,
        reason: &str,
    ) -> Result<Receipt> {
        if ![
            "accepted",
            "closed",
            "cancelled",
            "superseded",
            "investigation_completed",
        ]
        .contains(&outcome)
        {
            return Err("未知结束结果".into());
        }
        text(reason, "结束说明", 2000)?;
        let store = self.store()?;
        let mut task = Self::load(&store, task_id)?;
        if task.revision != expected_revision {
            return Err(format!("任务已更新：当前 revision={}", task.revision));
        }
        let round = task.rounds.last_mut().unwrap();
        if round.closure.is_some() {
            return Err("本轮已经结束".into());
        }
        if outcome == "accepted" && !human_acceptance_ready(round) {
            return Err(
                "当前计划的必验项尚未完成人工验收；可先记录验收结果，或注明原因结束本轮".into(),
            );
        }
        let outcome = if outcome == "accepted" {
            "accepted"
        } else {
            "closed"
        };
        let at = now();
        round.closure = Some(Closure {
            outcome: outcome.into(),
            reason: reason.into(),
            actor: "human".into(),
            closed_at: at,
            verification_ids: round.verifications.iter().map(|v| v.id.clone()).collect(),
        });
        let label = match outcome {
            "accepted" => "验收通过",
            _ => "已结束",
        };
        round.events.push(Event {
            id: id(),
            at,
            actor: "human".into(),
            kind: "closed".into(),
            summary: format!("{label}：{reason}"),
        });
        task.revision += 1;
        task.updated_at = at;
        store.write(&Self::key(&task.project_id, &task.id), &task)?;
        Ok(receipt(&task))
    }
}

fn new_round(
    number: usize,
    previous_round_id: Option<String>,
    reason: String,
    objective: String,
    description: String,
    at: i64,
) -> Round {
    Round {
        id: id(),
        number,
        opened_at: at,
        opened_reason: reason,
        previous_round_id,
        objective,
        description,
        phase: Phase::InProgress,
        sources: vec![],
        code_refs: vec![],
        plans: vec![],
        verifications: vec![],
        deliveries: vec![],
        closure: None,
        events: vec![],
    }
}
fn receipt(task: &Task) -> Receipt {
    let r = task.rounds.last().unwrap();
    Receipt {
        task_id: task.id.clone(),
        round_id: r.id.clone(),
        revision: task.revision,
        plan_revision: r.plans.last().map(|p| p.revision),
        criterion_ids: r
            .plans
            .last()
            .map(|p| p.input.criteria.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default(),
        source_linked: r.sources.iter().any(|s| s.verified),
        status: Some(task_status(task).into()),
    }
}
pub(super) fn append_plan(round: &mut Round, mut input: PlanInput, at: i64) -> Result<()> {
    text(&input.summary, "方案摘要", 4000)?;
    if input.criteria.is_empty() || input.criteria.len() > 30 {
        return Err("验收点需要 1–30 项".into());
    }
    let old_ids: HashSet<_> = round
        .plans
        .iter()
        .flat_map(|p| p.input.criteria.iter().map(|c| c.id.clone()))
        .collect();
    let mut used = HashSet::new();
    for c in &mut input.criteria {
        text(&c.title, "验收项标题", 300)?;
        text(&c.steps, "验收步骤", 2000)?;
        text(&c.expected, "预期结果", 2000)?;
        if c.precondition.chars().count() > 2000 {
            return Err("前置条件过长".into());
        }
        if c.id.is_empty() {
            c.id = id();
        } else if !old_ids.contains(&c.id) {
            return Err("新增验收项请省略 ID，已有验收项请使用返回的 ID".into());
        }
        if !used.insert(c.id.clone()) {
            return Err("验收项 ID 重复".into());
        }
    }
    if input.affected_areas.len() > 30
        || input.affected_areas.iter().any(|s| s.chars().count() > 500)
    {
        return Err("影响范围过多或过长".into());
    }
    let revision = round.plans.last().map_or(1, |p| p.revision + 1);
    round.plans.push(Plan {
        revision,
        created_at: at,
        input,
    });
    Ok(())
}
pub(super) fn validate_verification(round: &Round, input: &VerificationInput) -> Result<()> {
    let plan = round
        .plans
        .iter()
        .find(|p| p.revision == input.plan_revision)
        .ok_or("验收计划版本不存在")?;
    if input.stage == Scope::Both {
        return Err("INVALID_VERIFICATION_STAGE: verification.stage 请选择 local 或 release；两个阶段请分别提交".into());
    }
    text(&input.target, "实际验收版本", 1000)?;
    text(&input.environment, "验收环境", 1000)?;
    if input.results.is_empty() || input.results.len() > plan.input.criteria.len() {
        return Err("验收结果数量无效".into());
    }
    let mut used = HashSet::new();
    for result in &input.results {
        let c = plan
            .input
            .criteria
            .iter()
            .find(|c| c.id == result.criterion_id)
            .ok_or("验收项不在指定计划中")?;
        if c.scope != Scope::Both && c.scope != input.stage {
            return Err(format!(
                "CRITERION_STAGE_MISMATCH: 验收项不适用于这个验收阶段 (criterionId={}, criterionScope={}, submittedStage={})；请按验收项适用阶段分别提交",
                c.id,
                json!(c.scope),
                json!(input.stage)
            ));
        }
        if !used.insert(&result.criterion_id) {
            return Err("验收项结果重复".into());
        }
        text(&result.evidence, "验收证据或说明", 3000)?;
    }
    Ok(())
}
pub fn human_acceptance_ready(round: &Round) -> bool {
    let Some(plan) = round.plans.last() else {
        return false;
    };
    let required = plan
        .input
        .criteria
        .iter()
        .filter(|c| c.required)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return false;
    }
    required.into_iter().all(|c| {
        let stage = if c.scope == Scope::Local {
            Scope::Local
        } else {
            Scope::Release
        };
        // One target per stage: a result from an older installed version cannot fill a gap in the latest one.
        let latest = round
            .verifications
            .iter()
            .rev()
            .find(|v| v.input.plan_revision == plan.revision && v.input.stage == stage);
        latest.is_some_and(|latest| {
            round
                .verifications
                .iter()
                .rev()
                .filter(|v| {
                    v.input.plan_revision == plan.revision
                        && v.input.stage == stage
                        && v.input.target == latest.input.target
                        && v.input.environment == latest.input.environment
                })
                .find_map(|v| {
                    v.input
                        .results
                        .iter()
                        .find(|r| r.criterion_id == c.id)
                        .map(|r| v.actor == "human" && r.result == TestResult::Passed)
                })
                .unwrap_or(false)
        })
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
