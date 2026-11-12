//! Generated, language-neutral CodePet core protocol types.

mod generated;

pub use generated::*;

/// Mechanism-level failure shared by language-neutral transport implementations.
#[derive(Debug)]
pub struct TransportError {
    pub message: String,
}
impl TransportError {
    pub fn new(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TransportError {}
impl From<TransportError> for ProtocolError {
    fn from(value: TransportError) -> Self {
        // Keep the existing published Provider error code while moving its mechanism.
        Self {
            code: "provider_mux_error".into(),
            message: value.message,
            retryable: false,
            details: None,
        }
    }
}
