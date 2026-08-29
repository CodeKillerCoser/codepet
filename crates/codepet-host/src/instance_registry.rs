use crate::catalog::{PluginCatalog, PluginInstanceConfig};
use crate::persistence::{persistence_io, write_json_atomically};
use crate::{HostError, HostResult};
use codepet_provider_sdk::{
    DeviceId, JsonObject, ProviderInstanceId, ProviderInstanceRoute, ProviderPluginId,
    RoutedResourceId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

const INSTANCE_REGISTRY_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ProviderInstanceRecord {
    pub instance_id: ProviderInstanceId,
    pub plugin_id: ProviderPluginId,
    pub instance_kind: String,
    pub device_id: DeviceId,
    pub display_name: String,
    pub settings: JsonObject,
    pub enabled: bool,
}

impl ProviderInstanceRecord {
    pub fn route(&self) -> ProviderInstanceRoute {
        ProviderInstanceRoute {
            device_id: self.device_id.clone(),
            provider_instance_id: self.instance_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct InstanceRegistryDocument {
    version: u32,
    device_id: DeviceId,
    instances: Vec<ProviderInstanceRecord>,
}

#[derive(Clone, Debug, Default)]
struct InstanceRegistryState {
    instances: BTreeMap<ProviderInstanceId, ProviderInstanceRecord>,
}

#[derive(Clone, Debug)]
pub struct ProviderInstanceRegistry {
    device_id: DeviceId,
    path: Option<PathBuf>,
    state: Arc<RwLock<InstanceRegistryState>>,
}

impl ProviderInstanceRegistry {
    pub fn open(path: impl Into<PathBuf>, device_id: DeviceId) -> HostResult<Self> {
        validate_device_id(&device_id)?;
        let path = path.into();
        let state = if path.exists() {
            load_registry(&path, &device_id)?
        } else {
            let state = InstanceRegistryState::default();
            persist_registry(&path, &device_id, &state)?;
            state
        };
        Ok(Self {
            device_id,
            path: Some(path),
            state: Arc::new(RwLock::new(state)),
        })
    }

    pub fn in_memory(device_id: DeviceId) -> HostResult<Self> {
        validate_device_id(&device_id)?;
        Ok(Self {
            device_id,
            path: None,
            state: Arc::new(RwLock::new(InstanceRegistryState::default())),
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn synchronize_catalog(&self, catalog: &PluginCatalog) -> HostResult<Vec<ProviderInstanceRecord>> {
        let mut synchronized = self.mutate(|next| {
            let mut synchronized = Vec::new();
            for descriptor in catalog.descriptors() {
                for config in &descriptor.instances {
                    let record = synchronize_instance(
                        next,
                        &self.device_id,
                        &descriptor.plugin_id,
                        config,
                    )?;
                    synchronized.push(record);
                }
            }
            Ok(synchronized)
        })?;
        synchronized.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
        Ok(synchronized)
    }

    pub fn create(
        &self,
        plugin_id: ProviderPluginId,
        instance_kind: String,
        display_name: String,
        settings: JsonObject,
        requested_instance_id: Option<ProviderInstanceId>,
        enabled: bool,
    ) -> HostResult<ProviderInstanceRecord> {
        let config = PluginInstanceConfig {
            instance_id: requested_instance_id,
            instance_kind,
            display_name,
            settings,
            enabled,
        };
        self.mutate(|next| synchronize_instance(next, &self.device_id, &plugin_id, &config))
    }

    pub fn remove(&self, instance_id: &str) -> HostResult<Option<ProviderInstanceRecord>> {
        self.mutate(|next| Ok(next.instances.remove(instance_id)))
    }

    pub fn list(&self) -> HostResult<Vec<ProviderInstanceRecord>> {
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        Ok(state.instances.values().cloned().collect())
    }

    pub fn list_for_plugin(&self, plugin_id: &str) -> HostResult<Vec<ProviderInstanceRecord>> {
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        Ok(state
            .instances
            .values()
            .filter(|record| record.plugin_id == plugin_id)
            .cloned()
            .collect())
    }

    pub fn resolve_route(
        &self,
        route: &ProviderInstanceRoute,
        expected_plugin_id: Option<&str>,
    ) -> HostResult<ProviderInstanceRecord> {
        if route.device_id != self.device_id {
            return Err(route_error(
                "wrong_device_route",
                "Provider route targets a different device",
                route,
            ));
        }
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        let record = state
            .instances
            .get(&route.provider_instance_id)
            .cloned()
            .ok_or_else(|| {
                route_error(
                    "unknown_provider_instance",
                    "Provider instance is not registered",
                    route,
                )
            })?;
        if record.device_id != route.device_id {
            return Err(route_error(
                "provider_instance_device_mismatch",
                "Provider instance is registered to a different device",
                route,
            ));
        }
        if let Some(expected_plugin_id) = expected_plugin_id {
            if record.plugin_id != expected_plugin_id {
                return Err(route_error(
                    "provider_instance_plugin_mismatch",
                    "Provider instance belongs to a different plugin",
                    route,
                )
                .with_detail("expectedPluginId", expected_plugin_id.to_string())
                .with_detail("actualPluginId", record.plugin_id));
            }
        }
        Ok(record)
    }

    pub fn resolve_resource(
        &self,
        resource: &RoutedResourceId,
        expected_plugin_id: Option<&str>,
    ) -> HostResult<ProviderInstanceRecord> {
        self.resolve_route(
            &ProviderInstanceRoute {
                device_id: resource.device_id.clone(),
                provider_instance_id: resource.provider_instance_id.clone(),
            },
            expected_plugin_id,
        )
    }

    fn mutate<T>(
        &self,
        mutation: impl FnOnce(&mut InstanceRegistryState) -> HostResult<T>,
    ) -> HostResult<T> {
        let mut state = self.state.write().map_err(|_| registry_lock_error())?;
        let mut next = state.clone();
        let output = mutation(&mut next)?;
        if let Some(path) = self.path.as_ref() {
            persist_registry(path, &self.device_id, &next)?;
        }
        *state = next;
        Ok(output)
    }
}

fn load_registry(path: &Path, device_id: &str) -> HostResult<InstanceRegistryState> {
    let payload = fs::read(path)
        .map_err(|error| persistence_io("read Provider instance registry", path, error))?;
    let document: InstanceRegistryDocument = serde_json::from_slice(&payload).map_err(|error| {
        HostError::new(
            "invalid_provider_instance_registry",
            format!("decode Provider instance registry {}: {error}", path.display()),
        )
        .with_detail("path", path.display().to_string())
    })?;
    if document.version != INSTANCE_REGISTRY_VERSION {
        return Err(HostError::new(
            "unsupported_provider_instance_registry_version",
            format!(
                "unsupported Provider instance registry version: {}",
                document.version
            ),
        ));
    }
    if document.device_id != device_id {
        return Err(HostError::new(
            "provider_instance_registry_device_mismatch",
            "Provider instance registry belongs to a different device",
        )
        .with_detail("expectedDeviceId", device_id.to_string())
        .with_detail("actualDeviceId", document.device_id));
    }
    let mut instances = BTreeMap::new();
    for record in document.instances {
        validate_record(&record)?;
        if record.device_id != device_id {
            return Err(HostError::new(
                "provider_instance_registry_device_mismatch",
                "Provider instance registry belongs to a different device",
            )
            .with_detail("expectedDeviceId", device_id.to_string())
            .with_detail("actualDeviceId", record.device_id));
        }
        if instances.insert(record.instance_id.clone(), record).is_some() {
            return Err(HostError::new(
                "duplicate_provider_instance",
                "Provider instance registry contains duplicate ids",
            ));
        }
    }
    Ok(InstanceRegistryState { instances })
}

fn persist_registry(
    path: &Path,
    device_id: &str,
    state: &InstanceRegistryState,
) -> HostResult<()> {
    let document = InstanceRegistryDocument {
        version: INSTANCE_REGISTRY_VERSION,
        device_id: device_id.to_string(),
        instances: state.instances.values().cloned().collect(),
    };
    write_json_atomically(path, &document)
}

fn synchronize_instance(
    state: &mut InstanceRegistryState,
    device_id: &str,
    plugin_id: &str,
    config: &PluginInstanceConfig,
) -> HostResult<ProviderInstanceRecord> {
    if plugin_id.trim().is_empty()
        || config.instance_kind.trim().is_empty()
        || config.display_name.trim().is_empty()
    {
        return Err(HostError::new(
            "invalid_provider_instance",
            "Provider instance plugin, kind, and display name must not be empty",
        ));
    }
    let existing_match = state
        .instances
        .values()
        .find(|record| {
            record.plugin_id == plugin_id
                && record.instance_kind == config.instance_kind
                && record.display_name == config.display_name
        })
        .map(|record| record.instance_id.clone());
    let instance_id = config
        .instance_id
        .clone()
        .or(existing_match)
        .unwrap_or_else(|| format!("instance-{}", Uuid::new_v4()));
    if instance_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_provider_instance",
            "Provider instance id must not be empty",
        ));
    }
    if let Some(existing) = state.instances.get(&instance_id) {
        if existing.plugin_id != plugin_id || existing.device_id != device_id {
            return Err(HostError::new(
                "provider_instance_identity_conflict",
                format!("Provider instance id is already registered: {instance_id}"),
            )
            .with_detail("instanceId", instance_id));
        }
    }
    let record = ProviderInstanceRecord {
        instance_id: instance_id.clone(),
        plugin_id: plugin_id.to_string(),
        instance_kind: config.instance_kind.clone(),
        device_id: device_id.to_string(),
        display_name: config.display_name.clone(),
        settings: config.settings.clone(),
        enabled: config.enabled,
    };
    validate_record(&record)?;
    state.instances.insert(instance_id, record.clone());
    Ok(record)
}

fn validate_record(record: &ProviderInstanceRecord) -> HostResult<()> {
    if record.instance_id.trim().is_empty()
        || record.plugin_id.trim().is_empty()
        || record.instance_kind.trim().is_empty()
        || record.device_id.trim().is_empty()
        || record.display_name.trim().is_empty()
    {
        return Err(HostError::new(
            "invalid_provider_instance",
            "persisted Provider instance contains an empty identity field",
        ));
    }
    Ok(())
}

fn validate_device_id(device_id: &str) -> HostResult<()> {
    if device_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_device_identity",
            "Provider instance registry device id must not be empty",
        ));
    }
    Ok(())
}

fn registry_lock_error() -> HostError {
    HostError::new(
        "provider_instance_registry_unavailable",
        "Provider instance registry lock is unavailable",
    )
    .retryable(true)
}

fn route_error(code: &str, message: &str, route: &ProviderInstanceRoute) -> HostError {
    HostError::new(code, message)
        .with_detail("deviceId", route.device_id.clone())
        .with_detail("providerInstanceId", route.provider_instance_id.clone())
}
