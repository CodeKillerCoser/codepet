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
    let contains = |text: &str| text.to_lowercase().contains(query);
    [
        task.title.as_str(),
        task.description.as_str(),
        run.title.as_str(),
        run.goal.as_str(),
        run.solution.as_str(),
        run.description.as_str(),
    ]
    .into_iter()
    .any(contains)
        || run.timeline.iter().any(|event| {
            contains(&event.summary)
                || event.changes.iter().any(|change| {
                    contains(&change.field)
                        || value_contains(&change.before, query)
                        || value_contains(&change.after, query)
                })
        })
}

fn value_contains(value: &Value, query: &str) -> bool {
    match value {
        Value::String(text) => text.to_lowercase().contains(query),
        Value::Array(items) => items.iter().any(|item| value_contains(item, query)),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, item)| key.to_lowercase().contains(query) || value_contains(item, query)),
        _ => value.to_string().contains(query),
    }
}

impl Ledger {
    pub fn find_runs(&self, input: FindTaskRunsInput) -> Result<Value> {
        if let Some(id) = &input.task_id {
            valid_id(id)?;
        }
        if let Some(id) = &input.run_id {
            valid_id(id)?;
        }
        if input.activity_after.is_some() && input.activity_before.is_some() {
            return Err("activityAfter 与 activityBefore 不能同时使用".into());
        }
        let limit = input.activity_limit.unwrap_or(20);
        if !(1..=50).contains(&limit) {
            return Err("活动分页大小必须为 1–50".into());
        }
        if (input.activity_after.is_some()
            || input.activity_before.is_some()
            || input.activity_limit.is_some())
            && input.task_id.is_none()
            && input.run_id.is_none()
        {
            return Err("读取活动需要 taskId 或 runId".into());
        }
        if input.task_id.is_some() || input.run_id.is_some() {
            let task = self.find_task_for_run(
                input.task_id.as_deref(),
                input.run_id.as_deref(),
                documents::Resolve::TextOnly,
            )?;
            let run = select_run(&task, input.run_id.as_deref())?;
            let mut result = run_summary(&task, run);
            // Old callers can finish paging while switching to read-task-run-details.
            if input.history {
                result["history"] = history_page(&task, 0)["items"].take();
            }
            if input.activity_after.is_some()
                || input.activity_before.is_some()
                || input.activity_limit.is_some()
            {
                result["activity"] = activity_page(
                    run,
                    input.activity_after.as_deref(),
                    input.activity_before.as_deref(),
                    limit,
                )?;
            }
            return Ok(result);
        }

        let tasks = self.list()?;
        let projects = Self::projects(&self.store()?)?;
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

impl Ledger {
    fn find_task_for_run(
        &self,
        task_id: Option<&str>,
        run_id: Option<&str>,
        mode: documents::Resolve,
    ) -> Result<Task> {
        if let Some(id) = run_id {
            valid_id(id)?;
        }
        if let Some(id) = task_id {
            return Self::load_with_documents(&self.store()?, id, mode);
        }
        let id = run_id.ok_or("请提供 runId 或 taskId")?;
        self.list_with_documents(mode)?
            .into_iter()
            .find(|task| task.runs.iter().any(|run| run.id == id))
            .ok_or("任务或执行不存在".into())
    }

    pub fn read_run_details(&self, input: ReadTaskRunDetailsInput) -> Result<Value> {
        let mode = if matches!(&input.detail, RunDetail::Git { .. }) {
            documents::Resolve::Full
        } else {
            documents::Resolve::TextOnly
        };
        let task =
            self.find_task_for_run(input.task_id.as_deref(), input.run_id.as_deref(), mode)?;
        let run = select_run(&task, input.run_id.as_deref())?;
        let mut page = match input.detail {
            RunDetail::Context => json!({
                "taskDescription":task.description,"taskCreatedBy":task.created_by,
                "goal":run.goal,"solution":run.solution,"description":run.description,
                "agentConversation":run.agent_conversation,
            }),
            RunDetail::Activity {
                limit,
                before,
                after,
            } => {
                let limit = limit.unwrap_or(10);
                if !(1..=20).contains(&limit) || (before.is_some() && after.is_some()) {
                    return Err("活动分页最多 20 条；before 和 after 请任选其一".into());
                }
                activity_page(run, after.as_deref(), before.as_deref(), limit)?
            }
            RunDetail::Git { commit, offset } => {
                git_page(run, commit.as_deref(), offset.unwrap_or(0))?
            }
            RunDetail::Verification {
                plan_revision,
                offset,
            } => {
                let plan = &run.validation_plan;
                let revision = plan_revision.unwrap_or(plan.revision);
                let records: Vec<_> = plan
                    .records
                    .iter()
                    .rev()
                    .filter(|record| record.input.plan_revision == revision)
                    .collect();
                let offset = offset.unwrap_or(0);
                let items = records
                    .iter()
                    .skip(offset)
                    .take(1)
                    .map(|record| verification(record, true))
                    .collect::<Vec<_>>();
                json!({"planRevision": revision,
                    "items": items, "total": records.len(),
                    "nextOffset": next_offset(offset, items.len(), records.len()),
                    "criteria": plan_revision.and_then(|revision| plan.revisions.iter().find(|p| p.revision == revision).map(|p| &p.input.criteria)).or_else(|| (revision == plan.revision).then_some(&plan.items))})
            }
            RunDetail::History { offset } => history_page(&task, offset.unwrap_or(0)),
        };
        page["taskId"] = json!(task.id);
        page["runId"] = json!(run.id);
        Ok(page)
    }
}

fn select_run<'a>(task: &'a Task, run_id: Option<&str>) -> Result<&'a TaskRun> {
    match run_id {
        Some(id) => task.runs.iter().find(|run| run.id == id),
        None => task.runs.last(),
    }
    .ok_or("执行不存在".into())
}

