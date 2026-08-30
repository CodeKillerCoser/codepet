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
    CODEX_RUNTIME_PROVIDER_ID,
};
use crate::settings::{configured_app_data_dir, load_app_settings};
use codepet_host::{
    DeviceRegistry, HostError, PluginCatalog, PluginCatalogConfig, PluginManager,
    PluginManagerConfig, ProviderGatewayService, ProviderInstanceRegistry,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::{Mutex as AsyncMutex, Notify};

pub const RUNTIME_GATEWAY_EVENT: &str = "runtime-gateway-event";
pub const CODEX_DESKTOP_COMPANION_EVENT: &str = "codex-desktop-companion-event";

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
    shutdown_started: Arc<AtomicBool>,
    shutdown_completed: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

impl Default for ProviderHostState {
    fn default() -> Self {
        match configured_provider_runtime() {
            Ok((manager, gateway)) => Self::new(manager, gateway),
            Err(error) => {
                crate::app_log::error(
                    "provider_host",
                    &format!("failed to initialize Provider Host boundary error={error:?}"),
                );
                Self::unavailable()
            }
        }
    }
}

impl ProviderHostState {
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
            shutdown_started: Arc::new(AtomicBool::new(false)),
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    fn unavailable() -> Self {
        Self {
            manager: None,
            gateway: None,
            started: Arc::new(AtomicBool::new(false)),
            codex_refresh_generation: Arc::new(AtomicU64::new(0)),
            codex_refresh_lock: Arc::new(AsyncMutex::new(())),
            claude_refresh_generation: Arc::new(AtomicU64::new(0)),
            claude_refresh_lock: Arc::new(AsyncMutex::new(())),
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
            let updated = manager
                .replace_instance_setting(
                    target.plugin_id,
                    target.instance_kind,
                    target.executable_setting,
                    executable,
                )
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
        if !gateway.start_event_forwarding() {
            crate::app_log::error(
                "provider_host",
                "Provider Gateway event forwarding was already started or unavailable",
            );
        }
        for diagnostic in manager.catalog_diagnostics() {
            crate::app_log::error(
                "provider_host",
                &format!("{} path={:?}", diagnostic.message, diagnostic.path),
            );
        }
        tauri::async_runtime::spawn(async move {
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
        });
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

fn configured_provider_runtime(
) -> Result<(Arc<PluginManager>, Arc<ProviderGatewayService>), HostError> {
    let settings = load_app_settings().map_err(HostError::from)?;
    let data_directory = configured_app_data_dir(&settings);
    let provider_host_directory = data_directory.join("provider-host");
    let device = DeviceRegistry::open(
        provider_host_directory.join("device-identity.json"),
        "This Device",
    )?;
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
    let mut catalog_config = PluginCatalogConfig::for_data_directory(&data_directory);
    for directory in settings.provider_plugins.directories {
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
    let mut catalog = PluginCatalog::discover(catalog_config);
    let runtime_service = AgentRuntimeService::default();
    for provider_id in [CODEX_RUNTIME_PROVIDER_ID, CLAUDE_RUNTIME_PROVIDER_ID] {
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
    let manager = Arc::new(PluginManager::new(
        device,
        catalog,
        instances,
        PluginManagerConfig::default(),
    )?);
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone())?);
    Ok((manager, gateway))
}

#[derive(Clone, Copy)]
struct ProviderRuntimeTarget {
    plugin_id: &'static str,
    instance_kind: &'static str,
    executable_setting: &'static str,
    display_name: &'static str,
}

fn provider_runtime_target(provider_id: &str) -> Option<ProviderRuntimeTarget> {
    match provider_id {
        CODEX_RUNTIME_PROVIDER_ID => Some(ProviderRuntimeTarget {
            plugin_id: "dev.codepet.codex",
            instance_kind: "codex",
            executable_setting: "appServerExecutable",
            display_name: "Codex",
        }),
        CLAUDE_RUNTIME_PROVIDER_ID => Some(ProviderRuntimeTarget {
            plugin_id: "dev.codepet.claude",
            instance_kind: "claude",
            executable_setting: "claudeExecutable",
            display_name: "Claude",
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
    use super::{inject_runtime_executable, ProviderHostState, ProviderGatewayService};
    use crate::agent_runtime::{
        AgentRuntime, AgentRuntimeSource, AgentRuntimeStatus, CLAUDE_RUNTIME_PROVIDER_ID,
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

    #[tokio::test]
    async fn claude_runtime_executable_is_injected_into_the_manifest_instance() {
        let directory = tempfile::tempdir().unwrap();
        let plugin_directory = directory.path().join("providers/claude");
        std::fs::create_dir_all(&plugin_directory).unwrap();
        std::fs::write(
            plugin_directory.join("codepet-provider.json"),
            serde_json::to_vec(&serde_json::json!({
                "manifestVersion": 1,
                "pluginId": "dev.codepet.claude",
                "displayName": "Claude",
                "executable": "codepet-provider-claude",
                "enabled": true,
                "instances": [{
                    "instanceId": "claude",
                    "instanceKind": "claude",
                    "displayName": "Claude",
                    "settings": { "claudeExecutable": "/stale/claude" },
                    "enabled": true
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let mut catalog = PluginCatalog::discover(
            PluginCatalogConfig::default().with_directory(directory.path().join("providers")),
        );
        let resolved = directory.path().join("resolved-claude");
        let runtime = AgentRuntime {
            provider_id: CLAUDE_RUNTIME_PROVIDER_ID.to_string(),
            display_name: "Claude Code".to_string(),
            status: AgentRuntimeStatus::Ready,
            resolved_executable: Some(resolved.to_string_lossy().to_string()),
            source: Some(AgentRuntimeSource::Configured),
            configured_executable: Some(resolved.to_string_lossy().to_string()),
            version: Some("2.1.251".to_string()),
            diagnostic: None,
        };
        assert_eq!(inject_runtime_executable(&mut catalog, &runtime).unwrap(), 1);

        let device = DeviceRegistry::open(directory.path().join("device.json"), "Test Device")
            .unwrap();
        let instances = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            device.identity().device_id.clone(),
        )
        .unwrap();
        let manager = PluginManager::new(
            device,
            catalog,
            instances,
            PluginManagerConfig::default(),
        )
        .unwrap();
        let snapshot = manager.snapshot("dev.codepet.claude").await.unwrap();
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(
            snapshot.instances[0]
                .record
                .settings
                .get("claudeExecutable")
                .and_then(serde_json::Value::as_str),
            Some(resolved.to_string_lossy().as_ref())
        );
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
