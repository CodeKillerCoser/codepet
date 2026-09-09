mod client;
mod desktop_takeover;
mod directory;
mod mapper;
mod protocol;
mod user_message;
mod provider;

pub use codepet_provider_sdk::ProviderEventSink;
pub use provider::{CodexProvider, ExecutionLifecycleHook};
pub use protocol::{CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID};