fn short(value: &str, length: usize) -> String {
    value.chars().take(length).collect()
}

fn verification(record: &Verification, include_evidence: bool) -> Value {
    let input = &record.input;
    let results: Vec<_> = input
        .results
        .iter()
        .map(|result| {
            if include_evidence {
                json!(result)
            } else {
                json!({"criterionId":result.criterion_id,"result":result.result})
            }
        })
        .collect();
    json!({"id":record.id,"planRevision":input.plan_revision,"stage":input.stage,
        "target":input.target,"environment":input.environment,"results":results,
        "actor":record.actor,"agent":record.agent,"createdAt":record.created_at})
}

fn run_summary(task: &Task, run: &TaskRun) -> Value {
    let plan = &run.validation_plan;
    let records: Vec<_> = plan
        .records
        .iter()
        .rev()
        .filter(|record| record.input.plan_revision == plan.revision)
        .collect();
    let stage_summary = |stage: Scope| {
        let latest = records.iter().find(|record| record.input.stage == stage)?;
        let mut summary = verification(latest, false);
        let mut seen = std::collections::HashSet::new();
        summary["results"] =
            json!(records
            .iter()
            .filter(|record| record.input.stage == stage
                && record.input.target == latest.input.target
                && record.input.environment == latest.input.environment)
            .flat_map(|record| record.input.results.iter().map(move |result| (record, result)))
            .filter(|(_, result)| seen.insert(result.criterion_id.as_str()))
            .map(|(record, result)| json!({"criterionId":result.criterion_id,"result":result.result,
                "recordId":record.id,"actor":record.actor,"agent":record.agent}))
            .collect::<Vec<_>>());
        Some(summary)
    };
    let latest_verification = records.first().map(|record| verification(record, false));
    let progress = run.timeline.last().map(|entry| json!({
        "id":entry.id,"at":entry.at,"actor":entry.actor,"kind":entry.kind,"summary":short(&entry.summary, 400),
    }));
    let recent_conclusion = run
        .timeline
        .iter()
        .rev()
        .find(|entry| entry.kind == "finished")
        .map(|entry| short(&entry.summary, 400));
    json!({
        "taskId":task.id, "taskTitle":task.title, "taskDescription":short(&task.description, 400),
        "runId":run.id, "isCurrentRun":task.runs.last().is_some_and(|latest| latest.id == run.id),
        "revision":task.revision, "runCount":task.runs.len(), "taskSummary":task.summary(),
        "run":{"id":run.id,"title":run.title,"kind":run.kind,"goal":short(&run.goal, 600),
            "solution":short(&run.solution, 600),"status":run.status,"createdBy":run.created_by,
            "latestProgress":progress,"conclusion":recent_conclusion,
            "validationPlan":{"revision":plan.revision,
                "items":plan.items.iter().map(|item| json!({"id":item.id,"title":short(&item.title, 120),
                    "scope":item.scope,"required":item.required})).collect::<Vec<_>>(),
                "latestVerification":latest_verification,
                "latestByStage":{"local":stage_summary(Scope::Local),"release":stage_summary(Scope::Release)}}}
    })
}

