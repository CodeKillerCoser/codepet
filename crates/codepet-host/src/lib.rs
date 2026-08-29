//! CodePet device identity, out-of-process Provider host, and Gateway v1 boundary.

pub use codepet_gateway_sdk as gateway_sdk;
pub use codepet_provider_sdk as provider_sdk;

mod catalog;
mod device;
mod error;
mod gateway;
mod instance_registry;
mod manager;
mod persistence;
mod process;

pub use catalog::{
    CatalogDiagnostic, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
    PluginInstanceConfig, DEFAULT_PROVIDER_BINARY_NAMES, PLUGIN_MANIFEST_FILE_NAME,
};
pub use device::{DeviceDiagnostic, DeviceIdentity, DeviceRegistry};
pub use error::{HostError, HostResult};
pub use gateway::{event_cursor_sequence, GatewayEventSubscription, ProviderGatewayService};
pub use instance_registry::{ProviderInstanceRecord, ProviderInstanceRegistry};
pub use manager::{
    ManagerEvent, PluginManager, PluginManagerConfig, PluginRuntimeSnapshot,
    PluginRuntimeState, ProviderInstanceRuntimeSnapshot,
};
pub use process::{
    PluginProcess, PluginProcessDiagnostics, PluginProcessExit, PluginProcessOptions,
    ProviderRpcClient, StderrDiagnostic,
};
