use super::*;
use codepet_provider_sdk::RuntimeGetInstalledRequest;
use std::path::{Path, PathBuf};
use serde_json::{json, Value};

const OPENCODE: &str = "dev.codepet.opencode";
const CODEX: &str = "dev.codepet.codex";
const CLAUDE: &str = "dev.codepet.claude";

fn controlled_plugin(root: &Path, id: &str) -> (PluginDescriptor, PathBuf) {
    let control = root.join(format!("{id}.json"));
    set_control(&control, json!({}));
    let mut descriptor = plugin(id, &[id.rsplit('.').next().unwrap()]);
    descriptor.env.insert("CODEPET_FAKE_RUNTIME_CONTROL".into(), control.to_string_lossy().into());
    (descriptor, control)
}
fn set_control(path: &Path, value: Value) {
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.expect("fixture request did not reach Provider");
}
async fn initialize(manager: &PluginManager, ids: &[&str]) {
    let _ = manager.start_enabled().await;
    for id in ids {
        manager.runtime_get_installed(id, RuntimeGetInstalledRequest { refresh: None }).await.unwrap();
    }
}
async fn version(manager: &PluginManager, id: &str) -> String {
    manager.snapshot(id).await.unwrap().runtime_inventory.unwrap().selected.unwrap().version
}
async fn visible(gateway: &ProviderGatewayService, id: &str) -> bool {
    gateway.provider_list(ProviderListRequest {}).await.unwrap().providers.iter().any(|provider| provider.id == id)
}
fn refresh(manager: Arc<PluginManager>, id: &'static str) -> tokio::task::JoinHandle<codepet_host::HostResult<codepet_provider_sdk::RuntimeGetInstalledResponse>> {
    tokio::spawn(async move { manager.runtime_get_installed(id, RuntimeGetInstalledRequest { refresh: Some(true) }).await })
}

#[tokio::test]
async fn failed_and_timed_out_providers_do_not_cancel_a_parallel_success_or_hide_cached_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (opencode, open_control) = controlled_plugin(dir.path(), OPENCODE);
    let (codex, codex_control) = controlled_plugin(dir.path(), CODEX);
    let mut claude = plugin(CLAUDE, &["claude"]);
    claude.executable = dir.path().join("missing-provider");
    let manager = build_manager("device-independent-scans", vec![claude, codex, opencode]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    initialize(&manager, &[CODEX, OPENCODE]).await;
    let open_marker = dir.path().join("open-started");
    let codex_marker = dir.path().join("codex-started");
    let open_gate = dir.path().join("open-release");
    let codex_gate = dir.path().join("codex-release");
    set_control(&open_control, json!({"marker":open_marker,"gate":open_gate,"version":"2.0.0"}));
    set_control(&codex_control, json!({"marker":codex_marker,"gate":codex_gate,"version":"2.0.0"}));
    let batch_manager = manager.clone();
    let batch = tokio::spawn(async move {
        batch_manager.refresh_runtime_inventories(vec![CLAUDE.into(), CODEX.into(), OPENCODE.into()]).await
    });
    wait_for_file(&open_marker).await;
    wait_for_file(&codex_marker).await; // Both entered despite Claude's immediate failure.
    assert!(visible(&gateway, "opencode").await);
    assert!(visible(&gateway, "codex").await);
    std::fs::write(open_gate, b"go").unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while version(&manager, OPENCODE).await != "2.0.0" { tokio::task::yield_now().await; }
    }).await.expect("healthy Provider must publish before its stalled peer finishes");
    assert!(!batch.is_finished());
    let results = batch.await.unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].1.is_err());
    assert!(results[1].1.is_err()); // The blocked Codex RPC timed out independently.
    assert!(results[2].1.is_ok());
    assert_eq!(version(&manager, CODEX).await, "1.0.0");
    assert!(manager.snapshot(CODEX).await.unwrap().runtime_inventory_diagnostic.is_some());
    assert!(manager.snapshot(OPENCODE).await.unwrap().runtime_inventory_diagnostic.is_none());
    assert!(visible(&gateway, "opencode").await);
    assert!(visible(&gateway, "codex").await);
    std::fs::write(codex_gate, b"go").unwrap();
    manager.shutdown().await;
}

