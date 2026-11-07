use codepet_provider_codex::CodexProvider;
use codepet_provider_sdk::{
    serve_stdio, StdioServerOptions, MAX_PROVIDER_FRAME_BYTES,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let options = StdioServerOptions {
        max_frame_bytes: MAX_PROVIDER_FRAME_BYTES,
        restart_after_idle: true,
        ..StdioServerOptions::default()
    };
    let mut options = options;
    if let Some(value) = std::env::var_os("CODEPET_PROVIDER_IDLE_TIMEOUT_SECONDS") {
        let seconds = value.to_str().and_then(|value| value.parse::<u64>().ok()).filter(|value| *value > 0);
        let Some(seconds) = seconds else {
            eprintln!("CODEPET_PROVIDER_IDLE_TIMEOUT_SECONDS must be a positive integer");
            std::process::exit(1);
        };
        options.idle_timeout = std::time::Duration::from_secs(seconds);
    }
    if let Err(error) = serve_stdio(options, CodexProvider::new).await {
        eprintln!("Codex Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}
