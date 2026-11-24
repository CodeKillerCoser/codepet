use super::{
    id,
    model::*,
    now, source, text, validation,
    workspace::{self, Context},
    Ledger, Project,
};
use crate::Result;
use serde::Deserialize;
use serde_json::json;

fn event(run: &mut TaskRun, actor: &Actor, kind: &str, summary: String, reference: Option<String>) {
    let at = now();
    run.updated_at = at;
    run.timeline.push(TimelineEvent {
        id: id(),
        at,
        actor: actor.source().into(),
        kind: kind.into(),
        summary,
        reference,
        agent_workspace: Some(run.agent_workspace.clone()),
    });
}
fn new_run(description: String, workspace: AgentWorkspace, actor: &Actor) -> TaskRun {
    let at = now();
    TaskRun {
        id: id(),
        title: String::new(),
        kind: TaskKind::Feature,
        goal: String::new(),
        solution: String::new(),
        description,
        status: RunStatus::InProgress,
        validation_plan: ValidationPlan::default(),
        agent_workspace: workspace,
        agent_conversation: vec![],
        timeline: vec![],
        created_by: actor.source().into(),
        created_at: at,
        updated_at: at,
        closure: None,
    }
}
fn link(run: &mut TaskRun, ids: &[String], actor: &Actor, repository: &str) -> Result<()> {
    if !actor.is_human() && ids.is_empty() {
        return Err(
            "agentConversation 必填且不能为空；提交实际对话 ID 或 codex://threads/<id> 链接".into(),
        );
    }
    if ids.len() > 20 {
        return Err("一次最多关联 20 条对话".into());
    }
    for raw in ids {
        let conversation_id = raw.strip_prefix("codex://threads/").unwrap_or(raw);
        text(conversation_id, "对话 ID", 160)?;
        let agent = if actor.is_human() {
            "codex"
        } else {
            actor.source()
        };
        let verified = if agent == "codex" {
            super::valid_id(conversation_id).map_err(|_| {
                "Codex 对话必须是有效 UUID 或 codex://threads/<UUID> 链接".to_owned()
            })?;
            crate::sources::local::directory(crate::sources::local::CODEX_ID)
                .ok()
                .map(|home| source::verify(&home, conversation_id, repository))
                .transpose()?
                .unwrap_or(false)
        } else {
            false
        };
        if let Some(existing) = run
            .agent_conversation
            .iter_mut()
            .find(|c| c.agent == agent && c.conversation_id == conversation_id)
        {
            existing.verified |= verified;
        } else {
            run.agent_conversation.push(AgentConversation {
                agent: agent.into(),
                conversation_id: conversation_id.into(),
                verified,
            });
        }
    }
    Ok(())
}
fn check_revision(task: &Task, expected: u64) -> Result<()> {
    if task.revision != expected {
        Err(format!(
            "任务已更新：当前 revision={}；请读取后合并",
            task.revision
        ))
    } else {
        Ok(())
    }
}
fn check_new_run(task: &Task, actor: &Actor) -> Result<()> {
    match task.runs.last().unwrap().status {
        RunStatus::Published => Ok(()),
        RunStatus::Closed if actor.is_human() => Ok(()),
        RunStatus::Closed => Err(
            "任务已完成并锁定；请用户在 App 手动改为开发中，Agent 不能重新开启或新增执行".into(),
        ),
        _ => Err("当前执行仍在处理中，请更新当前执行".into()),
    }
}
fn project<'a>(projects: &'a [Project], task: &Task) -> Result<&'a Project> {
    projects
        .iter()
        .find(|p| p.id == task.project_id)
        .ok_or("项目不存在".into())
}
fn same_project(project: &Project, ctx: &Context) -> Result<()> {
    if project.path != ctx.repository {
        Err("工作区不属于任务项目，请创建独立任务".into())
    } else {
        Ok(())
    }
}
fn description(value: &str) -> Result<()> {
    if value.chars().count() > 8000 {
        Err("描述不能超过 8000 字".into())
    } else {
        Ok(())
    }
}

fn set_run_content(
    run: &mut TaskRun,
    title: String,
    kind: TaskKind,
    goal: String,
    solution: String,
) -> Result<()> {
    text(&title, "执行标题", 160)?;
    description(&goal)?;
    description(&solution)?;
    run.title = title;
    run.kind = kind;
    run.goal = goal;
    run.solution = solution;
    Ok(())
}

