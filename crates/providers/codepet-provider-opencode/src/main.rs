use codepet_provider_opencode::OpenCodeProvider;
use codepet_provider_sdk::{
    serve_stdio, StdioServerOptions, MAX_PROVIDER_FRAME_BYTES,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let options = StdioServerOptions {
        max_frame_bytes: MAX_PROVIDER_FRAME_BYTES,
        ..StdioServerOptions::default()
    };
    if let Err(error) = serve_stdio(options, OpenCodeProvider::new).await {
        eprintln!("OpenCode Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}
