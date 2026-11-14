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

#[tokio::test]
async fn uninstall_removes_package_and_preserves_other_providers_and_reimport() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-remove", vec![plugin("existing", &["existing-instance"])]);
    assert!(manager.start_enabled().await[0].1.is_ok());
    let before = manager.snapshot("existing").await.unwrap();
    let root = directory.path().join("installed");
    let manifest = package(&directory.path().join("source"), "custom");
    manager.import_plugin(manifest.clone(), root.clone()).await.unwrap();
    assert!(manager.remove_plugin("existing", root.clone()).await.is_err());
    assert_eq!(manager.snapshot("existing").await.unwrap().generation, before.generation);
    manager.remove_plugin("custom", root.clone()).await.unwrap();
    assert!(manager.snapshot("custom").await.is_err());
    assert!(!root.join("custom").exists());
    assert!(!directory.path().join("removed-providers").exists());
    assert!(directory.path().join("source/asset.txt").exists());
    let catalog = PluginCatalog::discover(PluginCatalogConfig::default().with_directory(&root));
    assert!(catalog.diagnostics().is_empty());
    assert!(manager.import_plugin(manifest, root).await.unwrap().loaded);
    assert_eq!(manager.snapshot("existing").await.unwrap().generation, before.generation);
    manager.shutdown().await;
}

#[tokio::test]
async fn removal_unregisters_imported_plugin_even_if_package_was_archived_externally() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-missing-remove", vec![plugin("existing", &["existing-instance"])]);
    assert!(manager.start_enabled().await[0].1.is_ok());
    let before = manager.snapshot("existing").await.unwrap();
    let root = directory.path().join("installed");
    let manifest = package(&directory.path().join("source"), "custom");
    manager.import_plugin(manifest.clone(), root.clone()).await.unwrap();
    let archived = directory.path().join("external-archive");
    std::fs::rename(root.join("custom"), &archived).unwrap();
    manager.remove_plugin("custom", root.clone()).await.unwrap();
    assert!(manager.snapshot("custom").await.is_err());
    assert!(archived.join("asset.txt").exists());
    assert_eq!(manager.snapshot("existing").await.unwrap().generation, before.generation);
    assert!(manager.import_plugin(manifest, root).await.unwrap().loaded);
    manager.shutdown().await;
}

#[tokio::test]
async fn registry_uninstall_closes_live_process_and_subscriptions_and_updates_next_heartbeat() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-registry-uninstall", vec![plugin("existing", &["existing-instance"])]);
    manager.enable_connection_heartbeats();
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    let _phone = manager.remote_connections().register("same-phone-connection".into());
    assert!(manager.start_enabled().await[0].1.is_ok());
    let before = manager.snapshot("existing").await.unwrap();
    let root = directory.path().join("installed");
    let manifest = package(&directory.path().join("source"), "custom");
    let marker = directory.path().join("custom.pid");
    let mut payload: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    payload["env"]["CODEPET_FAKE_PID_MARKER"] = serde_json::json!(marker);
    std::fs::write(&manifest, serde_json::to_vec(&payload).unwrap()).unwrap();
    manager.unsubscribe_events("not-installed", "local-observer").await.unwrap();
    let imported = manager.import_plugin(manifest.clone(), root.clone()).await.unwrap();
    assert!(imported.loaded, "{:?}", imported.diagnostic);
    let mut subscription = manager.subscribe_events("custom", "local-observer").await.unwrap();
    let first = gateway.protocol_ping(codepet_gateway_sdk::PingRequest { sequence: 41 }).await.unwrap();
    assert!(first.providers.iter().any(|p| p.id == "custom-instance"));
    #[cfg(unix)]
    assert!(pid_is_alive(&marker));

    manager.remove_plugin("custom", root.clone()).await.unwrap();

    assert!(!root.join("custom").exists());
    assert!(!directory.path().join("removed-providers").exists());
    assert!(manager.snapshot("custom").await.is_err());
    #[cfg(unix)]
    assert!(!pid_is_alive(&marker), "uninstall must await actual child termination");
    tokio::time::timeout(Duration::from_secs(2), async {
        while subscription.receiver.recv().await.is_some() {}
    }).await.expect("all local subscription senders must close on uninstall");
    let next = gateway.protocol_ping(codepet_gateway_sdk::PingRequest { sequence: 42 }).await.unwrap();
    assert_eq!(next.sequence, 42);
    assert!(!next.providers.iter().any(|p| p.id == "custom-instance"));
    assert!(next.providers.iter().any(|p| p.id == "existing-instance"));
    assert_eq!(manager.snapshot("existing").await.unwrap().generation, before.generation);
    assert_eq!(manager.remote_connections().snapshot().connections.len(), 1);
    assert!(manager.subscribe_events("custom", "late-observer").await.is_err());
    assert!(gateway.provider_describe(ProviderDescribeRequest { provider_id: "custom-instance".into() }).await.is_err());
    // A clean reinstall must not attach to an old connection/subscription hub.
    assert!(manager.import_plugin(manifest, root).await.unwrap().loaded);
    let _new_subscription = manager.subscribe_events("custom", "local-observer").await.unwrap();
    manager.shutdown().await;
}

