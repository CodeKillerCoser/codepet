//! Process serving: transport selection, Provider dispatch, events and lifecycle.
mod options;
mod events;
mod stdio;
mod mux;
mod io;
mod heartbeat;

pub use options::{StdioServerOptions, StdioServerError};
pub use events::ProviderEventSink;
pub use stdio::{serve_stdio, serve_stdio_with_io};
pub use mux::serve_mux_with_io;
pub use heartbeat::{run_provider_heartbeats, PROVIDER_HEARTBEAT_INTERVAL, PROVIDER_HEARTBEAT_TIMEOUT};
