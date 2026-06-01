#[test]
fn app_setup_creates_a_system_tray_icon() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();

    assert!(lib_rs.contains("TrayIconBuilder"));
    assert!(lib_rs.contains("default_window_icon"));
    assert!(lib_rs.contains("install_tray_icon(&handle)"));
}
