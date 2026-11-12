//! Provider wire adapters shared by Host and Provider runtimes.
pub(crate) mod codec;
mod frame;
mod mux;

use crate::generated::ProtocolError;
pub use frame::{EncodedProviderFrame, ProviderFrameCodec, ProviderFrameEncodeMetrics};
pub use mux::{MuxDriver, MuxIncoming, MuxMessage, ProviderMux};

pub(crate) fn error(message: impl ToString) -> ProtocolError {
    ProtocolError { code: "provider_mux_error".into(), message: message.to_string(), retryable: false, details: None }
}

pub use codec::ProviderMessageCodec;
pub(crate) use mux::ResponseFallback;
