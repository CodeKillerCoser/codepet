use super::gateway::Gateway;
use super::generated::{
    EventSequence, ProtocolError, ProtocolEvent, ProtocolRequest, ProtocolResponse,
};
use super::provider_host_compat::CompatProviderGateway;
use super::transport::{LocalTransport, Transport};
use crate::agent::codex_desktop_ipc::{
    CodexDesktopCompanionAdapter, CodexDesktopCompanionSnapshot,
};
use crate::agent_runtime::{
    AgentRuntime, AgentRuntimeService, CLAUDE_RUNTIME_PROVIDER_ID,
    CODEX_RUNTIME_PROVIDER_ID, OPENCODE_RUNTIME_PROVIDER_ID,
};
use crate::platform::host_identity::computer_name;
use crate::settings::{configured_app_data_dir, load_app_settings};
use codepet_lan_channel_sdk::DeviceDescriptor;
use codepet_host::{
    DeviceRegistry, HostError, PluginCatalog, PluginCatalogConfig, PluginManager,
    PluginManagerConfig, ProviderGatewayService, ProviderInstanceRegistry,
    RemoteAccessConfig, RemoteAccessManager,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::{Mutex as AsyncMutex, Notify};

pub const RUNTIME_GATEWAY_EVENT: &str = "runtime-gateway-event";
pub const CODEX_DESKTOP_COMPANION_EVENT: &str = "codex-desktop-companion-event";
pub const BUNDLED_PROVIDER_PLUGINS_DIRECTORY_ENV: &str =
    "CODEPET_BUNDLED_PROVIDER_PLUGINS_DIR";

#[derive(Clone)]
pub struct RuntimeGatewayState {
    compat: CompatProviderGateway,
}

impl Default for RuntimeGatewayState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl RuntimeGatewayState {
    pub fn new(gateway: Option<Arc<ProviderGatewayService>>) -> Self {
        Self {
            compat: CompatProviderGateway::new(gateway),
        }
    }

    pub async fn request(&self, request: ProtocolRequest) -> ProtocolResponse {
        self.compat.request(request).await
    }

    pub fn replay(
        &self,
        after_event_sequence: Option<EventSequence>,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        self.compat.replay(after_event_sequence)
    }
}

#[derive(Clone)]
pub(crate) struct ProviderHostState {
    manager: Option<Arc<PluginManager>>,
    gateway: Option<Arc<ProviderGatewayService>>,
    started: Arc<AtomicBool>,
    codex_refresh_generation: Arc<AtomicU64>,
    codex_refresh_lock: Arc<AsyncMutex<()>>,
    claude_refresh_generation: Arc<AtomicU64>,
    claude_refresh_lock: Arc<AsyncMutex<()>>,
    opencode_refresh_generation: Arc<AtomicU64>,
    opencode_refresh_lock: Arc<AsyncMutex<()>>,
    shutdown_started: Arc<AtomicBool>,
    shutdown_completed: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

impl ProviderHostState {
    pub(crate) fn from_app<R: Runtime>(
        app: &AppHandle<R>,
    ) -> Result<(Self, Arc<RemoteAccessManager>), HostError> {
        let bundled_directory = bundled_provider_plugins_directory(app)?;
        let (manager, gateway, remote_access) =
            configured_provider_runtime(&bundled_directory)?;
        Ok((Self::new(manager, gateway), remote_access))
    }

    pub(crate) fn new(
        manager: Arc<PluginManager>,
        gateway: Arc<ProviderGatewayService>,
    ) -> Self {
        Self {
            manager: Some(manager),
            gateway: Some(gateway),
            started: Arc::new(AtomicBool::new(false)),
            codex_refresh_generation: Arc::new(AtomicU64::new(0)),
            codex_refresh_lock: Arc::new(AsyncMutex::new(())),
            claude_refresh_generation: Arc::new(AtomicU64::new(0)),
            claude_refresh_lock: Arc::new(AsyncMutex::new(())),
            opencode_refresh_generation: Arc::new(AtomicU64::new(0)),
            opencode_refresh_lock: Arc::new(AsyncMutex::new(())),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            manager: None,
            gateway: None,
            started: Arc::new(AtomicBool::new(false)),
            codex_refresh_generation: Arc::new(AtomicU64::new(0)),
            codex_refresh_lock: Arc::new(AsyncMutex::new(())),
            claude_refresh_generation: Arc::new(AtomicU64::new(0)),
            claude_refresh_lock: Arc::new(AsyncMutex::new(())),
            opencode_refresh_generation: Arc::new(AtomicU64::new(0)),
            opencode_refresh_lock: Arc::new(AsyncMutex::new(())),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn gateway(&self) -> Option<Arc<ProviderGatewayService>> {
        self.gateway.clone()
    }

    pub(crate) fn refresh_runtime_in_background(&self, runtime: AgentRuntime) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let Some(target) = provider_runtime_target(&runtime.provider_id) else {
            return;
        };
        let (generation_counter, refresh_lock) = match runtime.provider_id.as_str() {
            CODEX_RUNTIME_PROVIDER_ID => (
                self.codex_refresh_generation.clone(),
                self.codex_refresh_lock.clone(),
            ),
            CLAUDE_RUNTIME_PROVIDER_ID => (
                self.claude_refresh_generation.clone(),
                self.claude_refresh_lock.clone(),
            ),
            OPENCODE_RUNTIME_PROVIDER_ID => (
                self.opencode_refresh_generation.clone(),
                self.opencode_refresh_lock.clone(),
            ),
            _ => return,
        };
        let generation = generation_counter
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        tauri::async_runtime::spawn(async move {
            let _refresh = refresh_lock.lock().await;
            if generation_counter.load(Ordering::SeqCst) != generation {
                return;
            }
            let executable = runtime
                .resolved_executable
                .map(serde_json::Value::String);
            let version = runtime.version.map(serde_json::Value::String);
            let updated = async {
                let executable_updates = manager
                    .replace_instance_setting(
                        target.plugin_id,
                        target.instance_kind,
                        target.executable_setting,
                        executable,
                    )
                    .await?;
                let version_updates = match target.version_setting {
                    Some(version_setting) => {
                        manager
                            .replace_instance_setting(
                                target.plugin_id,
                                target.instance_kind,
                                version_setting,
                                version,
                            )
                            .await?
                    }
                    None => 0,
                };
                Ok::<_, codepet_host::HostError>(executable_updates.max(version_updates))
            }
            .await;
            if generation_counter.load(Ordering::SeqCst) != generation {
                return;
            }
            match updated {
                Ok(0) => {}
                Ok(_) => {
                    if let Err(error) = manager.restart_plugin(target.plugin_id).await {
                        crate::app_log::error(
                            "provider_host",
                            &format!(
                                "failed to refresh {} Provider plugin error={error:?}",
                                target.display_name
                            ),
                        );
                    }
                }
                Err(error) => crate::app_log::error(
                    "provider_host",
                    &format!(
                        "failed to update {} Provider instance settings error={error:?}",
                        target.display_name
                    ),
                ),
            }
        });
    }

    pub(crate) fn start_in_background(&self) {
        let (Some(manager), Some(gateway)) = (self.manager.clone(), self.gateway.clone()) else {
            return;
        };
        if self
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        for diagnostic in manager.catalog_diagnostics() {
            crate::app_log::error(
                "provider_host",
                &format!("{} path={:?}", diagnostic.message, diagnostic.path),
            );
        }
        spawn_provider_host_startup(manager, gateway);
    }

    pub(crate) fn shutdown_completed(&self) -> bool {
        self.shutdown_completed.load(Ordering::SeqCst)
    }

    pub(crate) async fn shutdown_once(&self) -> bool {
        if self
            .shutdown_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            loop {
                let notified = self.shutdown_notify.notified();
                if self.shutdown_completed() {
                    break;
                }
                notified.await;
            }
            return true;
        }

        if let Some(manager) = self.manager.as_ref() {
            let shutdown_timeout = manager.shutdown_timeout();
            let force_kill = match tokio::time::timeout(shutdown_timeout, manager.shutdown()).await {
                Ok(outcomes) => {
                    let mut failed = false;
                    for (plugin_id, outcome) in outcomes {
                        if let Err(error) = outcome {
                            failed = true;
                            crate::app_log::error(
                                "provider_host",
                                &format!(
                                    "Provider shutdown failed plugin_id={plugin_id} error={error:?}"
                                ),
                            );
                        }
                    }
                    failed
                }
                Err(_) => {
                    crate::app_log::error(
                        "provider_host",
                        "Provider Manager exceeded its bounded shutdown window",
                    );
                    true
                }
            };
            if force_kill {
                for (plugin_id, outcome) in manager
                    .force_kill_all("Tauri Provider Host shutdown timeout")
                    .await
                {
                    if let Err(error) = outcome {
                        crate::app_log::error(
                            "provider_host",
                            &format!(
                                "Provider force kill failed plugin_id={plugin_id} error={error:?}"
                            ),
                        );
                    }
                }
            }
        }

        self.shutdown_completed.store(true, Ordering::SeqCst);
        self.shutdown_notify.notify_waiters();
        true
    }
}

fn spawn_provider_host_startup(
    manager: Arc<PluginManager>,
    gateway: Arc<ProviderGatewayService>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        if !gateway.start_event_forwarding() {
            crate::app_log::error(
                "provider_host",
                "Provider Gateway event forwarding was already started or unavailable",
            );
        }
        for (plugin_id, outcome) in manager.start_enabled().await {
            match outcome {
                Ok(()) => crate::app_log::info(
                    "provider_host",
                    &format!("Provider plugin initialized plugin_id={plugin_id}"),
                ),
                Err(error) => {
                    let snapshot = manager.snapshot(&plugin_id).await.ok();
                    crate::app_log::error(
                        "provider_host",
                        &format!(
                            "Provider plugin initialization failed plugin_id={plugin_id} error={error:?} state={:?} stderr={:?}",
                            snapshot.as_ref().map(|snapshot| snapshot.state),
                            snapshot
                                .as_ref()
                                .and_then(|snapshot| snapshot.stderr_diagnostics.last())
                        ),
                    );
                }
            }
        }
    })
}

