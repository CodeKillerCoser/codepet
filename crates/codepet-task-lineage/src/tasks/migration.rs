//! Atomic import into v4. v3/v2 source records remain untouched.
use super::{legacy, legacy_model as old, model::*, Project};
use crate::{store::Store, Result};
use serde_json::json;
use std::path::Path;

fn author(events: &[old::Event]) -> String {
    if events.first().is_some_and(|e| e.actor == "human") {
        "human"
    } else {
        "unknown"
    }
    .into()
}
fn observation(old: &old::CodeObservation) -> AgentWorkspace {
    AgentWorkspace {
        path: old.workspace.clone(),
        branch: old.branch.clone(),
        head: old.head.clone(),
        upstream: old.upstream.clone(),
        change_status: ChangeStatus {
            has_uncommitted_changes: old.dirty,
            has_unpushed_commits: None,
        },
        changed_files: vec![],
        related_commits: old.commits.iter().map(|c| c.sha.clone()).collect(),
        observed_at: old.observed_at,
        error: old
            .error
            .clone()
            .or_else(|| Some("历史观测没有文件清单；需要在当前执行重新观测".into())),
    }
}
fn run(old: &old::Round, workspace: &str, title: &str, kind: &TaskKind) -> TaskRun {
    let mut timeline: Vec<_> = old
        .events
        .iter()
        .map(|e| TimelineEvent {
            id: e.id.clone(),
            at: e.at,
            actor: e.actor.clone(),
            kind: e.kind.clone(),
            summary: e.summary.clone(),
            reference: None,
            agent_workspace: None,
        })
        .collect();
    for (index, code) in old.code_refs.iter().enumerate() {
        timeline.push(TimelineEvent {
            id: format!("{}-git-{index}", old.id),
            at: code.observed_at,
            actor: "unknown".into(),
            kind: "workspace_snapshot".into(),
            summary: "历史工作区观测".into(),
            reference: None,
            agent_workspace: Some(observation(code)),
        });
    }
    for (index, reference) in old.deliveries.iter().enumerate() {
        timeline.push(TimelineEvent {
            id: format!("{}-delivery-{index}", old.id),
            at: super::now(),
            actor: "unknown".into(),
            kind: "imported_delivery".into(),
            summary: "迁移历史构建／发布引用（原始发生时间未知）".into(),
            reference: Some(reference.clone()),
            agent_workspace: None,
        });
    }
    timeline.sort_by_key(|e| e.at);
    let mut description = vec![old.opened_reason.clone()];
    for text in [&old.objective, &old.description] {
        if !text.trim().is_empty() && !description.contains(text) {
            description.push(text.clone());
        }
    }
    if let Some(plan) = old.plans.last() {
        if !description.contains(&plan.input.summary) {
            description.push(plan.input.summary.clone());
        }
        if !plan.input.affected_areas.is_empty() {
            description.push(format!(
                "影响范围：{}",
                plan.input.affected_areas.join("、")
            ));
        }
    }
    let mut agent_workspace = old
        .code_refs
        .last()
        .map(observation)
        .unwrap_or(AgentWorkspace {
            path: workspace.into(),
            branch: None,
            head: None,
            upstream: None,
            change_status: ChangeStatus {
                has_uncommitted_changes: None,
                has_unpushed_commits: None,
            },
            changed_files: vec![],
            related_commits: vec![],
            observed_at: old.opened_at,
            error: Some("历史执行没有工作区观测".into()),
        });
    for code in &old.code_refs {
        for commit in &code.commits {
            if !agent_workspace.related_commits.contains(&commit.sha) {
                agent_workspace.related_commits.push(commit.sha.clone());
            }
        }
    }
    TaskRun {
        id: old.id.clone(),
        title: title.into(),
        kind: kind.clone(),
        goal: old.objective.clone(),
        solution: old
            .plans
            .last()
            .map_or(String::new(), |p| p.input.summary.clone()),
        description: description.join("\n\n"),
        status: if old.closure.is_some() {
            RunStatus::Closed
        } else {
            match old.phase {
                old::Phase::InProgress => RunStatus::InProgress,
                old::Phase::ReadyForDelivery => RunStatus::ReadyForDelivery,
                old::Phase::Published => RunStatus::Published,
            }
        },
        validation_plan: ValidationPlan {
            revision: old.plans.last().map_or(0, |p| p.revision),
            items: old
                .plans
                .last()
                .map_or(vec![], |p| p.input.criteria.clone()),
            revisions: old.plans.clone(),
            records: old
                .verifications
                .iter()
                .map(|v| Verification {
                    id: v.id.clone(),
                    actor: v.actor.clone(),
                    agent: if v.actor == "human" {
                        "human"
                    } else {
                        "unknown"
                    }
                    .into(),
                    created_at: v.created_at,
                    agent_workspace: v.workspace_observation.as_ref().map(observation),
                    input: v.input.clone(),
                })
                .collect(),
        },
        agent_workspace,
        agent_conversation: old
            .sources
            .iter()
            .map(|s| AgentConversation {
                agent: "codex".into(),
                conversation_id: s.thread_id.clone(),
                verified: s.verified,
            })
            .collect(),
        timeline,
        created_by: author(&old.events),
        created_at: old.opened_at,
        updated_at: old.events.last().map_or(old.opened_at, |e| e.at),
        closure: old.closure.as_ref().map(|c| Closure {
            accepted: c.outcome == "accepted",
            reason: c.reason.clone(),
            closed_at: c.closed_at,
        }),
    }
}
pub(super) fn import(store: &Store, data: &Path) -> Result<()> {
    if data.join("tasks/v3/projects.json").exists() {
        return import_v3(store, data);
    }
    let tasks = legacy::Ledger::new(data).list()?;
    let mut projects: Vec<Project> = vec![];
    let mut changes = vec![];
    for old in tasks {
        if old.rounds.is_empty() {
            return Err(format!("旧任务 {} 缺少处理历史", old.id));
        }
        if !projects.iter().any(|p| p.id == old.project_id) {
            projects.push(Project {
                id: old.project_id.clone(),
                key: crate::stable_id(old.repository_path.as_bytes()),
                name: old.project_name.clone(),
                path: old.repository_path.clone(),
            });
        }
        let description = if old.objective.trim().is_empty()
            || old.objective == old.title
            || old.description.contains(&old.objective)
        {
            old.description.clone()
        } else {
            format!("{}\n\n{}", old.objective, old.description)
                .trim()
                .into()
        };
        let kind = if old.kind == "feature" {
            TaskKind::Feature
        } else {
            TaskKind::Bugfix
        };
        let runs = old
            .rounds
            .iter()
            .map(|r| run(r, &old.workspace, &old.title, &kind))
            .collect();
        let task = Task {
            schema_version: 4,
            id: old.id.clone(),
            title: old.title,
            description,
            project_id: old.project_id.clone(),
            created_by: author(&old.rounds[0].events),
            created_at: old.created_at,
            runs,
            revision: old.revision,
            updated_at: old.updated_at,
        };
        changes.push((super::Ledger::key(&task.project_id, &task.id), json!(task)));
    }
    changes.push(("projects.json".into(), json!(projects)));
    store.commit(changes)
}