fn next_offset(offset: usize, size: usize, total: usize) -> Option<usize> {
    let next = offset.saturating_add(size);
    (next < total).then_some(next)
}

fn history_page(task: &Task, offset: usize) -> Value {
    let items: Vec<_> = task
        .runs
        .iter()
        .rev()
        .skip(offset)
        .take(10)
        .map(|run| {
            json!({
                "id":run.id,"title":run.title,"kind":run.kind,"goal":short(&run.goal, 400),
                "status":run.status,"createdBy":run.created_by,"createdAt":run.created_at,
            })
        })
        .collect();
    json!({"items":items,"total":task.runs.len(),"nextOffset":next_offset(offset,items.len(),task.runs.len())})
}

fn git_page(run: &TaskRun, commit: Option<&str>, offset: usize) -> Result<Value> {
    let workspace = &run.agent_workspace;
    if let Some(sha) = commit {
        let change = workspace
            .commit_changes
            .iter()
            .find(|change| change.commit == sha)
            .ok_or("未保存这个关联提交的文件清单")?;
        let files = &change.files[offset.min(change.files.len())..];
        return Ok(
            json!({"commit":sha,"comparison":"first_parent","total":change.files.len(),
            "error":change.error,"files":files.iter().take(20).collect::<Vec<_>>(),
            "nextOffset":next_offset(offset, files.len().min(20), change.files.len())}),
        );
    }
    let files = &workspace.changed_files[offset.min(workspace.changed_files.len())..];
    Ok(json!({
        "path":workspace.path,"branch":workspace.branch,"head":workspace.head,"upstream":workspace.upstream,
        "observedAt":workspace.observed_at,"error":workspace.error,"changeStatus":workspace.change_status,
        "changedFiles":files.iter().take(20).collect::<Vec<_>>(),"total":workspace.changed_files.len(),
        "nextOffset":next_offset(offset,files.len().min(20),workspace.changed_files.len()),
        "runFileCount":workspace.run_files.as_ref().map(Vec::len),
        "runFiles":workspace.run_files.as_ref().map(|files| files.iter().skip(offset).take(20).collect::<Vec<_>>()),
        "nextRunFileOffset":workspace.run_files.as_ref().and_then(|files| next_offset(offset,files.len().saturating_sub(offset).min(20),files.len())),
        "relatedCommits":workspace.related_commits.iter().map(|sha| json!({"commit":sha,
            "fileCount":workspace.commit_changes.iter().find(|change| change.commit == *sha).map(|change| change.files.len())})).collect::<Vec<_>>(),
    }))
}

fn activity_page(
    run: &TaskRun,
    after: Option<&str>,
    before: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let index = |id: &str| {
        run.timeline
            .iter()
            .position(|entry| entry.id == id)
            .ok_or("活动游标不存在于此执行")
    };
    let (start, end) = if let Some(id) = after {
        let start = index(id)? + 1;
        (start, (start + limit).min(run.timeline.len()))
    } else {
        let end = before.map(index).transpose()?.unwrap_or(run.timeline.len());
        (end.saturating_sub(limit), end)
    };
    let entries = &run.timeline[start..end];
    Ok(
        json!({"items":entries.iter().map(|entry| json!({"id":entry.id,"at":entry.at,
        "actor":entry.actor,"actorRole":entry.actor_role,"kind":entry.kind,"summary":entry.summary,
        "reference":entry.reference,"replyTo":entry.reply_to,"agentConversation":entry.agent_conversation,
        "workspace":entry.agent_workspace.as_ref().map(|workspace| &workspace.path)})).collect::<Vec<_>>(),
        "total":run.timeline.len(),"hasEarlier":start>0,"hasMore":end<run.timeline.len(),
        "firstCursor":entries.first().map(|entry|&entry.id),"nextCursor":entries.last().map(|entry|&entry.id)}),
    )
}

#[cfg(test)]
mod tests;
