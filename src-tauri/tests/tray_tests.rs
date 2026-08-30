#[test]
fn app_setup_creates_a_system_tray_icon() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();

    assert!(lib_rs.contains("TrayIconBuilder"));
    assert!(lib_rs.contains("default_window_icon"));
    assert!(lib_rs.contains("install_tray_icon(&handle)"));
}

#[test]
fn tray_and_run_events_share_the_bounded_provider_shutdown_path() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();

    assert!(lib_rs.contains("request_app_exit(app, 0)"));
    assert!(lib_rs.contains("RunEvent::ExitRequested"));
    assert!(lib_rs.contains("state::<ProviderHostState>()"));
    assert!(lib_rs.contains("state::<RemoteAccessRuntime>()"));
    assert!(lib_rs.contains("remote_access.shutdown_once().await"));
    assert!(lib_rs.contains("shutdown_once().await"));
}

#[test]
fn remote_access_commands_are_registered_in_the_tauri_invoke_handler() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();
    let invoke_handler = lib_rs
        .split(".invoke_handler(tauri::generate_handler![")
        .nth(1)
        .and_then(|source| source.split("])").next())
        .expect("Tauri invoke handler declaration");

    for command in [
        "remote_access_status",
        "retry_remote_access",
        "list_remote_clients",
        "start_remote_pairing",
        "get_remote_pairing_status",
        "cancel_remote_pairing",
        "revoke_remote_credential",
    ] {
        assert!(
            invoke_handler.contains(command),
            "missing Tauri command {command}"
        );
    }
}
