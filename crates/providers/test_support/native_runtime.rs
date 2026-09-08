use codepet_provider_sdk::*;
use serde_json::json;

// Opt-in: probes installed binaries without sending prompts or changing user settings.
pub async fn check_native_runtime(
    provider: impl Provider,
    plugin: &str,
    kind: &str,
    mut settings: JsonObject,
) {
    let data = tempfile::tempdir().unwrap();
    let directory = data.path().join("data with spaces 中文");
    std::fs::create_dir_all(&directory).unwrap();
    settings.insert("dataDirectory".into(), json!(directory));
    if kind == "opencode" {
        let workspace = data.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        settings.insert("workspaceRoot".into(), json!(workspace));
    }
    provider
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "runtime-smoke".into(),
            host_device_id: "runtime-smoke-device".into(),
            host_version: "test".into(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        })
        .await
        .unwrap();
    let result: Result<(), ProtocolError> = async {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
        let installed = loop {
            let inventory = tokio::time::timeout(std::time::Duration::from_secs(1), provider.runtime_get_installed(RuntimeGetInstalledRequest { refresh: None })).await.expect("inventory RPC blocked")?;
            if inventory.scanning != Some(true) { break inventory; }
            assert!(tokio::time::Instant::now() < deadline, "scan did not complete");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };
        assert!(!installed.installed.is_empty(), "No {kind} runtime found");
        for runtime in &installed.installed {
            println!(
                "{kind}: {:?} {} ({}) minimum={:?} rejected={:?}",
                runtime.source, runtime.executable_path, runtime.version, runtime.minimum_version, runtime.incompatibility_reason
            );
        }
        let compatible = installed.installed.iter().find(|runtime| runtime.incompatibility_reason.is_none()).expect("no compatible installation");
        let selected = provider
            .runtime_select(RuntimeSelectRequest {
                candidate: RuntimeCandidate {
                    executable_path: compatible.executable_path.clone(),
                    source: RuntimeCandidateSource::Configured,
                },
            })
            .await?
            .selected;
        let reread = provider
            .runtime_get_installed(RuntimeGetInstalledRequest { refresh: None })
            .await?;
        assert_eq!(
            reread.selected.as_ref().map(|r| &r.executable_path),
            Some(&selected.executable_path)
        );
        if std::env::var_os("CODEPET_NATIVE_INVENTORY_ONLY").is_some() { return Ok(()); }
        let route = ProviderInstanceRoute {
            device_id: "runtime-smoke-device".into(),
            provider_plugin_id: plugin.into(),
            provider_instance_id: "native-runtime".into(),
        };
        provider
            .instance_create(InstanceCreateRequest {
                route: route.clone(),
                instance_kind: kind.into(),
                display_name: "Native runtime smoke".into(),
                settings,
            })
            .await?;
        let timer = std::time::Instant::now();
        let started = tokio::time::timeout(std::time::Duration::from_secs(10), provider.instance_start(InstanceStartRequest {route:route.clone()})).await.expect("handshake exceeded Host startup budget")?;
        println!("{kind}: handshake {:?}", timer.elapsed());
        assert!(matches!(started.instance.status, InstanceStatus::Starting | InstanceStatus::Ready));
        tokio::time::timeout(std::time::Duration::from_secs(90), async {
            loop {
                let snapshot = provider.instance_start(InstanceStartRequest { route: route.clone() }).await.unwrap();
                if snapshot.instance.status == InstanceStatus::Ready { break; }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }).await.expect("startup metadata did not complete");
        println!("{kind}: ready with isolated data directory");
        provider
            .conversation_list(ConversationListRequest {
                query: None, reader_scope: None,
                route: route.clone(),
                cursor: None,
                limit: Some(10),
                project_filter: serde_json::from_value(json!({"kind":"all"})).unwrap(),
            })
            .await?;
        provider
            .instance_stop(InstanceStopRequest { route })
            .await?;
        Ok(())
    }
    .await;
    let shutdown = provider.provider_shutdown(ProviderShutdownRequest {}).await;
    assert!(result.is_ok(), "{kind} native runtime: {result:?}");
    assert!(shutdown.is_ok(), "{kind} shutdown: {shutdown:?}");
}
