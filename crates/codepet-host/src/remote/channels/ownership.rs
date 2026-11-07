//! Transport-independent ownership. No Gateway/Provider methods or network IO.
use codepet_channel_control_sdk::ControlStatus;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

#[derive(Default)]
pub(crate) struct Ownership { groups: Mutex<BTreeMap<String, Group>> }
#[derive(Default)]
struct Group {
    connections: BTreeMap<String, Arc<AtomicBool>>,
    active: Option<String>,
    switch_id: Option<String>,
}
pub(crate) struct OwnershipLease {
    registry: Arc<Ownership>, scope: String, id: String,
    pub(crate) active: Arc<AtomicBool>,
}
impl Ownership {
    pub(crate) fn prepare(self: &Arc<Self>, authenticated_credential: &str, session: &str) -> OwnershipLease {
        let scope = format!("{}:{}:{}", authenticated_credential.len(), authenticated_credential, session);
        let id = uuid::Uuid::new_v4().to_string();
        let active = Arc::new(AtomicBool::new(false));
        self.groups.lock().unwrap().entry(scope.clone()).or_default().connections.insert(id.clone(), active.clone());
        OwnershipLease { registry: self.clone(), scope, id, active }
    }
}
impl OwnershipLease {
    pub(crate) fn status(&self) -> ControlStatus {
        let groups = self.registry.groups.lock().unwrap();
        let group = &groups[&self.scope];
        ControlStatus { connection_id: self.id.clone(), active_connection_id: group.active.clone(), last_switch_id: group.switch_id.clone() }
    }
    /// Compare-and-swap, with idempotence limited to the last committed switch.
    /// Callers resolve lost ACKs by querying the same authoritative group.
    pub(crate) fn activate(&self, expected: Option<&str>, switch_id: &str) -> bool {
        let mut groups = self.registry.groups.lock().unwrap();
        let group = groups.get_mut(&self.scope).unwrap();
        if group.switch_id.as_deref() == Some(switch_id) {
            return group.active.as_deref() == Some(&self.id);
        }
        if group.active.as_deref() != expected { return false; }
        if let Some(old) = group.active.as_ref().and_then(|id| group.connections.get(id)) { old.store(false, Ordering::SeqCst); }
        group.active = Some(self.id.clone());
        group.switch_id = Some(switch_id.to_owned());
        self.active.store(true, Ordering::SeqCst);
        true
    }
}
impl Drop for OwnershipLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        let mut groups = self.registry.groups.lock().unwrap();
        if let Some(group) = groups.get_mut(&self.scope) {
            group.connections.remove(&self.id);
            if group.active.as_deref() == Some(&self.id) { group.active = None; }
            if group.connections.is_empty() { groups.remove(&self.scope); }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cas_lost_ack_old_owner_fencing_and_independent_sessions() {
        let state = Arc::new(Ownership::default());
        let old = state.prepare("credential", "session");
        let new = state.prepare("credential", "session");
        let independent = state.prepare("credential", "other-session");
        let other_device = state.prepare("other-credential", "session");
        assert!(old.activate(None, "switch1"));
        assert!(!new.activate(None, "switch2"));
        assert!(new.activate(Some(&old.id), "switch2"));
        assert!(new.activate(Some(&old.id), "switch2")); // lost ACK retry
        assert!(!old.active.load(Ordering::SeqCst));
        assert_eq!(new.status().active_connection_id.as_deref(), Some(new.id.as_str()));
        assert!(!old.activate(Some(&new.id), "switch2")); // same ID, wrong target
        assert!(independent.activate(None, "switch2"));
        assert!(other_device.activate(None, "switch2"));
        drop(old);
        assert!(new.active.load(Ordering::SeqCst));
        drop(new); drop(independent); drop(other_device);
        assert!(state.groups.lock().unwrap().is_empty());
    }
    #[test]
    fn thousand_switches_leave_one_owner_and_no_registry_leak() {
        let state = Arc::new(Ownership::default());
        let mut current: Option<OwnershipLease> = None;
        for i in 0..1000 {
            let next = state.prepare("credential", "session");
            assert!(next.activate(current.as_ref().map(|v| v.id.as_str()), &i.to_string()));
            if let Some(old) = current.as_ref() { assert!(!old.active.load(Ordering::SeqCst)); }
            current = Some(next);
            assert_eq!(state.groups.lock().unwrap().values().next().unwrap().connections.len(), 1);
        }
        drop(current);
        assert!(state.groups.lock().unwrap().is_empty());
    }
}
