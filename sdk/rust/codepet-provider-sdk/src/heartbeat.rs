//! Host liveness and client-presence reconciliation, independent of Harness adapters.
use crate::{InstanceStartRequest, InstanceStopRequest, ProtocolError, ProtocolServer,
    ProviderInstanceRoute, ProviderPingRequest, ProviderPingResponse};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinSet;

pub const PROVIDER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
pub const PROVIDER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs one bounded exchange at a time. A presence change wakes the next exchange immediately.
/// Returning false ends the driver (for example when the owning process generation exits).
pub async fn run_provider_heartbeats<F, Fut>(
    mut clients: watch::Receiver<crate::ClientConnectionsSnapshot>,
    mut runtime_changes: watch::Receiver<u64>, mut exchange: F,
) where
    F: FnMut(u64, crate::ClientConnectionsSnapshot) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let mut sequence = 0;
    loop {
        sequence += 1;
        runtime_changes.borrow_and_update();
        let snapshot = clients.borrow_and_update().clone();
        if !exchange(sequence, snapshot).await { break; }
        tokio::select! {
            _ = tokio::time::sleep(PROVIDER_HEARTBEAT_INTERVAL) => {},
            changed = clients.changed() => { if changed.is_err() { break; } },
            changed = runtime_changes.changed() => { if changed.is_err() { break; } },
        }
    }
}

pub(crate) struct HostHeartbeat {
    state: Mutex<Option<(ProviderPingRequest, Instant)>>,
    desired: watch::Sender<Option<ProviderPingRequest>>,
}

impl HostHeartbeat {
    pub(crate) fn new() -> Arc<Self> {
        let (desired, _) = watch::channel(None);
        Arc::new(Self { state: Mutex::new(None), desired })
    }

    pub(crate) fn ping(&self, request: ProviderPingRequest) -> Result<ProviderPingResponse, ProtocolError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((previous, _)) = state.as_ref() {
            if request.host_session_id != previous.host_session_id
                || request.sequence <= previous.sequence
                || request.clients.revision < previous.clients.revision
                || (request.clients.revision == previous.clients.revision && request.clients != previous.clients)
            {
                return Err(ProtocolError { code: "stale_heartbeat".into(),
                    message: "Host heartbeat belongs to an old session or sequence".into(),
                    retryable: false, details: None });
            }
        }
        let response = ProviderPingResponse {
            sequence: request.sequence, clients_revision: request.clients.revision,
        };
        *state = Some((request.clone(), Instant::now()));
        self.desired.send_replace(Some(request));
        Ok(response)
    }

    fn expire(&self) { self.expire_at(Instant::now()); }

    fn expire_at(&self, now: Instant) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.as_ref().is_some_and(|(_, at)| now.duration_since(*at) >= PROVIDER_HEARTBEAT_TIMEOUT)
            && self.desired.borrow().is_some()
        {
            self.desired.send_replace(None);
        }
    }

    pub(crate) async fn run<P: ProtocolServer + 'static>(self: Arc<Self>, provider: Arc<P>) {
        let mut receiver = self.desired.subscribe();
        let mut known = HashSet::new();
        let mut workers = JoinSet::new();
        let mut clock = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = clock.tick() => self.expire(),
                result = receiver.changed() => { if result.is_err() { break; } },
                _ = workers.join_next(), if !workers.is_empty() => {}
            }
            let snapshot = receiver.borrow_and_update().clone();
            if let Some(snapshot) = snapshot {
                for route in snapshot.instances {
                    let key = serde_json::to_string(&route).expect("route serializes");
                    if known.insert(key) {
                        workers.spawn(manage_instance(provider.clone(), route, self.desired.subscribe()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ping(sequence: u64, revision: u64) -> ProviderPingRequest {
        ProviderPingRequest { sequence, host_session_id: "host-run".into(),
            clients: crate::ClientConnectionsSnapshot { revision, connections: vec![] }, instances: vec![] }
    }

    #[test]
    fn stale_presence_cannot_replace_current_snapshot_or_refresh_deadline() {
        let heartbeat = HostHeartbeat::new();
        heartbeat.ping(ping(1, 4)).unwrap();
        let at = heartbeat.state.lock().unwrap().as_ref().unwrap().1;
        assert!(heartbeat.ping(ping(2, 3)).is_err());
        assert!(heartbeat.ping(ping(1, 4)).is_err());
        heartbeat.expire_at(at + PROVIDER_HEARTBEAT_TIMEOUT);
        assert!(heartbeat.desired.borrow().is_none());
        assert!(heartbeat.ping(ping(1, 4)).is_err());
        heartbeat.ping(ping(3, 5)).unwrap();
        assert!(heartbeat.desired.borrow().is_some());
    }
}

fn wanted(receiver: &watch::Receiver<Option<ProviderPingRequest>>, route: &ProviderInstanceRoute) -> bool {
    receiver.borrow().as_ref().is_some_and(|value|
        !value.clients.connections.is_empty() && value.instances.contains(route))
}

async fn wait_inactive(receiver: &mut watch::Receiver<Option<ProviderPingRequest>>, route: &ProviderInstanceRoute) {
    while wanted(receiver, route) {
        if receiver.changed().await.is_err() { break; }
    }
}

async fn manage_instance<P: ProtocolServer + 'static>(
    provider: Arc<P>, route: ProviderInstanceRoute,
    mut receiver: watch::Receiver<Option<ProviderPingRequest>>,
) {
    loop {
        let desired = wanted(&receiver, &route);
        receiver.borrow_and_update();
        // Lifecycle methods are idempotent. Reconcile with the adapter each time,
        // so a crash or an explicit stop cannot leave a stale local "active" flag.
        if desired {
            let start = provider.instance_start(InstanceStartRequest { route: route.clone() });
            let mut changes = receiver.clone();
            let cancelled = tokio::select! {
                _ = start => false,
                _ = wait_inactive(&mut changes, &route) => true,
            };
            if cancelled {
                // Stop also cancels in-flight initialize; the adapter owns generation checks.
                let _ = provider.instance_stop(InstanceStopRequest { route: route.clone() }).await;
            }
        } else {
            let _ = provider.instance_stop(InstanceStopRequest { route: route.clone() }).await;
        }
        if wanted(&receiver, &route) != desired { continue; }
        if receiver.changed().await.is_err() { break; }
    }
}
