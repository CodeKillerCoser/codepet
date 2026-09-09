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
    AgentRuntime, AgentRuntimeCandidate, AgentRuntimeDiagnostic, AgentRuntimeInstallation,
    AgentRuntimeSource, AgentRuntimeStatus, CLAUDE_RUNTIME_PROVIDER_ID,
    CODEX_RUNTIME_PROVIDER_ID, OPENCODE_RUNTIME_PROVIDER_ID,
};
use crate::platform::host_identity::computer_name;
use crate::settings::{configured_app_data_dir, load_app_settings};
use codepet_lan_channel_sdk::DeviceDescriptor;
use codepet_provider_sdk::{
    RuntimeCandidate, RuntimeCandidateSource, RuntimeGetInstalledRequest, RuntimeSelectRequest,
};
use codepet_host::{
    DeviceRegistry, HostError, PluginCatalog, PluginCatalogConfig, PluginManager,
    PluginManagerConfig, PluginRuntimeState, ProviderGatewayService, ProviderInstanceRegistry,
    RemoteAccessConfig, RemoteAccessManager, StderrDiagnostic,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::Notify;

pub const RUNTIME_GATEWAY_EVENT: &str = "runtime-gateway-event";
pub const CODEX_DESKTOP_COMPANION_EVENT: &str = "codex-desktop-companion-event";

#[tauri::command]
pub(crate) async fn codepet_gateway_request(state: tauri::State<'_, ProviderHostState>, request: codepet_gateway_sdk::ProtocolRequest) -> Result<codepet_gateway_sdk::JsonRpcResponse, String> {
    let gateway = state.gateway.as_ref().ok_or("Provider Gateway is unavailable")?;
    Ok(gateway.dispatch_for_local_client("desktop-main", request).await)
}
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
    pet: Option<Arc<codepet_host::PetGateway>>,
    gateway: Option<Arc<ProviderGatewayService>>,
    started: Arc<AtomicBool>,
    shutdown_started: Arc<AtomicBool>,
    shutdown_completed: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderConnectionView {
    provider_id: String,
    connection_status: codepet_provider_sdk::ConnectionStatus,
    generation: u64,
    instances: Vec<ProviderInstanceConnectionView>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderInstanceConnectionView { id: String, status: codepet_provider_sdk::InstanceStatus }

#[tauri::command]
pub(crate) async fn provider_connection_status(state: tauri::State<'_, ProviderHostState>) -> Result<Vec<ProviderConnectionView>, String> {
    Ok(state.connection_views().await)
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
            pet: Some(codepet_host::PetGateway::new(manager.clone(), crate::settings::configured_app_data_dir(&crate::settings::load_app_settings().unwrap_or_default()).join("pet-sources.json"))),
            manager: Some(manager),
            gateway: Some(gateway),
            started: Arc::new(AtomicBool::new(false)),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            manager: None,
            pet: None,
            gateway: None,
            started: Arc::new(AtomicBool::new(false)),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn start_pet(&self) {
        if let Some(pet) = self.pet.clone() { tauri::async_runtime::spawn(async move { pet.start(); }); }
    }

    pub(crate) fn gateway(&self) -> Option<Arc<ProviderGatewayService>> {
        self.gateway.clone()
    }

    pub(crate) async fn connection_views(&self) -> Vec<ProviderConnectionView> {
        let Some(manager) = self.manager.as_ref() else { return vec![]; };
        manager.snapshots().await.into_iter().map(|snapshot| ProviderConnectionView {
            provider_id: snapshot.catalog.plugin_id,
            connection_status: snapshot.connection_status,
            generation: snapshot.generation,
            instances: snapshot.instances.into_iter().map(|instance| ProviderInstanceConnectionView {
                id: instance.record.instance_id,
                status: instance.instance.map(|i| i.status).unwrap_or(codepet_provider_sdk::InstanceStatus::Created),
            }).collect(),
        }).collect()
    }

    pub(crate) async fn runtime_views(&self) -> Vec<AgentRuntime> {
        let Some(manager) = self.manager.as_ref() else { return Vec::new() };
        let settings = load_app_settings().ok();
        let mut views = Vec::new();
        for snapshot in manager.snapshots().await.into_iter().filter(|snapshot| snapshot.catalog.enabled) {
            let plugin_id = snapshot.catalog.plugin_id.clone();
            let display_name = snapshot.reported.as_ref().map(|reported| reported.display_name.clone())
                .unwrap_or_else(|| snapshot.catalog.display_name.clone());
            let configured_executable = settings.as_ref()
                .and_then(|settings| configured_runtime_selection(settings, &plugin_id));
            let initializing = snapshot.state == PluginRuntimeState::Starting
                || (snapshot.state == PluginRuntimeState::Stopped && snapshot.generation == 0
                    && !self.shutdown_started.load(Ordering::SeqCst));
            if initializing {
                views.push(AgentRuntime {
                    provider_id: plugin_id,
                    display_name,
                    status: AgentRuntimeStatus::Loading,
                    resolved_executable: None,
                    source: None,
                    configured_executable,
                    version: None,
                    diagnostic: None,
                    installed: Vec::new(),
                });
                continue;
            }
            let inventory = manager.runtime_get_installed(&plugin_id, RuntimeGetInstalledRequest { refresh: None }).await;
            match inventory {
                Ok(inventory) => {
                    let installed = inventory.installed.into_iter().map(|installation| AgentRuntimeInstallation {
                        executable_path: installation.executable_path,
                        version: installation.version,
                        source: agent_runtime_source(installation.source),
                        minimum_version: installation.minimum_version,
                        incompatibility_reason: installation.incompatibility_reason,
                    }).collect::<Vec<_>>();
                    let selected = inventory.selected.map(|installation| AgentRuntimeInstallation {
                        executable_path: installation.executable_path,
                        version: installation.version,
                        source: agent_runtime_source(installation.source),
                        minimum_version: installation.minimum_version,
                        incompatibility_reason: installation.incompatibility_reason,
                    });
                    let selection_unconfirmed = configured_executable.is_some() && selected.is_none();
                    views.push(AgentRuntime {
                        provider_id: plugin_id,
                        display_name: display_name.clone(),
                        status: if inventory.scanning==Some(true) {
                            AgentRuntimeStatus::Loading
                        } else if selection_unconfirmed {
                            AgentRuntimeStatus::InvalidConfiguredExecutable
                        } else if !installed.iter().any(|runtime| runtime.incompatibility_reason.is_none()) {
                            AgentRuntimeStatus::Unavailable
                        } else {
                            AgentRuntimeStatus::Ready
                        },
                        resolved_executable: selected.as_ref().map(|installation| installation.executable_path.clone()),
                        source: selected.as_ref().map(|installation| installation.source),
                        configured_executable,
                        version: selected.as_ref().map(|installation| installation.version.clone()),
                        diagnostic: if inventory.scanning==Some(true) { None } else if let Some(message)=inventory.scan_error {
                            Some(AgentRuntimeDiagnostic {code:"runtime-scan-failed".into(),message})
                        } else if selection_unconfirmed {
                            Some(AgentRuntimeDiagnostic {
                                code: "provider-selection-unconfirmed".to_string(),
                                message: format!("{display_name} Provider did not confirm the persisted runtime selection"),
                            })
                        } else {
                            (!installed.iter().any(|runtime| runtime.incompatibility_reason.is_none())).then(|| AgentRuntimeDiagnostic {
                                code: "runtime-not-found".to_string(),
                                message: format!("{display_name} Provider did not find a compatible local runtime"),
                            })
                        },
                        installed,
                    });
                }
                Err(error) => views.push(AgentRuntime {
                    provider_id: plugin_id,
                    display_name,
                    status: AgentRuntimeStatus::Unavailable,
                    resolved_executable: None,
                    source: None,
                    configured_executable,
                    version: None,
                    diagnostic: Some(AgentRuntimeDiagnostic {
                        code: if snapshot.state == PluginRuntimeState::Ready { "runtime-api-unavailable" } else { "provider-unavailable" }.to_string(),
                        message: error.to_string(),
                    }),
                    installed: Vec::new(),
                }),
            }
        }
        views
    }

    pub(crate) async fn rescan_runtime(&self, plugin_id: &str) -> Result<(), String> {
        self.manager.as_ref().ok_or_else(|| "Provider Host is unavailable".to_string())?
            .runtime_get_installed(plugin_id, RuntimeGetInstalledRequest { refresh: Some(true) }).await.map(|_| ()).map_err(|error| error.to_string())
    }

    pub(crate) async fn runtime_view(&self, plugin_id: &str) -> Result<AgentRuntime, String> {
        self.runtime_views().await.into_iter().find(|runtime| runtime.provider_id == plugin_id)
            .ok_or_else(|| format!("unknown Provider plugin: {plugin_id}"))
    }

    pub(crate) async fn select_runtime(
        &self,
        provider_id: &str,
        candidate: AgentRuntimeCandidate,
    ) -> Result<AgentRuntimeInstallation, String> {
        let manager = self.manager.as_ref().ok_or_else(|| "Provider Host is unavailable".to_string())?;
        let response = manager.runtime_select(
            provider_id,
            RuntimeSelectRequest { candidate: provider_runtime_candidate(candidate) },
        ).await.map_err(|error| error.to_string())?;
        manager.remember_runtime_selection(provider_id, RuntimeCandidate {
            executable_path: response.selected.executable_path.clone(),
            source: response.selected.source,
        }).await.map_err(|error| error.to_string())?;
        manager.restart_plugin(provider_id).await.map_err(|error| error.to_string())?;
        Ok(AgentRuntimeInstallation {
            executable_path: response.selected.executable_path,
            version: response.selected.version,
            source: agent_runtime_source(response.selected.source),
            minimum_version: response.selected.minimum_version,
            incompatibility_reason: response.selected.incompatibility_reason,
        })
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

        if let Some(pet) = self.pet.as_ref() { pet.stop(); }
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

fn provider_runtime_candidate(candidate: AgentRuntimeCandidate) -> RuntimeCandidate {
    RuntimeCandidate {
        executable_path: candidate.executable_path,
        source: match candidate.source {
            AgentRuntimeSource::Configured => RuntimeCandidateSource::Configured,
            AgentRuntimeSource::Environment => RuntimeCandidateSource::Environment,
            AgentRuntimeSource::CurrentPath => RuntimeCandidateSource::CurrentPath,
            AgentRuntimeSource::LoginShell => RuntimeCandidateSource::LoginShell,
            AgentRuntimeSource::MacosApplication => RuntimeCandidateSource::MacosApplication,
            AgentRuntimeSource::WindowsApplication => RuntimeCandidateSource::WindowsApplication,
        },
    }
}

fn agent_runtime_source(source: RuntimeCandidateSource) -> AgentRuntimeSource {
    match source {
        RuntimeCandidateSource::Configured => AgentRuntimeSource::Configured,
        RuntimeCandidateSource::Environment => AgentRuntimeSource::Environment,
        RuntimeCandidateSource::CurrentPath => AgentRuntimeSource::CurrentPath,
        RuntimeCandidateSource::LoginShell => AgentRuntimeSource::LoginShell,
        RuntimeCandidateSource::MacosApplication => AgentRuntimeSource::MacosApplication,
        RuntimeCandidateSource::WindowsApplication => AgentRuntimeSource::WindowsApplication,
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

fn provider_manager_config(settings: &crate::settings::AppSettings) -> PluginManagerConfig {
    let mut config = PluginManagerConfig::default();
    config.provider_data_root = Some(configured_app_data_dir(settings).join("providers"));
    config.process.max_frame_bytes = codepet_host::provider_sdk::MAX_PROVIDER_FRAME_BYTES;
    config.process.stderr_observer = Some(record_provider_transport_diagnostic);
    for (provider_id, preference) in &settings.agent_runtimes.by_provider {
        let Some(executable_path) = preference.configured_executable.clone() else { continue };
        let plugin_id = match provider_id.as_str() {
            CODEX_RUNTIME_PROVIDER_ID => "dev.codepet.codex",
            CLAUDE_RUNTIME_PROVIDER_ID => "dev.codepet.claude",
            OPENCODE_RUNTIME_PROVIDER_ID => "dev.codepet.opencode",
            plugin_id => plugin_id,
        };
        if plugin_id != provider_id && settings.agent_runtimes.by_provider.contains_key(plugin_id) {
            continue;
        }
        config.runtime_selections.insert(plugin_id.to_string(), RuntimeCandidate {
            executable_path,
            source: RuntimeCandidateSource::Configured,
        });
    }
    config
}

fn record_provider_transport_diagnostic(diagnostic: &StderrDiagnostic) {
    if diagnostic
        .line
        .contains("\"schema\":\"codepet.provider.transport.v1\"")
    {
        crate::app_log::info(
            "provider_transport",
            &format!(
                "Provider Runtime transport metric truncated={} payload={}",
                diagnostic.truncated, diagnostic.line
            ),
        );
    }
}

fn configured_runtime_selection(
    settings: &crate::settings::AppSettings,
    plugin_id: &str,
) -> Option<String> {
    settings.agent_runtimes.by_provider.get(plugin_id)
        .and_then(|preference| preference.configured_executable.clone())
        .or_else(|| {
            let legacy_id = match plugin_id {
                "dev.codepet.codex" => CODEX_RUNTIME_PROVIDER_ID,
                "dev.codepet.claude" => CLAUDE_RUNTIME_PROVIDER_ID,
                "dev.codepet.opencode" => OPENCODE_RUNTIME_PROVIDER_ID,
                _ => return None,
            };
            settings.agent_runtimes.by_provider.get(legacy_id)
                .and_then(|preference| preference.configured_executable.clone())
        })
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
    let catalog = PluginCatalog::discover(catalog_config);
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
        provider_manager_config(&settings),
    )?);
    match crate::app::event_journal::journal() {
        Ok(journal) => manager.set_event_journal(journal.clone()),
        Err(error) => crate::app_log::error("event-journal", &error),
    }
    manager.enable_connection_heartbeats();
    let gateway = Arc::new(ProviderGatewayService::with_remote_identity_and_state_path(
        manager.clone(),
        {
            let identity = remote_access.remote_host_identity();
            codepet_host::RemoteHostIdentity {
                device_id: identity.device_id,
                descriptor: identity.descriptor,
            }
        },
        provider_host_directory.join("conversation-state.sqlite"),
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
    if let Some(host) = app.try_state::<ProviderHostState>() {
        let host = host.inner().clone();
        if let Some(manager) = host.manager.as_ref() {
            let mut runtime_changes = manager.subscribe_runtime_changes();
            let runtime_app = app.clone();
            tauri::async_runtime::spawn(async move {
                while runtime_changes.changed().await.is_ok() {
                    let _ = runtime_app.emit("provider-runtime-changed", ());
                }
            });
            let mut changes = manager.subscribe_status_changes();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let _ = app.emit("provider-connection-status", host.connection_views().await);
                    if host.shutdown_started.load(Ordering::SeqCst) { break; }
                    if changes.changed().await.is_err() { break; }
                }
            });
        }
    }
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
    use crate::agent_runtime::AgentRuntimeStatus;
    use super::{
        configured_runtime_selection, local_device_descriptor, non_empty_system_value,
        provider_catalog_config, provider_manager_config, spawn_provider_host_startup,
        ProviderGatewayService, ProviderHostState,
    };
    use codepet_host::{
        DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
        PluginInstanceConfig, PluginManager, PluginManagerConfig,
        PluginRuntimeState, ProviderInstanceRegistry,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn runtime_selection_display_uses_persisted_plugin_value_with_legacy_fallback() {
        let mut settings = crate::settings::AppSettings::default();
        settings.agent_runtimes.by_provider.insert(
            "codex".to_string(),
            crate::settings::AgentRuntimePreferenceSettings {
                configured_executable: Some("/legacy/codex".to_string()),
            },
        );
        assert_eq!(
            configured_runtime_selection(&settings, "dev.codepet.codex").as_deref(),
            Some("/legacy/codex")
        );
        settings.agent_runtimes.by_provider.insert(
            "dev.codepet.codex".to_string(),
            crate::settings::AgentRuntimePreferenceSettings {
                configured_executable: Some("/selected/codex".to_string()),
            },
        );
        assert_eq!(
            configured_runtime_selection(&settings, "dev.codepet.codex").as_deref(),
            Some("/selected/codex")
        );
    }
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
    fn provider_host_uses_the_shared_provider_frame_v1_limit() {
        assert_eq!(
            provider_manager_config(&crate::settings::AppSettings::default()).process.max_frame_bytes,
            codepet_host::provider_sdk::MAX_PROVIDER_FRAME_BYTES
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

    #[cfg(any())]
    #[test]
    fn three_catalog_runtime_executables_are_overridden_by_resolver_values() {
        let directory = tempfile::tempdir().unwrap();
        let plugin_directory = directory.path().join("providers");
        let descriptors = [
            PluginDescriptor {
                plugin_id: "dev.codepet.codex".to_string(),
                display_name: "Codex".to_string(),
                icon: Some("https://avatars.githubusercontent.com/u/14957082?s=200&v=4".to_string()),
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
                installed: Vec::new(),
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
                installed: Vec::new(),
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
    config.provider_data_root = Some(configured_app_data_dir(settings).join("providers"));
        config.process.request_timeout = Duration::from_secs(2);
        config.process.shutdown_timeout = Duration::from_millis(100);
        let manager = Arc::new(
            PluginManager::new(device, catalog, instances, config).unwrap(),
        );
        let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
        assert!(gateway.start_event_forwarding());
        let provider_host = ProviderHostState::new(manager.clone(), gateway);
        let initial_views = provider_host.runtime_views().await;
        assert_eq!(initial_views[0].status, AgentRuntimeStatus::Loading);
        assert!(initial_views[0].diagnostic.is_none());
        let startup_manager = manager.clone();
        let startup = tokio::spawn(async move { startup_manager.start_enabled().await });
        wait_for_file(&initialize_marker).await;
        wait_for_file(&pid_marker).await;
        let starting_views = provider_host.runtime_views().await;
        assert_eq!(starting_views[0].status, AgentRuntimeStatus::Loading);
        assert!(starting_views[0].diagnostic.is_none());

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
        let stopped_views = provider_host.runtime_views().await;
        assert_eq!(stopped_views[0].status, AgentRuntimeStatus::Unavailable);
        assert_eq!(stopped_views[0].diagnostic.as_ref().unwrap().code, "provider-unavailable");
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

#[tauri::command]
pub(crate) async fn pet_gateway_request(state: tauri::State<'_, ProviderHostState>, request: codepet_host::pet_gateway::protocol::ProtocolRequest) -> Result<codepet_host::pet_gateway::protocol::ProtocolResponse, String> {
    let pet = state.pet.clone().ok_or("Pet Gateway unavailable")?;
    Ok(codepet_host::pet_gateway::protocol::dispatch(pet.as_ref(), request).await)
}
