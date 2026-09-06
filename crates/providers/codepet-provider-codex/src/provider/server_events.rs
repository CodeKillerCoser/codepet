//! One reader per instance Server. Conversation slots serialize operations, not processes.
use super::*;
use std::collections::VecDeque;

impl CodexInstanceRuntime {
    pub(super) fn start_server_forwarder(
        self: &Arc<Self>, generation: String, session: CodexAppServerSession,
        incoming: Receiver<Result<CodexIncoming, CodexAppServerError>>,
    ) {
        let owner = Arc::downgrade(self);
        thread::spawn(move || {
            let mut pending = VecDeque::new();
            loop {
                let message = incoming.recv_timeout(Duration::from_millis(25));
                let Some(runtime) = owner.upgrade() else { return; };
                if lock(&runtime.mutable).server_generation.as_deref() != Some(&generation) { return; }
                match message {
                    Ok(Ok(message)) => pending.push_back(message),
                    Ok(Err(error)) => { runtime.fail_server(&generation, CodexProtocolMapper::error(error)); return; }
                    Err(RecvTimeoutError::Disconnected) => {
                        runtime.fail_server(&generation, CodexProtocolMapper::error(CodexAppServerError::Shutdown)); return;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
                // A resume/start response and its events can arrive before the RPC installs its slot.
                // Keep those events while continuing to dispatch other conversations.
                let count = pending.len();
                for _ in 0..count {
                    let message = pending.pop_front().expect("pending event");
                    if incoming_conversation_id(&message).is_some_and(|id| session.is_ephemeral_thread(id)) {
                        continue;
                    }
                    let events = match message {
                        CodexIncoming::Notification(CodexNotification::ThreadNameUpdated { thread_id, thread_name }) => {
                            runtime.conversation_upsert_event(&session, &thread_id, thread_name).into_iter().collect()
                        }
                        CodexIncoming::Notification(CodexNotification::ThreadStatusChanged { thread_id, status }) => {
                            runtime.conversation_status_upsert_event(&session, &thread_id, status).into_iter().collect()
                        }
                        message => {
                            if let Some(conversation_id) = incoming_conversation_id(&message).map(str::to_owned) {
                                let Some(slot) = runtime.execution_slot(&conversation_id, &generation) else {
                                    pending.push_back(message);
                                    continue;
                                };
                                let _operation = lock(&slot.operation);
                                if !slot.matches_generation(&generation) { continue; }
                                if let CodexIncoming::Notification(CodexNotification::TurnCompleted { turn, .. }) = &message {
                                    if turn.status == CodexTurnStatus::InProgress {
                                        eprintln!("Codex ignored nonterminal turn/completed");
                                        continue;
                                    }
                                    slot.finish_active_turn(&generation, Some(&turn.id));
                                }
                                match runtime.map_execution_incoming(&conversation_id, &generation, &session, message) {
                                    Ok(events) => {
                                        for event in events {
                                            if let Err(error) = runtime.events.publish(event) {
                                                if runtime.handle_event_error(&generation, error).is_err() { return; }
                                            }
                                        }
                                        continue;
                                    }
                                    Err(error) => { eprintln!("Codex conversation event mapping failed: {}", error.message); continue; }
                                }
                            } else {
                                match lock(&runtime.mapper).events(message) {
                                    Ok(events) => events,
                                    Err(error) => { eprintln!("Codex server event mapping failed: {}", error.message); continue; }
                                }
                            }
                        }
                    };
                    for event in events {
                        if let Err(error) = runtime.events.publish(event) {
                            if runtime.handle_event_error(&generation, error).is_err() { return; }
                        }
                    }
                }
                if pending.len() > 4096 {
                    let dropped = pending.len() - 4096;
                    pending.drain(..dropped);
                    eprintln!("Codex dropped {dropped} unroutable events at the pending event bound; Server remains available");
                }
            }
        });
    }

    pub(super) fn handle_event_error(&self, generation: &str, error: ProtocolError) -> Result<(), ProtocolError> {
        if matches!(error.code.as_str(), "provider_frame_write_failed" | "provider_frame_flush_failed") {
            self.fail_server(generation, error.clone());
            Err(error)
        } else {
            eprintln!("Codex event delivery rejected: {}: {}", error.code, error.message);
            Ok(())
        }
    }
}
