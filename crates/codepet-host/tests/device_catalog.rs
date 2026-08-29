use codepet_host::{
    DeviceIdentity, DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager,
    PluginManagerConfig, ProviderInstanceRegistry,
};
use serde_json::json;
use std::fs;

#[test]
fn device_identity_persists_and_corruption_is_rebuilt_with_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("device-identity.json");
    let first = DeviceRegistry::open(&path, "Test Device").unwrap();
    let first_id = first.identity().device_id.clone();
    assert!(first_id.starts_with("device-"));

    let reopened = DeviceRegistry::open(&path, "Renamed Device").unwrap();
    assert_eq!(reopened.identity().device_id, first_id);
    assert_eq!(reopened.identity().display_name, "Test Device");

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
    fs::write(
        plugin_directory.join("codepet-provider.json"),
        serde_json::to_vec_pretty(&json!({
            "manifestVersion": 1,
            "pluginId": "dev.codepet.fake",
            "displayName": "Fake",
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
    let descriptor = catalog.descriptor("dev.codepet.fake").unwrap();
    assert_eq!(
        descriptor.executable,
        plugin_directory.join("bin/codepet-provider-fake")
    );

    let registry_path = directory.path().join("provider-instances.json");
    let registry = ProviderInstanceRegistry::open(&registry_path, "device-test".to_string()).unwrap();
    let first = registry.synchronize_catalog(&catalog).unwrap();
    assert_eq!(first.len(), 1);
    let first_id = first[0].instance_id.clone();
    assert!(first_id.starts_with("instance-"));

    let reopened = ProviderInstanceRegistry::open(&registry_path, "device-test".to_string()).unwrap();
    let second = reopened.synchronize_catalog(&catalog).unwrap();
    assert_eq!(second[0].instance_id, first_id);
    assert_eq!(second[0].settings["profile"], json!("test"));

    let dynamic = reopened
        .create(
            "dev.codepet.fake".to_string(),
            "fake".to_string(),
            "Dynamic Fake".to_string(),
            Default::default(),
            None,
            true,
        )
        .unwrap();
    let reopened = ProviderInstanceRegistry::open(&registry_path, "device-test".to_string()).unwrap();
    let manager = PluginManager::new(
        DeviceRegistry::from_identity(DeviceIdentity {
            version: 1,
            device_id: "device-test".to_string(),
            display_name: "Device Test".to_string(),
            created_at: 1,
        })
        .unwrap(),
        catalog,
        reopened,
        PluginManagerConfig::default(),
    )
    .unwrap();
    let snapshot = manager.snapshot("dev.codepet.fake").await.unwrap();
    assert!(snapshot
        .instances
        .iter()
        .any(|instance| instance.record.instance_id == dynamic.instance_id));
}

#[test]
fn instance_registry_rejects_a_different_device() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instances.json");
    ProviderInstanceRegistry::open(&path, "device-a".to_string()).unwrap();
    let error = ProviderInstanceRegistry::open(&path, "device-b".to_string()).unwrap_err();
    assert_eq!(error.code, "provider_instance_registry_device_mismatch");

    let identity = DeviceIdentity {
        version: 1,
        device_id: "device-a".to_string(),
        display_name: "Device A".to_string(),
        created_at: 1,
    };
    assert_eq!(
        DeviceRegistry::from_identity(identity)
            .unwrap()
            .identity()
            .device_id,
        "device-a"
    );
}
