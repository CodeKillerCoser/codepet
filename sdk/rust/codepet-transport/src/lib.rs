//! Independent byte transport. No Host, Provider methods or business handlers.
mod generated {
    pub use codepet_core_sdk::{
        ProviderTransportHello, ProviderTransportLimits, ProviderTransportSelection,
    };
}
pub mod protocol;
pub mod transport;
pub use transport::error::TransportError;
pub use transport::mux::{Class, Connection, Driver, Incoming, MessageCodec, Received};
pub use transport::{default_transport_limits, MUX_PROFILE, TRANSPORT_ENV};
pub mod payload;
pub mod profile;
pub mod serial;
pub use profile::Profile;
#[cfg(test)]
mod tests;
