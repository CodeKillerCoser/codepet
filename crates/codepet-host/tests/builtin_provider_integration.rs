use codepet_gateway_sdk::{
    ConversationCreateRequest, ConversationListRequest, GatewayProviderRoute, ProtocolServer,
    ProviderInstance, ProviderListRequest, ProviderStatus,
};
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    PluginProcessOptions, PluginRuntimeState, ProviderGatewayService, ProviderInstanceRegistry,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const CODEX_PLUGIN_ID: &str = "dev.codepet.codex";
const CLAUDE_PLUGIN_ID: &str = "dev.codepet.claude";
const OPENCODE_PLUGIN_ID: &str = "dev.codepet.opencode";

#[tokio::test]
async fn codepet_host_runs_all_sdk_based_builtin_providers_end_to_end() {
    let workspace = crates_workspace();
    build_builtin_provider_fixtures(&workspace);
    let executable = |name: &str| {
        workspace
            .join("target/debug")
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    };
    let providers = [
        (
            "codex",
            "codepet-provider-codex",
            "appServerExecutable",
            executable("codex-app-server-fixture"),
        ),
        (
            "claude",
            "codepet-provider-claude",
            "claudeExecutable",
            executable("claude-stream-fixture"),
        ),
        (
            "opencode",
            "codepet-provider-opencode",
            "serverExecutable",
            executable("opencode-server-fixture"),
        ),
    ];
    for (_, provider_package, _, fixture) in &providers {
        assert!(executable(provider_package).is_file());
        assert!(fixture.is_file());
    }

    let directory = tempfile::tempdir().unwrap();
    let plugin_root = directory.path().join("provider-plugins");
    for (name, provider_package, executable_setting, fixture) in providers {
        let plugin_directory = plugin_root.join(name);
        std::fs::create_dir_all(&plugin_directory).unwrap();
        let manifest_path = workspace
            .join("providers")
            .join(provider_package)
            .join("codepet-provider.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
        manifest["executable"] =
            Value::String(executable(provider_package).to_string_lossy().into_owned());
        manifest["instances"][0]["settings"][executable_setting] =
            Value::String(fixture.to_string_lossy().into_owned());
        if name == "opencode" {
            manifest["instances"][0]["settings"]["serverVersion"] =
                Value::String("1.18.25".to_string());
        }
        std::fs::write(
            plugin_directory.join("codepet-provider.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    let device =
        DeviceRegistry::open(directory.path().join("device.json"), "CodePet Test").unwrap();
    let device_id = device.identity().device_id.clone();
    let catalog = PluginCatalog::discover(
        PluginCatalogConfig::default().with_directory(&plugin_root),
    );
    assert!(catalog.diagnostics().is_empty());
    let instances = ProviderInstanceRegistry::open(
        directory.path().join("provider-instances.json"),
        device_id.clone(),
    )
    .unwrap();
    let manager = Arc::new(
        PluginManager::new(
            device,
            catalog,
            instances,
            PluginManagerConfig {
                process: PluginProcessOptions {
                    request_timeout: Duration::from_secs(10),
                    shutdown_timeout: Duration::from_secs(5),
                    ..PluginProcessOptions::default()
                },
                ..PluginManagerConfig::default()
            },
        )
        .unwrap(),
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    assert!(gateway.start_event_forwarding());

    let outcomes = manager.start_enabled().await;
    assert_eq!(outcomes.len(), 3);
    for (plugin_id, outcome) in outcomes {
        if let Err(error) = outcome {
            let snapshot = manager.snapshot(&plugin_id).await.unwrap();
            panic!(
                "CodePet failed to start bundled Provider {plugin_id}: {error:?}; stderr={:?}",
                snapshot.stderr_diagnostics
            );
        }
        let snapshot = manager.snapshot(&plugin_id).await.unwrap();
        assert_eq!(snapshot.state, PluginRuntimeState::Ready);
        assert_eq!(snapshot.instances.len(), 1);
    }

    let listed = ProtocolServer::provider_list(
        gateway.as_ref(),
        ProviderListRequest { device_id: None },
    )
    .await
    .unwrap();
    assert_eq!(listed.providers.len(), 3);
    let codex = ready_provider(&listed.providers, CODEX_PLUGIN_ID, &device_id);
    let claude = ready_provider(&listed.providers, CLAUDE_PLUGIN_ID, &device_id);
    let opencode = ready_provider(&listed.providers, OPENCODE_PLUGIN_ID, &device_id);

    let codex_conversations = conversation_list(gateway.as_ref(), codex).await;
    assert_eq!(codex_conversations.len(), 1);
    assert_eq!(
        codex_conversations[0].resource.native_resource_id,
        "thread-listed"
    );

    let opencode_conversations = conversation_list(gateway.as_ref(), opencode).await;
    assert_eq!(opencode_conversations.len(), 1);
    assert_eq!(
        opencode_conversations[0].resource.native_resource_id,
        "ses_fixture"
    );

    let claude_conversation = ProtocolServer::conversation_create(
        gateway.as_ref(),
        ConversationCreateRequest {
            route: claude.route.clone(),
            title: Some("CodePet Claude integration".to_string()),
            permission_level: "workspace-write".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some(directory.path().to_string_lossy().into_owned()),
        },
    )
    .await
    .unwrap()
    .conversation;
    assert_eq!(
        claude_conversation.resource.provider_plugin_id,
        CLAUDE_PLUGIN_ID
    );
    assert!(!claude_conversation.resource.native_resource_id.is_empty());

    let shutdown = manager.shutdown().await;
    assert_eq!(shutdown.len(), 3);
    for (plugin_id, outcome) in shutdown {
        let snapshot = manager.snapshot(&plugin_id).await.unwrap();
        assert!(
            outcome.is_ok(),
            "Provider {plugin_id} must stop cleanly: {outcome:?}; exit={:?}; stderr={:?}",
            snapshot.process_exit,
            snapshot.stderr_diagnostics
        );
    }
}

fn ready_provider<'a>(
    providers: &'a [ProviderInstance],
    plugin_id: &str,
    device_id: &str,
) -> &'a ProviderInstance {
    let provider = providers
        .iter()
        .find(|provider| provider.plugin_id == plugin_id)
        .unwrap_or_else(|| panic!("Provider {plugin_id} must be visible through the Gateway"));
    assert_eq!(provider.status, ProviderStatus::Ready);
    assert_eq!(provider.route.device_id, device_id);
    provider
}

async fn conversation_list(
    gateway: &ProviderGatewayService,
    provider: &ProviderInstance,
) -> Vec<codepet_gateway_sdk::Conversation> {
    ProtocolServer::conversation_list(
        gateway,
        ConversationListRequest {
            route: Some(GatewayProviderRoute {
                device_id: provider.route.device_id.clone(),
                provider_plugin_id: provider.route.provider_plugin_id.clone(),
                provider_instance_id: provider.route.provider_instance_id.clone(),
            }),
            cursor: None,
            limit: Some(10),
        },
    )
    .await
    .unwrap()
    .conversations
}

fn crates_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("codepet-host is inside the crates workspace")
        .to_path_buf()
}

fn build_builtin_provider_fixtures(workspace: &Path) {
    let status = Command::new(cargo_executable())
        .arg("build")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .args([
            "-p",
            "codepet-provider-codex",
            "-p",
            "codepet-provider-claude",
            "-p",
            "codepet-provider-opencode",
            "--bins",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "built-in Provider fixtures must compile");
}

fn cargo_executable() -> PathBuf {
    let configured = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"));
    if configured.is_file() {
        return configured;
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(&configured))
        .find(|candidate| candidate.is_file())
        .expect("cargo executable must be available for built-in Provider integration tests")
}
