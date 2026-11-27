//! Test-only stdio bridge for the Remote's cross-language PTY test.
//! Production continues to use the authenticated LAN/RTC channel adapters.
use std::io::{BufRead, Write};
mod terminal_host;

#[tokio::main]
async fn main() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = terminal_host::start(directory.path());
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let request = serde_json::from_str(&line).unwrap();
        let response = gateway
            .dispatch_for_caller_scope("test-phone", request)
            .await;
        println!("{}", serde_json::to_string(&response).unwrap());
        std::io::stdout().flush().unwrap();
    }
}
