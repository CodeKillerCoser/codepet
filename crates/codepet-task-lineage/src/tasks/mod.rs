//! Canonical task/run ledger shared by the App and MCP. Legacy v3/v2 are read only for migration.
mod activity;
#[allow(dead_code)]
mod delivery;
mod diffs;
mod documents;
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
    lock_wait: Duration,
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
            lock_wait: Duration::from_secs(2),
        }
    }

    /// Interactive tools can report contention sooner than a background writer.
    pub fn with_lock_wait(mut self, wait: Duration) -> Self {
        self.lock_wait = wait;
        self
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
                Err(e) if e.contains("busy") && start.elapsed() < self.lock_wait => {
                    std::thread::sleep(
                        Duration::from_millis(4)
                            .min(self.lock_wait.saturating_sub(start.elapsed())),
                    )
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
    fn read_task(store: &Store, key: &str, mode: documents::Resolve) -> Result<Option<Task>> {
        store
            .read::<Value>(key)?
            .map(|stored| documents::decode(stored, store.root(), mode))
            .transpose()
    }

    fn load_with_documents(store: &Store, task_id: &str, mode: documents::Resolve) -> Result<Task> {
        valid_id(task_id)?;
        for project in Self::projects(store)? {
            if let Some(task) = Self::read_task(store, &Self::key(&project.id, task_id), mode)? {
                return Ok(task);
            }
        }
        Err("任务不存在".into())
    }

    fn load(store: &Store, task_id: &str) -> Result<Task> {
        Self::load_with_documents(store, task_id, documents::Resolve::Full)
    }

    pub fn get(&self, task_id: &str) -> Result<Task> {
        Self::load(&self.store()?, task_id)
    }

    fn list_with_documents(&self, mode: documents::Resolve) -> Result<Vec<Task>> {
        let store = self.store()?;
        let mut snapshot = vec![];
        for p in Self::projects(&store)? {
            snapshot.extend(store.list_bytes(&format!("projects/{}/tasks", p.id))?);
        }
        drop(store);
        let mut tasks = snapshot
            .into_iter()
            .map(|bytes| {
                let value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                documents::decode(value, &self.root, mode)
            })
            .collect::<Result<Vec<_>>>()?;
        tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        Ok(tasks)
    }

    pub fn list(&self) -> Result<Vec<Task>> {
        self.list_with_documents(documents::Resolve::Full)
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
        let key = Self::receipt_key(op, operation.unwrap_or(&fingerprint), actor);
        let cached = self.cached(&self.store()?, &key, &fingerprint)?;
        Ok((key, fingerprint, cached))
    }
    fn receipt_key(operation: &str, operation_id: &str, actor: &Actor) -> String {
        format!(
            "receipts/{}.json",
            stable_id(format!(
                "{operation}:{}:{}:{operation_id}",
                actor.role(),
                actor.source()
            ))
        )
    }

    pub fn operation_receipt(&self, input: GetTaskOperationInput, actor: Actor) -> Result<Value> {
        text(&input.operation_id, "operationId", 64 * 1024)?;
        // Receipts are atomically published last, after all task and diff files.
        // They are immutable and can be read during a different writer's transaction.
        let operations = input
            .operation
            .as_ref()
            .map_or(&TaskOperation::ALL[..], std::slice::from_ref);
        let mut receipt = None;
        for operation in operations {
            let key = Self::receipt_key(operation.tool_name(), &input.operation_id, &actor);
            if let Some(saved) = Store::read_published::<SavedReceipt>(&self.root, &key)? {
                if receipt.replace(saved).is_some() {
                    return Err("operationId 匹配多种写入，请用 operation 指定工具名称".into());
                }
            }
        }
        Ok(match receipt {
            Some(saved) => {
                json!({"operationId":input.operation_id,"status":"committed","receipt":saved.receipt})
            }
            None => json!({"operationId":input.operation_id,"status":"unconfirmed"}),
        })
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
        self.transact_run(key, fingerprint, None, apply)
    }
    fn transact_run(
        &self,
        key: String,
        fingerprint: String,
        run_id: Option<&str>,
        apply: impl FnOnce(&Store, &mut Vec<Project>) -> Result<Task>,
    ) -> Result<Receipt> {
        let store = self.store()?;
        if let Some(receipt) = self.cached(&store, &key, &fingerprint)? {
            return Ok(receipt);
        }
        let mut projects = Self::projects(&store)?;
        let mut task = apply(&store, &mut projects)?;
        let task_key = Self::key(&task.project_id, &task.id);
        let previous = Self::read_task(&store, &task_key, documents::Resolve::Full)?;
        activity::validate(previous.as_ref(), &task)?;
        task.revision += 1;
        task.updated_at = now();
        let run = match run_id {
            Some(id) => task
                .runs
                .iter()
                .find(|run| run.id == id)
                .ok_or("执行不存在")?,
            None => task.runs.last().unwrap(),
        };
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
            activity_id: run.timeline.last().map(|entry| entry.id.clone()),
        };
        let repository = projects
            .iter()
            .find(|p| p.id == task.project_id)
            .ok_or("项目不存在")?
            .path
            .clone();
        let mut writes = if previous
            .as_ref()
            .and_then(|task| task.runs.last())
            .is_none_or(|old| {
                let run = task.runs.last().unwrap();
                old.id != run.id || old.agent_workspace != run.agent_workspace
            }) {
            diffs::prepare(&store, &task, &repository, true)?
        } else {
            vec![]
        };
        writes.extend(vec![
            ("projects.json".into(), json!(projects)),
            (task_key, documents::encode(&task, &store)?),
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
mod activity_tests;
#[cfg(test)]
mod cross_project_tests;
