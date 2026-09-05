//! Generated CodePet Host ↔ out-of-process Provider plugin SDK.

mod generated;
mod frame;
mod stdio;
mod heartbeat;
mod item_text;
pub use item_text::{truncate_tool_item_text, DEFAULT_TOOL_TEXT_BYTES};
pub use heartbeat::{run_provider_heartbeats, PROVIDER_HEARTBEAT_INTERVAL, PROVIDER_HEARTBEAT_TIMEOUT};

pub use generated::*;
pub use frame::*;
pub use generated::ProtocolServer as Provider;
pub use stdio::{
    serve_stdio, serve_stdio_with_io, ProviderEventSink, StdioServerError,
    StdioServerOptions,
};

pub use codepet_core_sdk::{ClientConnectionInfo, ConnectionStatus};
