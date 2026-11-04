//! One instance-wide activity projection owns automatic idle release decisions.
use crate::generated::{Approval, ApprovalStatus, Conversation, ConversationStatus, InstanceStatus,
    JsonRpcResponse, JsonRpcResponsePayload, ProtocolEvent, ProtocolMethod, ProtocolRequest,
    ProviderInstance, ProviderInstanceRoute, TurnTask, TurnStatus};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Default)]
struct InstanceActivity {
    route: Option<ProviderInstanceRoute>,
    ready: bool,
    idle_since: Option<Instant>,
    // Terminal IDs survive until teardown; delayed start replies cannot resurrect work.
    turns: HashMap<(String, String), bool>,
    approvals: HashMap<(String, String), ((String, String), bool)>,
    conversations: HashMap<String, Option<String>>,
    uncertain: HashSet<String>,
    generation: u64,
}
impl InstanceActivity {
    fn busy(&self) -> bool {
        self.turns.values().any(|active| *active)
            || self.approvals.values().any(|(_, pending)| *pending)
            || !self.conversations.is_empty() || !self.uncertain.is_empty()
    }
    fn reconcile_idle(&mut self, now: Instant) {
        if self.busy() { self.idle_since = None; }
        else if self.ready && self.idle_since.is_none() { self.idle_since = Some(now); }
    }
}