fn bundled_provider_plugins_directory<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<PathBuf, HostError> {
    if let Some(configured) = std::env::var_os(BUNDLED_PROVIDER_PLUGINS_DIRECTORY_ENV)
        .filter(|value| !value.is_empty())
    {
        let configured = PathBuf::from(configured);
        if !configured.is_absolute() {
            return Err(HostError::new(
                "bundled_provider_directory_not_absolute",
                format!(
                    "{} must be an absolute path, received {}",
                    BUNDLED_PROVIDER_PLUGINS_DIRECTORY_ENV,
                    configured.display()
                ),
            ));
        }
        return Ok(configured);
    }
    app.path()
        .resolve("provider-plugins", BaseDirectory::Resource)
        .map_err(|error| {
            HostError::new(
                "bundled_provider_directory_resolution_failed",
                format!("resolve bundled Provider plugin resource directory: {error}"),
            )
        })
}

fn provider_catalog_config(
    data_directory: &Path,
    bundled_provider_directory: &Path,
    additional_directories: &[String],
) -> PluginCatalogConfig {
    let mut catalog_config = PluginCatalogConfig::for_data_directory(data_directory)
        .with_directory(bundled_provider_directory);
    for directory in additional_directories {
        let directory = directory.trim();
        if !directory.is_empty() {
            let directory = PathBuf::from(directory);
            let directory = if directory.is_absolute() {
                directory
            } else {
                data_directory.join(directory)
            };
            catalog_config = catalog_config.with_directory(directory);
        }
    }
    catalog_config
}

