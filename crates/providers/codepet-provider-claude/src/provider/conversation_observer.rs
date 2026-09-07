use super::*;

impl ClaudeInstanceRuntime {
    pub(super) fn start_atomic_poll(self: &Arc<Self>) {
        let generation = lock(&self.mutable).lifecycle_generation;
        let loader = Arc::downgrade(self);
        let publisher = loader.clone();
        let task = conversation_atoms::spawn_summary_poll(move |_previous| -> ProtocolFuture<'static, Vec<Conversation>> {
            let owner = loader.clone();
            Box::pin(async move {
                let runtime = owner.upgrade().ok_or_else(conversation_atoms::generation_changed)?;
                if lock(&runtime.mutable).lifecycle_generation != generation { return Err(conversation_atoms::generation_changed()); }
                let epoch = runtime.atoms.event_epoch();
                let rows = runtime.collect_atomic_summaries().await?;

                if lock(&runtime.mutable).lifecycle_generation != generation || runtime.atoms.event_epoch() != epoch { return Err(conversation_atoms::generation_changed()); }
                Ok(rows)
            })
        }, move |result| {
            let Some(runtime) = publisher.upgrade() else { return false; };
            let mut state = lock(&runtime.mutable);
            if state.lifecycle_generation != generation || !matches!(state.status, InstanceStatus::Starting | InstanceStatus::Ready) { return false; }
            let ready = result.is_ok() && state.status == InstanceStatus::Ready;
            if state.atomic_facts_ready != ready {
                state.atomic_facts_ready = ready;
                state.atomic_facts_epoch = state.atomic_facts_epoch.saturating_add(1);
                if runtime.events.publish(ProtocolEvent::EventInstanceStatusChanged { jsonrpc: "2.0".into(), params: InstanceStatusChangedEvent {
                    instance: runtime.snapshot_locked(&state), previous_status: Some(state.status),
                } }).is_err() { return false; }
            }
            match result {
                Ok(delta) if ready => {
                    if conversation_atoms::publish_summary_delta(&runtime.route, delta, runtime.events.as_ref()).is_err() { state.atomic_facts_ready = false; return false; }
                }
                Err(error) => eprintln!("Claude summary reconciliation unavailable: {}", error.message),
                _ => {}
            }
            true
        }, Duration::from_secs(5));
        let mut state = lock(&self.mutable);
        if let Some(previous) = state.atomic_task.replace(task) { previous.abort(); }
    }
}
