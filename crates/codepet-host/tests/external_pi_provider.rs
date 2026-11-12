//! Opt-in acceptance test: the provider is built outside this Cargo workspace from the
//! installed App SDK. No builtin Provider implementation or fixture executable is used.
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    ProviderGatewayService, ProviderInstanceRegistry,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn request(gateway: &ProviderGatewayService, method: &str, params: Value) -> Value {
    let request = serde_json::from_value(
        json!({"jsonrpc":"2.0", "id":method, "method":method, "params":params}),
    )
    .unwrap();
    let response = gateway
        .dispatch_for_local_client("pi-validation-desktop", request)
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert!(value.get("error").is_none(), "{method}: {value}");
    value["result"].clone()
}

#[tokio::test]
#[ignore = "Requires CODEPET_PI_PACKAGE and a pi model endpoint; run examples/providers/pi/tests/host-validation.ts"]
async fn custom_pi_package_imports_runs_streams_interrupts_and_survives_restart() {
    let source =
        PathBuf::from(std::env::var("CODEPET_PI_PACKAGE").expect("Set CODEPET_PI_PACKAGE"));
    let workspace = std::env::var("CODEPET_PI_WORKSPACE").expect("Set CODEPET_PI_WORKSPACE");
    let live = std::env::var_os("CODEPET_PI_LIVE").is_some();
    let model =
        std::env::var("CODEPET_PI_MODEL").unwrap_or_else(|_| "codepet-validation/pi-test".into());
    let root = tempfile::tempdir().unwrap();
    let device = DeviceRegistry::open(root.path().join("device.json"), "Pi validation").unwrap();
    let registry = ProviderInstanceRegistry::open(
        root.path().join("instances.json"),
        device.identity().device_id.clone(),
    )
    .unwrap();
    let manager = Arc::new(
        PluginManager::new(
            device,
            PluginCatalog::discover(PluginCatalogConfig::default()),
            registry,
            PluginManagerConfig {
                provider_data_root: Some(root.path().join("data/providers")),
                provider_logs_root: Some(root.path().join("logs/providers")),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    manager.enable_connection_heartbeats();
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    let imported = manager
        .import_plugin(source.clone(), root.path().join("installed"))
        .await
        .unwrap();
    let id = "dev.codepet.example.pi";
    assert!(
        imported.loaded,
        "{:?}; {:?}",
        imported.diagnostic,
        manager.snapshot(id).await.unwrap().stderr_diagnostics
    );
    let listed = request(&gateway, "provider.list", json!({})).await;
    assert_eq!(listed["providers"].as_array().unwrap().len(), 1);
    let cursor = gateway.current_event_cursor();
    let created = request(&gateway, "conversation.create", json!({"providerId":"pi-example","permissionLevel":"read-only","workspaceRoot":workspace,"title":"Pi external validation"})).await;
    let conversation = created["conversation"]["resource"].clone();
    request(
        &gateway,
        "conversation.acquireInteraction",
        json!({"conversation":conversation}),
    )
    .await;
    let send = json!({"conversation":conversation,"clientRequestId":"external-once","capabilityRevision":"pi-readonly-v1",
        "input":{"kind":"text","text":"Use the read tool to read proof.txt. Reply with exactly its entire content and nothing else."}, "selection":{"accessModeId":"read-only","model":{"kind":"flat","modelId":model}}});
    let sent = request(&gateway, "turn.send", send.clone()).await;
    assert_eq!(sent["accepted"], true);
    let turn_id = sent["turn"]["resource"]["nativeResourceId"].clone();
    let mut subscription = gateway.subscribe_events(Some(&cursor)).unwrap();
    let mut delta_text = String::new();
    let mut delta_item = Value::Null;
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let event = serde_json::to_value(subscription.next_event().await.unwrap()).unwrap();
            if event["method"] == "turn.outputDelta" {
                delta_text.push_str(event["params"]["payload"]["delta"].as_str().unwrap());
                delta_item = event["params"]["payload"]["itemId"].clone();
            }
            if event["method"] == "turn.upserted"
                && event["params"]["payload"]["turn"]["status"] == "completed"
            {
                break;
            }
            assert_ne!(
                event["params"]["payload"]["turn"]["status"], "failed",
                "{event}"
            );
        }
    })
    .await
    .expect("Pi must complete a model/tool turn through real mux + Host + Gateway");
    assert_eq!(delta_text, "PI_VALIDATION_OK");
    let history = request(
        &gateway,
        "conversation.get",
        json!({"conversation":conversation}),
    )
    .await;
    let assistant = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["resource"]["nativeResourceId"], delta_item);
    assert!(history["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["kind"] == "tool" && item["status"] == "completed"));
    if live {
        manager.restart_plugin(id).await.unwrap();
        let reloaded = request(
            &gateway,
            "conversation.get",
            json!({"conversation":conversation}),
        )
        .await;
        assert!(reloaded["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["role"] == "assistant" && item["status"] == "completed"));
        manager.shutdown().await;
        println!("PI_LIVE_PROVIDER_OK: real cloud model {model}, Gateway, real pi read tool, streamed exact answer, stable item IDs, persisted history after restart");
        return;
    }
    assert_eq!(
        request(&gateway, "turn.send", send).await["turn"]["resource"]["nativeResourceId"],
        turn_id
    );
    let slow = request(&gateway, "turn.send", json!({"conversation":conversation,"clientRequestId":"external-interrupt","capabilityRevision":"pi-readonly-v1",
        "input":{"kind":"text","text":"WAIT_FOR_INTERRUPT"},"selection":{"accessModeId":"read-only","model":{"kind":"flat","modelId":"codepet-validation/pi-test"}}})).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = serde_json::to_value(subscription.next_event().await.unwrap()).unwrap();
            if event["method"] == "turn.outputDelta"
                && event["params"]["payload"]["delta"] == "waiting"
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    let interrupted = request(
        &gateway,
        "turn.interrupt",
        json!({"conversation":conversation,"turn":slow["turn"]["resource"]}),
    )
    .await;
    assert_eq!(interrupted["turn"]["status"], "interrupted");
    manager.restart_plugin(id).await.unwrap();
    let reloaded = request(
        &gateway,
        "conversation.get",
        json!({"conversation":conversation,"limit":1}),
    )
    .await;
    assert_eq!(reloaded["items"][1]["status"], "interrupted");
    assert!(reloaded["pageInfo"]["nextCursor"].is_string());
    assert!(root
        .path()
        .join("data/providers")
        .join(id)
        .join("usage.sqlite")
        .exists());
    let log = std::fs::read_to_string(
        root.path()
            .join("logs/providers")
            .join(id)
            .join("provider.log"),
    )
    .unwrap();
    assert!(log.contains("Pi Provider initialized"));
    assert_eq!(
        manager
            .import_plugin(source, root.path().join("installed"))
            .await
            .unwrap_err()
            .code,
        "duplicate_plugin_id"
    );
    manager.shutdown().await;
    println!("PI_EXTERNAL_PROVIDER_OK: import, mux bootstrap, initialize, Gateway, real pi SDK read tool, delta/item identity, idempotency, interrupt, private SQLite, restart, stderr logs, duplicate import");
}