fn provider_manager_config() -> PluginManagerConfig {
    let mut config = PluginManagerConfig::default();
    config.process.max_frame_bytes =
        codepet_host::provider_sdk::MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES;
    config
}

fn configured_provider_runtime(
    bundled_provider_directory: &Path,
) -> Result<
    (
        Arc<PluginManager>,
        Arc<ProviderGatewayService>,
        Arc<RemoteAccessManager>,
    ),
    HostError,
> {
    let settings = load_app_settings().map_err(HostError::from)?;
    let data_directory = configured_app_data_dir(&settings);
    let provider_host_directory = data_directory.join("provider-host");
    let device = Arc::new(DeviceRegistry::open(
        provider_host_directory.join("device-identity.json"),
        computer_name(),
    )?);
    for diagnostic in device.diagnostics() {
        crate::app_log::error(
            "provider_host",
            &format!(
                "{} path={} recovered_path={:?}",
                diagnostic.message,
                diagnostic.path.display(),
                diagnostic.recovered_path
            ),
        );
    }
    let catalog_config = provider_catalog_config(
        &data_directory,
        bundled_provider_directory,
        &settings.provider_plugins.directories,
    );
    let mut catalog = PluginCatalog::discover(catalog_config);
    let runtime_service = AgentRuntimeService::default();
    for provider_id in [
        CODEX_RUNTIME_PROVIDER_ID,
        CLAUDE_RUNTIME_PROVIDER_ID,
        OPENCODE_RUNTIME_PROVIDER_ID,
    ] {
        let target = provider_runtime_target(provider_id).expect("known runtime Provider target");
        let runtime = runtime_service.detect(provider_id).map_err(|error| {
            HostError::new(
                format!("{provider_id}_runtime_resolution_failed"),
                format!(
                    "failed to resolve {} executable: {error}",
                    target.display_name
                ),
            )
        })?;
        inject_runtime_executable(&mut catalog, &runtime)?;
    }
    let instances = ProviderInstanceRegistry::open(
        provider_host_directory.join("provider-instances.json"),
        device.identity().device_id.clone(),
    )?;
    let local_device_descriptor = local_device_descriptor(&device.identity().display_name);
    let remote_access = Arc::new(RemoteAccessManager::open(
        RemoteAccessConfig::for_data_directory(data_directory.join("remote-access")),
        device.clone(),
        local_device_descriptor,
    )?);
    let manager = Arc::new(PluginManager::with_device_registry(
        device,
        catalog,
        instances,
        provider_manager_config(),
    )?);
    let gateway = Arc::new(ProviderGatewayService::with_remote_identity(
        manager.clone(),
        {
            let identity = remote_access.remote_host_identity();
            codepet_gateway_sdk::GatewayHostIdentity {
                device_id: identity.device_id,
                descriptor: identity.descriptor,
            }
        },
    )?);
    Ok((manager, gateway, remote_access))
}

