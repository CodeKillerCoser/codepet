//! Plugin-level Provider Registry. The manager's single active catalog is the
//! source for desktop views, Gateway lists and heartbeat snapshots. This module
//! owns package installation, enumeration and complete live uninstallation;
//! instance_registry persists stable identities/history, not a second live list.
use super::*;
use crate::providers::catalog::PLUGIN_MANIFEST_FILE_NAME;
use std::path::{Path, PathBuf};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportResult {
    pub provider_id: String,
    pub display_name: String,
    pub loaded: bool,
    pub diagnostic: Option<String>,
}

struct PreparedImport {
    staging: tempfile::TempDir,
    destination: PathBuf,
    descriptor: PluginDescriptor,
}

fn import_error(error: impl std::fmt::Display) -> HostError {
    HostError::new("provider_import_failed", error.to_string())
}

// A manifest selects a portable package, not just one executable: retain sibling assets/args.
fn copy_package(source: &Path, destination: &Path) -> HostResult<()> {
    std::fs::create_dir_all(destination).map_err(import_error)?;
    for entry in std::fs::read_dir(source).map_err(import_error)? {
        let entry = entry.map_err(import_error)?;
        let kind = entry.file_type().map_err(import_error)?;
        let target = destination.join(entry.file_name());
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            return Err(import_error(format!("Provider package must contain regular files/directories: {}", entry.path().display())));
        }
        if kind.is_dir() {
            copy_package(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target).map_err(import_error)?;
        }
    }
    Ok(())
}

fn prepare_import(manifest: &Path, root: &Path) -> HostResult<PreparedImport> {
    let manifest = manifest.canonicalize().map_err(import_error)?;
    let source = manifest
        .parent()
        .ok_or_else(|| import_error("Missing package directory"))?;
    let mut descriptor = PluginCatalog::read_one(&manifest)?;
    let id = &descriptor.plugin_id;
    if id == "."
        || id == ".."
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err(import_error(
            "Provider ID must use letters, digits, '.', '_' or '-'",
        ));
    }
    let external = descriptor
        .env
        .get("CODEPET_PROVIDER_EXTERNAL_COMMAND")
        .is_some_and(|v| v == "1");
    let relative = if external {
        None
    } else {
        let executable = descriptor.executable.canonicalize().map_err(import_error)?;
        if !executable.is_file() {
            return Err(import_error("Provider executable is not a file"));
        }
        Some(
            executable
                .strip_prefix(source)
                .map_err(|_| {
                    import_error("Provider executable must be inside its package directory")
                })?
                .to_owned(),
        )
    };
    std::fs::create_dir_all(root).map_err(import_error)?;
    let root = root.canonicalize().map_err(import_error)?;
    if root.starts_with(source) {
        return Err(import_error(
            "Provider package cannot contain the installation directory",
        ));
    }
    let destination = root.join(id);
    if destination.exists() {
        return Err(HostError::new(
            "duplicate_plugin_id",
            format!("Provider {id} is already installed"),
        ));
    }
    // Hidden staging manifests are never discoverable as a partially copied plugin.
    let staging = tempfile::Builder::new()
        .prefix(".provider-import-")
        .tempdir_in(&root)
        .map_err(import_error)?;
    let package = staging.path().join("package");
    copy_package(source, &package)?;
    let mut payload: Value =
        serde_json::from_slice(&std::fs::read(&manifest).map_err(import_error)?)
            .map_err(import_error)?;
    if let Some(relative) = &relative {
        let field = if payload.get("command").is_some() {
            "command"
        } else {
            "executable"
        };
        payload[field] = Value::String(relative.to_string_lossy().into_owned());
    }
    std::fs::write(
        package.join(PLUGIN_MANIFEST_FILE_NAME),
        serde_json::to_vec_pretty(&payload).map_err(import_error)?,
    )
    .map_err(import_error)?;
    if let Some(relative) = relative {
        descriptor.executable = destination.join(relative);
    }
    descriptor.env.insert(
        "CODEPET_PROVIDER_PACKAGE_DIRECTORY".into(),
        destination.to_string_lossy().into_owned(),
    );
    Ok(PreparedImport {
        staging,
        destination,
        descriptor,
    })
}

impl PluginManager {
    pub async fn snapshots(&self) -> Vec<PluginRuntimeSnapshot> {
        self.inner
            .plugins
            .read()
            .await
            .values()
            .map(PluginEntry::snapshot)
            .collect()
    }

    pub async fn snapshot(&self, plugin_id: &str) -> HostResult<PluginRuntimeSnapshot> {
        self.inner
            .plugins
            .read()
            .await
            .get(plugin_id)
            .map(PluginEntry::snapshot)
            .ok_or_else(|| unknown_plugin(plugin_id))
    }


