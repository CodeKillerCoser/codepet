use crate::generated::{ProtocolError, ProtocolEvent, ProviderWireMessage};
use crate::message::{error, codec::serialized_size};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::{mpsc, Semaphore, OwnedSemaphorePermit};

/// Publishes typed Provider notifications without exposing stdout or JSON-RPC framing.
pub trait ProviderEventSink: Send + Sync + 'static {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError>;
}

impl<F> ProviderEventSink for F
where
    F: Fn(ProtocolEvent) -> Result<(), ProtocolError> + Send + Sync + 'static,
{
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        self(event)
    }
}

pub(super) type QueuedEvent = (ProviderWireMessage, OwnedSemaphorePermit);

pub(super) fn queued_events(output_closed: Arc<AtomicBool>) -> (Arc<dyn ProviderEventSink>, mpsc::Receiver<QueuedEvent>) {
    let (events, event_rx) = mpsc::channel(256);
    let event_slots = Arc::new(Semaphore::new(256 * 1024 * 1024));
    let sink_closed = output_closed.clone();
    let sink: Arc<dyn ProviderEventSink> = Arc::new(move |event| {
        if sink_closed.load(Ordering::SeqCst) { return Ok(()); }
        let message = ProviderWireMessage::Event(event);
        let size = serialized_size(&message, 128 * 1024 * 1024)?;
        let permit = event_slots.clone().try_acquire_many_owned(size as u32).map_err(|_| error("event byte budget is full"))?;
        events.try_send((message, permit)).map_err(error)
    });
    (sink, event_rx)
}
