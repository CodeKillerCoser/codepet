//! Application payload bytes, excluding TLS/ICE/CPG1 headers and retransmits.
use serde::Serialize;
use std::{collections::VecDeque, sync::Mutex, time::Instant};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTrafficSnapshot {
    pub channel: String,
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub sent_bytes_per_second: f64,
    pub received_bytes_per_second: f64,
}

struct State {
    channel: String,
    sent: u64,
    received: u64,
    buckets: VecDeque<(u64, u64, u64)>,
}

pub struct ConnectionTraffic {
    started: Instant,
    state: Mutex<State>,
}

impl ConnectionTraffic {
    pub fn new(channel: &str) -> Self {
        Self {
            started: Instant::now(),
            state: Mutex::new(State {
                channel: channel.into(),
                sent: 0,
                received: 0,
                buckets: VecDeque::new(),
            }),
        }
    }
    pub fn set_channel(&self, channel: &str) {
        self.state.lock().unwrap().channel = channel.into();
    }
    pub fn sent(&self, bytes: usize) {
        self.record(bytes as u64, 0);
    }
    pub fn received(&self, bytes: usize) {
        self.record(0, bytes as u64);
    }
    fn record(&self, sent: u64, received: u64) {
        let now = self.started.elapsed().as_millis() as u64;
        self.record_at(now, sent, received);
    }
    fn record_at(&self, now: u64, sent: u64, received: u64) {
        let mut state = self.state.lock().unwrap();
        state.sent += sent;
        state.received += received;
        let tick = now / 100;
        if state.buckets.back().is_none_or(|row| row.0 != tick) {
            state.buckets.push_back((tick, 0, 0));
        }
        let row = state.buckets.back_mut().unwrap();
        row.1 += sent;
        row.2 += received;
        while state.buckets.front().is_some_and(|row| row.0 + 30 <= tick) {
            state.buckets.pop_front();
        }
    }
    pub fn snapshot(&self) -> ConnectionTrafficSnapshot {
        self.snapshot_at(self.started.elapsed().as_millis() as u64)
    }
    fn snapshot_at(&self, now: u64) -> ConnectionTrafficSnapshot {
        let state = self.state.lock().unwrap();
        let (sent, received) = state
            .buckets
            .iter()
            .filter(|row| row.0 + 30 > now / 100)
            .fold((0, 0), |(a, b), row| (a + row.1, b + row.2));
        let seconds = (now as f64 / 1000.0).clamp(0.1, 3.0);
        ConnectionTrafficSnapshot {
            channel: state.channel.clone(),
            sent_bytes: state.sent,
            received_bytes: state.received,
            sent_bytes_per_second: sent as f64 / seconds,
            received_bytes_per_second: received as f64 / seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_rate_expires_while_totals_survive_and_directions_stay_separate() {
        let traffic = ConnectionTraffic::new("ipv6-direct");
        traffic.record_at(500, 300, 0);
        traffic.record_at(1000, 0, 900);
        let first = traffic.snapshot_at(1500);
        assert_eq!(first.sent_bytes_per_second, 200.0);
        assert_eq!(first.received_bytes_per_second, 600.0);
        let idle = traffic.snapshot_at(5000);
        assert_eq!((idle.sent_bytes, idle.received_bytes), (300, 900));
        assert_eq!(
            idle.sent_bytes_per_second + idle.received_bytes_per_second,
            0.0
        );
    }
}
