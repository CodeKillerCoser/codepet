use std::fmt::{Display, Formatter};

/// Transport failures carry no RPC code, request ID or Provider response policy.
#[derive(Debug)]
pub struct TransportError {
    pub message: String,
}

impl TransportError {
    pub(crate) fn new(message: impl ToString) -> Self { Self { message: message.to_string() } }
}

impl Display for TransportError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { f.write_str(&self.message) }
}

impl std::error::Error for TransportError {}
