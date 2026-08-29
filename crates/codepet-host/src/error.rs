use codepet_provider_sdk::ProtocolError;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

pub type HostResult<T> = Result<T, HostError>;

#[derive(Clone, Debug, PartialEq)]
pub struct HostError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: Option<BTreeMap<String, Value>>,
}

impl HostError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            details: None,
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.details
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

    pub fn into_protocol_error(self) -> ProtocolError {
        ProtocolError {
            code: self.code,
            message: self.message,
            retryable: self.retryable,
            details: self.details,
        }
    }
}

impl Display for HostError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HostError {}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        Self::new("host_io_error", error.to_string()).retryable(true)
    }
}

impl From<serde_json::Error> for HostError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("host_json_error", error.to_string())
    }
}

impl From<ProtocolError> for HostError {
    fn from(error: ProtocolError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            details: error.details,
        }
    }
}

impl From<HostError> for ProtocolError {
    fn from(error: HostError) -> Self {
        error.into_protocol_error()
    }
}
