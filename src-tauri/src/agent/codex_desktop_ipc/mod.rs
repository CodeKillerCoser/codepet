mod client;
mod mapper;
mod protocol;
mod provider;
mod state;
mod transport;

pub use provider::{
    CodexDesktopCompanionSnapshot,
    CodexProviderAdapter as CodexDesktopCompanionAdapter,
};
