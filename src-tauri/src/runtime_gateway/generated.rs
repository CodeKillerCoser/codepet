//! Compatibility import surface for the existing in-process Runtime Gateway.
//!
//! The wire DTOs and dispatcher are generated into `codepet-gateway-sdk` from
//! the language-neutral v0 compatibility profile under `protocol/gateway/v1`.
//! New remote transports should use the SDK's v1 root instead of this module.

pub use codepet_gateway_sdk::compat_v0::*;
