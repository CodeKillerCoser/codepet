use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager, PluginManagerConfig,
    ProviderGatewayService, ProviderInstanceRegistry,
};
use std::sync::Arc;

pub fn start(path: &std::path::Path) -> Arc<ProviderGatewayService> {
    let device = DeviceRegistry::open(path.join("device.json"), "Terminal test").unwrap();
    let instances = ProviderInstanceRegistry::open(
        path.join("instances.json"),
        device.identity().device_id.clone(),
    )
    .unwrap();
    let catalog = PluginCatalog::discover(
        PluginCatalogConfig::default().with_directory(path.join("empty-plugins")),
    );
    let manager = Arc::new(
        PluginManager::new(device, catalog, instances, PluginManagerConfig::default()).unwrap(),
    );
    Arc::new(ProviderGatewayService::new(manager).unwrap())
}
