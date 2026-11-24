//! Canonical task/run ledger shared by the App and MCP. Legacy v3/v2 are read only for migration.
#[allow(dead_code)]
mod delivery;
mod legacy;
mod legacy_model;
mod migration;
mod model;
mod operations;
mod source;
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
            if let Some(task) = store.read::<Task>(&Self::key(&project.id, task_id))? {
                if task.schema_version != 4 || task.runs.is_empty() {
                    return Err("任务格式版本或执行历史无效".into());
                }
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
            .any(|t| t.schema_version != 4 || t.runs.is_empty())
        {
            return Err("任务格式版本或执行历史无效".into());
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
        view["projectName"] = json!(project.name);
        Ok(view)
    }
    pub fn find(&self, input: FindTasksInput) -> Result<Value> {
        let scope = input
            .workspace
            .as_ref()
            .map(|path| {
                workspace::inspect(&WorkspaceInput {
                    path: path.clone(),
                    related_commits: vec![],
                })
            })
            .transpose()?;
        let projects = Self::projects(&self.store()?)?;
        let allowed = |task: &Task| {
            scope.as_ref().is_none_or(|scope| {
                projects
                    .iter()
                    .any(|p| p.id == task.project_id && p.path == scope.repository)
            })
        };
        if let Some(id) = input.task_id {
            let task = self.get(&id)?;
            if !allowed(&task) {
                return Err("任务不属于此项目".into());
            }
            let run = if let Some(run_id) = input.run_id {
                task.runs
                    .iter()
                    .find(|r| r.id == run_id)
                    .ok_or("执行不存在")?
            } else {
                task.runs.last().unwrap()
            };
            let mut observed = json!(run.agent_workspace);
            observed["changedFileCount"] = json!(run.agent_workspace.changed_files.len());
            if run.agent_workspace.changed_files.len() > 20 {
                observed["changedFiles"] = json!(&run.agent_workspace.changed_files[..20]);
                observed["filesTruncated"] = json!(true);
            }
            // Saved evidence retains complete snapshots. Normal MCP reads expose
            // the latest facts once, rather than repeating file lists in events/tests.
            let verification = run.validation_plan.records.last().map(|record| {
                let mut result = json!(record.input);
                result["id"] = json!(record.id);
                result["actor"] = json!(record.actor);
                result["agent"] = json!(record.agent);
                result["createdAt"] = json!(record.created_at);
                result
            });
            let progress = run.timeline.last().map(|entry| json!({"at":entry.at,"actor":entry.actor,"kind":entry.kind,"summary":entry.summary.chars().take(2000).collect::<String>(),"reference":entry.reference}));
            let mut result = json!({"taskId":task.id,"title":task.title,"description":task.description,"createdBy":task.created_by,"revision":task.revision,"runCount":task.runs.len(),"run":{"id":run.id,"title":run.title,"kind":run.kind,"goal":run.goal,"solution":run.solution,"description":run.description,"status":run.status,"createdBy":run.created_by,"validationPlan":{"revision":run.validation_plan.revision,"items":run.validation_plan.items,"latestVerification":verification},"agentWorkspace":observed,"agentConversation":run.agent_conversation,"latestProgress":progress}});
            if input.history {
                result["history"]=json!(task.runs.iter().rev().take(10).map(|r|json!({"id":r.id,"title":r.title,"kind":r.kind,"goal":r.goal,"description":r.description,"status":r.status,"createdBy":r.created_by,"createdAt":r.created_at})).collect::<Vec<_>>());
            }
            return Ok(result);
        }
        if input.run_id.is_some() {
            return Err("查询指定执行需要 taskId".into());
        }
        let q = input.query.unwrap_or_default().to_lowercase();
        let tasks = self.list()?;
        let matches = tasks
            .iter()
            .filter(|t| {
                allowed(t)
                    && (q.is_empty()
                        || format!(
                            "{} {} {}",
                            t.title,
                            t.description,
                            t.runs
                                .iter()
                                .flat_map(|r| [
                                    r.title.as_str(),
                                    r.goal.as_str(),
                                    r.solution.as_str(),
                                    r.description.as_str()
                                ]
                                .into_iter()
                                .chain(r.timeline.iter().map(|e| e.summary.as_str())))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                        .to_lowercase()
                        .contains(&q))
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"total":matches.len(),"truncated":matches.len()>20,"tasks":matches.iter().take(20).map(|t|json!({"taskId":t.id,"title":t.title,"runKind":t.runs.last().unwrap().kind,"status":t.runs.last().unwrap().status,"runId":t.runs.last().unwrap().id,"runCount":t.runs.len(),"revision":t.revision,"createdBy":t.created_by,"project":projects.iter().find(|p|p.id==t.project_id).map(|p|&p.name),"updatedAt":t.updated_at})).collect::<Vec<_>>()}),
        )
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
        store.commit(vec![
            ("projects.json".into(), json!(projects)),
            (Self::key(&task.project_id, &task.id), json!(task)),
            (
                key,
                json!(SavedReceipt {
                    fingerprint,
                    receipt: receipt.clone()
                }),
            ),
        ])?;
        Ok(receipt)
    }
}
#[cfg(test)]
mod records_tests;
