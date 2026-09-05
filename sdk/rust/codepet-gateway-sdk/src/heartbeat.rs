//! Application heartbeat admission for a single authenticated Gateway connection.
use std::time::{Duration, Instant};

pub const GATEWAY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
pub const GATEWAY_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct GatewayHeartbeat {
    last_sequence: Option<u64>,
    last_ping: Instant,
}

impl Default for GatewayHeartbeat {
    fn default() -> Self { Self { last_sequence: None, last_ping: Instant::now() } }
}

impl GatewayHeartbeat {
    pub fn accept(&mut self, sequence: u64) -> bool {
        if self.last_sequence.is_some_and(|previous| sequence <= previous) { return false; }
        self.last_sequence = Some(sequence);
        self.last_ping = Instant::now();
        true
    }

    pub fn deadline(&self) -> Instant { self.last_ping + GATEWAY_HEARTBEAT_TIMEOUT }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_ping_cannot_extend_liveness() {
        let mut heartbeat = GatewayHeartbeat::default();
        assert!(heartbeat.accept(4));
        let deadline = heartbeat.deadline();
        assert!(!heartbeat.accept(4));
        assert!(!heartbeat.accept(3));
        assert_eq!(heartbeat.deadline(), deadline);
        assert!(heartbeat.accept(5));
    }
}
