use super::generated::{EventSequence, ProtocolError, ProtocolEvent, PROTOCOL_VERSION};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub const DEFAULT_EVENT_WINDOW_CAPACITY: usize = 256;

struct EventState {
    sequence: EventSequence,
    events: VecDeque<ProtocolEvent>,
}

pub struct GatewayEventBus {
    capacity: usize,
    state: Mutex<EventState>,
    sender: broadcast::Sender<ProtocolEvent>,
}

impl GatewayEventBus {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let (sender, _) = broadcast::channel(capacity);
        Self {
            capacity,
            state: Mutex::new(EventState {
                sequence: 0,
                events: VecDeque::with_capacity(capacity),
            }),
            sender,
        }
    }

    pub fn current_sequence(&self) -> EventSequence {
        self.state.lock().map(|state| state.sequence).unwrap_or(0)
    }

    pub fn publish(&self, mut event: ProtocolEvent) -> Result<ProtocolEvent, ProtocolError> {
        let assigned = {
            let mut state = self.state.lock().map_err(|_| event_state_error())?;
            let sequence = state.sequence.checked_add(1).ok_or_else(|| ProtocolError {
                code: "event_sequence_exhausted".to_string(),
                message: "runtime gateway event sequence is exhausted".to_string(),
                retryable: false,
                details: None,
            })?;
            set_event_sequence(&mut event, sequence);
            state.sequence = sequence;
            state.events.push_back(event.clone());
            while state.events.len() > self.capacity {
                state.events.pop_front();
            }
            event
        };
        let _ = self.sender.send(assigned.clone());
        Ok(assigned)
    }

    pub fn replay(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        let state = self.state.lock().map_err(|_| event_state_error())?;
        if let Some(after_sequence) = after_sequence {
            if after_sequence > state.sequence {
                return Err(sequence_error(
                    "invalid_event_sequence",
                    "requested event sequence is ahead of the gateway",
                    after_sequence,
                    state.sequence,
                ));
            }
            if let Some(oldest_sequence) = state.events.front().map(event_sequence) {
                if after_sequence.saturating_add(1) < oldest_sequence {
                    return Err(sequence_error(
                        "event_replay_unavailable",
                        "requested events are outside the in-memory replay window",
                        after_sequence,
                        state.sequence,
                    ));
                }
            }
        }
        Ok(state
            .events
            .iter()
            .filter(|event| after_sequence.map_or(true, |sequence| event_sequence(event) > sequence))
            .cloned()
            .collect())
    }

    pub fn subscribe(
        &self,
        after_sequence: Option<EventSequence>,
    ) -> Result<EventSubscription, ProtocolError> {
        let receiver = self.sender.subscribe();
        let replay = self.replay(after_sequence)?;
        Ok(EventSubscription {
            replay: replay.into(),
            receiver,
            last_sequence: after_sequence.unwrap_or(0),
        })
    }
}

#[derive(Clone)]
pub struct ProviderEventSink {
    events: Arc<GatewayEventBus>,
}

impl ProviderEventSink {
    pub(crate) fn new(events: Arc<GatewayEventBus>) -> Self {
        Self { events }
    }

    pub fn publish(&self, event: ProtocolEvent) -> Result<ProtocolEvent, ProtocolError> {
        self.events.publish(event)
    }
}

pub struct EventSubscription {
    replay: VecDeque<ProtocolEvent>,
    receiver: broadcast::Receiver<ProtocolEvent>,
    last_sequence: EventSequence,
}

impl EventSubscription {
    pub async fn next_event(&mut self) -> Result<ProtocolEvent, ProtocolError> {
        loop {
            let event = match self.replay.pop_front() {
                Some(event) => event,
                None => match self.receiver.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        let mut details = BTreeMap::new();
                        details.insert("skippedEvents".to_string(), serde_json::Value::from(skipped));
                        return Err(ProtocolError {
                            code: "event_subscription_lagged".to_string(),
                            message: "event subscriber fell behind the live event stream".to_string(),
                            retryable: true,
                            details: Some(details),
                        });
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(ProtocolError {
                            code: "event_subscription_closed".to_string(),
                            message: "runtime gateway event stream is closed".to_string(),
                            retryable: true,
                            details: None,
                        });
                    }
                },
            };
            let sequence = event_sequence(&event);
            if sequence > self.last_sequence {
                self.last_sequence = sequence;
                return Ok(event);
            }
        }
    }
}

pub fn event_sequence(event: &ProtocolEvent) -> EventSequence {
    match event {
        ProtocolEvent::ProviderStatusChanged { event_sequence, .. }
        | ProtocolEvent::ConversationUpserted { event_sequence, .. }
        | ProtocolEvent::TurnUpserted { event_sequence, .. }
        | ProtocolEvent::TurnOutputDelta { event_sequence, .. }
        | ProtocolEvent::ApprovalRequested { event_sequence, .. }
        | ProtocolEvent::ApprovalResolved { event_sequence, .. } => *event_sequence,
    }
}

fn set_event_sequence(event: &mut ProtocolEvent, sequence: EventSequence) {
    match event {
        ProtocolEvent::ProviderStatusChanged {
            protocol_version,
            event_sequence,
            ..
        }
        | ProtocolEvent::ConversationUpserted {
            protocol_version,
            event_sequence,
            ..
        }
        | ProtocolEvent::TurnUpserted {
            protocol_version,
            event_sequence,
            ..
        }
        | ProtocolEvent::TurnOutputDelta {
            protocol_version,
            event_sequence,
            ..
        }
        | ProtocolEvent::ApprovalRequested {
            protocol_version,
            event_sequence,
            ..
        }
        | ProtocolEvent::ApprovalResolved {
            protocol_version,
            event_sequence,
            ..
        } => {
            *protocol_version = PROTOCOL_VERSION;
            *event_sequence = sequence;
        }
    }
}

fn event_state_error() -> ProtocolError {
    ProtocolError {
        code: "gateway_state_error".to_string(),
        message: "runtime gateway event state is unavailable".to_string(),
        retryable: true,
        details: None,
    }
}

fn sequence_error(
    code: &str,
    message: &str,
    requested: EventSequence,
    current: EventSequence,
) -> ProtocolError {
    let mut details = BTreeMap::new();
    details.insert("requestedSequence".to_string(), serde_json::Value::from(requested));
    details.insert("currentSequence".to_string(), serde_json::Value::from(current));
    ProtocolError {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        details: Some(details),
    }
}
