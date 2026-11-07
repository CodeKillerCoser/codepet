//! Authenticated client presence. LAN discovery and Provider processes do not own this state.
use codepet_provider_sdk::{ClientConnectionInfo, ClientConnectionsSnapshot};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tokio::sync::watch;
use super::traffic::{ConnectionTraffic, ConnectionTrafficSnapshot};

pub struct RemoteConnections {
    connections: Mutex<BTreeMap<String, String>>,
    traffic: Mutex<BTreeMap<String, (String, Arc<ConnectionTraffic>)>>,
    pub(crate) ownership: Arc<super::channels::ownership::Ownership>,
    visibility: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    snapshots: watch::Sender<ClientConnectionsSnapshot>,
}

impl Default for RemoteConnections {
    fn default() -> Self {
        let (snapshots, _) = watch::channel(ClientConnectionsSnapshot { revision: 0, connections: vec![] });
        Self { ownership: Arc::default(), visibility: Mutex::default(), connections: Mutex::new(BTreeMap::new()), traffic: Mutex::new(BTreeMap::new()), snapshots }
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

    pub fn register_with_traffic(self: &Arc<Self>, client_id: String, traffic: Arc<ConnectionTraffic>) -> RemoteConnection {
        let connection = self.register(client_id.clone());
        self.traffic.lock().unwrap().insert(connection.id.clone(), (client_id, traffic));
        connection
    }
    pub(crate) fn register_controlled(self: &Arc<Self>, client_id: String, traffic: Arc<ConnectionTraffic>, visible: Arc<AtomicBool>) -> RemoteConnection {
        let id = uuid::Uuid::new_v4().to_string();
        let mut entries = self.connections.lock().unwrap();
        self.visibility.lock().unwrap().insert(id.clone(), visible);
        entries.insert(id.clone(), client_id.clone());
        self.traffic.lock().unwrap().insert(id.clone(), (client_id, traffic));
        self.publish(&entries);
        RemoteConnection { owner: self.clone(), id }
    }
    pub(crate) fn refresh(&self) { self.publish(&self.connections.lock().unwrap()); }
    fn visible(&self, id: &str) -> bool {
        self.visibility.lock().unwrap().get(id).is_none_or(|active| active.load(Ordering::SeqCst))
    }
    pub fn traffic_for_client(&self, client_id: &str) -> Vec<ConnectionTrafficSnapshot> {
        self.traffic.lock().unwrap().iter().filter(|(key, (id, _))| id == client_id && self.visible(key))
            .map(|(_, (_, traffic))| traffic.snapshot()).collect()
    }

    fn publish(&self, entries: &BTreeMap<String, String>) {
        let revision = self.snapshots.borrow().revision.saturating_add(1);
        self.snapshots.send_replace(ClientConnectionsSnapshot { revision,
            connections: entries.iter().filter(|(id, _)| self.visible(id)).map(|(id, client)| ClientConnectionInfo {
                connection_id: id.clone(), client_id: client.clone(),
            }).collect() });
    }
}

pub struct RemoteConnection { owner: Arc<RemoteConnections>, id: String }

impl Drop for RemoteConnection {
    fn drop(&mut self) {
        let mut entries = self.owner.connections.lock().unwrap_or_else(|e| e.into_inner());
        entries.remove(&self.id);
        self.owner.traffic.lock().unwrap().remove(&self.id);
        self.owner.visibility.lock().unwrap().remove(&self.id);
        self.owner.publish(&entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traffic_belongs_to_each_live_connection_and_is_removed_on_close() {
        let state = Arc::new(RemoteConnections::default());
        let meter = Arc::new(ConnectionTraffic::new("lan")); meter.sent(123);
        let first = state.register_with_traffic("phone".into(), meter);
        let second = state.register_with_traffic("phone".into(), Arc::new(ConnectionTraffic::new("relay")));
        assert_eq!(state.traffic_for_client("phone").len(), 2);
        assert!(state.traffic_for_client("other").is_empty());
        drop(first);
        let remaining = state.traffic_for_client("phone");
        assert_eq!(remaining[0].channel, "relay");
        assert_eq!(remaining[0].sent_bytes, 0);
        drop(second);
        assert!(state.traffic_for_client("phone").is_empty());
    }
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
