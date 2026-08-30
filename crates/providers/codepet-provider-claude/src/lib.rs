mod client;
mod protocol;
mod provider;

pub use client::{ClaudeCliError, ClaudeProcessControl, ClaudeTurnLaunch};
pub use protocol::{decode_claude_output, ClaudeOutput};
pub use provider::{ClaudeProvider, ProviderEventSink, CLAUDE_INSTANCE_KIND, CLAUDE_PLUGIN_ID};
