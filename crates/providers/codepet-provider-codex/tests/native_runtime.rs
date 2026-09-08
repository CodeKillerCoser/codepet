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
        supported_versions: VersionRange { min_version: PROTOCOL_VERSION, max_version: PROTOCOL_VERSION },
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
        .installed
        .iter()
        .any(|runtime| runtime.source == RuntimeCandidateSource::WindowsApplication));
    for runtime in installed.installed {
        println!(
            "Windows layout: {} ({})",
            runtime.executable_path, runtime.version
        );
    }
}