fn local_device_descriptor(device_name: &str) -> DeviceDescriptor {
    let info = os_info::get();
    let operating_system = match info.os_type() {
        os_info::Type::Unknown => std::env::consts::OS.to_string(),
        os_type => os_type.to_string(),
    };
    let system_version = match info.version() {
        os_info::Version::Unknown => "unknown".to_string(),
        version => version.to_string(),
    };
    DeviceDescriptor {
        device_name: device_name.to_string(),
        operating_system: non_empty_system_value(operating_system, std::env::consts::OS),
        system_version: non_empty_system_value(system_version, "unknown"),
    }
}

fn non_empty_system_value(value: String, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[derive(Clone, Copy)]
struct ProviderRuntimeTarget {
    plugin_id: &'static str,
    instance_kind: &'static str,
    executable_setting: &'static str,
    version_setting: Option<&'static str>,
    display_name: &'static str,
}

fn provider_runtime_target(provider_id: &str) -> Option<ProviderRuntimeTarget> {
    match provider_id {
        CODEX_RUNTIME_PROVIDER_ID => Some(ProviderRuntimeTarget {
            plugin_id: "dev.codepet.codex",
            instance_kind: "codex",
            executable_setting: "appServerExecutable",
            version_setting: None,
            display_name: "Codex",
        }),
        CLAUDE_RUNTIME_PROVIDER_ID => Some(ProviderRuntimeTarget {
            plugin_id: "dev.codepet.claude",
            instance_kind: "claude",
            executable_setting: "claudeExecutable",
            version_setting: None,
            display_name: "Claude",
        }),
        OPENCODE_RUNTIME_PROVIDER_ID => Some(ProviderRuntimeTarget {
            plugin_id: "dev.codepet.opencode",
            instance_kind: "opencode",
            executable_setting: "serverExecutable",
            version_setting: Some("serverVersion"),
            display_name: "OpenCode",
        }),
        _ => None,
    }
}

fn inject_runtime_executable(
    catalog: &mut PluginCatalog,
    runtime: &AgentRuntime,
) -> Result<usize, HostError> {
    let Some(target) = provider_runtime_target(&runtime.provider_id) else {
        return Ok(0);
    };
    catalog.update_instance_settings(
        target.plugin_id,
        target.instance_kind,
        |settings| {
            settings.remove(target.executable_setting);
            if let Some(executable) = runtime.resolved_executable.as_ref() {
                settings.insert(
                    target.executable_setting.to_string(),
                    serde_json::Value::String(executable.clone()),
                );
            }
            if let Some(version_setting) = target.version_setting {
                settings.remove(version_setting);
                if let Some(version) = runtime.version.as_ref() {
                    settings.insert(
                        version_setting.to_string(),
                        serde_json::Value::String(version.clone()),
                    );
                }
            }
        },
    )
}

#[derive(Clone)]
pub struct CodexDesktopCompanionState {
    gateway: Arc<Gateway>,
    transport: LocalTransport,
    codex_desktop: Option<Arc<CodexDesktopCompanionAdapter>>,
}

impl Default for CodexDesktopCompanionState {
    fn default() -> Self {
        let gateway = Arc::new(Gateway::default());
        let codex_desktop = Arc::new(CodexDesktopCompanionAdapter::spawn(gateway.event_sink()));
        if let Err(error) = gateway.registry().register(codex_desktop.clone()) {
            crate::app_log::error(
                "codex_desktop_companion",
                &format!("failed to register Codex Desktop IPC adapter error={error:?}"),
            );
        }
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex_desktop: Some(codex_desktop),
        }
    }
}

impl CodexDesktopCompanionState {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex_desktop: None,
        }
    }

    pub fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }

    pub fn transport(&self) -> &LocalTransport {
        &self.transport
    }

    pub fn snapshot(&self) -> Result<CodexDesktopCompanionSnapshot, ProtocolError> {
        self.codex_desktop
            .as_ref()
            .map(|adapter| adapter.snapshot())
            .ok_or_else(|| ProtocolError {
                code: "companion_state_error".to_string(),
                message: "Codex Desktop companion adapter is not registered".to_string(),
                retryable: true,
                details: None,
            })
    }
}

#[tauri::command]
pub async fn runtime_gateway_request(
    state: tauri::State<'_, RuntimeGatewayState>,
    request: ProtocolRequest,
) -> Result<ProtocolResponse, ProtocolError> {
    Ok(state.request(request).await)
}

#[tauri::command]
pub fn runtime_gateway_replay(
    state: tauri::State<'_, RuntimeGatewayState>,
    after_event_sequence: Option<EventSequence>,
) -> Result<Vec<ProtocolEvent>, ProtocolError> {
    state.replay(after_event_sequence)
}

#[tauri::command]
pub async fn codex_desktop_companion_request(
    state: tauri::State<'_, CodexDesktopCompanionState>,
    request: ProtocolRequest,
) -> Result<ProtocolResponse, ProtocolError> {
    let transport = state.transport().clone();
    Ok(transport.request(request).await)
}

