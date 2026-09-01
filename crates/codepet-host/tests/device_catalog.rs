use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    ProviderInstanceRegistry,
};
use serde_json::json;
use std::fs;

#[test]
fn device_identity_persists_and_corruption_is_rebuilt_with_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("device-identity.json");
    let first = DeviceRegistry::open(&path, "Test Device").unwrap();
    let first_id = first.identity().device_id.clone();
    let first_created_at = first.identity().created_at;
    assert!(first_id.starts_with("device-"));

    let reopened = DeviceRegistry::open(&path, "Renamed Device").unwrap();
    assert_eq!(reopened.identity().device_id, first_id);
    assert_eq!(reopened.identity().display_name, "Renamed Device");
    assert_eq!(reopened.identity().created_at, first_created_at);
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted["deviceId"], first_id);
    assert_eq!(persisted["displayName"], "Renamed Device");

    fs::write(&path, b"not-json").unwrap();
    let rebuilt = DeviceRegistry::open(&path, "Recovered Device").unwrap();
    assert_ne!(rebuilt.identity().device_id, first_id);
    assert_eq!(rebuilt.diagnostics().len(), 1);
    assert_eq!(rebuilt.diagnostics()[0].code, "device_identity_rebuilt");
    assert!(rebuilt.diagnostics()[0]
        .recovered_path
        .as_ref()
        .unwrap()
        .exists());
}

#[tokio::test]
async fn catalog_discovers_only_explicit_manifests_and_instance_ids_remain_stable() {
    let directory = tempfile::tempdir().unwrap();
    let plugin_directory = directory.path().join("provider-plugins").join("fake");
    fs::create_dir_all(&plugin_directory).unwrap();
    fs::write(plugin_directory.join("ignored-binary"), b"not executable discovery").unwrap();
    let manifest_path = plugin_directory.join("codepet-provider.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&json!({
            "manifestVersion": 1,
            "pluginId": "dev.codepet.fake",
            "displayName": "Fake",
            "icon": "fake",
            "executable": "bin/codepet-provider-fake",
            "enabled": true,
            "instances": [{
                "instanceKind": "fake",
                "displayName": "Local Fake",
                "settings": { "profile": "test" },
                "enabled": true
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let catalog = PluginCatalog::discover(PluginCatalogConfig::for_data_directory(
        directory.path(),
    ));
    assert!(catalog.diagnostics().is_empty());

    let registry_path = directory.path().join("provider-instances.json");
    let device = DeviceRegistry::open(directory.path().join("device.json"), "Device Test").unwrap();
    let device_id = device.identity().device_id.clone();
    let registry = ProviderInstanceRegistry::open(&registry_path, device_id.clone()).unwrap();
    let first = registry.synchronize_catalog(&catalog).unwrap();
    assert_eq!(first.len(), 1);
    let first_id = first[0].instance_id.clone();
    assert!(first_id.starts_with("instance-"));
    let discovered = PluginManager::new(
        device.clone(),
        catalog.clone(),
        registry.clone(),
        PluginManagerConfig::default(),
    )
    .unwrap()
    .snapshot("dev.codepet.fake")
    .await
    .unwrap();
    assert_eq!(
        discovered.catalog.executable,
        plugin_directory.join("bin/codepet-provider-fake")
    );
    assert_eq!(discovered.catalog.icon.as_deref(), Some("fake"));

    let reopened = ProviderInstanceRegistry::open(&registry_path, device_id.clone()).unwrap();
    let second = reopened.synchronize_catalog(&catalog).unwrap();
    assert_eq!(second[0].instance_id, first_id);
    assert_eq!(second[0].settings["profile"], json!("test"));
    let plugin_mismatch = reopened
        .resolve_route(
            &codepet_host::provider_sdk::ProviderInstanceRoute {
                device_id: device_id.clone(),
                provider_plugin_id: "dev.codepet.other".to_string(),
                provider_instance_id: first_id.clone(),
            },
            Some("dev.codepet.other"),
        )
        .unwrap_err();
    assert_eq!(plugin_mismatch.code, "provider_instance_plugin_mismatch");

    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    assert!(persisted["instances"][0].get("settings").is_none());
    assert!(persisted["instances"][0].get("enabled").is_none());

    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["instances"] = json!([]);
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let catalog_without_instances = PluginCatalog::discover(
        PluginCatalogConfig::for_data_directory(directory.path()),
    );
    reopened
        .synchronize_catalog(&catalog_without_instances)
        .unwrap();
    assert!(reopened.list().unwrap().is_empty());
    let reopened = ProviderInstanceRegistry::open(&registry_path, device_id).unwrap();
    let manager = PluginManager::new(
        device,
        catalog_without_instances,
        reopened,
        PluginManagerConfig::default(),
    )
    .unwrap();
    let snapshot = manager.snapshot("dev.codepet.fake").await.unwrap();
    assert!(snapshot.instances.is_empty());
}

#[test]
fn instance_registry_rejects_a_different_device() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instances.json");
    ProviderInstanceRegistry::open(&path, "device-a".to_string()).unwrap();
    let error = ProviderInstanceRegistry::open(&path, "device-b".to_string()).unwrap_err();
    assert_eq!(error.code, "provider_instance_registry_device_mismatch");

}
