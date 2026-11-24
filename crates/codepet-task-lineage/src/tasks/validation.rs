//! Keep the existing version/target/actor acceptance rules under the new record shape.
use super::{id, legacy, legacy_model as old, model::*, now};
use crate::Result;

pub(super) fn legacy_round(run: &TaskRun) -> old::Round {
    old::Round {
        id: run.id.clone(),
        number: 1,
        opened_at: run.created_at,
        opened_reason: run.description.clone(),
        previous_round_id: None,
        objective: run.goal.clone(),
        description: String::new(),
        phase: match run.status {
            RunStatus::Published => old::Phase::Published,
            RunStatus::ReadyForDelivery => old::Phase::ReadyForDelivery,
            _ => old::Phase::InProgress,
        },
        sources: vec![],
        code_refs: vec![],
        plans: run.validation_plan.revisions.clone(),
        verifications: run
            .validation_plan
            .records
            .iter()
            .map(|v| old::Verification {
                id: v.id.clone(),
                actor: v.actor.clone(),
                created_at: v.created_at,
                workspace_observation: None,
                input: v.input.clone(),
            })
            .collect(),
        deliveries: vec![],
        closure: None,
        events: vec![],
    }
}
pub(super) fn set_plan(run: &mut TaskRun, input: ChecklistInput) -> Result<()> {
    let mut old = legacy_round(run);
    legacy::append_plan(
        &mut old,
        old::PlanInput {
            summary: if !run.solution.trim().is_empty() {
                run.solution.chars().take(4000).collect()
            } else if !run.goal.trim().is_empty() {
                run.goal.chars().take(4000).collect()
            } else {
                "验收计划".into()
            },
            affected_areas: vec![],
            criteria: input.items,
        },
        now(),
    )?;
    run.validation_plan.revisions = old.plans;
    let latest = run.validation_plan.revisions.last().unwrap();
    run.validation_plan.revision = latest.revision;
    run.validation_plan.items = latest.input.criteria.clone();
    Ok(())
}
pub(super) fn verify(run: &mut TaskRun, input: VerificationInput, actor: &Actor) -> Result<()> {
    legacy::validate_verification(&legacy_round(run), &input)?;
    if input.results.iter().any(|r| r.result == TestResult::Failed) {
        run.status = RunStatus::InProgress;
    }
    run.validation_plan.records.push(Verification {
        id: id(),
        actor: actor.role().into(),
        agent: actor.source().into(),
        created_at: now(),
        agent_workspace: Some(run.agent_workspace.clone()),
        input,
    });
    Ok(())
}
pub fn human_acceptance_ready(run: &TaskRun) -> bool {
    legacy::human_acceptance_ready(&legacy_round(run))
}
