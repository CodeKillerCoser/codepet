use crate::providers::catalog::{PluginCatalog, PluginDescriptor, PluginInstanceConfig};
use rusqlite::{params, OptionalExtension};
use crate::{HostError, HostResult};
use codepet_provider_sdk::{
    DeviceId, JsonObject, ProviderInstanceId, ProviderInstanceRoute, ProviderPluginId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
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
            provider_plugin_id: self.plugin_id.clone(),
            provider_instance_id: self.instance_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct InstanceIdentity {
    instance_id: ProviderInstanceId,
    plugin_id: ProviderPluginId,
    instance_kind: String,
    display_name: String,
}

#[derive(Clone, Debug, Default)]
struct InstanceRegistryState {
    identities: BTreeMap<ProviderInstanceId, InstanceIdentity>,
    records: BTreeMap<ProviderInstanceId, ProviderInstanceRecord>,
}

#[derive(Clone, Debug)]
pub struct ProviderInstanceRegistry {
    device_id: DeviceId,
    path: PathBuf,
    state: Arc<RwLock<InstanceRegistryState>>,
}

impl ProviderInstanceRegistry {
    pub fn open(path: impl Into<PathBuf>, device_id: DeviceId) -> HostResult<Self> {
        validate_device_id(&device_id)?;
        let path = path.into();
        let identities = load_registry(&path, &device_id)?;
        Ok(Self {
            device_id,
            path,
            state: Arc::new(RwLock::new(InstanceRegistryState {
                identities,
                records: BTreeMap::new(),
            })),
        })
    }

    pub(crate) fn database_path(&self) -> &Path { &self.path }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn synchronize_catalog(
        &self,
        catalog: &PluginCatalog,
    ) -> HostResult<Vec<ProviderInstanceRecord>> {
        self.synchronize(catalog, false)
    }

    pub(crate) fn register_plugin(&self, descriptor: PluginDescriptor) -> HostResult<Vec<ProviderInstanceRecord>> {
        self.synchronize(&PluginCatalog::from_entries([descriptor])?, true)
    }

    fn synchronize(&self, catalog: &PluginCatalog, append: bool) -> HostResult<Vec<ProviderInstanceRecord>> {
        let mut state = self.state.write().map_err(|_| registry_lock_error())?;
        if append && catalog.descriptors().any(|descriptor| state.records.values().any(|record| record.plugin_id == descriptor.plugin_id)) {
            return Err(HostError::new("duplicate_plugin_id", "Provider instance records already exist"));
        }
        let previous = load_registry(&self.path,&self.device_id)?;
        let mut identities = if append { state.identities.clone() } else { BTreeMap::new() };
        let mut records = if append { state.records.clone() } else { BTreeMap::new() };
        let mut configured_keys = BTreeSet::new();

        for descriptor in catalog.descriptors() {
            for config in &descriptor.instances {
                validate_config(&descriptor.plugin_id, config)?;
                let key = (
                    descriptor.plugin_id.clone(),
                    config.instance_kind.clone(),
                    config.display_name.clone(),
                );
                if !configured_keys.insert(key.clone()) {
                    return Err(HostError::new(
                        "duplicate_provider_instance_config",
                        "Provider manifest contains duplicate instance identity fields",
                    )
                    .with_detail("pluginId", descriptor.plugin_id.clone())
                    .with_detail("instanceKind", config.instance_kind.clone())
                    .with_detail("displayName", config.display_name.clone()));
                }

                let previous_id = previous.values().find_map(|identity| {
                    (identity.plugin_id == key.0
                        && identity.instance_kind == key.1
                        && identity.display_name == key.2)
                        .then(|| identity.instance_id.clone())
                });
                let instance_id = config
                    .instance_id
                    .clone()
                    .or(previous_id)
                    .unwrap_or_else(|| format!("instance-{}", Uuid::new_v4()));
                if instance_id.trim().is_empty() {
                    return Err(HostError::new(
                        "invalid_provider_instance",
                        "Provider instance id must not be empty",
                    ));
                }
                if let Some(existing) = previous.get(&instance_id) {
                    if existing.plugin_id != descriptor.plugin_id
                        || existing.instance_kind != config.instance_kind
                        || existing.display_name != config.display_name
                    {
                        return Err(HostError::new(
                            "provider_instance_identity_conflict",
                            format!("Provider instance id is already mapped: {instance_id}"),
                        )
                        .with_detail("instanceId", instance_id));
                    }
                }
                let identity = InstanceIdentity {
                    instance_id: instance_id.clone(),
                    plugin_id: descriptor.plugin_id.clone(),
                    instance_kind: config.instance_kind.clone(),
                    display_name: config.display_name.clone(),
                };
                if identities
                    .insert(instance_id.clone(), identity)
                    .is_some()
                {
                    return Err(HostError::new(
                        "duplicate_provider_instance",
                        format!("Provider manifest reuses instance id: {instance_id}"),
                    ));
                }
                records.insert(
                    instance_id.clone(),
                    ProviderInstanceRecord {
                        instance_id,
                        plugin_id: descriptor.plugin_id.clone(),
                        instance_kind: config.instance_kind.clone(),
                        device_id: self.device_id.clone(),
                        display_name: config.display_name.clone(),
                        settings: config.settings.clone(),
                        enabled: config.enabled,
                    },
                );
            }
        }

        persist_registry(&self.path, catalog, &identities)?;
        state.identities = identities;
        state.records = records;
        Ok(state.records.values().cloned().collect())
    }

    pub fn list(&self) -> HostResult<Vec<ProviderInstanceRecord>> {
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        Ok(state.records.values().cloned().collect())
    }

    pub(crate) fn replace_setting_for_kind(
        &self,
        plugin_id: &str,
        instance_kind: &str,
        key: &str,
        value: Option<Value>,
    ) -> HostResult<Vec<ProviderInstanceRecord>> {
        if plugin_id.trim().is_empty()
            || instance_kind.trim().is_empty()
            || key.trim().is_empty()
        {
            return Err(HostError::new(
                "invalid_provider_instance_setting",
                "Provider plugin id, instance kind, and setting key must not be empty",
            ));
        }
        let mut state = self.state.write().map_err(|_| registry_lock_error())?;
        let mut updated = Vec::new();
        for record in state.records.values_mut() {
            if record.plugin_id == plugin_id && record.instance_kind == instance_kind {
                match value.as_ref() {
                    Some(value) => {
                        record.settings.insert(key.to_string(), value.clone());
                    }
                    None => {
                        record.settings.remove(key);
                    }
                }
                updated.push(record.clone());
            }
        }
        Ok(updated)
    }

    pub(crate) fn list_for_plugin(&self, plugin_id: &str) -> HostResult<Vec<ProviderInstanceRecord>> {
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        Ok(state
            .records
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
        validate_route(route)?;
        if route.device_id != self.device_id {
            return Err(route_error(
                "wrong_device_route",
                "Provider route targets a different device",
                route,
            ));
        }
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        let record = state
            .records
            .get(&route.provider_instance_id)
            .cloned()
            .ok_or_else(|| {
                route_error(
                    "unknown_provider_instance",
                    "Provider instance is not configured by a manifest",
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
        if record.plugin_id != route.provider_plugin_id {
            return Err(route_error(
                "provider_instance_plugin_mismatch",
                "Provider instance belongs to a different plugin",
                route,
            )
            .with_detail("expectedPluginId", route.provider_plugin_id.clone())
            .with_detail("actualPluginId", record.plugin_id));
        }
        if let Some(expected_plugin_id) = expected_plugin_id {
            if expected_plugin_id.trim().is_empty() {
                return Err(HostError::new(
                    "invalid_provider_plugin_route",
                    "expected Provider plugin id must not be empty",
                ));
            }
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

    pub(crate) fn resolve_provider_id(
        &self,
        provider_id: &str,
        expected_plugin_id: Option<&str>,
    ) -> HostResult<ProviderInstanceRecord> {
        if provider_id.trim().is_empty() {
            return Err(HostError::new(
                "invalid_provider_resource",
                "Provider resource providerId must not be empty",
            ));
        }
        let state = self.state.read().map_err(|_| registry_lock_error())?;
        let record = state.records.get(provider_id).cloned().ok_or_else(|| {
            HostError::new(
                "unknown_provider_instance",
                "Provider resource targets an unknown Provider instance",
            )
            .with_detail("providerId", provider_id.to_string())
        })?;
        if let Some(expected_plugin_id) = expected_plugin_id {
            if record.plugin_id != expected_plugin_id {
                return Err(HostError::new(
                    "provider_instance_plugin_mismatch",
                    "Provider resource belongs to a different plugin",
                )
                .with_detail("expectedPluginId", expected_plugin_id.to_string())
                .with_detail("actualPluginId", record.plugin_id));
            }
        }
        Ok(record)
    }
}

fn load_registry(path: &Path, device_id: &str) -> HostResult<BTreeMap<ProviderInstanceId, InstanceIdentity>> {
    let mut db = crate::storage::open(path)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(crate::storage::error)?;
    let owner:Option<String> = tx.query_row("SELECT device_id FROM registry_identity WHERE id=1",[],|r|r.get(0)).optional().map_err(crate::storage::error)?;
    if owner.as_deref().is_some_and(|id|id!=device_id) {
        return Err(HostError::new("provider_instance_registry_device_mismatch", "Provider registry belongs to a different device"));
    }
    tx.execute("INSERT OR IGNORE INTO registry_identity VALUES(1,?1)",[device_id]).map_err(crate::storage::error)?;
    let mut identities = BTreeMap::new();
    {
        let mut statement = tx.prepare("SELECT id,provider_id,instance_kind,display_name FROM provider_instances").map_err(crate::storage::error)?;
        let rows = statement.query_map([], |r| Ok(InstanceIdentity { instance_id:r.get(0)?,plugin_id:r.get(1)?,instance_kind:r.get(2)?,display_name:r.get(3)? })).map_err(crate::storage::error)?;
        for identity in rows {
            let identity = identity.map_err(crate::storage::error)?;
            validate_identity(&identity)?;
            identities.insert(identity.instance_id.clone(),identity);
        }
    }
    tx.commit().map_err(crate::storage::error)?;
    Ok(identities)
}
fn persist_registry(path: &Path, catalog: &PluginCatalog, identities: &BTreeMap<ProviderInstanceId, InstanceIdentity>) -> HostResult<()> {
    let mut db = crate::storage::open(path)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(crate::storage::error)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    for descriptor in catalog.descriptors() {
        tx.execute("INSERT INTO providers(id,display_name,created_at,updated_at) VALUES(?1,?2,?3,?3) ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,updated_at=excluded.updated_at WHERE display_name!=excluded.display_name",
            params![descriptor.plugin_id,descriptor.display_name,now]).map_err(crate::storage::error)?;
    }
    for identity in identities.values() {
        let existing:Option<(String,String,String)>=tx.query_row("SELECT provider_id,instance_kind,display_name FROM provider_instances WHERE id=?1",[&identity.instance_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(crate::storage::error)?;
        if existing.is_some_and(|e|e!=(identity.plugin_id.clone(),identity.instance_kind.clone(),identity.display_name.clone())) {
            return Err(HostError::new("provider_instance_identity_conflict","Provider instance ID already belongs to another identity"));
        }
        tx.execute("INSERT OR IGNORE INTO providers(id,display_name,created_at,updated_at) VALUES(?1,?1,?2,?2)",params![identity.plugin_id,now]).map_err(crate::storage::error)?;
        tx.execute("INSERT INTO provider_instances VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO NOTHING",params![identity.instance_id,identity.plugin_id,identity.instance_kind,identity.display_name]).map_err(crate::storage::error)?;
    }
    // An absent/disabled plugin must not delete conversations or its stable identity.
    tx.commit().map_err(crate::storage::error)?;
    Ok(())
}

fn validate_config(plugin_id: &str, config: &PluginInstanceConfig) -> HostResult<()> {
    if plugin_id.trim().is_empty()
        || config.instance_kind.trim().is_empty()
        || config.display_name.trim().is_empty()
    {
        return Err(HostError::new(
            "invalid_provider_instance",
            "Provider instance plugin, kind, and display name must not be empty",
        ));
    }
    Ok(())
}

fn validate_identity(identity: &InstanceIdentity) -> HostResult<()> {
    if identity.instance_id.trim().is_empty()
        || identity.plugin_id.trim().is_empty()
        || identity.instance_kind.trim().is_empty()
        || identity.display_name.trim().is_empty()
    {
        return Err(HostError::new(
            "invalid_provider_instance",
            "persisted Provider instance mapping contains an empty identity field",
        ));
    }
    Ok(())
}

fn validate_route(route: &ProviderInstanceRoute) -> HostResult<()> {
    if route.device_id.trim().is_empty()
        || route.provider_plugin_id.trim().is_empty()
        || route.provider_instance_id.trim().is_empty()
    {
        return Err(route_error(
            "invalid_provider_route",
            "Provider route deviceId, providerPluginId, and providerInstanceId must not be empty",
            route,
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
        .with_detail("providerPluginId", route.provider_plugin_id.clone())
        .with_detail("providerInstanceId", route.provider_instance_id.clone())
}