#[tauri::command]
pub fn codex_desktop_companion_replay(
    state: tauri::State<'_, CodexDesktopCompanionState>,
    after_event_sequence: Option<EventSequence>,
) -> Result<Vec<ProtocolEvent>, ProtocolError> {
    state.transport().replay(after_event_sequence)
}

#[tauri::command]
pub fn codex_desktop_companion_snapshot(
    state: tauri::State<'_, CodexDesktopCompanionState>,
) -> Result<CodexDesktopCompanionSnapshot, ProtocolError> {
    state.snapshot()
}

pub fn start_runtime_gateway_event_bridge<R: Runtime>(
    app: AppHandle<R>,
    state: &RuntimeGatewayState,
) -> Result<(), ProtocolError> {
    let mut subscription = state.compat.subscribe_current()?;
    tauri::async_runtime::spawn(async move {
        loop {
            match subscription.next_event().await {
                Ok(event) => {
                    let _ = app.emit(RUNTIME_GATEWAY_EVENT, event);
                }
                Err(error) => {
                    crate::app_log::error(
                        "runtime_gateway",
                        &format!("Provider Gateway compat event bridge stopped error={error:?}"),
                    );
                    break;
                }
            }
        }
    });
    Ok(())
}

pub fn start_codex_desktop_companion_event_bridge<R: Runtime>(
    app: AppHandle<R>,
    state: &CodexDesktopCompanionState,
) -> Result<(), ProtocolError> {
    start_local_event_bridge(
        app,
        state.gateway(),
        state.transport(),
        CODEX_DESKTOP_COMPANION_EVENT,
        "codex_desktop_companion",
    )
}

