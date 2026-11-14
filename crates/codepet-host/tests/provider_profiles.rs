//! These fixtures were authored independently from only the exported schemas/guide.
//! They contain no Host implementation or Rust SDK. Node 24+ is a test prerequisite.
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    PluginProcess, PluginProcessOptions, ProviderGatewayService, ProviderInstanceRegistry,
};
use codepet_provider_sdk::{
    ProviderDescribeRequest, ProviderInitializeRequest, ProviderPingRequest, ProtocolVersionOffer,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/providers/independent/packages")
}
fn package(directory: &Path, profile: &str, changes: Value) -> PathBuf {
    fn copy(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target)
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let source = fixtures().join(profile);
    copy(&source, directory);
    let path = directory.join("codepet-provider.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["env"]["ECHO_CHUNK_SIZE"] = json!("65536");
    // A custom external runtime is explicit, never copied or interpreted as a package file.
    let node = std::env::var("CODEPET_TEST_NODE").unwrap_or_else(|_| "node".into());
    manifest["command"] = json!(node);
    for (key, value) in changes.as_object().unwrap() {
        manifest["env"][key] = value.clone();
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    path
}
async fn manager(root: &Path) -> Arc<PluginManager> {
    let device = DeviceRegistry::open(root.join("device.json"), "Profile test").unwrap();
    let registry = ProviderInstanceRegistry::open(
        root.join("instances.json"),
        device.identity().device_id.clone(),
    )
    .unwrap();
    Arc::new(
        PluginManager::new(
            device,
            PluginCatalog::discover(PluginCatalogConfig::default()),
            registry,
            PluginManagerConfig {
                provider_data_root: Some(root.join("data/providers")),
                provider_logs_root: Some(root.join("logs/providers")),
                ..Default::default()
            },
        )
        .unwrap(),
    )
}
async fn request(gateway: &ProviderGatewayService, method: &str, params: Value) -> Value {
    let wire = serde_json::from_value(
        json!({"jsonrpc":"2.0","id":method,"method":method,"params":params}),
    )
    .unwrap();
    let response = serde_json::to_value(
        gateway
            .dispatch_for_local_client("profile-desktop", wire)
            .await,
    )
    .unwrap();
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}
fn permission_level(provider: &Value) -> &str {
    let capabilities = &provider["capabilities"];
    let access = capabilities["conversationCreate"]["selection"]["accessMode"]
        .as_object().map(|_| &capabilities["conversationCreate"]["selection"]["accessMode"])
        .unwrap_or(&capabilities["turnSend"]["accessMode"]);
    access["defaultId"].as_str()
        .or_else(|| access["options"].as_array()?.iter()
            .find(|option| option["enabled"] != false)?["id"].as_str())
        .unwrap_or("read-only")
}

async fn initialize(process: &PluginProcess) {
    process
        .client()
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "profile-host".into(),
            host_device_id: "profile-device".into(),
            host_version: "1".into(),
            protocol: ProtocolVersionOffer {
                min_version: 2,
                version: 2,
            },
            directories: None,
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn external_schema_only_plugins_import_and_run_all_three_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let manager = manager(root).await;
    manager.enable_connection_heartbeats();
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    for profile in ["jsonl", "zstd", "mux"] {
        let manifest = package(
            &root.join(format!("source-{profile}")),
            profile,
            json!({"ECHO_PARTIAL_BYTES":"7","ECHO_PARTIAL_DELAY_MS":"1","ECHO_TURN_DELAY_MS":"1"}),
        );
        let imported = manager
            .import_plugin(manifest, root.join("installed"))
            .await
            .unwrap();
        assert!(
            imported.loaded,
            "{profile}: {:?}; {:?}",
            imported.diagnostic,
            manager
                .snapshot(&format!("independent.echo.{profile}"))
                .await
                .unwrap()
                .stderr_diagnostics
        );
        let inventory = manager.runtime_get_installed(&format!("independent.echo.{profile}"),
            codepet_provider_sdk::RuntimeGetInstalledRequest { refresh: None }).await.unwrap();
        assert_eq!(inventory.selection_required, Some(false));
        assert!(inventory.harness_list.is_empty() && inventory.selected.is_none());
        let instance = format!("independent-echo-{profile}");
        let described = request(
            &gateway,
            "provider.describe",
            json!({"providerId":instance}),
        )
        .await;
        let revision = described["capabilities"]["revision"].clone();
        assert!(revision.is_string(), "{described}");
        let created=request(&gateway,"conversation.create",json!({"providerId":instance,"permissionLevel":permission_level(&described),"workspaceRoot":root,"title":"Profile conformance"})).await;
        let conversation = created["conversation"]["resource"].clone();
        let selection = request(
            &gateway,
            "conversation.acquireInteraction",
            json!({"conversation":conversation}),
        )
        .await["selection"]
            .clone();
        let cursor = gateway.current_event_cursor();
        let mut events = gateway.subscribe_events(Some(&cursor)).unwrap();
        let input = format!("中文\u{2028}\n{profile}:{}", "z".repeat(128 * 1024));
        let send = json!({"conversation":conversation,"clientRequestId":format!("send-{profile}"),"capabilityRevision":revision,"input":{"kind":"text","text":input},"selection":selection});
        let sent = request(&gateway, "turn.send", send.clone()).await;
        assert_eq!(sent["accepted"], true);
        let mut output = String::new();
        let mut item_id = Value::Null;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = serde_json::to_value(events.next_event().await.unwrap()).unwrap();
                let payload = &event["params"]["payload"];
                if event["method"] == "turn.outputDelta" {
                    output.push_str(payload["delta"].as_str().unwrap());
                    item_id = payload["itemId"].clone();
                }
                if event["method"] == "turn.upserted" && payload["turn"]["status"] == "completed" {
                    break;
                }
                assert_ne!(payload["turn"]["status"], "failed", "{event}");
            }
        })
        .await
        .expect("external profile must complete and publish ordered events");
        assert_eq!(output, input);
        let history = request(
            &gateway,
            "conversation.get",
            json!({"conversation":conversation,"limit":1}),
        )
        .await;
        assert!(history["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource"]["nativeResourceId"] == item_id
                && item["status"] == "completed"));
        assert_eq!(
            request(&gateway, "turn.send", send).await["turn"]["resource"],
            sent["turn"]["resource"]
        );
    }
    assert_eq!(
        request(&gateway, "provider.list", json!({})).await["providers"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_requests_and_stalled_frames_only_block_their_own_connection() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (index, profile) in ["jsonl", "zstd", "mux"].into_iter().enumerate() {
        let manifest = package(
            &root.join(format!("slow-{profile}")),
            profile,
            json!({"ECHO_DESCRIBE_DELAY_MS":"2000"}),
        );
        let descriptor = PluginCatalog::read_one(&manifest).unwrap();
        let slow =
            Arc::new(PluginProcess::spawn(&descriptor, PluginProcessOptions::default()).unwrap());
        initialize(&slow).await;
        let manifest = package(
            &root.join(format!("fast-{profile}")),
            ["zstd", "mux", "jsonl"][index],
            json!({}),
        );
        let fast = PluginProcess::spawn(
            &PluginCatalog::read_one(&manifest).unwrap(),
            PluginProcessOptions::default(),
        )
        .unwrap();
        initialize(&fast).await;
        let work = slow.clone();
        let pending = tokio::spawn(async move {
            work.client()
                .provider_describe(ProviderDescribeRequest {})
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ping = ProviderPingRequest {
            sequence: 1,
            host_session_id: "independent-health".into(),
            clients: codepet_provider_sdk::ClientConnectionsSnapshot {
                revision: 0,
                connections: vec![],
            },
            instances: vec![],
        };
        let reply = tokio::time::timeout(
            Duration::from_secs(1),
            fast.client().provider_ping(ping.clone()),
        )
        .await
        .expect("another Provider must not wait for slow request")
        .unwrap();
        assert_eq!(reply.sequence, 1);
        // Serial wire still supports independent RPC execution and out-of-order replies by ID.
        tokio::time::timeout(Duration::from_secs(1), slow.client().provider_ping(ping))
            .await
            .expect("slow business request must not serialize request correlation")
            .unwrap();
        assert!(!pending.is_finished());
        pending.await.unwrap().unwrap();
        slow.shutdown().await.unwrap();
        fast.shutdown().await.unwrap();

        let manifest = package(
            &root.join(format!("partial-{profile}")),
            profile,
            json!({"ECHO_PARTIAL_BYTES":"7","ECHO_STALL_AFTER_PARTIAL":"1"}),
        );
        let broken = PluginProcess::spawn(
            &PluginCatalog::read_one(&manifest).unwrap(),
            PluginProcessOptions {
                request_timeout: Duration::from_millis(250),
                ..Default::default()
            },
        )
        .unwrap();
        let manifest = package(
            &root.join(format!("survivor-{profile}")),
            "jsonl",
            json!({}),
        );
        let survivor = PluginProcess::spawn(
            &PluginCatalog::read_one(&manifest).unwrap(),
            PluginProcessOptions::default(),
        )
        .unwrap();
        let blocked = broken
            .client()
            .provider_initialize(ProviderInitializeRequest {
                host_client_id: "h".into(),
                host_device_id: "d".into(),
                host_version: "1".into(),
                protocol: ProtocolVersionOffer {
                    min_version: 2,
                    version: 2,
                },
                directories: None,
            });
        let healthy = initialize(&survivor);
        let (result, _) = tokio::join!(blocked, healthy);
        assert!(result.is_err());
        survivor
            .client()
            .provider_describe(ProviderDescribeRequest {})
            .await
            .unwrap();
        broken.shutdown().await.ok();
        survivor.shutdown().await.unwrap();
    }
}

/// Explicit paid-model acceptance; the standalone pi CLI owns credentials/configuration.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Requires CODEPET_TEST_PI_COMMAND; explicitly calls the user's configured model"]
async fn schema_only_pi_plugin_runs_deepseek_flash_through_jsonrpc_profile() {
    let command = std::env::var("CODEPET_TEST_PI_COMMAND").expect("Set standalone pi CLI path");
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("proof.txt"), "PI_SCHEMA_ONLY_VALIDATION_OK").unwrap();
    let manager = manager(root).await;
    manager.enable_connection_heartbeats();
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    let manifest = package(
        &root.join("pi-source"),
        "pi-jsonl",
        json!({"ECHO_PI_COMMAND":command}),
    );
    let imported = manager
        .import_plugin(manifest, root.join("installed"))
        .await
        .unwrap();
    assert!(imported.loaded, "{:?}", imported.diagnostic);
    let create_capabilities = request(&gateway, "provider.describe", json!({"providerId":"independent-pi-jsonl"})).await;
    let created=request(&gateway,"conversation.create",json!({"providerId":"independent-pi-jsonl","permissionLevel":permission_level(&create_capabilities),"workspaceRoot":root,"title":"Independent pi contract acceptance"})).await;
    let conversation = created["conversation"]["resource"].clone();
    let selection = request(
        &gateway,
        "conversation.acquireInteraction",
        json!({"conversation":conversation}),
    )
    .await["selection"]
        .clone();
    assert!(
        created["conversation"]["model"]
            .as_str()
            .is_some_and(|value| value.contains("deepseek-flash")),
        "configured Flash model required: {}",
        created["conversation"]["model"]
    );
    let described = request(
        &gateway,
        "provider.describe",
        json!({"providerId":"independent-pi-jsonl"}),
    )
    .await;
    let mut events = gateway
        .subscribe_events(Some(&gateway.current_event_cursor()))
        .unwrap();
    let sent=request(&gateway,"turn.send",json!({"conversation":conversation,"clientRequestId":"schema-only-live","capabilityRevision":described["capabilities"]["revision"],
        "input":{"kind":"text","text":"Use the read tool to read proof.txt in the current directory. Reply with exactly the entire file content, with no explanation or formatting."},"selection":selection})).await;
    assert_eq!(sent["accepted"], true);
    let mut output = String::new();
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let event = serde_json::to_value(events.next_event().await.unwrap()).unwrap();
            let payload = &event["params"]["payload"];
            if event["method"] == "turn.outputDelta" {
                output.push_str(payload["delta"].as_str().unwrap());
            }
            if event["method"] == "turn.upserted" && payload["turn"]["status"] == "completed" {
                break;
            }
            assert_ne!(payload["turn"]["status"], "failed", "{event}");
        }
    })
    .await
    .expect("live Flash turn must complete");
    assert_eq!(output.trim(), "PI_SCHEMA_ONLY_VALIDATION_OK");
    let history = request(
        &gateway,
        "conversation.get",
        json!({"conversation":conversation}),
    )
    .await;
    assert!(history["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["role"] == "assistant" && item["status"] == "completed"));
    manager.shutdown().await;
    println!("INDEPENDENT_PI_FLASH_OK: manifest v2, JSONL, real pi RPC/read tool, DeepSeek Flash, delta and complete history; no Rust bridge or Host source used by plugin author");
}

#[tokio::test]
async fn older_schema_only_v2_plugins_remain_usable_without_feature_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let manager = manager(root).await;
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    for profile in ["jsonl", "zstd", "mux"] {
        let manifest = package(&root.join(format!("legacy-{profile}")), profile, json!({"ECHO_PROTOCOL_VERSION":"2"}));
        let imported = manager.import_plugin(manifest, root.join("installed")).await.unwrap();
        assert!(imported.loaded, "{:?}", imported.diagnostic);
        let id = format!("independent.echo.{profile}");
        let descriptor = manager.snapshot(&id).await.unwrap().reported.unwrap();
        assert_eq!(descriptor.protocol.version, 2);
        assert_eq!(descriptor.features, None);
        let described = request(&gateway, "provider.describe", json!({"providerId":format!("independent-echo-{profile}")})).await;
        assert_eq!(described["provider"]["pluginId"], id);
        assert!(described["provider"].get("features").is_none());
        let inventory = manager.runtime_get_installed(&id, codepet_provider_sdk::RuntimeGetInstalledRequest { refresh: None }).await.unwrap();
        assert_eq!(inventory.selection_required, Some(false));
        manager.remove_plugin(&id, root.join("installed")).await.unwrap();
    }
    manager.shutdown().await;
}
