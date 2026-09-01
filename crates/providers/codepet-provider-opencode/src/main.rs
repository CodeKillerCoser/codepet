use codepet_provider_opencode::OpenCodeProvider;
use codepet_provider_sdk::{serve_stdio, StdioServerOptions};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = serve_stdio(StdioServerOptions::default(), OpenCodeProvider::new).await {
        eprintln!("OpenCode Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}
