use crate::generated::ProviderTransportLimits;
use crate::transport::error::TransportError;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn error(message: impl ToString) -> TransportError { TransportError::new(message) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class { Normal = 0, Small = 1, Control = 2 }
impl Class {
    pub(super) fn parse(v: u8) -> Result<Self, TransportError> { match v { 0 => Ok(Self::Normal), 1 => Ok(Self::Small), 2 => Ok(Self::Control), _ => Err(error("invalid message class")) } }
}
pub(super) struct Lane { pub(super) bytes: Arc<Semaphore>, pub(super) slots: Arc<Semaphore>, pub(super) workers: Arc<Semaphore> }
pub(super) struct Resources { pub(super) limits: ProviderTransportLimits, pub(super) lanes: [Lane; 3] }
impl Resources {
    pub(super) fn new(limits: ProviderTransportLimits) -> Self {
        let lanes = [
            (limits.receive_budget_bytes, limits.normal_streams, 2),
            (limits.small_receive_budget_bytes, limits.small_streams, 2),
            (limits.control_receive_budget_bytes, limits.control_streams, 1),
        ].map(|(bytes, slots, workers)| Lane { bytes: Arc::new(Semaphore::new(bytes as usize)), slots: Arc::new(Semaphore::new(slots as usize)), workers: Arc::new(Semaphore::new(workers)) });
        Self { limits, lanes }
    }
    pub(super) fn caps(&self, class: Class) -> (usize, usize) {
        let v = &self.limits;
        match class {
            Class::Normal => (v.max_encoded_message_bytes as usize, v.max_decoded_message_bytes as usize),
            Class::Small => (v.small_message_bytes.min(v.max_encoded_message_bytes) as usize, v.small_message_bytes as usize),
            Class::Control => (v.control_message_bytes.min(v.max_encoded_message_bytes) as usize, v.control_message_bytes as usize),
        }
    }
    pub(super) fn idle(&self) -> Duration { Duration::from_millis(self.limits.idle_timeout_ms) }
}