fn start_local_event_bridge<R: Runtime>(
    app: AppHandle<R>,
    gateway: &Arc<Gateway>,
    transport: &LocalTransport,
    event_name: &'static str,
    log_target: &'static str,
) -> Result<(), ProtocolError> {
    let current_sequence = gateway.current_event_sequence();
    let mut subscription = transport.subscribe(Some(current_sequence))?;
    tauri::async_runtime::spawn(async move {
        loop {
            match subscription.next_event().await {
                Ok(event) => {
                    let _ = app.emit(event_name, event);
                }
                Err(error) => {
                    crate::app_log::error(
                        log_target,
                        &format!("local event bridge stopped error={error:?}"),
                    );
                    break;
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        inject_runtime_executable, local_device_descriptor, non_empty_system_value,
        provider_catalog_config, provider_manager_config, spawn_provider_host_startup,
        ProviderGatewayService, ProviderHostState,
    };
    use crate::agent_runtime::{
        AgentRuntime, AgentRuntimeSource, AgentRuntimeStatus, CLAUDE_RUNTIME_PROVIDER_ID,
        CODEX_RUNTIME_PROVIDER_ID, OPENCODE_RUNTIME_PROVIDER_ID,
    };
    use codepet_host::{
        DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
        PluginInstanceConfig, PluginManager, PluginManagerConfig,
        PluginRuntimeState, ProviderInstanceRegistry,
    };
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Barrier;

    #[test]
    fn local_device_descriptor_has_stable_name_and_non_empty_system_fields() {
        let descriptor = local_device_descriptor("Test Device");
        assert_eq!(descriptor.device_name, "Test Device");
        assert!(!descriptor.operating_system.trim().is_empty());
        assert!(!descriptor.system_version.trim().is_empty());
        assert_eq!(
            non_empty_system_value("Unknown".to_string(), "fallback"),
            "fallback"
        );
        assert_eq!(
            non_empty_system_value("  value  ".to_string(), "fallback"),
            "value"
        );
    }

    #[test]
    fn provider_host_accepts_bounded_complete_conversation_history_frames() {
        assert_eq!(
            provider_manager_config().process.max_frame_bytes,
            codepet_host::provider_sdk::MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES
        );
        assert!(
            provider_manager_config().process.max_frame_bytes
                > codepet_host::provider_sdk::DEFAULT_MAX_JSON_LINE_BYTES
        );
    }

    #[test]
    fn provider_host_startup_enters_tauri_runtime_before_gateway_forwarding() {
        let directory = tempfile::tempdir().unwrap();
        let device = DeviceRegistry::open(directory.path().join("device.json"), "Test Device")
            .unwrap();
        let instances = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            device.identity().device_id.clone(),
        )
        .unwrap();
        let manager = Arc::new(
            PluginManager::new(
                device,
                PluginCatalog::discover(PluginCatalogConfig::default()),
                instances,
                PluginManagerConfig::default(),
            )
            .unwrap(),
        );
        let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());

        let startup = spawn_provider_host_startup(manager, gateway);
        tauri::async_runtime::block_on(startup).unwrap();
    }

    #[test]
    fn catalog_discovers_all_three_providers_from_the_bundled_directory() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory = directory.path().join("data");
        let bundled_directory = directory.path().join("bundled/provider-plugins");
        for (name, plugin_id) in [
            ("codex", "dev.codepet.codex"),
            ("opencode", "dev.codepet.opencode"),
            ("claude", "dev.codepet.claude"),
        ] {
            let provider_directory = bundled_directory.join(name);
            std::fs::create_dir_all(&provider_directory).unwrap();
            std::fs::write(
                provider_directory.join("codepet-provider.json"),
                serde_json::to_vec(&serde_json::json!({
                    "manifestVersion": 1,
                    "pluginId": plugin_id,
                    "displayName": name,
                    "executable": format!("codepet-provider-{name}"),
                    "enabled": true,
                    "instances": [{
                        "instanceId": name,
                        "instanceKind": name,
                        "displayName": name,
                        "settings": {},
                        "enabled": true
                    }]
                }))
                .unwrap(),
            )
            .unwrap();
        }

        let catalog = PluginCatalog::discover(provider_catalog_config(
            &data_directory,
            &bundled_directory,
            &[],
        ));
        assert!(catalog.diagnostics().is_empty());
        let registry = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            "device-bundled-catalog".to_string(),
        )
        .unwrap();
        let mut plugin_ids = registry
            .synchronize_catalog(&catalog)
            .unwrap()
            .into_iter()
            .map(|record| record.plugin_id)
            .collect::<Vec<_>>();
        plugin_ids.sort();
        assert_eq!(
            plugin_ids,
            [
                "dev.codepet.claude",
                "dev.codepet.codex",
                "dev.codepet.opencode"
            ]
        );
    }

    #[test]
    fn three_catalog_runtime_executables_are_overridden_by_resolver_values() {
        let directory = tempfile::tempdir().unwrap();
        let plugin_directory = directory.path().join("providers");
        let descriptors = [
            PluginDescriptor {
                plugin_id: "dev.codepet.codex".to_string(),
                display_name: "Codex".to_string(),
                icon: Some("https://raw.githubusercontent.com/openai/codex/main/codex-rs/skills/src/assets/samples/openai-docs/assets/openai.png".to_string()),
                executable: directory.path().join("codepet-provider-codex"),
                args: Vec::new(),
                env: Default::default(),
                enabled: true,
                instances: vec![PluginInstanceConfig {
                    instance_id: Some("codex".to_string()),
                    instance_kind: "codex".to_string(),
                    display_name: "Codex".to_string(),
                    settings: [
                        (
                            "appServerExecutable".to_string(),
                            serde_json::json!("manifest-codex"),
                        ),
                        ("preserved".to_string(), serde_json::json!(true)),
                    ]
                    .into_iter()
                    .collect(),
                    enabled: true,
                }],
            },
            PluginDescriptor {
                plugin_id: "dev.codepet.opencode".to_string(),
                display_name: "OpenCode".to_string(),
                icon: Some("https://opencode.ai/favicon-96x96-v3.png".to_string()),
                executable: directory.path().join("codepet-provider-opencode"),
                args: Vec::new(),
                env: Default::default(),
                enabled: true,
                instances: vec![PluginInstanceConfig {
                    instance_id: Some("opencode".to_string()),
                    instance_kind: "opencode".to_string(),
                    display_name: "OpenCode".to_string(),
                    settings: [
                        (
                            "serverExecutable".to_string(),
                            serde_json::json!("manifest-opencode"),
                        ),
                        (
                            "serverVersion".to_string(),
                            serde_json::json!("manifest-version"),
                        ),
                        ("preserved".to_string(), serde_json::json!(true)),
                    ]
                    .into_iter()
                    .collect(),
                    enabled: true,
                }],
            },
            PluginDescriptor {
                plugin_id: "dev.codepet.claude".to_string(),
                display_name: "Claude".to_string(),
                icon: Some("https://claude.ai/favicon.ico".to_string()),
                executable: directory.path().join("codepet-provider-claude"),
                args: Vec::new(),
                env: Default::default(),
                enabled: true,
                instances: vec![PluginInstanceConfig {
                    instance_id: Some("claude".to_string()),
                    instance_kind: "claude".to_string(),
                    display_name: "Claude".to_string(),
                    settings: [
                        (
                            "claudeExecutable".to_string(),
                            serde_json::json!("manifest-claude"),
                        ),
                        ("preserved".to_string(), serde_json::json!(true)),
                    ]
                    .into_iter()
                    .collect(),
                    enabled: true,
                }],
            },
        ];
        for (index, descriptor) in descriptors.into_iter().enumerate() {
            let directory = plugin_directory.join(index.to_string());
            std::fs::create_dir_all(&directory).unwrap();
            let mut manifest = serde_json::to_value(descriptor).unwrap();
            manifest
                .as_object_mut()
                .unwrap()
                .insert("manifestVersion".to_string(), serde_json::json!(1));
            std::fs::write(
                directory.join("codepet-provider.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        let mut catalog = PluginCatalog::discover(
            PluginCatalogConfig::default().with_directory(&plugin_directory),
        );
        let codex_executable = directory.path().join("resolved/codex");
        let opencode_executable = directory.path().join("resolved/opencode");
        let claude_executable = directory.path().join("resolved/claude");

        for (provider_id, display_name, executable, version) in [
            (
                CODEX_RUNTIME_PROVIDER_ID,
                "Codex",
                codex_executable.to_string_lossy().to_string(),
                None,
            ),
            (
                OPENCODE_RUNTIME_PROVIDER_ID,
                "OpenCode",
                opencode_executable.to_string_lossy().to_string(),
                Some("1.18.25"),
            ),
            (
                CLAUDE_RUNTIME_PROVIDER_ID,
                "Claude Code",
                claude_executable.to_string_lossy().to_string(),
                Some("2.1.251"),
            ),
        ] {
            let runtime = AgentRuntime {
                provider_id: provider_id.to_string(),
                display_name: display_name.to_string(),
                status: AgentRuntimeStatus::Ready,
                resolved_executable: Some(executable.clone()),
                source: Some(AgentRuntimeSource::Configured),
                configured_executable: Some(executable),
                version: version.map(str::to_string),
                diagnostic: None,
            };
            assert_eq!(inject_runtime_executable(&mut catalog, &runtime).unwrap(), 1);
        }

        let instances = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            "device-runtime-settings".to_string(),
        )
        .unwrap();
        let records = instances.synchronize_catalog(&catalog).unwrap();
        let codex = records
            .iter()
            .find(|record| record.plugin_id == "dev.codepet.codex")
            .unwrap();
        let opencode = records
            .iter()
            .find(|record| record.plugin_id == "dev.codepet.opencode")
            .unwrap();
        let claude = records
            .iter()
            .find(|record| record.plugin_id == "dev.codepet.claude")
            .unwrap();
        assert_eq!(
            codex.settings["appServerExecutable"],
            serde_json::json!(codex_executable)
        );
        assert_eq!(
            opencode.settings["serverExecutable"],
            serde_json::json!(opencode_executable)
        );
        assert_eq!(opencode.settings["serverVersion"], serde_json::json!("1.18.25"));
        assert_eq!(
            claude.settings["claudeExecutable"],
            serde_json::json!(claude_executable)
        );
        assert_eq!(codex.settings["preserved"], serde_json::json!(true));
        assert_eq!(opencode.settings["preserved"], serde_json::json!(true));
        assert_eq!(claude.settings["preserved"], serde_json::json!(true));

        for (provider_id, display_name) in [
            (CODEX_RUNTIME_PROVIDER_ID, "Codex"),
            (OPENCODE_RUNTIME_PROVIDER_ID, "OpenCode"),
            (CLAUDE_RUNTIME_PROVIDER_ID, "Claude Code"),
        ] {
            let runtime = AgentRuntime {
                provider_id: provider_id.to_string(),
                display_name: display_name.to_string(),
                status: AgentRuntimeStatus::Unavailable,
                resolved_executable: None,
                source: None,
                configured_executable: None,
                version: None,
                diagnostic: None,
            };
            assert_eq!(inject_runtime_executable(&mut catalog, &runtime).unwrap(), 1);
        }
        let records = instances.synchronize_catalog(&catalog).unwrap();
        let codex = records
            .iter()
            .find(|record| record.plugin_id == "dev.codepet.codex")
            .unwrap();
        let opencode = records
            .iter()
            .find(|record| record.plugin_id == "dev.codepet.opencode")
            .unwrap();
        let claude = records
            .iter()
            .find(|record| record.plugin_id == "dev.codepet.claude")
            .unwrap();
        assert!(!codex.settings.contains_key("appServerExecutable"));
        assert!(!opencode.settings.contains_key("serverExecutable"));
        assert!(!opencode.settings.contains_key("serverVersion"));
        assert!(!claude.settings.contains_key("claudeExecutable"));
        assert_eq!(codex.settings["preserved"], serde_json::json!(true));
        assert_eq!(opencode.settings["preserved"], serde_json::json!(true));
        assert_eq!(claude.settings["preserved"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn concurrent_provider_host_shutdown_force_kills_visible_startup_process() {
        let directory = tempfile::tempdir().unwrap();
        let initialize_marker = directory.path().join("initialize-received");
        let shutdown_marker = directory.path().join("shutdown-received");
        let pid_marker = directory.path().join("provider-pid");
        let fixture_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("crates/Cargo.toml");
        let descriptor = PluginDescriptor {
            plugin_id: "dev.codepet.shutdown-race".to_string(),
            display_name: "Shutdown Race Fixture".to_string(),
            icon: None,
            executable: fake_provider_executable(&fixture_manifest),
            args: Vec::new(),
            env: [
                (
                    "CODEPET_FAKE_PLUGIN_ID".to_string(),
                    "dev.codepet.shutdown-race".to_string(),
                ),
                (
                    "CODEPET_FAKE_INITIALIZE_DELAY_MS".to_string(),
                    "300".to_string(),
                ),
                (
                    "CODEPET_FAKE_INITIALIZE_MARKER".to_string(),
                    initialize_marker.display().to_string(),
                ),
                (
                    "CODEPET_FAKE_SHUTDOWN_RESPONSE_DELAY_MS".to_string(),
                    "500".to_string(),
                ),
                (
                    "CODEPET_FAKE_SHUTDOWN_MARKER".to_string(),
                    shutdown_marker.display().to_string(),
                ),
                (
                    "CODEPET_FAKE_PID_MARKER".to_string(),
                    pid_marker.display().to_string(),
                ),
            ]
            .into_iter()
            .collect(),
            enabled: true,
            instances: vec![PluginInstanceConfig {
                instance_id: Some("instance-shutdown-race".to_string()),
                instance_kind: "fake".to_string(),
                display_name: "Shutdown Race".to_string(),
                settings: Default::default(),
                enabled: true,
            }],
        };
        let plugin_directory = directory.path().join("providers/race");
        std::fs::create_dir_all(&plugin_directory).unwrap();
        let mut manifest = serde_json::to_value(descriptor).unwrap();
        manifest
            .as_object_mut()
            .unwrap()
            .insert("manifestVersion".to_string(), serde_json::json!(1));
        std::fs::write(
            plugin_directory.join("codepet-provider.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let device = DeviceRegistry::open(directory.path().join("device.json"), "Race Device")
            .unwrap();
        let instances = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            device.identity().device_id.clone(),
        )
        .unwrap();
        let catalog = PluginCatalog::discover(
            PluginCatalogConfig::default().with_directory(directory.path().join("providers")),
        );
        let mut config = PluginManagerConfig::default();
        config.process.request_timeout = Duration::from_secs(2);
        config.process.shutdown_timeout = Duration::from_millis(100);
        let manager = Arc::new(
            PluginManager::new(device, catalog, instances, config).unwrap(),
        );
        let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
        assert!(gateway.start_event_forwarding());
        let provider_host = ProviderHostState::new(manager.clone(), gateway);
        let startup_manager = manager.clone();
        let startup = tokio::spawn(async move { startup_manager.start_enabled().await });
        wait_for_file(&initialize_marker).await;
        wait_for_file(&pid_marker).await;

        let barrier = Arc::new(Barrier::new(3));
        let first_host = provider_host.clone();
        let first_barrier = barrier.clone();
        let first = tokio::spawn(async move {
            first_barrier.wait().await;
            let result = first_host.shutdown_once().await;
            (result, first_host.shutdown_completed())
        });
        let second_host = provider_host.clone();
        let second_barrier = barrier.clone();
        let second = tokio::spawn(async move {
            second_barrier.wait().await;
            let result = second_host.shutdown_once().await;
            (result, second_host.shutdown_completed())
        });
        barrier.wait().await;
        let first = first.await.unwrap();
        let second = second.await.unwrap();
        assert_eq!(first, (true, true));
        assert_eq!(second, (true, true));
        assert!(shutdown_marker.exists());
        assert!(provider_host.shutdown_completed());
        assert!(provider_host.shutdown_once().await);
        let startup = startup.await.unwrap();
        assert!(startup[0].1.is_err());
        let snapshot = manager
            .snapshot("dev.codepet.shutdown-race")
            .await
            .unwrap();
        assert_eq!(snapshot.state, PluginRuntimeState::Stopped);
        assert!(snapshot
            .process_exit
            .as_ref()
            .is_some_and(|exit| !exit.success));
        let restart = manager
            .start_plugin("dev.codepet.shutdown-race")
            .await
            .unwrap_err();
        assert_eq!(restart.code, "provider_manager_shutting_down");
        #[cfg(unix)]
        assert!(!pid_is_alive(&pid_marker));
    }

    async fn wait_for_file(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !path.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    fn cargo_executable() -> PathBuf {
        let configured = std::env::var_os("CARGO")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cargo"));
        if configured.is_file() {
            return configured;
        }
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .map(|directory| directory.join(&configured))
            .find(|candidate| candidate.is_file())
            .expect("cargo executable must be available for the real Provider fixture")
    }

    fn fake_provider_executable(fixture_manifest: &Path) -> PathBuf {
        let target_directory = fixture_manifest.parent().unwrap().join("target");
        let status = Command::new(cargo_executable())
            .arg("build")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(fixture_manifest)
            .arg("--target-dir")
            .arg(&target_directory)
            .arg("-p")
            .arg("codepet-host")
            .arg("--bin")
            .arg("codepet-host-fake-provider")
            .status()
            .unwrap();
        assert!(status.success(), "real Provider fixture must compile");
        let executable = target_directory
            .join("debug")
            .join(format!(
                "codepet-host-fake-provider{}",
                std::env::consts::EXE_SUFFIX
            ));
        assert!(executable.is_file());
        executable
    }

    #[cfg(unix)]
    fn pid_is_alive(path: &Path) -> bool {
        extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }

        let pid = std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        unsafe { kill(pid, 0) == 0 }
    }
}
