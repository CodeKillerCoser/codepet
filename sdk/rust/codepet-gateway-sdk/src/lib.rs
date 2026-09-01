//! Generated CodePet Host ↔ Remote Client gateway SDK.

mod generated;

pub use generated::*;

/// Frozen Gateway v1 CodePet-envelope compatibility surface.
pub mod v1;

/// Temporary v0 wire surface used by the existing in-process Runtime Gateway.
/// It is generated from the compatibility profile under `protocol/gateway/v1`.
pub mod compat_v0;
