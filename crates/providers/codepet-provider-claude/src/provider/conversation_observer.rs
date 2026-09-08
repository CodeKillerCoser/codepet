use super::*;
use codepet_provider_data::conversation_atoms::SummaryPublication::{Applied, Retry, Stop};

impl ClaudeInstanceRuntime {
    pub(super) fn start_atomic_poll(self: &Arc<Self>) {
        let generation = lock(&self.mutable).lifecycle_generation;
        let loader = Arc::downgrade(self);
        let publisher = loader.clone();
        let task = conversation_atoms::spawn_summary_poll(move |_previous| -> ProtocolFuture<'static, (Vec<Conversation>, u64)> {
            let owner = loader.clone();
            Box::pin(async move {
                let runtime = owner.upgrade().ok_or_else(conversation_atoms::generation_changed)?;
                if lock(&runtime.mutable).lifecycle_generation != generation { return Err(conversation_atoms::generation_changed()); }
                let epoch = runtime.atoms.event_epoch();
                let rows = runtime.collect_atomic_summaries().await?;

                if lock(&runtime.mutable).lifecycle_generation != generation || runtime.atoms.event_epoch() != epoch { return Err(conversation_atoms::generation_changed()); }
                Ok((rows, epoch))
            })
        }, move |result| {
            let Some(runtime) = publisher.upgrade() else { return Stop; };
            let mut state = lock(&runtime.mutable);
            if state.lifecycle_generation != generation || !matches!(state.status, InstanceStatus::Starting | InstanceStatus::Ready) { return Stop; }
            // Failed revocations are retried before another scan can advertise readiness.
            if state.atomic_readiness_pending {
                if runtime.events.publish(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent { instance: runtime.snapshot_locked(&state), previous_status: Some(state.status) } }).is_err() { return Retry; }
                state.atomic_readiness_pending = false;
            }
            match result {
                Ok(delta) => {
                    let old_ready = state.atomic_facts_ready;
                    let old_epoch = state.atomic_facts_epoch;
                    if state.status != InstanceStatus::Ready { return Retry; }
                    let expected = delta.event_epoch;
                    let mut events = conversation_atoms::summary_delta_events(&runtime.route, delta);
                    if !old_ready {
                        state.atomic_facts_ready = true;
                        state.atomic_facts_epoch = old_epoch.saturating_add(1);
                        events.push(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent { instance: runtime.snapshot_locked(&state), previous_status: Some(state.status) } });
                    }
                    match runtime.atoms.commit_events(expected, events) {
                        Ok(true) => Applied,
                        Ok(false) => { state.atomic_facts_ready = old_ready; state.atomic_facts_epoch = old_epoch; Retry },
                        Err(_) => {
                            state.atomic_facts_ready = false;
                            state.atomic_facts_epoch = state.atomic_facts_epoch.saturating_add(1);
                            state.atomic_readiness_pending = true;
                            if runtime.events.publish(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent { instance: runtime.snapshot_locked(&state), previous_status: Some(state.status) } }).is_ok() { state.atomic_readiness_pending = false; }
                            Retry
                        },
                    }
                }
                Err(error) if error.code == "conversation_snapshot_changed" => Retry,
                Err(error) => {
                    eprintln!("Summary reconciliation unavailable: {}", error.message);
                    if state.atomic_facts_ready {
                        state.atomic_facts_ready = false;
                        state.atomic_facts_epoch = state.atomic_facts_epoch.saturating_add(1);
                        state.atomic_readiness_pending = true;
                        if runtime.events.publish(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent { instance: runtime.snapshot_locked(&state), previous_status: Some(state.status) } }).is_ok() { state.atomic_readiness_pending = false; }
                    }
                    Retry
                }
            }
        }, Duration::from_secs(5));
        let mut state = lock(&self.mutable);
        if let Some(previous) = state.atomic_task.replace(task) { previous.abort(); }
    }
}
