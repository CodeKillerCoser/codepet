use crate::agents::AgentView;
use crate::events::{frontend_event, PetEvent, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

pub const COLLECTOR_PORT: u16 = 47621;
const MAX_EVENTS: usize = 1000;
const MAX_FRONTEND_EVENTS: usize = 120;

#[derive(Clone, Default)]
pub struct SharedState {
    inner: Arc<Mutex<AppState>>,
}

#[derive(Default)]
struct AppState {
    agents: Vec<AgentView>,
    events: Vec<PetEvent>,
    approvals: HashMap<String, PendingApproval>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalBehavior {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecision {
    pub behavior: ApprovalBehavior,
    pub message: Option<String>,
}

#[derive(Clone)]
struct PendingApproval {
    decision: Option<ApprovalDecision>,
    notify: Arc<Notify>,
}

impl SharedState {
    pub fn set_agents(&self, agents: Vec<AgentView>) {
        if let Ok(mut state) = self.inner.lock() {
            state.agents = agents;
        }
    }

    pub fn push_event(&self, event: PetEvent) {
        if let Ok(mut state) = self.inner.lock() {
            if event.status == TaskStatus::WaitingApproval {
                state.approvals.insert(
                    event.id.clone(),
                    PendingApproval {
                        decision: None,
                        notify: Arc::new(Notify::new()),
                    },
                );
            }
            state.events.push(event);
            if state.events.len() > MAX_EVENTS {
                let overflow = state.events.len() - MAX_EVENTS;
                state.events.drain(0..overflow);
            }
        }
    }

    pub fn recent_events(&self) -> Vec<PetEvent> {
        self.inner
            .lock()
            .map(|state| {
                state.events
                    .iter()
                    .skip(state.events.len().saturating_sub(MAX_FRONTEND_EVENTS))
                    .map(frontend_event)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn event_by_id(&self, event_id: &str) -> Option<PetEvent> {
        self.inner
            .lock()
            .ok()
            .and_then(|state| state.events.iter().find(|event| event.id == event_id).cloned())
    }

    pub fn resolve_approval(&self, event_id: &str, decision: ApprovalDecision) -> bool {
        let notify = self.inner.lock().ok().and_then(|mut state| {
            let approval = state.approvals.get_mut(event_id)?;
            approval.decision = Some(decision);
            Some(approval.notify.clone())
        });
        if let Some(notify) = notify {
            notify.notify_waiters();
            true
        } else {
            false
        }
    }

    pub async fn wait_for_approval(&self, event_id: &str, timeout: Duration) -> Option<ApprovalDecision> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let approval = self.inner.lock().ok().and_then(|state| {
                state.approvals.get(event_id).map(|approval| {
                    (
                        approval.decision.clone(),
                        approval.notify.clone(),
                    )
                })
            })?;

            if let Some(decision) = approval.0 {
                return Some(decision);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            if tokio::time::timeout_at(deadline, approval.1.notified()).await.is_err() {
                return None;
            }
        }
    }
}
