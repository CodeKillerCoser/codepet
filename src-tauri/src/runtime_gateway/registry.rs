use super::generated::{ProtocolError, Provider, ProviderStatus};
use super::provider::ProviderAdapter;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: Arc<RwLock<BTreeMap<String, Arc<dyn ProviderAdapter>>>>,
}

impl ProviderRegistry {
    pub fn register(
        &self,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Result<Option<Arc<dyn ProviderAdapter>>, ProtocolError> {
        let provider_id = adapter.provider().id;
        if provider_id.trim().is_empty() {
            return Err(protocol_error(
                "invalid_provider",
                "provider id must not be empty",
                false,
                None,
            ));
        }
        let mut providers = self.providers.write().map_err(|_| registry_lock_error())?;
        Ok(providers.insert(provider_id, adapter))
    }

    pub fn remove(
        &self,
        provider_id: &str,
    ) -> Result<Option<Arc<dyn ProviderAdapter>>, ProtocolError> {
        let mut providers = self.providers.write().map_err(|_| registry_lock_error())?;
        Ok(providers.remove(provider_id))
    }

    pub fn list(&self) -> Result<Vec<Provider>, ProtocolError> {
        let providers = self.providers.read().map_err(|_| registry_lock_error())?;
        Ok(providers.values().map(|adapter| adapter.provider()).collect())
    }

    pub fn resolve(&self, provider_id: &str) -> Result<Arc<dyn ProviderAdapter>, ProtocolError> {
        let adapter = {
            let providers = self.providers.read().map_err(|_| registry_lock_error())?;
            providers.get(provider_id).cloned()
        }
        .ok_or_else(|| {
            protocol_error(
                "unknown_provider",
                &format!("provider is not registered: {provider_id}"),
                false,
                Some(HashMap::from([(
                    "providerId".to_string(),
                    serde_json::Value::String(provider_id.to_string()),
                )])),
            )
        })?;

        let provider = adapter.provider();
        if provider.status != ProviderStatus::Ready {
            let unavailable_reason = provider
                .extension
                .as_ref()
                .and_then(|extension| extension.data.get("unavailableReason"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let message = unavailable_reason
                .clone()
                .unwrap_or_else(|| format!("provider is not ready: {provider_id}"));
            let mut details = HashMap::from([
                (
                    "providerId".to_string(),
                    serde_json::Value::String(provider_id.to_string()),
                ),
                (
                    "status".to_string(),
                    serde_json::to_value(provider.status).unwrap_or(serde_json::Value::Null),
                ),
            ]);
            if let Some(reason) = unavailable_reason {
                details.insert(
                    "unavailableReason".to_string(),
                    serde_json::Value::String(reason),
                );
            }
            return Err(protocol_error(
                "provider_unavailable",
                &message,
                true,
                Some(details),
            ));
        }
        Ok(adapter)
    }

    pub fn ready_adapters(&self) -> Result<Vec<Arc<dyn ProviderAdapter>>, ProtocolError> {
        let providers = self.providers.read().map_err(|_| registry_lock_error())?;
        Ok(providers
            .values()
            .filter(|adapter| adapter.provider().status == ProviderStatus::Ready)
            .cloned()
            .collect())
    }
}

fn registry_lock_error() -> ProtocolError {
    protocol_error(
        "gateway_state_error",
        "provider registry lock is unavailable",
        true,
        None,
    )
}

fn protocol_error(
    code: &str,
    message: &str,
    retryable: bool,
    details: Option<HashMap<String, serde_json::Value>>,
) -> ProtocolError {
    ProtocolError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
        details: details.map(|details| details.into_iter().collect()),
    }
}