    /// Uninstall a managed package, closing its process/observers before unregistering.
    pub async fn remove_plugin(&self, plugin_id: &str, root: PathBuf) -> HostResult<()> {
        let _registry = self.inner.registry_operation.lock().await;
        let operation = self.plugin_operation(plugin_id)?;
        let _operation = operation.lock().await;
        let snapshot = self.snapshot(plugin_id).await?;
        let root = root.canonicalize().map_err(import_error)?;
        let installed = root.join(plugin_id);
        let package = snapshot.catalog.env.get("CODEPET_PROVIDER_PACKAGE_DIRECTORY")
            .map(PathBuf::from).ok_or_else(|| import_error("Only imported Provider packages can be removed"))?;
        // The Host-recorded package location must belong to managed imports even
        // when files were removed externally. Never infer ownership from absence.
        if installed.parent() != Some(root.as_path()) || package != installed {
            return Err(import_error("Only imported Provider packages can be removed"));
        }
        let package_exists = match std::fs::symlink_metadata(&installed) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                if installed.canonicalize().map_err(import_error)? != installed {
                    return Err(import_error("Provider package escapes the installation directory"));
                }
                true
            }
            Ok(_) => return Err(import_error("Provider package must be a regular directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(import_error(error)),
        };
        if let Err(error) = self.stop_plugin(plugin_id).await {
            // Graceful shutdown can fail even after forced termination succeeded.
            // Continue only when the process owner has confirmed it is gone.
            if self.inner.plugins.read().await.get(plugin_id).is_some_and(|entry| entry.process.is_some()) {
                return Err(error);
            }
        }
        // Dropping the hub closes every local observer. Subscription operations
        // share the per-plugin lifecycle gate, so no late hub can be inserted.
        self.inner.subscriptions.lock().map_err(|_| update_channel_error())?.remove(plugin_id);
        let catalog = {
            let plugins = self.inner.plugins.read().await;
            PluginCatalog::from_entries(plugins.iter().filter(|(id, _)| id.as_str() != plugin_id).map(|(_, entry)| entry.catalog.clone()))?
        };
        if package_exists {
            // Do not hold the global catalog lock or block an async worker while
            // deleting a package: unrelated Providers continue serving requests.
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(installed))
                .await.map_err(import_error)?.map_err(import_error)?;
        }
        // Failure here is reported; the stopped entry can be removed again even
        // though its files are already gone. Never report a partial uninstall as success.
        self.inner.instances.synchronize_catalog(&catalog)?;
        let mut plugins = self.inner.plugins.write().await;
        plugins.remove(plugin_id);
        drop(plugins);
        self.inner.status_changes.send_modify(|r| *r = r.saturating_add(1));
        self.inner.runtime_changes.send_modify(|r| *r = r.saturating_add(1));
        Ok(())
    }

    /// Adds a new package to this running manager. Existing routes/processes remain intact.
    pub async fn import_plugin(&self, manifest: PathBuf, root: PathBuf) -> HostResult<ProviderImportResult> {
        let _registry = self.inner.registry_operation.lock().await;
        let prepared = tokio::task::spawn_blocking(move || prepare_import(&manifest, &root))
            .await.map_err(import_error)??;
        let descriptor = prepared.descriptor.clone();
        let plugin_id = descriptor.plugin_id.clone();
        let operation = self.inner.plugin_operations.lock().map_err(|_| update_channel_error())?
            .entry(plugin_id.clone()).or_insert_with(|| Arc::new(Mutex::new(()))).clone();
        let _operation = operation.lock().await;
        {
            let mut plugins = self.inner.plugins.write().await;
            if self.inner.shutting_down.load(Ordering::SeqCst) { return Err(import_error("Provider Host is shutting down")); }
            if plugins.contains_key(&plugin_id) {
                return Err(HostError::new("duplicate_plugin_id", format!("Provider {plugin_id} is already registered")));
            }
            std::fs::rename(prepared.staging.path().join("package"), &prepared.destination).map_err(import_error)?;
            let records = match self.inner.instances.register_plugin(descriptor.clone()) {
                Ok(records) => records,
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&prepared.destination);
                    return Err(error);
                }
            };
            let instances = records.into_iter().filter(|record| record.plugin_id == plugin_id)
                .map(|record| (record.instance_id.clone(), ManagedInstance { record, instance: None, diagnostic: None })).collect();
            self.inner.plugin_operations.lock().map_err(|_| update_channel_error())?.insert(plugin_id.clone(), operation.clone());
            plugins.insert(plugin_id.clone(), PluginEntry {
                runtime_inventory_diagnostic: None, runtime_inventory_revision: 0,
                runtime_inventory: None, catalog: descriptor.clone(), reported: None,
                state: PluginRuntimeState::Stopped, diagnostic: None, generation: 0,
                connection_status: codepet_provider_sdk::ConnectionStatus::Offline,
                process: None, process_exit: None, stderr_diagnostics: Vec::new(), instances,
            });
        }
        let result = if descriptor.enabled {
            async { self.start_plugin(&plugin_id).await?; self.start_manifest_instances(&plugin_id).await }.await
        } else { Ok(()) };
        self.inner.status_changes.send_modify(|revision| *revision = revision.saturating_add(1));
        self.inner.runtime_changes.send_modify(|revision| *revision = revision.saturating_add(1));
        let snapshot = self.snapshot(&plugin_id).await?;
        Ok(ProviderImportResult {
            provider_id: plugin_id,
            display_name: snapshot.reported.map(|reported| reported.display_name).unwrap_or(descriptor.display_name),
            loaded: descriptor.enabled && result.is_ok(),
            diagnostic: result.err().map(|error| error.to_string()),
        })
    }
}
