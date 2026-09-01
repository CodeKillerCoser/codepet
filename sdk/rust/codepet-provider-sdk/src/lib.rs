//! Generated CodePet Host ↔ out-of-process Provider plugin SDK.

mod generated;
mod stdio;

pub use generated::*;
pub use generated::ProtocolServer as Provider;
pub use stdio::{
    serve_stdio, serve_stdio_with_io, ProviderEventSink, StdioServerError,
    StdioServerOptions,
};

/// Bounded transport limit for Provider methods that return complete conversation histories.
/// Both the Provider process and Host reader must opt into this limit explicitly.
pub const MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES: usize = 16 * 1024 * 1024;
