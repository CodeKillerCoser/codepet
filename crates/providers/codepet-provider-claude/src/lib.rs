mod client;
mod protocol;
mod provider;

pub use client::{ClaudeCliError, ClaudeProcessControl, ClaudeTurnLaunch};
pub use protocol::{decode_claude_output, ClaudeOutput};
pub use codepet_provider_sdk::ProviderEventSink;
pub use provider::{ClaudeProvider, CLAUDE_INSTANCE_KIND, CLAUDE_PLUGIN_ID};
