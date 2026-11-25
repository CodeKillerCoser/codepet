use codepet_task_lineage::tasks::{Ledger, RunStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Stable business evidence only. Observing Git or incrementing a receipt is not an edit.
pub fn documents(ledger: &Ledger) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for task in ledger.list()? {
        for run in &task.runs {
            let verifications = run
                .validation_plan
                .records
                .iter()
                .map(|r| {
                    json!({
                        "at":r.created_at,"actor":r.actor,"result":r.input,
                    })
                })
                .collect::<Vec<_>>();
            let mut seen = std::collections::HashSet::new();
            let timeline = run.timeline.iter().filter(|e| !e.kind.contains("workspace") && e.summary != "历史工作区观测")
                .filter(|e| seen.insert((e.at,e.kind.as_str(),e.summary.as_str())))
                .map(|e| json!({"at":e.at,"kind":e.kind,"summary":e.summary,"reference":e.reference})).collect::<Vec<_>>();
            let text = serde_json::to_string_pretty(&json!({
                "task":{"id":task.id,"title":task.title,"background":task.description},
                "run":{"id":run.id,"title":run.title,"kind":run.kind,"goal":run.goal,
                    "solution":run.solution,"conclusion":run.description,"status":run.status,
                    "workspace":run.agent_workspace.path,"changedFiles":run.agent_workspace.run_files,
                    "acceptanceCriteria":run.validation_plan.items,"verificationRecords":verifications,
                    "timeline":timeline,"closure":run.closure},
                "evidencePolicy":"Goals and acceptance criteria are plans. Verification records are reported checks; only closure records user acceptance."
            })).map_err(|e| e.to_string())?;
            if text.len() > 512 * 1024 {
                return Err("Run 文档超过 512 KiB，召回索引暂不可用".into());
            }
            let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
            result.push(json!({"runId":run.id,"taskId":task.id,"taskTitle":task.title,
                "text":text,"contentHash":hash,"eligible":!matches!(run.status, RunStatus::InProgress),
                "workspace":run.agent_workspace.path,"updatedAt":run.updated_at,
                "summary":{"taskId":task.id,"taskTitle":task.title,"runId":run.id,"title":run.title,
                    "kind":run.kind,"goal":run.goal.chars().take(400).collect::<String>(),"status":run.status,
                    "revision":task.revision,"runCount":task.runs.len(),"createdBy":run.created_by,
                    "isCurrentRun":task.runs.last().is_some_and(|r| r.id == run.id),
                    "workspace":run.agent_workspace.path,"createdAt":run.created_at,"updatedAt":run.updated_at}}));
        }
    }
    Ok(result)
}