#[tokio::test]
async fn registry_uninstall_waits_for_forced_exit_when_graceful_shutdown_times_out() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager_with_options("device-forced-uninstall", vec![], 256,
        PluginProcessOptions { shutdown_timeout: Duration::from_millis(30), ..PluginProcessOptions::default() });
    let manifest = package(&directory.path().join("source"), "custom");
    let marker = directory.path().join("custom.pid");
    let mut payload: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    payload["env"]["CODEPET_FAKE_SHUTDOWN_RESPONSE_DELAY_MS"] = serde_json::json!("500");
    payload["env"]["CODEPET_FAKE_PID_MARKER"] = serde_json::json!(marker);
    std::fs::write(&manifest, serde_json::to_vec(&payload).unwrap()).unwrap();
    let root = directory.path().join("installed");
    manager.import_plugin(manifest, root.clone()).await.unwrap();
    manager.remove_plugin("custom", root.clone()).await.unwrap();
    assert!(!root.join("custom").exists());
    #[cfg(unix)]
    assert!(!pid_is_alive(&marker));
    assert!(manager.snapshots().await.is_empty());
    manager.shutdown().await;
}

#[tokio::test]
async fn inventory_removes_every_instance_replays_removal_and_reimport_uses_newer_revision() {
    let directory = tempfile::tempdir().unwrap();
    let manager = build_manager("device-inventory", vec![plugin("existing", &["existing-instance"])]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    let manifest = package(&directory.path().join("source"), "custom");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    let mut second = value["instances"][0].clone();
    second["instanceId"] = serde_json::json!("custom-second");
    second["displayName"] = serde_json::json!("Second custom instance");
    value["instances"].as_array_mut().unwrap().push(second);
    std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let root = directory.path().join("installed");
    assert!(manager.import_plugin(manifest.clone(), root.clone()).await.unwrap().loaded);
    let before = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    assert_eq!(before.providers.len(), 3);
    assert_eq!(before.providers.iter().filter(|p| p.plugin_id.as_deref() == Some("custom")).count(), 2);
    let cursor = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&cursor)).unwrap();
    manager.remove_plugin("custom", root.clone()).await.unwrap();
    // No list/ping call triggers this: Registry changes must actively reach the stream.
    let mut removed = std::collections::BTreeSet::new();
    let mut revision = before.inventory_revision.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while removed.len() < 2 {
            match events.next_event().await.unwrap() {
                GatewayEvent::ProviderRemoved { params, .. } => {
                    assert!(params.payload.inventory_revision > revision);
                    revision = params.payload.inventory_revision;
                    removed.insert(params.payload.provider_id);
                }
                GatewayEvent::ProviderChanged { params, .. } => {
                    assert!(params.payload.inventory_revision.unwrap() > revision);
                    revision = params.payload.inventory_revision.unwrap();
                }
                _ => {}
            }
        }
    }).await.expect("both removals must arrive without a heartbeat");
    assert_eq!(removed, ["custom-instance".to_string(), "custom-second".to_string()].into());
    let pong = gateway.protocol_ping(codepet_gateway_sdk::PingRequest { sequence: 1 }).await.unwrap();
    assert!(pong.inventory_revision.unwrap() >= revision);
    assert_eq!(pong.inventory_epoch, before.inventory_epoch);
    assert_eq!(pong.providers.len(), 1);
    assert_eq!(gateway.replay_events(Some(&cursor)).unwrap().iter().filter(|e| matches!(e, GatewayEvent::ProviderRemoved { .. })).count(), 2);
    assert!(manager.import_plugin(manifest, root).await.unwrap().loaded);
    let reimported = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    assert!(reimported.inventory_revision > pong.inventory_revision);
    assert_eq!(reimported.providers.len(), 3);
    manager.shutdown().await;
}

