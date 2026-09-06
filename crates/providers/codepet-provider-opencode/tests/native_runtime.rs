#[path = "../../test_support/native_runtime.rs"]
mod support;

#[tokio::test]
#[ignore = "requires installed OpenCode; uses isolated data and no model requests"]
async fn installed_opencode_runtime_discovers_selects_and_starts() {
    support::check_native_runtime(
        codepet_provider_opencode::OpenCodeProvider::new(std::sync::Arc::new(|_| Ok(()))),
        "dev.codepet.opencode",
        "opencode",
        [("serverArgs".into(), serde_json::json!(["serve"]))].into(),
    )
    .await;
}
