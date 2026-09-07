//! CodePet Host ↔ out-of-process Provider SDK: generated contract and handwritten runtime.

mod generated;
mod transport;
mod message;
mod runtime;
mod content;
pub mod local_runtime;
pub mod process;
pub mod background_probe;

pub use generated::*;
pub use generated::ProtocolServer as Provider;
pub use transport::{default_transport_limits, MUX_PROFILE, TRANSPORT_ENV};
pub use transport::frame::{
    ProviderFrameEncoding, ProviderFrameHeader, PROVIDER_FRAME_MAGIC, PROVIDER_FRAME_VERSION,
    PROVIDER_FRAME_HEADER_BYTES, MAX_PROVIDER_FRAME_BYTES, PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES,
};
pub use message::{ProviderFrameCodec, ProviderFrameEncodeMetrics, EncodedProviderFrame,
    ProviderMux, MuxIncoming, MuxMessage, MuxDriver};
pub use runtime::{serve_stdio, serve_stdio_with_io, serve_mux_with_io, ProviderEventSink,
    StdioServerError, StdioServerOptions, run_provider_heartbeats,
    PROVIDER_HEARTBEAT_INTERVAL, PROVIDER_HEARTBEAT_TIMEOUT};
pub use content::{truncate_tool_item_text, DEFAULT_TOOL_TEXT_BYTES};
pub use codepet_core_sdk::{ClientConnectionInfo, ConnectionStatus};

pub mod conversation_state;
pub mod conversation_query;