#[tokio::test]
async fn explicit_features_support_observation_without_instances_and_do_not_probe_runtime() {
    let mut observer = plugin("observer", &[]);
    observer.env.insert("CODEPET_FAKE_FEATURES".into(), "[\"observation\",\"future-feature\"]".into());
    let mut combined = plugin("combined", &["combined-instance"]);
    combined.env.insert("CODEPET_FAKE_FEATURES".into(), "[\"agent\",\"observation\"]".into());
    let manager = build_manager("device-features", vec![observer, combined]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    for (_, outcome) in manager.start_enabled().await { outcome.unwrap(); }
    let snapshot = manager.snapshot("observer").await.unwrap();
    assert!(snapshot.instances.is_empty());
    assert!(snapshot.reported.unwrap().features.unwrap().contains(&codepet_provider_sdk::ProviderFeature::Observation));
    let inventory = manager.runtime_get_installed("observer", codepet_provider_sdk::RuntimeGetInstalledRequest { refresh: Some(true) }).await.unwrap();
    assert_eq!(inventory.selection_required, Some(false));
    // The fixture runtime handler fails without its control file, so success proves it was not called.
    let providers = gateway.provider_list(ProviderListRequest {}).await.unwrap().providers;
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "combined-instance");
    assert_eq!(providers[0].plugin_id.as_deref(), Some("combined"));
    assert_eq!(providers[0].features, Some(vec![codepet_provider_sdk::ProviderFeature::Agent, codepet_provider_sdk::ProviderFeature::Observation]));
    manager.shutdown().await;
}

#[tokio::test]
async fn inventory_epoch_is_shared_by_snapshots_and_changes_across_gateway_lifetimes() {
    let manager = build_manager("epoch-one", vec![]);
    let gateway = ProviderGatewayService::with_remote_identity(manager.clone(), RemoteHostIdentity {
        device_id: "epoch-one".into(), descriptor: device_descriptor("Epoch one"),
    }).unwrap();
    let handshake = gateway.protocol_handshake(handshake_request()).await.unwrap();
    let list = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    let ping = gateway.protocol_ping(codepet_gateway_sdk::PingRequest { sequence: 1 }).await.unwrap();
    assert!(list.inventory_epoch.as_ref().is_some_and(|epoch| !epoch.is_empty()));
    assert_eq!(handshake.inventory_epoch, list.inventory_epoch);
    assert_eq!(ping.inventory_epoch, list.inventory_epoch);
    let next_manager = build_manager("epoch-two", vec![]);
    let next = ProviderGatewayService::new(next_manager.clone()).unwrap().provider_list(ProviderListRequest {}).await.unwrap();
    assert_eq!(list.inventory_revision, next.inventory_revision);
    assert_ne!(list.inventory_epoch, next.inventory_epoch);
    manager.shutdown().await;
    next_manager.shutdown().await;
}
