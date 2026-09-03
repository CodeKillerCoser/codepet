use codepet_provider_claude::ClaudeProvider;
use codepet_provider_sdk::{
    serve_stdio, StdioServerOptions, MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let options = StdioServerOptions {
        max_frame_bytes: MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES,
        ..StdioServerOptions::default()
    };
    if let Err(error) = serve_stdio(options, ClaudeProvider::new).await {
        eprintln!("Claude Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}
