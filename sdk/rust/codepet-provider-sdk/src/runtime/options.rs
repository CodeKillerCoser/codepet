use crate::generated::ProtocolError;
use crate::transport::frame::MAX_PROVIDER_FRAME_BYTES;
use std::{fmt::{Display, Formatter}, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdioServerOptions {
    pub max_frame_bytes: usize,
    pub max_concurrent_requests: usize,
    pub max_pending_requests: usize,
    pub max_concurrent_control_requests: usize,
    pub max_pending_control_requests: usize,
    pub dispatch_drain_timeout: Duration,
}

impl Default for StdioServerOptions {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_PROVIDER_FRAME_BYTES,
            max_concurrent_requests: 16,
            max_pending_requests: 32,
            max_concurrent_control_requests: 2,
            max_pending_control_requests: 4,
            dispatch_drain_timeout: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdioServerError {
    message: String,
}

impl StdioServerError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for StdioServerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StdioServerError {}

impl From<ProtocolError> for StdioServerError {
    fn from(error: ProtocolError) -> Self {
        Self::new(error.message)
    }
}

pub(crate) fn validate_options(options: StdioServerOptions) -> Result<(), StdioServerError> {
    for (name, value) in [
        ("max_frame_bytes", options.max_frame_bytes),
        ("max_concurrent_requests", options.max_concurrent_requests),
        ("max_pending_requests", options.max_pending_requests),
        (
            "max_concurrent_control_requests",
            options.max_concurrent_control_requests,
        ),
        (
            "max_pending_control_requests",
            options.max_pending_control_requests,
        ),
    ] {
        if value == 0 {
            return Err(StdioServerError::new(format!(
                "Provider stdio option {name} must be greater than zero"
            )));
        }
    }
    if options.dispatch_drain_timeout.is_zero() {
        return Err(StdioServerError::new(
            "Provider stdio dispatch_drain_timeout must be greater than zero",
        ));
    }
    Ok(())
}
