//! Provider wire adapters shared by Host and Provider runtimes.
pub(crate) mod codec;
mod frame;
mod mux;

pub use frame::{ProviderFrameCodec, ProviderFrameEncodeMetrics, EncodedProviderFrame};
pub use mux::{ProviderMux, MuxIncoming, MuxMessage, MuxDriver};
use crate::generated::ProtocolError;

pub(crate) fn error(message: impl ToString) -> ProtocolError {
    ProtocolError { code: "provider_mux_error".into(), message: message.to_string(), retryable: false, details: None }
}
