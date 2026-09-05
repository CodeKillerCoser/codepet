use super::*;
use codepet_provider_sdk::{ConnectionStatus, ProviderPingRequest, run_provider_heartbeats};
use std::sync::Weak;
use std::time::Instant;

impl PluginManager {
    pub(super) fn spawn_heartbeat(&self, plugin_id: String, generation: u64, process: Weak<PluginProcess>) {
        let owner = Arc::downgrade(&self.inner);
        let clients = self.inner.remote_connections.subscribe();
        let host_session_id = Uuid::new_v4().to_string();
        let last_pong = Arc::new(StdMutex::new(Instant::now()));
        tokio::spawn(run_provider_heartbeats(clients, self.subscribe_status_changes(), move |sequence, clients| {
            let owner = owner.clone();
            let process = process.clone();
            let plugin_id = plugin_id.clone();
            let host_session_id = host_session_id.clone();
            let last_pong = last_pong.clone();
            async move {
                let (Some(inner), Some(process)) = (owner.upgrade(), process.upgrade()) else { return false; };
                let manager = PluginManager { inner };
                if manager.inner.shutting_down.load(Ordering::SeqCst) { return false; }
                let instances = {
                    let plugins = manager.inner.plugins.read().await;
                    let Some(entry) = plugins.get(&plugin_id) else { return false; };
                    if entry.generation != generation || entry.state != PluginRuntimeState::Ready
                        || !entry.process.as_ref().is_some_and(|p| Arc::ptr_eq(p, &process)) { return false; }
                    entry.instances.values().filter(|i| i.record.enabled && i.instance.is_some())
                        .map(|i| i.record.route()).collect()
                };
                let revision = clients.revision;
                let result = process.client().provider_ping(ProviderPingRequest {
                    sequence, host_session_id, clients, instances,
                }).await;
                let status = match result {
                    Ok(pong) if pong.sequence == sequence && pong.clients_revision == revision => {
                        *last_pong.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                        ConnectionStatus::Online
                    }
                    _ if last_pong.lock().unwrap_or_else(|e| e.into_inner()).elapsed()
                        >= codepet_provider_sdk::PROVIDER_HEARTBEAT_TIMEOUT => ConnectionStatus::Offline,
                    _ => return true,
                };
                let update = {
                    let mut plugins = manager.inner.plugins.write().await;
                    let Some(entry) = plugins.get_mut(&plugin_id) else { return false; };
                    if entry.generation != generation || !entry.process.as_ref().is_some_and(|p| Arc::ptr_eq(p, &process)) { return false; }
                    if entry.connection_status == status { return true; }
                    entry.connection_status = status;
                    HostUpdate::PluginStateChanged { snapshot: entry.snapshot(), previous_state: entry.state }
                };
                let _ = manager.send_update(update).await;
                true
            }
        }));
    }
}
