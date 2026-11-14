//! Runtime probes are independent per plugin; an in-flight refresh is not an empty inventory.
use super::*;

impl PluginManager {
    /// Start every selected probe before awaiting any of them. A failed/slow plugin
    /// cannot abort its peers. Dropping this batch waiter does not cancel the probes.
    pub async fn refresh_runtime_inventories(
        &self,
        plugin_ids: Vec<String>,
    ) -> Vec<(String, HostResult<RuntimeGetInstalledResponse>)> {
        let tasks: Vec<_> = plugin_ids.into_iter().map(|id| {
            let manager = self.clone();
            let plugin_id = id.clone();
            let task = tokio::spawn(async move {
                manager.runtime_get_installed(&plugin_id, RuntimeGetInstalledRequest { refresh: Some(true) }).await
            });
            (id, task)
        }).collect();
        let mut outcomes = Vec::with_capacity(tasks.len());
        for (id, task) in tasks {
            let result = task.await.unwrap_or_else(|error| Err(HostError::new(
                "runtime_scan_task_failed", format!("Runtime scan task failed for {id}: {error}"),
            )));
            outcomes.push((id, result));
        }
        outcomes
    }

    pub async fn runtime_get_installed(
        &self,
        plugin_id: &str,
        request: RuntimeGetInstalledRequest,
    ) -> HostResult<RuntimeGetInstalledResponse> {
        let process = self.process_for_plugin(plugin_id).await?;
        let (generation, revision) = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins.get_mut(plugin_id).ok_or_else(|| unknown_plugin(plugin_id))?;
            if !entry.process.as_ref().is_some_and(|current| Arc::ptr_eq(current, &process)) {
                return Err(stale_inventory_request());
            }
            if entry.reported.as_ref().and_then(|p| p.features.as_ref()).is_some_and(|features|
                !features.contains(&codepet_provider_sdk::ProviderFeature::RuntimeInventory)) {
                return Ok(RuntimeGetInstalledResponse { harness_list: vec![], selected: None,
                    selection_required: Some(false), scanning: None, scan_error: None });
            }
            if request.refresh != Some(true) {
                if let Some(inventory) = &entry.runtime_inventory { return Ok(inventory.clone()); }
            }
            entry.runtime_inventory_revision = entry.runtime_inventory_revision.wrapping_add(1);
            (entry.generation, entry.runtime_inventory_revision)
        };
        // Keep the last known inventory through timeout, cancellation and RPC errors.
        let result = process.client().runtime_get_installed(request).await.map_err(HostError::from)
            .and_then(|inventory| { validate_inventory(&inventory)?; Ok(inventory) });
        let (result, start) = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins.get_mut(plugin_id).ok_or_else(|| unknown_plugin(plugin_id))?;
            if entry.generation != generation || entry.state != PluginRuntimeState::Ready
                || !entry.process.as_ref().is_some_and(|current| Arc::ptr_eq(current, &process)) {
                return Err(stale_inventory_request());
            }
            // A newer notification or request owns the cache. In particular a slow
            // scanning=true reply cannot overwrite the completion notification.
            if entry.runtime_inventory_revision != revision {
                return entry.runtime_inventory.clone().map(Ok).unwrap_or(result);
            }
            match result {
                Ok(inventory) => {
                    let start = install_inventory(entry, inventory);
                    (Ok(entry.runtime_inventory.clone().expect("inventory installed")), start)
                }
                Err(error) => {
                    entry.runtime_inventory_diagnostic = Some(error.clone());
                    (Err(error), false)
                }
            }
        };
        self.inventory_updated(plugin_id, generation, start);
        result
    }

    pub(super) fn inventory_updated(&self, plugin_id: &str, generation: u64, start: bool) {
        self.inner.runtime_changes.send_modify(|version| *version = version.wrapping_add(1));
        if !start { return; }
        let manager = self.clone();
        let plugin_id = plugin_id.to_owned();
        tokio::spawn(async move {
            let Ok(operation) = manager.plugin_operation(&plugin_id) else { return; };
            let _operation = operation.lock().await;
            if manager.inner.shutting_down.load(Ordering::SeqCst)
                || !manager.inner.plugins.read().await.get(&plugin_id)
                    .is_some_and(|entry| entry.generation == generation && entry.state == PluginRuntimeState::Ready) {
                return;
            }
            let _ = manager.start_manifest_instances(&plugin_id).await;
        });
    }
}

fn stale_inventory_request() -> HostError {
    HostError::new("runtime_scan_stale", "Runtime scan belongs to an old Provider process").retryable(true)
}

pub(super) fn install_inventory(entry: &mut PluginEntry, mut inventory: RuntimeGetInstalledResponse) -> bool {
    let previous = entry.runtime_inventory.as_ref();
    // Scanning/failed discovery is not evidence that the installed runtime vanished.
    if inventory.selection_required != Some(false) && (inventory.scanning == Some(true)
        || (inventory.scan_error.is_some() && inventory.harness_list.is_empty() && inventory.selected.is_none())) {
        if let Some(previous) = previous {
            inventory.harness_list = previous.harness_list.clone();
            inventory.selected = previous.selected.clone();
        }
    }
    let start = inventory.scanning != Some(true) && inventory.selected.is_some()
        && previous.is_none_or(|old| old.scanning == Some(true) || old.selected.is_none());
    entry.runtime_inventory = Some(inventory);
    entry.runtime_inventory_diagnostic = None;
    entry.runtime_inventory_revision = entry.runtime_inventory_revision.wrapping_add(1);
    start
}

// An explicit self-managed response must not hide a scan failure or stale selection.
pub(super) fn validate_inventory(inventory: &RuntimeGetInstalledResponse) -> HostResult<()> {
    if inventory.selection_required == Some(false) && (!inventory.harness_list.is_empty()
        || inventory.selected.is_some() || inventory.scanning == Some(true) || inventory.scan_error.is_some()) {
        return Err(HostError::new("invalid_runtime_inventory", "Self-managed runtime inventory must be empty, completed, and error-free"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_self_management_is_distinct_from_a_missing_runtime() {
        let old: RuntimeGetInstalledResponse = serde_json::from_value(serde_json::json!({"harnessList":[],"selected":null})).unwrap();
        assert_eq!(old.selection_required, None);
        validate_inventory(&old).unwrap();
        let mut managed = old.clone(); managed.selection_required=Some(false);
        validate_inventory(&managed).unwrap();
        managed.scanning=Some(true); assert!(validate_inventory(&managed).is_err());
        managed.scanning=None; managed.scan_error=Some("failed".into()); assert!(validate_inventory(&managed).is_err());
    }
}