impl Ledger {
    pub fn create_task(&self, input: CreateTaskInput, actor: Actor) -> Result<Receipt> {
        let (key, fingerprint, cached) =
            self.request_key("create-task", &input, input.operation_id.as_deref(), &actor)?;
        if let Some(r) = cached {
            return Ok(r);
        }
        text(&input.title, "任务标题", 160)?;
        description(&input.description)?;
        description(&input.initial_run.description)?;
        let ctx = workspace::inspect(&input.agent_workspace)?;
        let mut run = new_run(
            input.initial_run.description,
            ctx.observation.clone(),
            &actor,
        );
        set_run_content(
            &mut run,
            if input.initial_run.title.trim().is_empty() {
                input.title.clone()
            } else {
                input.initial_run.title
            },
            input.initial_run.kind,
            if input.initial_run.goal.trim().is_empty() {
                input.title.clone()
            } else {
                input.initial_run.goal
            },
            input.initial_run.solution,
        )?;
        if let Some(plan) = input.initial_run.validation_plan {
            validation::set_plan(&mut run, plan)?;
        }
        link(
            &mut run,
            &input.initial_run.agent_conversation,
            &actor,
            &ctx.repository,
        )?;
        let summary = if run.description.is_empty() {
            "开始首次执行".into()
        } else {
            run.description.clone()
        };
        event(&mut run, &actor, "created", summary, None);
        self.transact(key, fingerprint, |_, projects| {
            let project = if let Some(p) = projects.iter().find(|p| p.key == ctx.key) {
                p.clone()
            } else {
                let p = Project {
                    id: id(),
                    key: ctx.key,
                    name: ctx.name,
                    path: ctx.repository,
                };
                projects.push(p.clone());
                p
            };
            let at = now();
            Ok(Task {
                schema_version: 4,
                id: id(),
                title: input.title,
                description: input.description,
                project_id: project.id,
                created_by: actor.source().into(),
                created_at: at,
                runs: vec![run],
                revision: 0,
                updated_at: at,
            })
        })
    }
    pub fn create_task_run(&self, input: CreateTaskRunInput, actor: Actor) -> Result<Receipt> {
        let (key, fingerprint, cached) = self.request_key(
            "create-task-run",
            &input,
            input.operation_id.as_deref(),
            &actor,
        )?;
        if let Some(r) = cached {
            return Ok(r);
        }
        description(&input.description)?;
        let previous = self.get(&input.task_id)?;
        check_new_run(&previous, &actor)?;
        let last = previous.runs.last().unwrap();
        let workspace = input.agent_workspace.clone().unwrap_or(WorkspaceInput {
            path: last.agent_workspace.path.clone(),
            related_commits: vec![],
        });
        let ctx = workspace::inspect(&workspace)?;
        let mut run = new_run(input.description.clone(), ctx.observation.clone(), &actor);
        set_run_content(
            &mut run,
            input.title.clone(),
            input.kind,
            if input.goal.trim().is_empty() {
                input.title.clone()
            } else {
                input.goal
            },
            input.solution,
        )?;
        let plan = input.validation_plan.or_else(|| {
            if last.validation_plan.items.is_empty() {
                None
            } else {
                let mut items = last.validation_plan.items.clone();
                for item in &mut items {
                    item.id.clear();
                }
                Some(ChecklistInput { items })
            }
        });
        if let Some(plan) = plan {
            validation::set_plan(&mut run, plan)?;
        }
        link(&mut run, &input.agent_conversation, &actor, &ctx.repository)?;
        let summary = if input.description.is_empty() {
            run.title.clone()
        } else {
            input.description
        };
        let summary = if last.status == RunStatus::Closed {
            format!("用户手动改为开发中：{summary}")
        } else {
            summary
        };
        event(&mut run, &actor, "created", summary, None);
        self.transact(key, fingerprint, |store, projects| {
            let mut task = Self::load(store, &input.task_id)?;
            check_revision(&task, input.expected_revision)?;
            same_project(project(projects, &task)?, &ctx)?;
            check_new_run(&task, &actor)?;
            task.runs.push(run);
            Ok(task)
        })
    }
    pub fn update_task_run(&self, input: UpdateTaskRunInput, actor: Actor) -> Result<Receipt> {
        self.update_run(input, actor, "update-task-run", None)
    }
    pub fn finish_task_run(&self, input: FinishTaskRunInput, actor: Actor) -> Result<Receipt> {
        text(&input.conclusion, "完成结论", 2000)?;
        workspace::validate_run_files(&input.changed_files)?;
        self.update_run(
            UpdateTaskRunInput {
                task_id: input.task_id,
                run_id: input.run_id,
                expected_revision: input.expected_revision,
                status: Some(RunStatus::ReadyForDelivery),
                note: Some(input.conclusion),
                agent_conversation: input.agent_conversation,
                verification: Some(input.verification),
                agent_workspace: input.agent_workspace,
                operation_id: input.operation_id,
                ..Default::default()
            },
            actor,
            "finish-task-run",
            Some(input.changed_files),
        )
    }
    fn update_run(
        &self,
        input: UpdateTaskRunInput,
        actor: Actor,
        operation: &str,
        finished_files: Option<Vec<String>>,
    ) -> Result<Receipt> {
        let (key, fingerprint, cached) = if let Some(files) = &finished_files {
            self.request_key(
                operation,
                &(&input, files),
                input.operation_id.as_deref(),
                &actor,
            )?
        } else {
            self.request_key(operation, &input, input.operation_id.as_deref(), &actor)?
        };
        if let Some(r) = cached {
            return Ok(r);
        }
        if let Some(value) = &input.title {
            text(value, "执行标题", 160)?;
        }
        if let Some(value) = &input.goal {
            description(value)?;
        }
        if let Some(value) = &input.solution {
            description(value)?;
        }
        if let Some(value) = &input.description {
            description(value)?;
        }
        if let Some(value) = &input.note {
            text(value, "进展摘要", 2000)?;
        }
        if let Some(value) = &input.delivery {
            text(value, "发布引用", 2000)?;
        }
        if input.status == Some(RunStatus::Closed) {
            return Err("结束执行只能通过 App 人工操作".into());
        }
        let mut changes = vec![];
        if let Some(value) = &input.title {
            changes.push(format!("执行标题：{value}"));
        }
        if let Some(value) = &input.goal {
            changes.push(format!(
                "目标：{}",
                value.chars().take(600).collect::<String>()
            ));
        }
        if let Some(value) = &input.solution {
            changes.push(format!(
                "方案：{}",
                value.chars().take(600).collect::<String>()
            ));
        }
        if input.kind.is_some() {
            changes.push("更新执行类型".into());
        }
        if input.status.is_some() {
            changes.push("更新执行状态".into());
        }
        if input.validation_plan.is_some() {
            changes.push("更新验收清单".into());
        }
        let verified = input.verification.is_some();
        if verified {
            changes.push("记录验收结果".into());
        }
        if input.description.is_some() {
            changes.push(
                input
                    .description
                    .clone()
                    .unwrap()
                    .chars()
                    .take(600)
                    .collect(),
            );
        }
        let previous = self.get(&input.task_id)?;
        let last = previous.runs.last().unwrap();
        let mut workspace = input.agent_workspace.clone().unwrap_or(WorkspaceInput {
            path: last.agent_workspace.path.clone(),
            related_commits: vec![],
        });
        for sha in &last.agent_workspace.related_commits {
            if !workspace.related_commits.contains(sha) {
                workspace.related_commits.push(sha.clone());
            }
        }
        // Milestone writes still work if a previously recorded worktree was removed.
        let ctx = match workspace::inspect(&workspace) {
            Ok(c) => Some(c),
            Err(_) if input.agent_workspace.is_none() => None,
            Err(e) => return Err(e),
        };
        self.transact(key, fingerprint, |store, projects| {
            let mut task = Self::load(store, &input.task_id)?;
            check_revision(&task, input.expected_revision)?;
            let repository = project(projects, &task)?.path.clone();
            if let Some(ctx) = &ctx {
                same_project(project(projects, &task)?, ctx)?;
            }
            let run = task.runs.last_mut().unwrap();
            if run.id != input.run_id {
                return Err("历史执行只读，请更新当前执行".into());
            }
            if run.status == RunStatus::Closed {
                return Err("执行已结束，请创建新的执行".into());
            }
            if operation == "finish-task-run" && run.status == RunStatus::Published {
                return Err(
                    "执行已发布；补记发布验证请使用 update-task-run，重新开发请新建执行".into(),
                );
            }
            if let Some(value) = input.title {
                run.title = value;
            }
            if let Some(value) = input.kind {
                run.kind = value;
            }
            if let Some(value) = input.goal {
                run.goal = value;
            }
            if let Some(value) = input.solution {
                run.solution = value;
            }
            if let Some(value) = input.description {
                run.description = value;
            }
            if let Some(ctx) = ctx {
                let retained = run.agent_workspace.related_commits.clone();
                let run_files = run.agent_workspace.run_files.clone();
                run.agent_workspace = ctx.observation;
                run.agent_workspace.run_files = run_files;
                for sha in retained {
                    if !run.agent_workspace.related_commits.contains(&sha) {
                        run.agent_workspace.related_commits.push(sha);
                    }
                }
            } else {
                run.agent_workspace.change_status = ChangeStatus {
                    has_uncommitted_changes: None,
                    has_unpushed_commits: None,
                };
                run.agent_workspace.changed_files.clear();
                run.agent_workspace.observed_at = now();
                run.agent_workspace.error = Some("已记录工作区不可用；代码状态未知".into());
            }
            if let Some(files) = finished_files {
                run.agent_workspace.run_files = Some(files);
            }
            if input.status == Some(RunStatus::Published)
                && run.status != RunStatus::Published
                && input.delivery.is_none()
            {
                return Err("标为已发布时需要实际发布版本或记录".into());
            }
            if let Some(status) = &input.status {
                run.status = status.clone();
            }
            if let Some(plan) = input.validation_plan {
                validation::set_plan(run, plan)?;
            }
            if let Some(verification) = input.verification {
                validation::verify(run, verification, &actor)?;
            }
            if input.status == Some(RunStatus::ReadyForDelivery)
                && !validation::development_ready(run)
            {
                run.status = RunStatus::InProgress;
            }
            link(run, &input.agent_conversation, &actor, &repository)?;
            let summary = input.note.unwrap_or_else(|| {
                if changes.is_empty() {
                    "更新工作区观测".into()
                } else {
                    changes.join("\n")
                }
            });
            event(
                run,
                &actor,
                if input.delivery.is_some() {
                    "delivery"
                } else if verified {
                    "verification"
                } else {
                    "milestone"
                },
                summary,
                input.delivery,
            );
            Ok(task)
        })
    }
    pub fn close(
        &self,
        task_id: &str,
        run_id: &str,
        expected_revision: u64,
        accepted: bool,
        reason: &str,
    ) -> Result<Receipt> {
        text(reason, "结束说明", 2000)?;
        let actor = Actor::human();
        let input = json!({"taskId":task_id,"runId":run_id,"expectedRevision":expected_revision,"accepted":accepted,"reason":reason});
        let (key, fingerprint, cached) = self.request_key("close", &input, None, &actor)?;
        if let Some(r) = cached {
            return Ok(r);
        }
        self.transact(key, fingerprint, |store, _| {
            let mut task = Self::load(store, task_id)?;
            check_revision(&task, expected_revision)?;
            let run = task.runs.last_mut().unwrap();
            if run.id != run_id || run.status == RunStatus::Closed {
                return Err("只能结束当前未关闭的执行".into());
            }
            if accepted && !validation::human_acceptance_ready(run) {
                return Err("当前清单尚未完成人工验收；可以注明原因直接结束".into());
            }
            run.status = RunStatus::Closed;
            run.closure = Some(Closure {
                accepted,
                reason: reason.into(),
                closed_at: now(),
            });
            event(run, &actor, "closed", reason.into(), None);
            Ok(task)
        })
    }
    // Human metadata correction is deliberately not an MCP tool.
    pub fn edit_task(&self, input: EditTaskInput) -> Result<Receipt> {
        text(&input.title, "标题", 160)?;
        description(&input.description)?;
        let actor = Actor::human();
        let (key, fingerprint, cached) = self.request_key("edit-task", &input, None, &actor)?;
        if let Some(r) = cached {
            return Ok(r);
        }
        self.transact(key, fingerprint, |store, _| {
            let mut task = Self::load(store, &input.task_id)?;
            check_revision(&task, input.expected_revision)?;
            task.title = input.title;
            task.description = input.description;
            event(
                task.runs.last_mut().unwrap(),
                &actor,
                "task_edited",
                "修订任务基本信息".into(),
                None,
            );
            Ok(task)
        })
    }
}
#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditTaskInput {
    pub task_id: String,
    pub expected_revision: u64,
    pub title: String,
    pub description: String,
}
