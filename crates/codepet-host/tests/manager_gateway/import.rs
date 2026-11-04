use super::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn package(root: &Path, id: &str) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let executable = format!("provider{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(env!("CARGO_BIN_EXE_codepet-host-fake-provider"), root.join(&executable)).unwrap();
    std::fs::write(root.join("asset.txt"), "retained asset").unwrap();
    let mut descriptor = plugin(id, &[&format!("{id}-instance")]);
    descriptor.executable = executable.into();
    let mut value = serde_json::to_value(descriptor).unwrap();
    value["manifestVersion"] = serde_json::json!(1);
    let manifest = root.join("codepet-provider.json");
    std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    manifest
}

#[tokio::test]
async fn import_plugin_loads_into_live_gateway_and_heartbeats_without_restarting_existing_provider() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-import", vec![plugin("existing", &["existing-instance"])]);
    manager.enable_connection_heartbeats();
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    manager.replace_instance_setting("existing", "fake", "liveSetting", Some(serde_json::json!("preserved"))).await.unwrap();
    let before = manager.snapshot("existing").await.unwrap();
    let _phone = manager.remote_connections().register("connected-phone".into());
    let manifest = package(&directory.path().join("source"), "custom");
    let marker = directory.path().join("started.txt");
    let mut payload: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    payload["env"]["CODEPET_FAKE_INSTANCE_START_MARKER"] = serde_json::json!(marker);
    std::fs::write(&manifest, serde_json::to_vec(&payload).unwrap()).unwrap();
    let root = directory.path().join("installed");
    let cursor = gateway.current_event_cursor();
    let imported = manager.import_plugin(manifest.clone(), root.clone()).await.unwrap();
    assert!(imported.loaded, "{:?}", imported.diagnostic);
    assert_eq!(std::fs::read_to_string(root.join("custom/asset.txt")).unwrap(), "retained asset");
    assert!(manager.snapshot("custom").await.unwrap().catalog.executable.starts_with(root.canonicalize().unwrap()));
    assert_eq!(manager.snapshot("existing").await.unwrap().generation, before.generation);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = manager.snapshot("custom").await.unwrap();
            if marker.exists() && snapshot.connection_status == codepet_provider_sdk::ConnectionStatus::Online
                && gateway.current_event_cursor() > cursor { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("imported provider must join existing phone heartbeat lifecycle");
    let listed = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    assert_eq!(listed.providers.len(), 2);
    assert!(listed.providers.iter().any(|provider| provider.identity.display_name == "Instance custom-instance"));
    assert_eq!(manager.import_plugin(manifest, root.clone()).await.unwrap_err().code, "duplicate_plugin_id");
    assert_eq!(manager.snapshot("existing").await.unwrap().generation, before.generation);
    manager.restart_plugin("existing").await.unwrap();
    assert_eq!(manager.snapshot("existing").await.unwrap().instances[0].record.settings["liveSetting"], serde_json::json!("preserved"));
    manager.shutdown().await;
    // The installed package is independently discoverable on the next launch.
    let catalog = PluginCatalog::discover(PluginCatalogConfig::default().with_directory(root));
    assert!(catalog.diagnostics().is_empty());
    let device = DeviceRegistry::open(directory.path().join("next-device.json"), "Next launch").unwrap();
    let registry = ProviderInstanceRegistry::open(directory.path().join("next-instances.json"), device.identity().device_id.clone()).unwrap();
    let restarted = PluginManager::new(device, catalog, registry, PluginManagerConfig::default()).unwrap();
    assert_eq!(restarted.snapshots().await.len(), 1);
}

#[tokio::test]
async fn import_plugin_rejects_invalid_packages_without_registering_or_publishing_them() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-import-invalid", vec![]);
    let root = directory.path().join("installed");
    let manifest = package(&directory.path().join("source"), "../escape");
    assert!(manager.import_plugin(manifest.clone(), root.clone()).await.is_err());
    let mut value: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["pluginId"] = serde_json::json!("external");
    value["executable"] = serde_json::json!(env!("CARGO_BIN_EXE_codepet-host-fake-provider"));
    std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(manager.import_plugin(manifest.clone(), root.clone()).await.is_err());
    std::fs::write(&manifest, "{}").unwrap();
    assert!(manager.import_plugin(manifest, root.clone()).await.is_err());
    assert!(manager.snapshots().await.is_empty());
    assert!(!root.exists());
    manager.shutdown().await;
}

#[tokio::test]
async fn import_plugin_keeps_failed_start_visible_and_rejects_after_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-import-fail", vec![]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    let manifest = package(&directory.path().join("source"), "broken");
    std::fs::write(directory.path().join("source").join(format!("provider{}", std::env::consts::EXE_SUFFIX)), "not an executable").unwrap();
    let root = directory.path().join("installed");
    let result = manager.import_plugin(manifest, root.clone()).await.unwrap();
    assert!(!result.loaded);
    assert!(result.diagnostic.is_some());
    assert!(manager.snapshot("broken").await.unwrap().diagnostic.is_some());
    manager.shutdown().await;
    let manifest = package(&directory.path().join("second"), "late");
    assert!(manager.import_plugin(manifest, root.clone()).await.is_err());
    assert!(!root.join("late").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn import_plugin_rejects_package_symlinks_and_removes_staging() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-import-link", vec![]);
    let manifest = package(&directory.path().join("source"), "linked");
    std::os::unix::fs::symlink("/", directory.path().join("source/escape")).unwrap();
    let root = directory.path().join("installed");
    assert!(manager.import_plugin(manifest, root.clone()).await.is_err());
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
    assert!(manager.snapshots().await.is_empty());
    manager.shutdown().await;
}