pub(super) struct Activity {
    instances: Mutex<HashMap<String, InstanceActivity>>,
    now: Arc<dyn Fn() -> Instant + Send + Sync>,
    // All admitted requests hold a read guard through response observation.
    // Idle release takes the write guard and rechecks activity before stop.
    pub gate: RwLock<()>,
}
impl Default for Activity {
    fn default() -> Self {
        Self { instances: Mutex::new(HashMap::new()), now: Arc::new(Instant::now), gate: RwLock::new(()) }
    }
}
impl Activity {
    #[cfg(test)]
    pub(super) fn with_clock(now: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        Self { now, ..Self::default() }
    }
    #[cfg(test)]
    pub fn busy(&self, route: &ProviderInstanceRoute) -> bool {
        self.instances.lock().unwrap_or_else(|e| e.into_inner())
            .get(&route.provider_instance_id).is_some_and(InstanceActivity::busy)
    }
    pub fn idle_routes(&self, timeout: Duration) -> Vec<ProviderInstanceRoute> {
        let now = (self.now)();
        self.instances.lock().unwrap_or_else(|e| e.into_inner()).values()
            .filter(|state| state.ready && !state.busy()
                && state.idle_since.is_some_and(|at| now.saturating_duration_since(at) >= timeout))
            .filter_map(|state| state.route.clone()).collect()
    }
    pub(super) fn instance(&self, instance: &ProviderInstance) {
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let state = instances.entry(instance.route.provider_instance_id.clone()).or_default();
        if matches!(instance.status, InstanceStatus::Starting | InstanceStatus::Stopped | InstanceStatus::Error) {
            let generation = state.generation.wrapping_add(1);
            *state = InstanceActivity { generation, ..InstanceActivity::default() };
        }
        state.route = Some(instance.route.clone());
        state.ready = instance.status == InstanceStatus::Ready;
        if !state.ready { state.idle_since = None; }
        state.reconcile_idle((self.now)());
    }
    // Only user/business mutations refresh the idle window. Polls, history and pings do not.
    pub fn request_activity(request: &ProtocolRequest) -> Option<String> {
        let id = match request {
            ProtocolRequest::InstanceStart { params, .. } => &params.route.provider_instance_id,
            ProtocolRequest::ConversationCreate { params, .. } => &params.route.provider_instance_id,
            ProtocolRequest::ConversationAcquireInteraction { params, .. } => &params.conversation.provider_instance_id,
            ProtocolRequest::TurnStart { params, .. } => &params.conversation.provider_instance_id,
            ProtocolRequest::TurnSteer { params, .. } => &params.conversation.provider_instance_id,
            ProtocolRequest::ApprovalResolve { params, .. } => &params.approval.provider_instance_id,
            _ => return None,
        };
        Some(id.clone())
    }
    pub fn mutation(&self, request: &ProtocolRequest) -> Option<(String, String, u64)> {
        let resource = match request {
            ProtocolRequest::TurnStart { params, .. } => &params.conversation,
            ProtocolRequest::TurnSteer { params, .. } => &params.conversation,
            _ => return None,
        };
        let states = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let state = states.get(&resource.provider_instance_id)?;
        Some((resource.provider_instance_id.clone(), resource.native_resource_id.clone(), state.generation))
    }
    pub fn mutation_response(&self, mutation: Option<(String, String, u64)>, response: &JsonRpcResponse) {
        let Some((instance, conversation, generation)) = mutation else { return; };
        let JsonRpcResponsePayload::Error { error } = &response.response else { return; };
        let unknown = error.data.as_ref().and_then(|data| data.get("details"))
            .and_then(|details| details.get("outcome")).and_then(|value| value.as_str()) == Some("unknown");
        if !unknown { return; }
        let mut states = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = states.get_mut(&instance) {
            // Without an accepted turn ID a terminal from another/older turn is
            // insufficient proof. Keep unknown execution until explicit cleanup.
            if state.ready && state.generation == generation {
                state.uncertain.insert(conversation);
                state.reconcile_idle((self.now)());
            }
        }
    }
    pub fn touch(&self, instance_id: &str) {
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = instances.get_mut(instance_id) {
            state.idle_since = None;
            state.reconcile_idle((self.now)());
        }
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
            if state.conversations.get(&key.0).is_some_and(|id| id.as_ref().is_none_or(|id| id == &key.1)) {
                state.conversations.remove(&key.0);
            }
        }
        state.turns.entry(key).and_modify(|previous| *previous &= active).or_insert(active);
        state.reconcile_idle((self.now)());
    }
    fn approval(&self, approval: &Approval) {
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let state = instances.entry(approval.resource.provider_id.clone()).or_default();
        let turn_key = (approval.conversation.native_resource_id.clone(), approval.turn.native_resource_id.clone());
        let pending = approval.status == ApprovalStatus::Pending && state.turns.get(&turn_key) != Some(&false);
        state.approvals.entry((approval.conversation.native_resource_id.clone(), approval.resource.native_resource_id.clone()))
            .and_modify(|(_, previous)| *previous &= pending).or_insert((turn_key, pending));
        state.reconcile_idle((self.now)());
    }
    fn conversation(&self, conversation: &Conversation) {
        if let Some(turn) = &conversation.active_turn { self.turn(turn); }
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let state = instances.entry(conversation.resource.provider_id.clone()).or_default();
        let id = &conversation.resource.native_resource_id;
        let turn = conversation.active_turn.as_ref().map(|turn| turn.resource.native_resource_id.clone());
        let terminal = turn.as_ref().is_some_and(|turn| state.turns.get(&(id.clone(), turn.clone())) == Some(&false));
        if matches!(conversation.status, ConversationStatus::Running | ConversationStatus::WaitingApproval | ConversationStatus::WaitingUserInput) && !terminal {
            state.conversations.insert(id.clone(), turn);
        } else { state.conversations.remove(id); }
        state.reconcile_idle((self.now)());
    }
    pub fn event(&self, event: &ProtocolEvent) {
        match event {
            ProtocolEvent::EventTurnUpserted { params, .. } => self.turn(&params.turn),
            ProtocolEvent::EventConversationUpserted { params, .. } => self.conversation(&params.conversation),
            ProtocolEvent::EventApprovalRequested { params, .. } => self.approval(&params.approval),
            ProtocolEvent::EventApprovalResolved { params, .. } => self.approval(&params.approval),
            ProtocolEvent::EventInstanceStatusChanged { params, .. } => self.instance(&params.instance),
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
        // Lifecycle events are authoritative; only create seeds a route for adapters
        // that return a ready instance without emitting a status transition.
        if method == ProtocolMethod::InstanceCreate {
            if let Some(value) = result.get("instance") {
                if let Ok(instance) = serde_json::from_value::<ProviderInstance>(value.clone()) {
                    let known = self.instances.lock().unwrap_or_else(|e| e.into_inner())
                        .get(&instance.route.provider_instance_id).is_some_and(|state| state.route.is_some());
                    if !known { self.instance(&instance); }
                }
            }
        }
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
