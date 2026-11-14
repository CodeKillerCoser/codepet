//! A versioned projection of the registry, never a second registration authority.
//! No Provider RPC, filesystem operation or business payload work runs under this gate.
use super::*;

pub(super) struct Inventory {
    epoch: String,
    revision: u64,
    providers: BTreeMap<String, gateway::ProviderSummary>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            providers: BTreeMap::new(),
        }
    }
}

impl Inventory {
    fn reconcile(
        &mut self,
        providers: &[GatewayProviderRuntime],
        events: &GatewayEventBus,
    ) -> Result<(), gateway::ProtocolError> {
        let current: BTreeMap<_, _> = providers
            .iter()
            .map(|p| (p.summary.id.clone(), p.summary.clone()))
            .collect();
        for id in self
            .providers
            .keys()
            .filter(|id| !current.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>()
        {
            let revision = self.next_revision()?;
            events.publish(gateway::ProtocolEvent::ProviderRemoved {
                jsonrpc: "2.0".into(),
                params: gateway::ProtocolEventParams {
                    event_cursor: event_cursor(0),
                    payload: gateway::ProviderRemovedEvent {
                        provider_id: id.clone(),
                        inventory_epoch: self.epoch.clone(),
                        inventory_revision: revision,
                    },
                },
            })?;
            self.providers.remove(&id);
        }
        for (id, provider) in current {
            if self.providers.get(&id) == Some(&provider) {
                continue;
            }
            let revision = self.next_revision()?;
            events.publish(gateway::ProtocolEvent::ProviderChanged {
                jsonrpc: "2.0".into(),
                params: gateway::ProtocolEventParams {
                    event_cursor: event_cursor(0),
                    payload: gateway::ProviderChangedEvent {
                        provider: provider.clone(),
                        inventory_epoch: Some(self.epoch.clone()),
                        inventory_revision: Some(revision),
                    },
                },
            })?;
            self.providers.insert(id, provider);
        }
        Ok(())
    }
    fn next_revision(&mut self) -> Result<u64, gateway::ProtocolError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(gateway_state_error)?;
        Ok(self.revision)
    }
}

impl ProviderGatewayService {
    pub(super) async fn provider_inventory_snapshot(
        &self,
        device_id: Option<&str>,
    ) -> Result<(String, u64, Vec<GatewayProviderRuntime>), gateway::ProtocolError> {
        // Serialize only snapshot + revision commits. Never wait for another Provider's RPC.
        let mut inventory = self.provider_inventory.lock().await;
        let providers = self.collect_gateway_providers(device_id).await?;
        inventory.reconcile(&providers, &self.events)?;
        Ok((inventory.epoch.clone(), inventory.revision, providers))
    }

    pub(super) fn start_provider_inventory(self: &Arc<Self>) {
        let mut status = self.manager.subscribe_status_changes();
        let mut runtime = self.manager.subscribe_runtime_changes();
        let service = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(current) = service.upgrade() else {
                    return;
                };
                if let Err(error) = current.provider_inventory_snapshot(None).await {
                    eprintln!("Provider inventory update failed: {error:?}");
                }
                drop(current);
                tokio::select! {
                    changed = status.changed() => if changed.is_err() { return; },
                    changed = runtime.changed() => if changed.is_err() { return; },
                }
            }
        });
    }
}
