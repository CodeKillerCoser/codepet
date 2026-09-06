//! Execution retention is independent of Remote client presence.
use crate::generated::{Approval, ApprovalStatus, InstanceStatus, JsonRpcResponse,
    JsonRpcResponsePayload, ProtocolEvent, ProtocolMethod, ProviderInstanceRoute, TurnTask, TurnStatus};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::RwLock;

#[derive(Default)]
struct InstanceActivity {
    // Keep terminal IDs until instance teardown: a late start response cannot resurrect a turn.
    turns: HashMap<(String, String), bool>,
    approvals: HashMap<(String, String), ((String, String), bool)>,
}

#[derive(Default)]
pub(super) struct Activity {
    instances: Mutex<HashMap<String, InstanceActivity>>,
    // A request owns a read guard before dispatch, through response observation. Idle stop
    // takes the write guard, so it cannot race an admitted request's startup window.
    pub gate: RwLock<()>,
}

impl Activity {
    pub fn busy(&self, route: &ProviderInstanceRoute) -> bool {
        self.instances.lock().unwrap_or_else(|e| e.into_inner())
            .get(&route.provider_instance_id).is_some_and(|state|
                state.turns.values().any(|active| *active) || state.approvals.values().any(|(_, pending)| *pending))
    }

    fn turn(&self, turn: &TurnTask) {
        let active = matches!(turn.status, TurnStatus::Queued | TurnStatus::Running | TurnStatus::WaitingApproval);
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let state = instances.entry(turn.resource.provider_id.clone()).or_default();
        let key = (turn.conversation.native_resource_id.clone(), turn.resource.native_resource_id.clone());
        if !active {
            for (turn_id, pending) in state.approvals.values_mut() {
                if turn_id == &key { *pending = false; }
            }
        }
        state.turns.entry(key)
            .and_modify(|previous| *previous &= active).or_insert(active);
    }

    fn approval(&self, approval: &Approval) {
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let state = instances.entry(approval.resource.provider_id.clone()).or_default();
        let turn_key = (approval.conversation.native_resource_id.clone(), approval.turn.native_resource_id.clone());
        let pending = approval.status == ApprovalStatus::Pending
            && state.turns.get(&turn_key) != Some(&false);
        state.approvals.entry((approval.conversation.native_resource_id.clone(), approval.resource.native_resource_id.clone()))
            .and_modify(|(_, previous)| *previous &= pending)
            .or_insert((turn_key, pending));
    }

    pub fn event(&self, event: &ProtocolEvent) {
        match event {
            ProtocolEvent::EventTurnUpserted { params, .. } => self.turn(&params.turn),
            ProtocolEvent::EventApprovalRequested { params, .. } => self.approval(&params.approval),
            ProtocolEvent::EventApprovalResolved { params, .. } => self.approval(&params.approval),
            ProtocolEvent::EventInstanceStatusChanged { params, .. }
                if matches!(params.instance.status, InstanceStatus::Stopped | InstanceStatus::Error) => {
                    self.instances.lock().unwrap_or_else(|e| e.into_inner())
                        .remove(&params.instance.route.provider_instance_id);
                }
            _ => {}
        }
    }

    pub fn response(&self, method: ProtocolMethod, response: &JsonRpcResponse) {
        let JsonRpcResponsePayload::Ok { result } = &response.response else { return; };
        if matches!(method, ProtocolMethod::TurnStart | ProtocolMethod::TurnSteer | ProtocolMethod::TurnInterrupt) {
            if let Some(value) = result.get("turn") {
                if let Ok(turn) = serde_json::from_value::<TurnTask>(value.clone()) { self.turn(&turn); }
            }
        }
        // Approval events are authoritative; a delayed resolve response must not recreate pending state.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::*;
    use serde_json::json;

    pub(super) fn route() -> ProviderInstanceRoute {
        ProviderInstanceRoute { device_id: "device".into(), provider_plugin_id: "plugin".into(), provider_instance_id: "instance".into() }
    }

    fn turn(id: &str, status: TurnStatus) -> TurnTask {
        TurnTask { resource: RoutedResourceId { provider_id: "instance".into(), native_resource_id: id.into() },
            conversation: RoutedResourceId { provider_id: "instance".into(), native_resource_id: "thread".into() },
            status, display_summary: None, started_at: None, updated_at: None, completed_at: None }
    }

    #[test]
    fn terminal_turn_cannot_be_resurrected_by_late_start_response_or_finish_another_turn() {
        let activity = Activity::default();
        activity.turn(&turn("a", TurnStatus::Running));
        activity.turn(&turn("b", TurnStatus::WaitingApproval));
        activity.turn(&turn("a", TurnStatus::Completed));
        assert!(activity.busy(&route()));
        activity.turn(&turn("b", TurnStatus::Interrupted));
        let reply = JsonRpcResponse { jsonrpc: "2.0".into(), id: Some("request".into()),
            response: JsonRpcResponsePayload::Ok { result: json!({"turn": turn("a", TurnStatus::Running)}) } };
        activity.response(ProtocolMethod::TurnStart, &reply);
        assert!(!activity.busy(&route()));
    }

    #[test]
    fn pending_approval_retains_instance_until_resolved_or_its_turn_ends() {
        let activity = Activity::default();
        let mut approval: Approval = serde_json::from_value(json!({
            "resource": {"providerId":"instance", "nativeResourceId":"approval"},
            "conversation": {"providerId":"instance", "nativeResourceId":"thread"},
            "turn": {"providerId":"instance", "nativeResourceId":"a"},
            "kind":"command", "title":"Allow command", "status":"pending", "decisions":[]
        })).unwrap();
        activity.approval(&approval);
        assert!(activity.busy(&route()));
        activity.turn(&turn("a", TurnStatus::Failed));
        activity.approval(&approval);
        assert!(!activity.busy(&route()));
        approval.resource.native_resource_id = "another".into();
        approval.turn.native_resource_id = "b".into();
        activity.approval(&approval);
        assert!(activity.busy(&route()));
        approval.status = ApprovalStatus::Approved;
        activity.approval(&approval);
        assert!(!activity.busy(&route()));
    }
}