fn import_v3(store: &Store, data: &Path) -> Result<()> {
    let source = Store::open(&data.join("tasks/v3"))?;
    let projects: Vec<Project> = source.read("projects.json")?.ok_or("旧项目索引不存在")?;
    let mut changes = vec![];
    for project in &projects {
        for mut value in
            source.list::<serde_json::Value>(&format!("projects/{}/tasks", project.id))?
        {
            if value["schemaVersion"] != 3 {
                return Err("旧任务格式版本无效".into());
            }
            let kind = value["kind"].clone();
            let title = value["title"].as_str().ok_or("旧任务缺少标题")?.to_string();
            let runs = value["runs"].as_array_mut().ok_or("旧任务缺少执行")?;
            if runs.is_empty() {
                return Err("旧任务缺少执行历史".into());
            }
            for (index, run) in runs.iter_mut().enumerate() {
                // Old free-form summaries remain intact. Do not guess a solution or
                // pretend the old task-level kind was recorded separately per run.
                run["title"] = json!(if index == 0 {
                    "首次处理".into()
                } else {
                    format!("第 {} 次处理", index + 1)
                });
                run["kind"] = kind.clone();
                run["goal"] = json!(title);
                run["solution"] = json!("");
            }
            value.as_object_mut().unwrap().remove("kind");
            value["schemaVersion"] = json!(4);
            let task: Task = serde_json::from_value(value).map_err(|e| e.to_string())?;
            changes.push((super::Ledger::key(&task.project_id, &task.id), json!(task)));
        }
    }
    changes.push(("projects.json".into(), json!(projects)));
    store.commit(changes)
}
