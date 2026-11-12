use crate::TransportError;

pub use crate::protocol::{JSONRPC_PROFILE, ZSTD_PROFILE};

/// Profiles are complete contracts, not independently toggled wire features.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    JsonRpc,
    Zstd,
    Mux,
}
impl Profile {
    pub fn parse(value: &str) -> Result<Self, TransportError> {
        match value {
            JSONRPC_PROFILE => Ok(Self::JsonRpc),
            ZSTD_PROFILE => Ok(Self::Zstd),
            crate::MUX_PROFILE => Ok(Self::Mux),
            _ => Err(TransportError::new(format!(
                "unsupported transport profile: {value}"
            ))),
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JsonRpc => JSONRPC_PROFILE,
            Self::Zstd => ZSTD_PROFILE,
            Self::Mux => crate::MUX_PROFILE,
        }
    }
}
