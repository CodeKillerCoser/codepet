#[test]
fn startup_does_not_block_on_codex_audit_replay() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();

    let setup_finished = lib_rs
        .find("setup_span.finish_ok")
        .expect("startup setup should record its total span");
    let codex_replay = lib_rs
        .find("codex_audit::replay_default_codex_audit_events")
        .expect("startup should still replay Codex audit history");

    assert!(
        codex_replay > setup_finished,
        "Codex audit replay scans transcript files and must run after startup setup finishes"
    );
}
