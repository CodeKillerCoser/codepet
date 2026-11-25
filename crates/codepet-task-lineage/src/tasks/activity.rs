//! Server-authored activity. Existing entries are immutable, even across later writers.
use super::{id, model::*, now, Result};
use serde_json::{json, Value};

pub(super) fn event<'a>(
    run: &'a mut TaskRun,
    actor: &Actor,
    kind: &str,
    summary: String,
    reference: Option<String>,
) -> &'a mut TimelineEvent {
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
        actor_role: Some(actor.role().into()),
        agent_conversation: vec![],
        changes: vec![],
        reply_to: None,
    });
    run.timeline.last_mut().unwrap()
}

pub(super) fn context(run: &mut TaskRun, start: usize, actor: &Actor, ids: &[String]) {
    let source = if actor.is_human() {
        "codex"
    } else {
        actor.source()
    };
    let conversations: Vec<_> = run
        .agent_conversation
        .iter()
        .filter(|c| {
            c.agent == source
                && ids.iter().any(|id| {
                    id.strip_prefix("codex://threads/").unwrap_or(id) == c.conversation_id
                })
        })
        .cloned()
        .collect();
    for entry in &mut run.timeline[start..] {
        entry.agent_conversation = conversations.clone();
    }
}

fn changed(changes: &mut Vec<ActivityChange>, field: &str, before: Value, after: Value) {
    if before != after {
        changes.push(ActivityChange {
            field: field.into(),
            before,
            after,
        });
    }
}
pub(super) fn initial(run: &TaskRun) -> Vec<ActivityChange> {
    [
        ("title", json!(run.title)),
        ("kind", json!(run.kind)),
        ("goal", json!(run.goal)),
        ("solution", json!(run.solution)),
        ("description", json!(run.description)),
        ("status", json!(run.status)),
        ("validationPlan", plan(run)),
    ]
    .into_iter()
    .map(|(field, after)| ActivityChange {
        field: field.into(),
        before: Value::Null,
        after,
    })
    .collect()
}
fn plan(run: &TaskRun) -> Value {
    json!({"revision":run.validation_plan.revision,"items":run.validation_plan.items})
}

fn status(value: &RunStatus) -> &'static str {
    match value {
        RunStatus::InProgress => "开发中",
        RunStatus::ReadyForDelivery => "待确认",
        RunStatus::Published => "已发布",
        RunStatus::Closed => "已完成",
    }
}

pub(super) fn update(
    run: &mut TaskRun,
    before: &TaskRun,
    actor: &Actor,
    note: Option<String>,
    delivery: Option<String>,
    finished: bool,
    conversations: &[String],
) {
    let start = run.timeline.len();
    let mut changes = vec![];
    for (field, old, new) in [
        ("title", json!(before.title), json!(run.title)),
        ("kind", json!(before.kind), json!(run.kind)),
        ("goal", json!(before.goal), json!(run.goal)),
        ("solution", json!(before.solution), json!(run.solution)),
        (
            "description",
            json!(before.description),
            json!(run.description),
        ),
    ] {
        changed(&mut changes, field, old, new);
    }
    if !changes.is_empty() {
        event(run, actor, "content_updated", "更新执行内容".into(), None).changes = changes;
    }
    let mut old = json!(before.agent_workspace);
    let mut new = json!(run.agent_workspace);
    old.as_object_mut().unwrap().remove("observedAt");
    new.as_object_mut().unwrap().remove("observedAt");
    if old != new {
        let field = if before.agent_workspace.path != run.agent_workspace.path {
            "workspace_changed"
        } else {
            "workspace_observed"
        };
        event(run, actor, field, "更新执行工作区与代码证据".into(), None).changes =
            vec![ActivityChange {
                field: "agentWorkspace".into(),
                before: old,
                after: new,
            }];
    }
    if before.validation_plan.revision != run.validation_plan.revision {
        let change = ActivityChange {
            field: "validationPlan".into(),
            before: plan(before),
            after: plan(run),
        };
        let summary = format!(
            "修订验收计划：第 {} 版 → 第 {} 版",
            before.validation_plan.revision, run.validation_plan.revision
        );
        event(run, actor, "plan_updated", summary, None).changes = vec![change];
    }
    if run.validation_plan.records.len() > before.validation_plan.records.len() {
        let record = run.validation_plan.records.last().unwrap();
        let change = ActivityChange {
            field: "verification".into(),
            before: Value::Null,
            after: json!(record),
        };
        let reference = Some(record.id.clone());
        event(
            run,
            actor,
            "verification",
            "记录逐项验证结果".into(),
            reference,
        )
        .changes = vec![change];
    }
    if json!(before.agent_conversation) != json!(run.agent_conversation) {
        let change = ActivityChange {
            field: "agentConversation".into(),
            before: json!(before.agent_conversation),
            after: json!(run.agent_conversation),
        };
        event(
            run,
            actor,
            "conversation_linked",
            "关联执行对话".into(),
            None,
        )
        .changes = vec![change];
    }
    if let Some(reference) = delivery {
        event(
            run,
            actor,
            "delivery",
            "登记发布记录".into(),
            Some(reference),
        );
    }
    if before.status != run.status {
        let change = ActivityChange {
            field: "status".into(),
            before: json!(before.status),
            after: json!(run.status),
        };
        let summary = format!("状态：{} → {}", status(&before.status), status(&run.status));
        event(run, actor, "status_changed", summary, None).changes = vec![change];
    }
    if let Some(note) = note {
        event(
            run,
            actor,
            if finished { "finished" } else { "comment" },
            note,
            None,
        );
    }
    if run.timeline.len() == start {
        event(
            run,
            actor,
            "milestone",
            "更新执行（无业务字段变更）".into(),
            None,
        );
    }
    context(run, start, actor, conversations);
}

pub(super) fn validate(previous: Option<&Task>, task: &Task) -> Result<()> {
    let mut ids: std::collections::HashSet<_> = previous
        .into_iter()
        .flat_map(|task| &task.runs)
        .flat_map(|run| &run.timeline)
        .map(|entry| &entry.id)
        .collect();
    if let Some(previous) = previous {
        for (index, old) in previous.runs.iter().enumerate() {
            let new = task
                .runs
                .get(index)
                .filter(|run| run.id == old.id)
                .ok_or("不能删除或重排已有执行")?;
            if new.timeline.len() < old.timeline.len()
                || json!(&new.timeline[..old.timeline.len()]) != json!(old.timeline)
            {
                return Err("活动时间线只允许追加，不能修改、删除或重排已有事件".into());
            }
        }
    }
    for run in &task.runs {
        let start = previous
            .and_then(|task| task.runs.iter().find(|old| old.id == run.id))
            .map_or(0, |old| old.timeline.len());
        for entry in &run.timeline[start..] {
            if entry.id.is_empty() || !ids.insert(&entry.id) {
                return Err("新增活动 ID 不能重复或为空".into());
            }
        }
    }
    Ok(())
}
