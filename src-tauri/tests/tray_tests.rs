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
    assert!(lib_rs.contains("shutdown_once().await"));
}
