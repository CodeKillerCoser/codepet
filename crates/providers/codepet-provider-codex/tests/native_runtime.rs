#[path = "../../test_support/native_runtime.rs"]
mod support;

#[tokio::test]
#[ignore = "requires installed Codex; uses isolated data and no model requests"]
async fn installed_codex_runtime_discovers_selects_and_starts() {
    support::check_native_runtime(
        codepet_provider_codex::CodexProvider::new(std::sync::Arc::new(|_| Ok(()))),
        "dev.codepet.codex",
        "codex",
        [(
            "appServerArgs".into(),
            serde_json::json!(["app-server", "--listen", "stdio://"]),
        )]
        .into(),
    )
    .await;
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Codex installed in a Windows application layout"]
async fn codex_windows_install_layout_without_path() {
    use codepet_provider_sdk::*;
    if std::env::var_os("CODEPET_LAYOUT_PROBE_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "codex_windows_install_layout_without_path",
                "--nocapture",
            ])
            .env("CODEPET_LAYOUT_PROBE_CHILD", "1")
            .env("PATH", "")
            .env_remove("CODE_PET_CODEX_BIN")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    let provider = codepet_provider_codex::CodexProvider::new(std::sync::Arc::new(|_| Ok(())));
    provider.provider_initialize(ProviderInitializeRequest { directories: None,
        host_client_id: "layout-smoke".into(), host_device_id: "layout-device".into(), host_version: "test".into(),
        protocol: ProtocolVersionOffer { min_version: PROTOCOL_VERSION, version: PROTOCOL_VERSION },
    }).await.unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
    let installed = loop {
        let inventory = provider.runtime_get_installed(RuntimeGetInstalledRequest { refresh: None }).await.unwrap();
        if inventory.scanning != Some(true) { break inventory; }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    provider.provider_shutdown(ProviderShutdownRequest {}).await.unwrap();
    assert!(installed
        .harness_list
        .iter()
        .any(|runtime| runtime.source == RuntimeCandidateSource::WindowsApplication));
    for runtime in installed.harness_list {
        println!(
            "Windows layout: {} ({})",
            runtime.executable_path, runtime.version
        );
    }
}

#[tokio::test]
#[ignore = "requires CODE_PET_CODEX_BIN and existing native projects; no model requests"]
async fn installed_codex_projects_and_chat_capabilities() {
    use codepet_provider_sdk::*;
    use serde_json::json;
    let executable = std::env::var("CODE_PET_CODEX_BIN").expect("explicit installed Codex path");
    let provider = codepet_provider_codex::CodexProvider::new(std::sync::Arc::new(|_| Ok(())));
    let route = ProviderInstanceRoute {
        device_id: "native-project-check".into(),
        provider_plugin_id: "dev.codepet.codex".into(),
        provider_instance_id: "codex".into(),
    };
    provider
        .provider_initialize(ProviderInitializeRequest {
            directories: None,
            host_client_id: "native-project-check".into(),
            host_device_id: route.device_id.clone(),
            host_version: "test".into(),
            protocol: ProtocolVersionOffer {
                version: PROTOCOL_VERSION,
                min_version: MIN_PROTOCOL_VERSION,
            },
        })
        .await
        .unwrap();
    let result: Result<(), ProtocolError> = async {
        provider.instance_create(InstanceCreateRequest {
            route: route.clone(), instance_kind: "codex".into(), display_name: "Native read-only compatibility check".into(),
            settings: [("appServerExecutable".into(), json!(executable)), ("appServerArgs".into(), json!(["app-server","--listen","stdio://"]))].into(),
        }).await?;
        provider.instance_start(InstanceStartRequest { route: route.clone() }).await?;
        let ready = tokio::time::timeout(std::time::Duration::from_secs(45), async {
            loop {
                let instance = provider.instance_start(InstanceStartRequest { route: route.clone() }).await?.instance;
                if instance.status == InstanceStatus::Ready { return Ok::<_, ProtocolError>(instance); }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.expect("metadata deadline")?;
        assert!(ready.capabilities.methods.contains(&ProviderCapability::TurnStart), "turn.start missing");
        assert!(ready.capabilities.turn_send.is_some(), "turn options missing");
        assert!(ready.capabilities.methods.contains(&ProviderCapability::ProjectList), "project.list missing");
        let first = provider.project_list(ProjectListRequest { route: route.clone(), cursor: None, limit: Some(1) }).await?;
        assert_eq!(first.projects.len(), 1, "this probe requires at least one existing native project to exercise its fields");
        println!("native Codex {:?}: project.list + turn.start + turnSend ready; nonempty first page decoded", ready.harness.version);
        if let Some(project) = first.projects.first() {
            let resource = ProviderResourceId { device_id: route.device_id.clone(), provider_plugin_id: route.provider_plugin_id.clone(), provider_instance_id: route.provider_instance_id.clone(), native_resource_id: project.resource.native_resource_id.clone() };
            let fetched = provider.project_get(ProjectGetRequest { project: resource }).await?;
            assert_eq!(fetched.project.position, project.position);
        }
        if let Some(cursor) = first.page_info.next_cursor {
            let second = provider.project_list(ProjectListRequest { route: route.clone(), cursor: Some(cursor), limit: Some(1) }).await?;
            assert_eq!(second.projects.len(), 1);
            assert!(second.projects[0].position > first.projects[0].position);
            println!("native continuation order preserved");
        }
        Ok(())
    }.await;
    let shutdown = provider.provider_shutdown(ProviderShutdownRequest {}).await;
    assert!(result.is_ok(), "native compatibility: {result:?}");
    assert!(shutdown.is_ok(), "shutdown: {shutdown:?}");
}
