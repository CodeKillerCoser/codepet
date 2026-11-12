use codepet_gateway_sdk::{
    ConversationCreateRequest, ConversationListRequest, ProtocolServer,
    ProviderListRequest, ProviderStatus, ProviderSummary,
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
const OPENCODE_PLUGIN_ID: &str = "dev.codepet.opencode";

#[tokio::test]
async fn gateway_idle_restart_keeps_provider_visible_and_ready_without_phone() {
    let workspace = crates_workspace();
    let target = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    assert!(Command::new(cargo_executable()).args(["build", "--quiet", "-p", "codepet-provider-codex", "--bins"])
        .arg("--manifest-path").arg(workspace.join("Cargo.toml")).arg("--target-dir").arg(&target).status().unwrap().success());
    let executable = |name: &str| target.join("debug").join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("plugins/codex"); std::fs::create_dir_all(&root).unwrap();
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(workspace.join("providers/codepet-provider-codex/codepet-provider.json")).unwrap()).unwrap();
    manifest["executable"] = serde_json::json!(executable("codepet-provider-codex"));
    manifest["instances"][0]["settings"]["appServerExecutable"] = serde_json::json!(executable("codex-app-server-fixture"));
    manifest["instances"][0]["settings"]["appServerArgs"] = serde_json::json!(["--approval-mode", "none"]);
    manifest["instances"][0]["settings"]["dataDirectory"] = serde_json::json!(directory.path().join("codex-home"));
    std::fs::write(root.join("codepet-provider.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let device = DeviceRegistry::open(directory.path().join("device.json"), "release test").unwrap();
    let registry = ProviderInstanceRegistry::open(directory.path().join("instances.json"), device.identity().device_id.clone()).unwrap();
    let catalog = PluginCatalog::discover(PluginCatalogConfig::default().with_directory(root.parent().unwrap()));
    let manager = Arc::new(PluginManager::new(device, catalog, registry, PluginManagerConfig { release_control_idle_timeout: Duration::from_secs(2), ..PluginManagerConfig::default() }).unwrap());
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap()); gateway.start_event_forwarding();
    manager.enable_connection_heartbeats();
    // No phone is required to start, idle-release, or serve a later local read.
    assert!(manager.start_enabled().await.iter().all(|(_, result)| result.is_ok()));
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    select_fixture_runtime(&manager, CODEX_PLUGIN_ID, &executable("codex-app-server-fixture")).await;
    let listed = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    let id = listed.providers[0].id.clone();
    let described = gateway.provider_describe(codepet_gateway_sdk::ProviderDescribeRequest { provider_id:id.clone() }).await.unwrap();
    assert!(described.capabilities.methods.contains(&codepet_gateway_sdk::GatewayCapability::ConversationReleaseInteraction));
    let conversation = codepet_gateway_sdk::RoutedResourceId { provider_id:id.clone(), native_resource_id:"thread-listed".into() };
    wait_for_idle_restart(&gateway).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let read = gateway.conversation_get(codepet_gateway_sdk::ConversationGetRequest {
            conversation:conversation.clone(), cursor:None, limit:Some(20)
        }).await;
        if let Ok(read) = read { assert_eq!(read.conversation.resource, conversation); break; }
        assert!(tokio::time::Instant::now() < deadline, "history must work after automatic restart");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_for_idle_restart(&gateway).await;
    let resumed = gateway.conversation_resume(codepet_gateway_sdk::ConversationResumeRequest {
        conversation, force:None, limit:Some(20)
    }).await.unwrap();
    assert!(resumed.interaction_acquired, "{:?}", resumed.interaction_error);
    assert!(resumed.history.is_some(), "{:?}", resumed.history_error);
    assert!(manager.remote_connections().snapshot().connections.is_empty());
    manager.shutdown().await;
}

#[tokio::test]
async fn gateway_release_interaction_restarts_codex_and_keeps_history_available() {
    let workspace = crates_workspace();
    let target = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    assert!(Command::new(cargo_executable()).args(["build", "--quiet", "-p", "codepet-provider-codex", "--bins"])
        .arg("--manifest-path").arg(workspace.join("Cargo.toml")).arg("--target-dir").arg(&target).status().unwrap().success());
    let executable = |name: &str| target.join("debug").join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("plugins/codex"); std::fs::create_dir_all(&root).unwrap();
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(workspace.join("providers/codepet-provider-codex/codepet-provider.json")).unwrap()).unwrap();
    manifest["executable"] = serde_json::json!(executable("codepet-provider-codex"));
    manifest["instances"][0]["settings"]["appServerExecutable"] = serde_json::json!(executable("codex-app-server-fixture"));
    manifest["instances"][0]["settings"]["appServerArgs"] = serde_json::json!(["--approval-mode", "none"]);
    manifest["instances"][0]["settings"]["dataDirectory"] = serde_json::json!(directory.path().join("codex-home"));
    std::fs::write(root.join("codepet-provider.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let device = DeviceRegistry::open(directory.path().join("device.json"), "release test").unwrap();
    let registry = ProviderInstanceRegistry::open(directory.path().join("instances.json"), device.identity().device_id.clone()).unwrap();
    let catalog = PluginCatalog::discover(PluginCatalogConfig::default().with_directory(root.parent().unwrap()));
    let manager = Arc::new(PluginManager::new(device, catalog, registry, PluginManagerConfig::default()).unwrap());
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap()); gateway.start_event_forwarding();
    manager.enable_connection_heartbeats();
    let _phone = manager.remote_connections().register("release-phone".into());
    assert!(manager.start_enabled().await.iter().all(|(_, result)| result.is_ok()));
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    select_fixture_runtime(&manager, CODEX_PLUGIN_ID, &executable("codex-app-server-fixture")).await;
    let listed = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    let id = listed.providers[0].id.clone();
    let described = gateway.provider_describe(codepet_gateway_sdk::ProviderDescribeRequest { provider_id:id.clone() }).await.unwrap();
    assert!(described.capabilities.methods.contains(&codepet_gateway_sdk::GatewayCapability::ConversationReleaseInteraction));
    let conversation = codepet_gateway_sdk::RoutedResourceId { provider_id:id.clone(), native_resource_id:"thread-listed".into() };
    assert!(gateway.conversation_resume(codepet_gateway_sdk::ConversationResumeRequest { conversation:conversation.clone(), force:None, limit:Some(20) }).await.unwrap().interaction_acquired);
    let release = gateway.conversation_release_interaction(codepet_gateway_sdk::ConversationReleaseInteractionRequest { conversation:conversation.clone() }).await.unwrap();
    assert!(release.released); assert_eq!(release.scope, "providerInstance");
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    assert!(gateway.conversation_get(codepet_gateway_sdk::ConversationGetRequest { conversation:conversation.clone(), cursor:None, limit:Some(20) }).await.is_ok());
    let restarted = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    assert_eq!(restarted.providers[0].runtime.status, ProviderStatus::Ready);
    let created = gateway.conversation_create(ConversationCreateRequest {
        provider_id: id.clone(), project: None, title: None,
        permission_level: "workspace-write".into(), model: Some("gpt-fixture".into()),
        reasoning_effort: Some("high".into()), workspace_root: Some(directory.path().to_string_lossy().into_owned()),
        workspace_mode: None,
    }).await.unwrap();
    assert_eq!(created.conversation.resource.provider_id, id, "a fresh conversation must work without resuming a released one");
    let resumed = gateway.conversation_resume(codepet_gateway_sdk::ConversationResumeRequest { conversation, force:None, limit:Some(20) }).await.unwrap();
    assert!(resumed.interaction_acquired, "{:?}", resumed.interaction_error);
    assert!(resumed.history.is_some(), "{:?}", resumed.history_error);
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    manager.shutdown().await;
}

#[tokio::test]
async fn codepet_host_runs_all_sdk_based_builtin_providers_end_to_end() {
    exercise_builtins(false).await;
}

#[tokio::test]
async fn host_recycles_only_capable_codex_while_claude_and_opencode_stay_ready() {
    exercise_builtins(true).await;
}

async fn exercise_builtins(recycle_only: bool) {
    let workspace = crates_workspace();
    build_builtin_provider_fixtures(&workspace);
    // Match Cargo's actual target directory, including CARGO_TARGET_DIR overrides.
    let target = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    let executable = |name: &str| {
        target.join("debug").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
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
        if name == "codex" {
            let home = directory.path().join("codex-home");
            std::fs::create_dir_all(&home).unwrap();
            manifest["instances"][0]["settings"]["dataDirectory"] = Value::String(home.to_string_lossy().into_owned());
        }
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
                release_control_idle_timeout: Duration::from_secs(if recycle_only { 2 } else { 300 }),
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

    manager.enable_connection_heartbeats();
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

    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    for (plugin, fixture) in [
        (CODEX_PLUGIN_ID, "codex-app-server-fixture"),
        ("dev.codepet.claude", "claude-stream-fixture"),
        (OPENCODE_PLUGIN_ID, "opencode-server-fixture"),
    ] { select_fixture_runtime(&manager, plugin, &executable(fixture)).await; }
    let first_phone = manager.remote_connections().register("phone-a".into());
    let second_phone = manager.remote_connections().register("phone-b".into());
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    let listed = ProtocolServer::provider_list(
        gateway.as_ref(),
        ProviderListRequest {},
    )
    .await
    .unwrap();
    assert_eq!(listed.providers.len(), 3, "listed={:?}; inventory={:?}", listed.providers.iter().map(|p| &p.id).collect::<Vec<_>>(), manager.snapshots().await.iter().map(|p| (&p.catalog.plugin_id, &p.runtime_inventory)).collect::<Vec<_>>());
    let codex = ready_provider(&listed.providers, "codex", &device_id);
    let claude = ready_provider(&listed.providers, "claude", &device_id);
    let opencode = ready_provider(&listed.providers, "opencode", &device_id);
    if recycle_only {
        for snapshot in manager.snapshots().await {
            let supports = snapshot.instances[0].instance.as_ref().unwrap().capabilities.methods
                .contains(&codepet_provider_sdk::ProviderCapability::InstanceReleaseControl);
            assert_eq!(supports, snapshot.catalog.plugin_id == CODEX_PLUGIN_ID);
        }
        let cursor = gateway.current_event_cursor();
        let mut events = gateway.subscribe_events(Some(&cursor)).unwrap();
        tokio::time::timeout(Duration::from_secs(12), async {
            let mut cycles = 0;
            let mut stopped = false;
            while cycles < 2 {
                if let codepet_gateway_sdk::ProtocolEvent::ProviderChanged { params, .. } = events.next_event().await.unwrap() {
                    let changed = params.payload.provider;
                    if changed.id != codex.id {
                        assert_eq!(changed.runtime.status, ProviderStatus::Ready, "non-participating provider {} changed state", changed.id);
                    } else if changed.runtime.status == ProviderStatus::Stopped { stopped = true; }
                    else if stopped && changed.runtime.status == ProviderStatus::Ready { cycles += 1; stopped = false; }
                }
            }
        }).await.expect("Host must recycle Codex independently of the other builtins");
        for snapshot in manager.snapshots().await {
            assert_eq!(snapshot.state, PluginRuntimeState::Ready);
            assert!(snapshot.instances.iter().all(|i| i.instance.as_ref().unwrap().status == codepet_provider_sdk::InstanceStatus::Ready));
        }
        assert!(manager.shutdown().await.iter().all(|(_, result)| result.is_ok()));
        return;
    }
    assert_eq!(
        codex.identity.icon.as_deref(),
        Some("https://avatars.githubusercontent.com/u/14957082?s=200&v=4")
    );
    assert!(codex
        .identity
        .default_workspace_root
        .as_deref()
        .is_some_and(|path| Path::new(path).ends_with(Path::new(".codepet").join("remote_workspace").join("codex"))));
    assert_eq!(
        claude.identity.icon.as_deref(),
        Some("https://cdn.prod.website-files.com/6889473510b50328dbb70ae6/68c33859cc6cd903686c66a2_apple-touch-icon.png")
    );
    assert!(claude
        .identity
        .default_workspace_root
        .as_deref()
        .is_some_and(|path| Path::new(path).ends_with(Path::new(".codepet").join("remote_workspace").join("claude"))));
    assert_eq!(
        opencode.identity.icon.as_deref(),
        Some("https://opencode.ai/favicon-96x96-v3.png")
    );
    assert!(opencode
        .identity
        .default_workspace_root
        .as_deref()
        .is_some_and(|path| Path::new(path).ends_with(Path::new(".codepet").join("remote_workspace").join("opencode"))));

    let codex_conversations = conversation_list(gateway.as_ref(), codex, &device_id, CODEX_PLUGIN_ID).await;
    assert_eq!(codex_conversations.len(), 1);
    assert_eq!(
        codex_conversations[0].resource.native_resource_id,
        "thread-listed"
    );

    let opencode_conversations =
        conversation_list(gateway.as_ref(), opencode, &device_id, OPENCODE_PLUGIN_ID).await;
    assert_eq!(opencode_conversations.len(), 1);
    assert_eq!(
        opencode_conversations[0].resource.native_resource_id,
        "ses_fixture"
    );
    let history = ProtocolServer::conversation_get(gateway.as_ref(), codepet_gateway_sdk::ConversationGetRequest {
        conversation:opencode_conversations[0].resource.clone(), cursor:None, limit:Some(100),
    }).await.unwrap();
    assert_eq!(history.items.len(), 5); // Includes two tool results; Host must accept both identities.
    // Ready precedes discovery. Verify the notified catalog reaches Gateway.
    let description = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let description = ProtocolServer::provider_describe(gateway.as_ref(), codepet_gateway_sdk::ProviderDescribeRequest {provider_id:opencode.id.clone()}).await.unwrap();
            if description.capabilities.conversation_create.as_ref().and_then(|c| c.selection.as_ref()).is_some_and(|c| c.model_catalog.is_some()) {break description;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("background model catalog did not reach Gateway");
    let controls = description.capabilities.conversation_create.unwrap().selection.unwrap();
    assert_eq!(controls.access_mode.unwrap().options.len(), 2);
    assert!(controls.model_catalog.is_some());

    let claude_conversation = ProtocolServer::conversation_create(
        gateway.as_ref(),
        ConversationCreateRequest {
            provider_id: claude.id.clone(),
            project: None,
            title: Some("CodePet Claude integration".to_string()),
            permission_level: "workspace-write".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some(directory.path().to_string_lossy().into_owned()),
            workspace_mode: None,
        },
    )
    .await
    .unwrap()
    .conversation;
    assert_eq!(claude_conversation.resource.provider_id, claude.id);
    assert!(!claude_conversation.resource.native_resource_id.is_empty());

    drop(first_phone);
    tokio::time::sleep(Duration::from_millis(100)).await;
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    drop(second_phone);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    for snapshot in manager.snapshots().await {
        assert_eq!(snapshot.state, PluginRuntimeState::Ready);
        assert_eq!(snapshot.connection_status, codepet_provider_sdk::ConnectionStatus::Online);
    }
    let reconnected = manager.remote_connections().register("phone-b".into());
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
    drop(reconnected);
    wait_for_instances(&manager, codepet_provider_sdk::InstanceStatus::Ready).await;
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

async fn wait_for_instances(manager: &PluginManager, status: codepet_provider_sdk::InstanceStatus) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if manager.snapshots().await.iter().all(|p|
                p.connection_status == codepet_provider_sdk::ConnectionStatus::Online &&
                p.instances.iter().all(|i| i.instance.as_ref().is_some_and(|i| i.status == status))) { return; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap_or_else(|_| panic!("instances did not reconcile to {status:?}"));
}

fn ready_provider<'a>(
    providers: &'a [ProviderSummary],
    name: &str,
    _device_id: &str,
) -> &'a ProviderSummary {
    let provider = providers
        .iter()
        .find(|provider| provider.id.contains(name))
        .unwrap_or_else(|| panic!("Provider {name} must be visible through the Gateway"));
    assert_eq!(provider.runtime.status, ProviderStatus::Ready);
    provider
}

async fn conversation_list(
    gateway: &ProviderGatewayService,
    provider: &ProviderSummary,
    _device_id: &str,
    _plugin_id: &str,
) -> Vec<codepet_gateway_sdk::Conversation> {
    ProtocolServer::conversation_list(
        gateway,
        ConversationListRequest {
            provider_id: provider.id.clone(),
            cursor: None,
            limit: Some(10),
            project_filter: codepet_gateway_sdk::ConversationProjectFilter::ConversationProjectFilterAll(
                codepet_gateway_sdk::ConversationProjectFilterAll {
                    kind: codepet_gateway_sdk::ConversationProjectFilterAllKind::All,
                },
            ),
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

async fn wait_for_idle_restart(gateway: &ProviderGatewayService) {
    let cursor = gateway.current_event_cursor();
    let mut events = gateway.subscribe_events(Some(&cursor)).unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut stopped = false;
        loop {
            if let codepet_gateway_sdk::ProtocolEvent::ProviderChanged { params, .. } = events.next_event().await.unwrap() {
                let status = params.payload.provider.runtime.status;
                if status == ProviderStatus::Stopped { stopped = true; }
                if stopped && status == ProviderStatus::Ready { break; }
            }
        }
    }).await.expect("idle runtime must publish Stopped then Ready without any start request");
    let listed = gateway.provider_list(ProviderListRequest {}).await.unwrap();
    assert_eq!(listed.providers.len(), 1);
    assert_eq!(listed.providers[0].runtime.status, ProviderStatus::Ready);
}

async fn select_fixture_runtime(manager: &PluginManager, plugin: &str, path: &Path) {
    // A configured Harness alone is not a detected runtime. Exercise the actual
    // scanner/selection boundary, independent of what is installed on this Mac.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let inventory = manager.runtime_get_installed(plugin, codepet_provider_sdk::RuntimeGetInstalledRequest { refresh: None }).await.unwrap();
        if inventory.scanning != Some(true) { break; }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let path = std::fs::canonicalize(path).unwrap().to_string_lossy().into_owned();
    manager.runtime_select(plugin, codepet_provider_sdk::RuntimeSelectRequest { candidate: codepet_provider_sdk::RuntimeCandidate {
        executable_path:path.clone(), source:codepet_provider_sdk::RuntimeCandidateSource::Configured,
    }}).await.unwrap();
    loop {
        let snapshot = manager.snapshot(plugin).await.unwrap();
        if snapshot.runtime_inventory.as_ref().is_some_and(|i| i.selected.as_ref().is_some_and(|r| r.executable_path == path && !r.version.is_empty())) { break; }
        assert!(tokio::time::Instant::now() < deadline, "fixture inventory was not reported: {:?}", snapshot.runtime_inventory);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
