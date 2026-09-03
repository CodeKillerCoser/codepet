#[test]
fn startup_does_not_run_background_transcript_scans() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();

    assert!(!lib_rs.contains("watch_default_codex_audit"));
    assert!(!lib_rs.contains("replay_default_codex_audit_events"));
    assert!(!lib_rs.contains("token_usage::refresh_default_usage_summary"));
}
