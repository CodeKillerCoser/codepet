//! CodePet device identity, out-of-process Provider host, and Gateway v1 boundary.

pub use codepet_gateway_sdk as gateway_sdk;
pub use codepet_lan_channel_sdk::PairingExchangeRequest;
pub use codepet_provider_sdk as provider_sdk;

mod catalog;
mod device;
mod error;
mod gateway;
mod instance_registry;
mod manager;
mod persistence;
mod process;
mod remote_access;
mod remote_mdns;
mod remote_listener;
mod remote_network;

pub use catalog::{
    CatalogDiagnostic, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
    PluginInstanceConfig, PLUGIN_MANIFEST_FILE_NAME,
};
pub use device::{DeviceDiagnostic, DeviceIdentity, DeviceRegistry};
pub use error::{HostError, HostResult};
pub use gateway::{GatewayEventSubscription, ProviderGatewayService};
pub use instance_registry::{ProviderInstanceRecord, ProviderInstanceRegistry};
pub use manager::{
    PluginManager, PluginManagerConfig, PluginRuntimeSnapshot, PluginRuntimeState,
    ProviderInstanceRuntimeSnapshot,
};
pub use process::{
    PluginProcess, PluginProcessExit, PluginProcessOptions, ProviderRpcClient, StderrDiagnostic,
};
pub use remote_access::{
    IssuedRemoteCredential, LanTlsIdentity, PairingSession, PairingStatus,
    PairingStatusKind, PairingWatchState, RemoteAccessConfig, RemoteAccessDiagnostic,
    RemoteAccessManager, RemoteClientIdentity, RemoteCredential, RemoteCredentialStore,
    PAIRING_SESSION_TTL,
};
pub use remote_mdns::{RemoteLanMdnsAdvertiser, REMOTE_LAN_MDNS_SERVICE_TYPE};
pub use remote_listener::{
    RemoteLanAdvertisedEndpoint, RemoteLanAdvertisedEndpointTransition,
    RemoteLanAdvertisementSource, RemoteLanServer, RemoteLanServerConfig,
    RemoteLanServerHandle,
};
pub use remote_network::select_remote_lan_ipv4;