#[tokio::test]
async fn aborted_refresh_and_rpc_failure_preserve_inventory_until_successful_empty_scan() {
    let dir = tempfile::tempdir().unwrap();
    let (provider, control) = controlled_plugin(dir.path(), OPENCODE);
    let manager = build_manager("device-cancel-scan", vec![provider]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap()); gateway.start_event_forwarding();
    initialize(&manager, &[OPENCODE]).await;
    let marker = dir.path().join("started"); let gate = dir.path().join("release");
    set_control(&control, json!({"marker":marker,"gate":gate,"version":"2.0.0"}));
    let request = refresh(manager.clone(), OPENCODE);
    wait_for_file(&marker).await;
    request.abort(); let _ = request.await;
    assert_eq!(version(&manager, OPENCODE).await, "1.0.0");
    assert!(visible(&gateway, "opencode").await);
    std::fs::write(gate, b"go").unwrap();
    set_control(&control, json!({"error":true}));
    assert!(refresh(manager.clone(), OPENCODE).await.unwrap().is_err());
    assert!(visible(&gateway, "opencode").await);
    set_control(&control, json!({"empty":true,"scanError":"probe failed"}));
    refresh(manager.clone(), OPENCODE).await.unwrap().unwrap();
    assert!(visible(&gateway, "opencode").await);
    set_control(&control, json!({"empty":true}));
    refresh(manager.clone(), OPENCODE).await.unwrap().unwrap();
    assert!(!visible(&gateway, "opencode").await, "successful absence must still hide an uninstalled builtin");
    assert!(manager.snapshot(OPENCODE).await.unwrap().runtime_inventory_diagnostic.is_none());
    manager.shutdown().await;
}

#[tokio::test]
async fn inventory_notification_wins_over_older_scanning_rpc_response() {
    let dir = tempfile::tempdir().unwrap();
    let (provider, control) = controlled_plugin(dir.path(), OPENCODE);
    let manager = build_manager("device-notification-race", vec![provider]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap()); gateway.start_event_forwarding();
    initialize(&manager, &[OPENCODE]).await;
    let gate = dir.path().join("release");
    set_control(&control, json!({"notifyVersion":"3.0.0","version":"2.0.0","scanning":true,"gate":gate}));
    let request = refresh(manager.clone(), OPENCODE);
    tokio::time::timeout(Duration::from_secs(2), async {
        while version(&manager, OPENCODE).await != "3.0.0" { tokio::task::yield_now().await; }
    }).await.unwrap();
    std::fs::write(gate, b"go").unwrap();
    let response = request.await.unwrap().unwrap();
    assert_eq!(response.scanning, Some(false));
    assert_eq!(response.selected.unwrap().version, "3.0.0");
    assert_eq!(version(&manager, OPENCODE).await, "3.0.0");
    manager.shutdown().await;
}

#[tokio::test]
async fn older_refresh_cannot_replace_a_newer_request_or_new_process_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let (provider, control) = controlled_plugin(dir.path(), OPENCODE);
    let manager = build_manager("device-generation-race", vec![provider]);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap()); gateway.start_event_forwarding();
    initialize(&manager, &[OPENCODE]).await;
    let marker = dir.path().join("started"); let gate = dir.path().join("release");
    set_control(&control, json!({"marker":marker,"gate":gate,"version":"2.0.0"}));
    let old = refresh(manager.clone(), OPENCODE);
    wait_for_file(&marker).await;
    set_control(&control, json!({"version":"3.0.0"}));
    refresh(manager.clone(), OPENCODE).await.unwrap().unwrap();
    std::fs::write(&gate, b"go").unwrap();
    assert_eq!(old.await.unwrap().unwrap().selected.unwrap().version, "3.0.0");
    assert_eq!(version(&manager, OPENCODE).await, "3.0.0");
    std::fs::remove_file(&marker).unwrap(); std::fs::remove_file(&gate).unwrap();
    set_control(&control, json!({"marker":marker,"gate":gate,"error":true}));
    let old = refresh(manager.clone(), OPENCODE); wait_for_file(&marker).await;
    set_control(&control, json!({"version":"4.0.0"}));
    manager.restart_plugin(OPENCODE).await.unwrap();
    manager.runtime_get_installed(OPENCODE, RuntimeGetInstalledRequest { refresh: None }).await.unwrap();
    let _ = old.await;
    assert_eq!(version(&manager, OPENCODE).await, "4.0.0");
    assert!(manager.snapshot(OPENCODE).await.unwrap().runtime_inventory_diagnostic.is_none());
    manager.shutdown().await;
}
