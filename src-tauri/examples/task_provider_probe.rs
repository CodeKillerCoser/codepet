//! Explicit local smoke: real Host -> Claude Provider -> native Harness, synthetic input only.
//! Arguments: Provider binary, native Claude executable, output directory.
#[path = "../src/task_lineage/provider_executor.rs"]
mod provider_executor;
use codepet_gateway_sdk::ProtocolServer;
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    ProviderGatewayService, ProviderInstanceRegistry,
};
use codepet_task_lineage::{
    domain::{Evidence, Message},
    extraction::TaskExtractor,
    management::{ExtractionSettings, Layout, ManagedExtractor},
};
use std::{path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("Expected Provider executable, Claude executable, output directory".into());
    }
    let root = PathBuf::from(&args[3]);
    std::fs::create_dir_all(root.join("plugins/claude")).map_err(|e| e.to_string())?;
    let manifest = serde_json::json!({"manifestVersion":1,"pluginId":"dev.codepet.claude","displayName":"Claude extraction probe","executable":args[1],"enabled":true,"args":[],"env":{},
        "instances":[{"instanceId":"extraction-probe","instanceKind":"claude","displayName":"Extraction probe","enabled":true,"settings":{"claudeExecutable":args[2]}}]});
    std::fs::write(
        root.join("plugins/claude/codepet-provider.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let device = DeviceRegistry::open(root.join("device.json"), "Extraction probe")
        .map_err(|e| e.to_string())?;
    let registry = ProviderInstanceRegistry::open(
        root.join("instances.json"),
        device.identity().device_id.clone(),
    )
    .map_err(|e| e.to_string())?;
    let manager = Arc::new(
        PluginManager::new(
            device,
            PluginCatalog::discover(
                PluginCatalogConfig::default().with_directory(root.join("plugins")),
            ),
            registry,
            PluginManagerConfig {
                provider_data_root: Some(root.join("provider-data")),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
    );
    let gateway =
        Arc::new(ProviderGatewayService::new(manager.clone()).map_err(|e| e.to_string())?);
    gateway.start_event_forwarding();
    let started = manager.start_enabled().await;
    if started.is_empty() || started.iter().any(|(_, r)| r.is_err()) {
        manager.shutdown().await;
        return Err(format!("Provider startup: {started:?}"));
    }
    let ready = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            let description = gateway
                .provider_describe(codepet_gateway_sdk::ProviderDescribeRequest {
                    provider_id: "extraction-probe".into(),
                })
                .await
                .map_err(|e| e.message)?;
            if description.provider.runtime.status == codepet_gateway_sdk::ProviderStatus::Ready {
                return Ok::<_, String>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    })
    .await;
    if !matches!(ready, Ok(Ok(()))) {
        manager.shutdown().await;
        return Err(format!("Provider readiness: {ready:?}"));
    }
    let runtime = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        let config = ExtractionSettings { prompt: "将登录表单的实现与验证整理为一个任务。".into(), ..Default::default() };
        let extractor = ManagedExtractor { inner: provider_executor::ProviderExecutor { gateway, instance_id: "extraction-probe".into(), runtime }, layout: Layout::initialize(&root)?, config };
        let messages: Vec<_> = [("user", "修复登录表单键盘焦点。"), ("assistant", "登录框已能用 Tab 聚焦，测试通过。")].iter().enumerate().map(|(index,(role,text))| Message {
            evidence: Evidence { event_id: format!("synthetic-{index}"), file: "synthetic".into(), byte_offset: index as u64, generation: 0 },
            thread_id: "synthetic-login".into(), role: (*role).into(), text: (*text).into(), timestamp: Some(chrono::Utc::now().to_rfc3339()), turn_id: None,
        }).collect();
        let result = extractor.extract(&messages, &[])?;
        if result.extraction.tasks.is_empty() { return Err("Expected at least one extracted task".into()); }
        println!("{}", serde_json::json!({"tasks":result.extraction.tasks.len(),"requestedModel":result.requested_model,"provider":"extraction-probe"}));
        Ok::<_,String>(())
    }).await.map_err(|e|e.to_string());
    let shutdown = manager.shutdown().await;
    result??;
    if shutdown.iter().any(|(_, r)| r.is_err()) {
        return Err(format!("Provider cleanup failed: {shutdown:?}"));
    }
    Ok(())
}
