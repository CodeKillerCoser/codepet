//! Generated CodePet Host ↔ out-of-process Provider plugin SDK.

mod generated;
mod frame;
mod stdio;

pub use generated::*;
pub use frame::*;
pub use generated::ProtocolServer as Provider;
pub use stdio::{
    serve_stdio, serve_stdio_with_io, ProviderEventSink, StdioServerError,
    StdioServerOptions,
};
