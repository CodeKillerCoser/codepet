//! Application heartbeats report liveness; they never own Harness lifecycle.
use crate::generated::{ProtocolError, ProviderPingRequest, ProviderPingResponse};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub use super::protocol::{PROVIDER_HEARTBEAT_INTERVAL, PROVIDER_HEARTBEAT_TIMEOUT};

/// Runs one bounded exchange at a time. A presence change wakes the next exchange immediately.
/// Returning false ends the driver (for example when the owning process generation exits).
pub async fn run_provider_heartbeats<F, Fut>(
    mut clients: watch::Receiver<crate::generated::ClientConnectionsSnapshot>,
    mut runtime_changes: watch::Receiver<u64>, mut exchange: F,
) where
    F: FnMut(u64, crate::generated::ClientConnectionsSnapshot) -> Fut,
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
    state: Mutex<Option<ProviderPingRequest>>,
}

impl HostHeartbeat {
    pub(crate) fn new() -> Arc<Self> { Arc::new(Self { state: Mutex::new(None) }) }

    pub(crate) fn ping(&self, request: ProviderPingRequest) -> Result<ProviderPingResponse, ProtocolError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = state.as_ref() {
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
        let response = ProviderPingResponse { sequence: request.sequence, clients_revision: request.clients.revision };
        *state = Some(request);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ping(sequence: u64, revision: u64) -> ProviderPingRequest {
        ProviderPingRequest { sequence, host_session_id: "host-run".into(),
            clients: crate::generated::ClientConnectionsSnapshot { revision, connections: vec![] }, instances: vec![] }
    }
    #[test]
    fn stale_presence_cannot_replace_current_snapshot() {
        let heartbeat = HostHeartbeat::new();
        heartbeat.ping(ping(1, 4)).unwrap();
        assert!(heartbeat.ping(ping(2, 3)).is_err());
        assert!(heartbeat.ping(ping(1, 4)).is_err());
        assert_eq!(heartbeat.state.lock().unwrap().as_ref().unwrap().sequence, 1);
        heartbeat.ping(ping(3, 5)).unwrap();
        assert_eq!(heartbeat.state.lock().unwrap().as_ref().unwrap().sequence, 3);
    }
}
