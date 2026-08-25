use super::event_bus::EventSubscription;
use super::gateway::Gateway;
use super::generated::{
    dispatch, EventSequence, ProtocolError, ProtocolEvent, ProtocolRequest, ProtocolResponse,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type TransportFuture<'a> = Pin<Box<dyn Future<Output = ProtocolResponse> + Send + 'a>>;

pub trait Transport: Send + Sync {
    fn request<'a>(&'a self, request: ProtocolRequest) -> TransportFuture<'a>;

    fn subscribe(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<EventSubscription, ProtocolError>;

    fn replay(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError>;
}

#[derive(Clone)]
pub struct LocalTransport {
    gateway: Arc<Gateway>,
}

impl LocalTransport {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self { gateway }
    }

    pub fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }

    pub async fn dispatch(&self, request: ProtocolRequest) -> ProtocolResponse {
        dispatch(self.gateway.as_ref(), request).await
    }

    pub fn subscribe_events(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<EventSubscription, ProtocolError> {
        self.gateway.subscribe_events(after_sequence)
    }

    pub fn replay_events(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        self.gateway.replay_events(after_sequence)
    }
}

impl Transport for LocalTransport {
    fn request<'a>(&'a self, request: ProtocolRequest) -> TransportFuture<'a> {
        Box::pin(async move { self.dispatch(request).await })
    }

    fn subscribe(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<EventSubscription, ProtocolError> {
        self.subscribe_events(after_sequence)
    }

    fn replay(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        self.replay_events(after_sequence)
    }
}
