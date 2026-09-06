#[path = "../../test_support/native_runtime.rs"]
mod support;

#[tokio::test]
#[ignore = "requires installed Claude Code; uses isolated data and no model requests"]
async fn installed_claude_runtime_discovers_selects_and_starts() {
    support::check_native_runtime(
        codepet_provider_claude::ClaudeProvider::new(std::sync::Arc::new(|_| Ok(()))),
        "dev.codepet.claude",
        "claude",
        Default::default(),
    )
    .await;
}
