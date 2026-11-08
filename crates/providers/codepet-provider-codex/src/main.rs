use codepet_provider_codex::CodexProvider;
use codepet_provider_sdk::{
    serve_stdio, StdioServerOptions, MAX_PROVIDER_FRAME_BYTES,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let options = StdioServerOptions {
        max_frame_bytes: MAX_PROVIDER_FRAME_BYTES,
        ..StdioServerOptions::default()
    };
    if let Err(error) = serve_stdio(options, CodexProvider::new).await {
        eprintln!("Codex Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}
