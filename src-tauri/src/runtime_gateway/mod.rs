//! Generated Code Pet Standard Protocol boundary.

pub mod event_bus;
pub mod gateway;
pub mod generated;
pub mod provider;
pub mod provider_host_compat;
pub mod remote_access;
pub mod registry;
pub mod tauri_bridge;
pub mod transport;

pub use event_bus::{event_sequence, EventSubscription, ProviderEventSink};
pub use gateway::Gateway;
pub use provider::{ProviderAdapter, ProviderFuture};
pub use registry::ProviderRegistry;
pub use remote_access::RemoteAccessRuntime;
pub use tauri_bridge::{
    CodexDesktopCompanionState, RuntimeGatewayState,
};
pub use transport::{LocalTransport, Transport, TransportFuture};
