//! Canonical task/run ledger shared by the App and MCP. Legacy v3/v2 are read only for migration.
#[allow(dead_code)]
mod delivery;
mod diffs;
mod find;
mod legacy;
mod legacy_model;
mod migration;
mod model;
mod operations;
mod source;
mod summary;
pub use summary::{RunCounts, TaskSummary};
mod validation;
mod workspace;
use crate::{stable_id, store::Store, Result};
pub use model::*;
pub use operations::EditTaskInput;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub use validation::human_acceptance_ready;
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn text(value: &str, name: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.chars().count() > max {
        Err(format!("{name}不能为空，且不能超过 {max} 字"))
    } else {
        Ok(())
    }
}
fn valid_id(value: &str) -> Result<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| "无效的任务或执行 ID".into())
}
#[derive(Clone)]
pub struct Ledger {
    root: PathBuf,
    data: PathBuf,
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
#[serde(rename_all = "camelCase")]
struct SavedReceipt {
    fingerprint: String,
    receipt: Receipt,
}
impl Ledger {
    pub fn new(data: &Path) -> Self {
        Self {
            root: data.join("tasks/v4"),
            data: data.into(),
        }
    }
    pub fn data_directory(&self) -> &Path {
        &self.data
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
        let store = loop {
            match Store::open(&self.root) {
                Ok(s) => break s,
                Err(e) if e.contains("busy") && start.elapsed() < Duration::from_secs(2) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(e),
            }
        };
        if store.read::<Vec<Project>>("projects.json")?.is_none() {
            migration::import(&store, &self.data)?;
        }
        Ok(store)
    }
    fn key(project: &str, task: &str) -> String {
        format!("projects/{project}/tasks/{task}.json")
    }
    fn projects(store: &Store) -> Result<Vec<Project>> {
        Ok(store.read("projects.json")?.unwrap_or_default())
    }
    fn load(store: &Store, task_id: &str) -> Result<Task> {
        valid_id(task_id)?;
        for project in Self::projects(store)? {
            if let Some(mut task) = store.read::<Task>(&Self::key(&project.id, task_id))? {
                if !matches!(task.schema_version, 4 | 5) || task.runs.is_empty() {
                    return Err("任务格式版本或执行历史无效".into());
                }
                task.normalize_completion();
                return Ok(task);
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
        if tasks
            .iter()
            .any(|t| !matches!(t.schema_version, 4 | 5) || t.runs.is_empty())
        {
            return Err("任务格式版本或执行历史无效".into());
        }
        for task in &mut tasks {
            task.normalize_completion();
        }
        tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        Ok(tasks)
    }
    pub fn view(&self, task: &Task) -> Result<Value> {
        let mut view = json!(task);
        let project = Self::projects(&self.store()?)?
            .into_iter()
            .find(|p| p.id == task.project_id)
            .ok_or("项目不存在")?;
        view["summary"] = json!(task.summary());
        view["projectName"] = json!(project.name);
        Ok(view)
    }
    /// Read immutable, explicitly linked commit files on demand, including old snapshots.
    pub fn commit_changes(&self, task_id: &str, run_id: &str) -> Result<Vec<CommitChange>> {
        let task = self.get(task_id)?;
        let run = task
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .ok_or("执行不存在")?;
        let project = Self::projects(&self.store()?)?
            .into_iter()
            .find(|p| p.id == task.project_id)
            .ok_or("项目不存在")?;
        Ok(run
            .agent_workspace
            .related_commits
            .iter()
            .map(|sha| {
                if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return CommitChange {
                        commit: sha.clone(),
                        files: vec![],
                        error: Some("提交引用不是有效 SHA".into()),
                    };
                }
                let repository =
                    workspace::evidence_repository(&run.agent_workspace, &project.path);
                let read = workspace::commit_changes(Path::new(&repository), sha);
                if read.error.is_some() {
                    if let Some(saved) = run
                        .agent_workspace
                        .commit_changes
                        .iter()
                        .find(|c| c.commit == *sha && c.error.is_none())
                    {
                        return saved.clone();
                    }
                }
                read
            })
            .collect())
    }

    fn request_key(
        &self,
        op: &str,
        input: &impl Serialize,
        operation: Option<&str>,
        actor: &Actor,
    ) -> Result<(String, String, Option<Receipt>)> {
        let bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 {
            return Err("任务登记过大，请只保留摘要与证据引用".into());
        }
        let fingerprint = stable_id(
            serde_json::to_vec(&(op, actor.role(), actor.source(), input))
                .map_err(|e| e.to_string())?,
        );
        let key = format!(
            "receipts/{}.json",
            stable_id(format!(
                "{op}:{}:{}:{}",
                actor.role(),
                actor.source(),
                operation.unwrap_or(&fingerprint)
            ))
        );
        let cached = self.cached(&self.store()?, &key, &fingerprint)?;
        Ok((key, fingerprint, cached))
    }
    fn cached(&self, store: &Store, key: &str, fingerprint: &str) -> Result<Option<Receipt>> {
        if let Some(saved) = store.read::<SavedReceipt>(key)? {
            if saved.fingerprint != fingerprint {
                return Err("同一操作 ID 不能用于不同参数".into());
            }
            Ok(Some(saved.receipt))
        } else {
            Ok(None)
        }
    }
    fn transact(
        &self,
        key: String,
        fingerprint: String,
        apply: impl FnOnce(&Store, &mut Vec<Project>) -> Result<Task>,
    ) -> Result<Receipt> {
        let store = self.store()?;
        if let Some(receipt) = self.cached(&store, &key, &fingerprint)? {
            return Ok(receipt);
        }
        let mut projects = Self::projects(&store)?;
        let mut task = apply(&store, &mut projects)?;
        task.revision += 1;
        task.updated_at = now();
        let run = task.runs.last().unwrap();
        let receipt = Receipt {
            task_id: task.id.clone(),
            run_id: run.id.clone(),
            revision: task.revision,
            plan_revision: run.validation_plan.revision,
            criterion_ids: run
                .validation_plan
                .items
                .iter()
                .map(|c| c.id.clone())
                .collect(),
            status: run.status.clone(),
            created_by: run.created_by.clone(),
        };
        let repository = projects
            .iter()
            .find(|p| p.id == task.project_id)
            .ok_or("项目不存在")?
            .path
            .clone();
        let mut writes = diffs::prepare(&store, &task, &repository, true)?;
        writes.extend(vec![
            ("projects.json".into(), json!(projects)),
            (Self::key(&task.project_id, &task.id), json!(task)),
            (
                key,
                json!(SavedReceipt {
                    fingerprint,
                    receipt: receipt.clone()
                }),
            ),
        ]);
        store.commit(writes)?;
        Ok(receipt)
    }
}
#[cfg(test)]
mod records_tests;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod summary_tests;

#[cfg(test)]
mod cross_project_tests;
