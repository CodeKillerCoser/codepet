use codepet_host::{PluginDescriptor, PluginProcess, PluginProcessOptions};
use codepet_provider_sdk::*;
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

// Exercise the actual Host timeout and mux boundary, without prompts or hook installation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires built Providers and installed native Harness binaries"]
async fn native_inventory_completes_through_host_with_duplicate_path_entries() {
    let binaries = std::env::var_os("CODEPET_NATIVE_PROVIDER_DIR").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target").join("debug"));
    let names = std::env::var("CODEPET_NATIVE_PROVIDER_NAMES").unwrap_or_else(|_| "claude,codex,opencode".into());
    for name in names.split(',') {
        let mut env = BTreeMap::new();
        let paths: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
        let repeated = (0..10).flat_map(|_| paths.iter().cloned()).collect::<Vec<_>>();
        env.insert("PATH".into(), std::env::join_paths(repeated).unwrap().to_string_lossy().into_owned());
        let descriptor = PluginDescriptor {
            plugin_id: format!("dev.codepet.{name}"), display_name: name.into(), icon: None,
            executable: binaries.join(format!("codepet-provider-{name}{}", std::env::consts::EXE_SUFFIX)),
            args: vec![], env, enabled: true, instances: vec![],
        };
        let process = PluginProcess::spawn(&descriptor, PluginProcessOptions::default()).unwrap();
        let mut inbound = process.take_inbound().await.unwrap();
        let (inventory_tx, mut inventory_rx) = tokio::sync::mpsc::unbounded_channel();
        let events = tokio::spawn(async move {
            while let Some(message) = inbound.recv().await {
                if let ProviderWireMessage::Event(ProtocolEvent::RuntimeInventoryChanged { params, .. }) = message {
                    let _ = inventory_tx.send(params);
                }
            }
        });
        let result = async {
            process.client().provider_initialize(ProviderInitializeRequest {
                host_client_id: "native-smoke".into(), host_device_id: "native-smoke-device".into(),
                host_version: "test".into(), supported_versions: VersionRange { min_version: PROTOCOL_VERSION, max_version: PROTOCOL_VERSION },
            }).await?;
            process.client().provider_describe(ProviderDescribeRequest {}).await?;
            let started = Instant::now();
            let snapshot = process.client().runtime_get_installed(RuntimeGetInstalledRequest { refresh: None }).await?;
            assert!(started.elapsed() < std::time::Duration::from_secs(2), "snapshot blocked on executable probing");
            println!("{name}: snapshot scanning={:?} in {:?}", snapshot.scanning, started.elapsed());
            let inventory = tokio::time::timeout(std::time::Duration::from_secs(180), inventory_rx.recv()).await.expect("missing scan notification").expect("notification stream closed");
            println!("{name}: {} native installations notified in {:?}", inventory.installed.len(), started.elapsed());
            println!("{name}: installations={:?}; scan_error={:?}", inventory.installed, inventory.scan_error);
            assert!(!inventory.installed.is_empty(), "{name}: {inventory:?}");
            assert_eq!(inventory.scanning, Some(false));
            let rescan_started = Instant::now();
            let refreshed = process.client().runtime_get_installed(RuntimeGetInstalledRequest { refresh: Some(true) }).await?;
            assert_eq!(refreshed.scanning, Some(true));
            assert!(rescan_started.elapsed() < std::time::Duration::from_secs(2));
            let completed = tokio::time::timeout(std::time::Duration::from_secs(180), inventory_rx.recv()).await.expect("missing rescan notification").unwrap();
            assert_eq!(completed.scanning, Some(false));
            assert!(!completed.installed.is_empty());
            Ok::<_, codepet_provider_sdk::ProtocolError>(())
        }.await;
        let shutdown = process.shutdown().await;
        events.abort();
        assert!(result.is_ok(), "{name}: {result:?}; stderr={:?}", process.stderr_diagnostics());
        assert!(shutdown.is_ok(), "{name}: {shutdown:?}");
    }
}
