#[test]
fn provider_crate_has_no_pet_desktop_or_host_dependency() {
    let cargo = include_str!("../Cargo.toml");
    let sources = [
        include_str!("../src/client.rs"),
        include_str!("../src/lib.rs"),
        include_str!("../src/main.rs"),
        include_str!("../src/mapper.rs"),
        include_str!("../src/protocol.rs"),
        include_str!("../src/provider.rs"),
    ]
    .join("\n");
    for forbidden in [
        "codepet-host",
        "codepet-pet",
        "tauri",
        "PetEvent",
        "activity_store",
        "companion",
        "desktop_ipc",
    ] {
        assert!(
            !cargo.contains(forbidden) && !sources.contains(forbidden),
            "OpenCode Provider crossed the isolation boundary through {forbidden}"
        );
    }
    assert!(cargo.contains("codepet-provider-sdk"));
}
