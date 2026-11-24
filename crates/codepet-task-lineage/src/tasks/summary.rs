use super::{Closure, RunStatus, Task};
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCounts {
    pub total: usize,
    pub in_progress: usize,
    pub ready_for_delivery: usize,
    pub published: usize,
    pub closed: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub status: RunStatus,
    pub run_counts: RunCounts,
    pub manually_completed: bool,
}

impl Task {
    /// Read compatibility only: an old current-run closure represented a human Task lock.
    /// Preserve that intent without rewriting historical runs or writing on reads.
    pub(super) fn normalize_completion(&mut self) {
        // Old writers reject schema 5, preventing them from dropping the Task completion lock.
        // Reads upgrade only in memory; the next atomic mutation persists the guarded format.
        self.schema_version = 5;
        if self.status_aggregation_version == 0 {
            if let Some(run) = self.runs.last().filter(|r| r.status == RunStatus::Closed) {
                self.completion = Some(run.closure.clone().unwrap_or(Closure {
                    accepted: false,
                    reason: "保留旧版人工完成状态".into(),
                    closed_at: run.updated_at,
                }));
            }
            self.status_aggregation_version = 1;
        }
    }

    pub fn summary(&self) -> TaskSummary {
        let mut counts = RunCounts {
            total: self.runs.len(),
            ..Default::default()
        };
        for run in &self.runs {
            match run.status {
                RunStatus::InProgress => counts.in_progress += 1,
                RunStatus::ReadyForDelivery => counts.ready_for_delivery += 1,
                RunStatus::Published => counts.published += 1,
                RunStatus::Closed => counts.closed += 1,
            }
        }
        let status = if self.completion.is_some() {
            RunStatus::Closed
        } else if counts.in_progress > 0 {
            RunStatus::InProgress
        } else if counts.ready_for_delivery > 0 {
            RunStatus::ReadyForDelivery
        } else if counts.published > 0 {
            RunStatus::Published
        } else if counts.total > 0 {
            RunStatus::Closed
        } else {
            // Empty histories are rejected by Ledger; never imply vacuous completion.
            RunStatus::InProgress
        };
        TaskSummary {
            status,
            run_counts: counts,
            manually_completed: self.completion.is_some(),
        }
    }
}
