#[test]
fn release_windows_binary_uses_gui_subsystem() {
    let main_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();

    assert!(
        main_rs.contains(r#"cfg_attr(not(debug_assertions), windows_subsystem = "windows")"#),
        "Windows release builds should not allocate a console window"
    );
}
