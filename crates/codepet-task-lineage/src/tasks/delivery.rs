//! Read projections of saved Git observations. No extra Git scan or model call.
use super::legacy::{Round, Task};
use crate::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryFact {
    pub label: String,
    pub scope: String,
    pub observed_at: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryFacts {
    pub summary: String,
    pub items: Vec<DeliveryFact>,
    pub note: String,
}

pub fn delivery_facts(round: &Round) -> DeliveryFacts {
    let mut facts=DeliveryFacts {summary:"Git 状态待确认".into(),items:vec![],note:"按登记时的本地 Git 观测显示；工作树改动由多个任务共享，上游使用本地缓存，不代表实时远端检查。".into()};
    let Some(latest) = round.code_refs.last() else {
        return facts;
    };
    if latest.error.is_some() {
        return facts;
    }
    if latest.dirty == Some(true) {
        facts.items.push(DeliveryFact {
            label: "待提交".into(),
            scope: "shared_worktree".into(),
            observed_at: latest.observed_at,
        });
    }
    // Keep the last recorded upstream observation for each explicitly linked SHA.
    let mut commits = BTreeMap::new();
    for observation in &round.code_refs {
        for commit in &observation.commits {
            commits.insert(
                &commit.sha,
                (commit.in_cached_upstream, observation.observed_at),
            );
        }
    }
    if !commits.is_empty() {
        let label = if commits.values().any(|(pushed, _)| *pushed == Some(false)) {
            "待推送"
        } else if commits.values().all(|(pushed, _)| *pushed == Some(true)) {
            "已推送"
        } else {
            "推送待确认"
        };
        let observed_at = commits.values().map(|(_, at)| *at).min().unwrap();
        facts.items.push(DeliveryFact {
            label: label.into(),
            scope: "linked_commits".into(),
            observed_at,
        });
    } else if facts.items.is_empty() {
        facts.items.push(DeliveryFact {
            label: "提交待关联".into(),
            scope: "linked_commits".into(),
            observed_at: latest.observed_at,
        });
    }
    facts.summary = facts
        .items
        .iter()
        .map(|f| {
            if f.scope == "shared_worktree" {
                format!("{}（共享工作树）", f.label)
            } else {
                f.label.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    facts
}

/// Desktop views enrich raw stored rounds; derived delivery information is never persisted.
pub fn task_view(task: &Task) -> Result<Value> {
    let mut view = serde_json::to_value(task).map_err(|e| e.to_string())?;
    for (index, round) in task.rounds.iter().enumerate() {
        view["rounds"][index]["deliveryFacts"] =
            serde_json::to_value(delivery_facts(round)).map_err(|e| e.to_string())?;
    }
    Ok(view)
}
