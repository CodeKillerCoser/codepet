//! Host owns local subscribers. A Provider sees one Host subscription, without Gateway identities.
use super::*;
use codepet_provider_sdk::{EventSubscribeRequest, EventUnsubscribeRequest, ProviderNotificationEvent};

pub struct ProviderEventSubscription {
    pub receiver: mpsc::Receiver<ProviderNotificationEvent>,
    pub message: String,
}
pub(super) struct SubscriptionHub {
    generation: u64,
    wire_id: String,
    message: String,
    subscribers: BTreeMap<String, mpsc::Sender<ProviderNotificationEvent>>,
}
impl PluginManager {
    pub async fn subscribe_events(&self, plugin_id: &str, subscriber_id: &str) -> HostResult<ProviderEventSubscription> {
        let _operation = self.inner.subscription_operations.lock().await;
        let (process, generation) = {
            let plugins = self.inner.plugins.read().await;
            let entry = plugins.get(plugin_id).ok_or_else(|| HostError::new("unknown_plugin", plugin_id))?;
            if entry.state != PluginRuntimeState::Ready { return Err(HostError::new("provider_not_ready", plugin_id)); }
            (entry.process.clone().ok_or_else(|| HostError::new("provider_not_ready", plugin_id))?, entry.generation)
        };
        let (sender, receiver) = mpsc::channel(256);
        let wire_id = {
            let mut hubs = self.inner.subscriptions.lock().unwrap();
            if let Some(hub) = hubs.get_mut(plugin_id).filter(|hub| hub.generation == generation) {
                if hub.subscribers.contains_key(subscriber_id) { return Err(HostError::new("duplicate_subscriber", subscriber_id)); }
                hub.subscribers.insert(subscriber_id.into(), sender);
                return Ok(ProviderEventSubscription { receiver, message: hub.message.clone() });
            }
            let wire_id = format!("host-{}", uuid::Uuid::new_v4());
            hubs.insert(plugin_id.into(), SubscriptionHub { generation, wire_id: wire_id.clone(), message: String::new(),
                subscribers: BTreeMap::from([(subscriber_id.into(), sender)]) });
            wire_id
        };
        match process.client().event_subscribe(EventSubscribeRequest { subscription_id: wire_id.clone() }).await {
            Ok(response) if response.subscription_id == wire_id => {
                if let Some(hub) = self.inner.subscriptions.lock().unwrap().get_mut(plugin_id) { hub.message = response.message.clone(); }
                Ok(ProviderEventSubscription { receiver, message: response.message })
            }
            result => {
                self.inner.subscriptions.lock().unwrap().remove(plugin_id);
                Err(match result { Err(e) => e.into(), Ok(_) => HostError::new("invalid_subscription", "Provider returned a different subscription id") })
            }
        }
    }
    pub async fn unsubscribe_events(&self, plugin_id: &str, subscriber_id: &str) -> HostResult<()> {
        let _operation = self.inner.subscription_operations.lock().await;
        let wire_id = {
            let mut hubs = self.inner.subscriptions.lock().unwrap();
            let Some(hub) = hubs.get_mut(plugin_id) else { return Ok(()); };
            hub.subscribers.remove(subscriber_id);
            if !hub.subscribers.is_empty() { return Ok(()); }
            hubs.remove(plugin_id).unwrap().wire_id
        };
        let process = self.inner.plugins.read().await.get(plugin_id).and_then(|p| p.process.clone());
        if let Some(process) = process {
            process.client().event_unsubscribe(EventUnsubscribeRequest { subscription_id: wire_id }).await.map_err(HostError::from)?;
        }
        Ok(())
    }
    pub(super) async fn deliver_notification(&self, plugin_id: &str, event: ProviderNotificationEvent) -> HostResult<()> {
        let generation = self.inner.plugins.read().await.get(plugin_id).map(|p| p.generation);
        let mut hubs = self.inner.subscriptions.lock().unwrap();
        if let Some(hub) = hubs.get_mut(plugin_id) {
            if Some(hub.generation) != generation || hub.wire_id != event.subscription_id { return Ok(()); }
            // A slow subscriber is disconnected independently, exposing its gap without blocking others.
            hub.subscribers.retain(|_, sender| sender.try_send(event.clone()).is_ok());
        }
        Ok(())
    }
}
