//! Run-level lookup. Workspace is a preference, never a visibility boundary.
use super::*;

fn normalized_path(path: &str) -> PathBuf {
    let path = Path::new(path);
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn matches_query(task: &Task, run: &TaskRun, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    [
        task.title.as_str(),
        task.description.as_str(),
        run.title.as_str(),
        run.goal.as_str(),
        run.solution.as_str(),
        run.description.as_str(),
    ]
    .into_iter()
    .chain(run.timeline.iter().map(|event| event.summary.as_str()))
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
    .contains(query)
}

impl Ledger {
    pub fn find_runs(&self, input: FindTaskRunsInput) -> Result<Value> {
        if let Some(id) = &input.task_id {
            valid_id(id)?;
        }
        if let Some(id) = &input.run_id {
            valid_id(id)?;
        }
        let tasks = self.list()?;
        let projects = Self::projects(&self.store()?)?;
        if input.task_id.is_some() || input.run_id.is_some() {
            let task = tasks
                .iter()
                .find(|task| {
                    input.task_id.as_ref().is_none_or(|id| task.id == *id)
                        && input
                            .run_id
                            .as_ref()
                            .is_none_or(|id| task.runs.iter().any(|run| run.id == *id))
                })
                .ok_or("任务或执行不存在")?;
            let run = match &input.run_id {
                Some(id) => task
                    .runs
                    .iter()
                    .find(|run| run.id == *id)
                    .ok_or("执行不存在")?,
                None => task.runs.last().ok_or("任务没有执行")?,
            };
            return Ok(run_detail(task, run, input.history));
        }

        let query = input.query.unwrap_or_default().trim().to_lowercase();
        let query = query.as_str();
        let workspace = input.workspace.as_deref().map(normalized_path);
        let mut matches = tasks
            .iter()
            .flat_map(|task| {
                task.runs
                    .iter()
                    .filter_map(move |run| matches_query(task, run, query).then_some((task, run)))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_task, left), (right_task, right)| {
            let preference = |task: &Task, run: &TaskRun| {
                workspace.as_ref().map_or(0, |path| {
                    if normalized_path(&run.agent_workspace.path) == *path {
                        2
                    } else if projects
                        .iter()
                        .any(|p| p.id == task.project_id && normalized_path(&p.path) == *path)
                    {
                        1
                    } else {
                        0
                    }
                })
            };
            preference(right_task, right)
                .cmp(&preference(left_task, left))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(json!({
            "total": matches.len(), "truncated": matches.len() > 20,
            "runs": matches.iter().take(20).map(|(task, run)| json!({
                "taskId":task.id, "taskTitle":task.title, "runId":run.id,
                "title":run.title, "kind":run.kind, "goal":run.goal.chars().take(400).collect::<String>(),
                "status":run.status, "createdBy":run.created_by,
                "revision":task.revision, "runCount":task.runs.len(),
                "taskSummary":task.summary(),
                "isCurrentRun":task.runs.last().is_some_and(|latest| latest.id == run.id),
                "workspace":run.agent_workspace.path,
                "project":projects.iter().find(|p|p.id == task.project_id).map(|p|&p.name),
                "createdAt":run.created_at, "updatedAt":run.updated_at,
            })).collect::<Vec<_>>()
        }))
    }
}

fn run_detail(task: &Task, run: &TaskRun, history: bool) -> Value {
    let mut observed = json!(run.agent_workspace);
    observed["changedFileCount"] = json!(run.agent_workspace.changed_files.len());
    if run.agent_workspace.changed_files.len() > 20 {
        observed["changedFiles"] = json!(&run.agent_workspace.changed_files[..20]);
        observed["filesTruncated"] = json!(true);
    }
    if let Some(files) = &run.agent_workspace.run_files {
        observed["runFileCount"] = json!(files.len());
        if files.len() > 20 {
            observed["runFiles"] = json!(&files[..20]);
            observed["runFilesTruncated"] = json!(true);
        }
    }
    observed["commitChangeCount"] = json!(run.agent_workspace.commit_changes.len());
    if let Some(changes) = observed["commitChanges"].as_array_mut() {
        changes.truncate(5);
        for change in changes {
            let files = change["files"].as_array().cloned().unwrap_or_default();
            change["fileCount"] = json!(files.len());
            if files.len() > 10 {
                change["files"] = json!(&files[..10]);
                change["filesTruncated"] = json!(true);
            }
        }
    }
    if run.agent_workspace.commit_changes.len() > 5 {
        observed["commitsTruncated"] = json!(true);
    }
    let verification = run.validation_plan.records.last().map(|record| {
        let mut result = json!(record.input);
        result["id"] = json!(record.id);
        result["actor"] = json!(record.actor);
        result["agent"] = json!(record.agent);
        result["createdAt"] = json!(record.created_at);
        result
    });
    let progress = run.timeline.last().map(|entry| {
        json!({
            "at":entry.at, "actor":entry.actor, "kind":entry.kind,
            "summary":entry.summary.chars().take(2000).collect::<String>(),
            "reference":entry.reference,
        })
    });
    let mut result = json!({
        "taskId":task.id, "taskTitle":task.title, "taskDescription":task.description,
        "taskCreatedBy":task.created_by, "runId":run.id,
        "isCurrentRun":task.runs.last().is_some_and(|latest| latest.id == run.id),
        "revision":task.revision, "runCount":task.runs.len(),
        "taskSummary":task.summary(),
        "run":{
            "id":run.id, "title":run.title, "kind":run.kind, "goal":run.goal,
            "solution":run.solution, "description":run.description, "status":run.status,
            "createdBy":run.created_by,
            "validationPlan":{"revision":run.validation_plan.revision,"items":run.validation_plan.items,"latestVerification":verification},
            "agentWorkspace":observed, "agentConversation":run.agent_conversation,
            "latestProgress":progress,
        }
    });
    if history {
        result["history"] = json!(task
            .runs
            .iter()
            .rev()
            .take(10)
            .map(|run| json!({
                "id":run.id, "title":run.title, "kind":run.kind, "goal":run.goal,
                "description":run.description, "status":run.status,
                "createdBy":run.created_by, "createdAt":run.created_at,
            }))
            .collect::<Vec<_>>());
    }
    result
}

#[cfg(test)]
mod tests;
