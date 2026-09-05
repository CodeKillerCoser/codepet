//! Authenticated client presence. LAN discovery and Provider processes do not own this state.
use codepet_provider_sdk::{ClientConnectionInfo, ClientConnectionsSnapshot};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub struct RemoteConnections {
    connections: Mutex<BTreeMap<String, String>>,
    snapshots: watch::Sender<ClientConnectionsSnapshot>,
}

impl Default for RemoteConnections {
    fn default() -> Self {
        let (snapshots, _) = watch::channel(ClientConnectionsSnapshot { revision: 0, connections: vec![] });
        Self { connections: Mutex::new(BTreeMap::new()), snapshots }
    }
}

impl RemoteConnections {
    pub fn snapshot(&self) -> ClientConnectionsSnapshot { self.snapshots.borrow().clone() }
    pub fn subscribe(&self) -> watch::Receiver<ClientConnectionsSnapshot> { self.snapshots.subscribe() }

    pub fn register(self: &Arc<Self>, client_id: String) -> RemoteConnection {
        let id = uuid::Uuid::new_v4().to_string();
        let mut entries = self.connections.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(id.clone(), client_id);
        self.publish(&entries);
        RemoteConnection { owner: self.clone(), id }
    }

    fn publish(&self, entries: &BTreeMap<String, String>) {
        let revision = self.snapshots.borrow().revision.saturating_add(1);
        self.snapshots.send_replace(ClientConnectionsSnapshot { revision,
            connections: entries.iter().map(|(id, client)| ClientConnectionInfo {
                connection_id: id.clone(), client_id: client.clone(),
            }).collect() });
    }
}

pub struct RemoteConnection { owner: Arc<RemoteConnections>, id: String }

impl Drop for RemoteConnection {
    fn drop(&mut self) {
        let mut entries = self.owner.connections.lock().unwrap_or_else(|e| e.into_inner());
        entries.remove(&self.id);
        self.owner.publish(&entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_and_two_clients_have_independent_lifetimes() {
        let state = Arc::new(RemoteConnections::default());
        let old = state.register("phone-a".into());
        let new = state.register("phone-a".into());
        let other = state.register("phone-b".into());
        drop(old);
        assert_eq!(state.snapshot().connections.len(), 2);
        drop(other);
        assert_eq!(state.snapshot().connections[0].client_id, "phone-a");
        drop(new);
        assert!(state.snapshot().connections.is_empty());
        assert_eq!(state.snapshot().revision, 6);
    }
}
