use crate::generated::{ProtocolError, ProtocolEvent, ProviderWireMessage};
use crate::message::{error, codec::serialized_size};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::{mpsc, Semaphore, OwnedSemaphorePermit};

/// Publishes typed Provider notifications without exposing stdout or JSON-RPC framing.
pub trait ProviderEventSink: Send + Sync + 'static {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError>;

    /// Ordered batch. The stdio sink admits the complete batch or none of it.
    /// Custom sinks may retain sequential publication semantics.
    fn publish_batch(&self, events: Vec<ProtocolEvent>) -> Result<(), ProtocolError> {
        events.into_iter().try_for_each(|event| self.publish(event))
    }
}

impl<F> ProviderEventSink for F
where
    F: Fn(ProtocolEvent) -> Result<(), ProtocolError> + Send + Sync + 'static,
{
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        self(event)
    }
}

pub(super) type QueuedEvent = Vec<(ProviderWireMessage, OwnedSemaphorePermit)>;

struct QueuedEvents {
    events: mpsc::Sender<QueuedEvent>,
    event_slots: Arc<Semaphore>,
    closed: Arc<AtomicBool>,
}

impl ProviderEventSink for QueuedEvents {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        self.publish_batch(vec![event])
    }

    fn publish_batch(&self, events: Vec<ProtocolEvent>) -> Result<(), ProtocolError> {
        if self.closed.load(Ordering::SeqCst) { return Ok(()); }
        if events.is_empty() { return Ok(()); }
        let mut batch = Vec::with_capacity(events.len());
        for event in events {
            let message = ProviderWireMessage::Event(event);
            let size = serialized_size(&message, 128 * 1024 * 1024)?;
            let permit = self.event_slots.clone().try_acquire_many_owned(size as u32).map_err(|_| error("event byte budget is full"))?;
            batch.push((message, permit));
        }
        // Preserve the byte budget while avoiding a partial >256-row scan.
        self.events.try_send(batch).map_err(error)
    }
}

pub(super) fn queued_events(output_closed: Arc<AtomicBool>) -> (Arc<dyn ProviderEventSink>, mpsc::Receiver<QueuedEvent>) {
    let (events, event_rx) = mpsc::channel(256);
    let event_slots = Arc::new(Semaphore::new(256 * 1024 * 1024));
    let sink: Arc<dyn ProviderEventSink> = Arc::new(QueuedEvents { events, event_slots, closed: output_closed });
    (sink, event_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: usize) -> ProtocolEvent {
        ProtocolEvent::EventConversationDeleted { jsonrpc: "2.0".into(), params: crate::ConversationDeletedEvent {
            conversation: crate::ProviderResourceId { device_id: "d".into(), provider_plugin_id: "p".into(), provider_instance_id: "i".into(), native_resource_id: id.to_string() },
        } }
    }

    #[test]
    fn first_scan_over_256_events_is_admitted_without_partial_publication() {
        let (sink, mut receiver) = queued_events(Arc::new(AtomicBool::new(false)));
        sink.publish_batch((0..600).map(event).collect()).unwrap();
        let batch = receiver.try_recv().unwrap();
        assert_eq!(batch.len(), 600);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn full_queue_rejects_whole_batch_and_allows_retry_after_drain() {
        let (sink, mut receiver) = queued_events(Arc::new(AtomicBool::new(false)));
        for id in 0..256 { sink.publish(event(id)).unwrap(); }
        assert!(sink.publish_batch((256..856).map(event).collect()).is_err());
        for _ in 0..256 { assert_eq!(receiver.try_recv().unwrap().len(), 1); }
        assert!(receiver.try_recv().is_err());
        sink.publish_batch((256..856).map(event).collect()).unwrap();
        assert_eq!(receiver.try_recv().unwrap().len(), 600);
    }
}
