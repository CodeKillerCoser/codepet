//! CodePet device identity, out-of-process Provider host, and Gateway v1 boundary.

pub use codepet_gateway_sdk as gateway_sdk;
pub use codepet_lan_channel_sdk::PairingExchangeRequest;
pub use codepet_provider_sdk as provider_sdk;

mod conversation_state;
mod device;
mod error;
mod gateway;
mod providers;
mod remote;
mod persistence;

pub use providers::catalog::{
    CatalogDiagnostic, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
    PluginInstanceConfig, PLUGIN_MANIFEST_FILE_NAME,
};
pub use device::{DeviceDiagnostic, DeviceIdentity, DeviceRegistry};
pub use error::{HostError, HostResult};
pub use gateway::{GatewayEventSubscription, ProviderGatewayService, RemoteHostIdentity};
pub use providers::instance_registry::{ProviderInstanceRecord, ProviderInstanceRegistry};
pub use providers::manager::{
    PluginManager, PluginManagerConfig, PluginRuntimeSnapshot, PluginRuntimeState,
    ProviderInstanceRuntimeSnapshot,
};
pub use providers::process::{
    PluginProcess, PluginProcessExit, PluginProcessOptions, ProviderRpcClient, StderrDiagnostic,
};
pub use remote::access::{
    IssuedRemoteCredential, LanTlsIdentity, PairingSession, PairingStatus,
    PairingStatusKind, PairingWatchState, RemoteAccessConfig, RemoteAccessDiagnostic,
    RemoteAccessManager, RemoteClientIdentity, RemoteCredential, RemoteCredentialStore,
    RemotePairingRequest, PAIRING_REQUEST_TTL, PAIRING_SESSION_TTL,
};
pub use remote::channels::lan::mdns::{RemoteLanMdnsAdvertiser, REMOTE_LAN_MDNS_SERVICE_TYPE};
pub use remote::channels::lan::listener::{
    RemoteLanAdvertisedEndpoint, RemoteLanAdvertisedEndpointTransition,
    RemoteLanAdvertisementSource, RemoteLanServer, RemoteLanServerConfig,
    RemoteLanServerHandle,
};
pub use remote::channels::lan::network::select_remote_lan_ipv4;

pub use remote::connections::RemoteConnections;
