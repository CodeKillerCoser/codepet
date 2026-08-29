use super::gateway::Gateway;
use super::generated::{
    EventSequence, ProtocolError, ProtocolEvent, ProtocolRequest, ProtocolResponse,
};
use super::transport::{LocalTransport, Transport};
use crate::agent::codex_app_server::CodexRemoteProviderAdapter;
use crate::agent::codex_desktop_ipc::{
    CodexDesktopCompanionAdapter, CodexDesktopCompanionSnapshot,
};
use crate::agent::codex_thread_scope::CodexThreadScope;
use crate::runtime_gateway::ProviderAdapter;
use crate::settings::{configured_app_data_dir, load_app_settings};
use codepet_host::{
    DeviceRegistry, HostError, ManagerEvent, PluginCatalog, PluginCatalogConfig,
    PluginManager, PluginManagerConfig, PluginRuntimeState, ProviderGatewayService,
    ProviderInstanceRegistry,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub const RUNTIME_GATEWAY_EVENT: &str = "runtime-gateway-event";
pub const CODEX_DESKTOP_COMPANION_EVENT: &str = "codex-desktop-companion-event";
pub const CODEX_DESKTOP_COMPANION_THREAD_EXCLUDED_EVENT: &str =
    "codex-desktop-companion-thread-excluded";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDesktopCompanionThreadExcluded {
    pub conversation_id: String,
}

#[derive(Clone)]
pub struct RuntimeGatewayState {
    gateway: Arc<Gateway>,
    transport: LocalTransport,
    codex_remote: Arc<Mutex<Option<Arc<CodexRemoteProviderAdapter>>>>,
    provider_manager: Option<Arc<PluginManager>>,
    provider_gateway_v1: Option<Arc<ProviderGatewayService>>,
    provider_plugins_started: Arc<AtomicBool>,
    thread_scope: CodexThreadScope,
    refresh_generation: Arc<AtomicU64>,
}

impl Default for RuntimeGatewayState {
    fn default() -> Self {
        Self::with_thread_scope(CodexThreadScope::default())
    }
}

impl RuntimeGatewayState {
    pub fn with_thread_scope(thread_scope: CodexThreadScope) -> Self {
        let gateway = Arc::new(Gateway::default());
        let provider_runtime = configured_provider_runtime();
        if let Err(error) = provider_runtime.as_ref() {
            crate::app_log::error(
                "provider_host",
                &format!("failed to initialize Provider Host boundary error={error:?}"),
            );
        }
        let (provider_manager, provider_gateway_v1) = provider_runtime
            .ok()
            .map(|(manager, gateway)| (Some(manager), Some(gateway)))
            .unwrap_or((None, None));
        let codex_remote = Arc::new(CodexRemoteProviderAdapter::initializing_scoped(
            gateway.event_sink(),
            thread_scope.clone(),
        ));
        if let Err(error) = gateway.registry().register(codex_remote.clone()) {
            crate::app_log::error(
                "runtime_gateway",
                &format!("failed to register Codex App Server provider error={error:?}"),
            );
        }
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex_remote: Arc::new(Mutex::new(Some(codex_remote))),
            provider_manager,
            provider_gateway_v1,
            provider_plugins_started: Arc::new(AtomicBool::new(false)),
            thread_scope,
            refresh_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex_remote: Arc::new(Mutex::new(None)),
            provider_manager: None,
            provider_gateway_v1: None,
            provider_plugins_started: Arc::new(AtomicBool::new(false)),
            thread_scope: CodexThreadScope::default(),
            refresh_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }

    pub fn transport(&self) -> &LocalTransport {
        &self.transport
    }

    pub fn with_provider_gateway_v1(
        mut self,
        provider_gateway_v1: Arc<ProviderGatewayService>,
    ) -> Self {
        self.provider_manager = Some(provider_gateway_v1.manager().clone());
        self.provider_gateway_v1 = Some(provider_gateway_v1);
        self
    }

    pub fn provider_manager(&self) -> Option<&Arc<PluginManager>> {
        self.provider_manager.as_ref()
    }

    pub fn provider_gateway_v1(&self) -> Option<&Arc<ProviderGatewayService>> {
        self.provider_gateway_v1.as_ref()
    }

    pub fn start_provider_plugins_in_background(&self) {
        let (Some(manager), Some(provider_gateway)) = (
            self.provider_manager.clone(),
            self.provider_gateway_v1.clone(),
        ) else {
            return;
        };
        if self
            .provider_plugins_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        provider_gateway.start_event_forwarding();
        for diagnostic in manager.catalog_diagnostics() {
            crate::app_log::error(
                "provider_host",
                &format!("{} path={:?}", diagnostic.message, diagnostic.path),
            );
        }
        let mut manager_events = manager.subscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                match manager_events.recv().await {
                    Ok(ManagerEvent::PluginStateChanged { snapshot, .. })
                        if matches!(
                            snapshot.state,
                            PluginRuntimeState::Degraded | PluginRuntimeState::Crashed
                        ) =>
                    {
                        crate::app_log::error(
                            "provider_host",
                            &format!(
                                "Provider plugin state changed plugin_id={} state={:?} diagnostic={:?} stderr={:?}",
                                snapshot.plugin_id,
                                snapshot.state,
                                snapshot.diagnostic,
                                snapshot.stderr_diagnostics.last()
                            ),
                        );
                    }
                    Ok(ManagerEvent::Diagnostic { plugin_id, error }) => {
                        crate::app_log::error(
                            "provider_host",
                            &format!(
                                "Provider plugin diagnostic plugin_id={plugin_id} error={error:?}"
                            ),
                        );
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        crate::app_log::error(
                            "provider_host",
                            &format!(
                                "Provider diagnostic listener skipped events count={skipped}"
                            ),
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        tauri::async_runtime::spawn(async move {
            for (plugin_id, outcome) in manager.start_enabled().await {
                match outcome {
                    Ok(()) => crate::app_log::info(
                        "provider_host",
                        &format!("Provider plugin initialized plugin_id={plugin_id}"),
                    ),
                    Err(error) => crate::app_log::error(
                        "provider_host",
                        &format!(
                            "Provider plugin initialization failed plugin_id={plugin_id} error={error:?}"
                        ),
                    ),
                }
            }
        });
    }

    pub fn refresh_codex_remote_provider(&self) -> Result<(), ProtocolError> {
        let generation = self
            .refresh_generation
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let previous_provider = self
            .codex_remote
            .lock()
            .map_err(|_| gateway_state_error())?
            .as_ref()
            .map(|adapter| adapter.provider());
        let replacement = Arc::new(CodexRemoteProviderAdapter::spawn_scoped_paused(
            self.gateway.event_sink(),
            self.thread_scope.clone(),
        ));
        if self.refresh_generation.load(Ordering::SeqCst) != generation {
            replacement.retire();
            return Ok(());
        }
        let mut slot = self.codex_remote.lock().map_err(|_| gateway_state_error())?;
        if self.refresh_generation.load(Ordering::SeqCst) != generation {
            replacement.retire();
            return Ok(());
        }
        if let Some(previous) = slot.take() {
            previous.retire();
        }
        self.gateway.registry().register(replacement.clone())?;
        *slot = Some(replacement.clone());
        replacement.activate(previous_provider.map(|provider| provider.status));
        Ok(())
    }

    pub fn refresh_codex_remote_provider_in_background(&self) {
        let state = self.clone();
        std::thread::spawn(move || {
            if let Err(error) = state.refresh_codex_remote_provider() {
                crate::app_log::error(
                    "runtime_gateway",
                    &format!("failed to initialize Codex App Server provider error={error:?}"),
                );
            }
        });
    }
}

fn gateway_state_error() -> ProtocolError {
    ProtocolError {
        code: "gateway_state_error".to_string(),
        message: "Codex App Server provider refresh lock is unavailable".to_string(),
        retryable: true,
        details: None,
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
    let catalog = PluginCatalog::discover(catalog_config);
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
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()));
    Ok((manager, gateway))
}

#[derive(Clone)]
pub struct CodexDesktopCompanionState {
    gateway: Arc<Gateway>,
    transport: LocalTransport,
    codex_desktop: Option<Arc<CodexDesktopCompanionAdapter>>,
    thread_scope: CodexThreadScope,
}

impl Default for CodexDesktopCompanionState {
    fn default() -> Self {
        Self::with_thread_scope(CodexThreadScope::default())
    }
}

impl CodexDesktopCompanionState {
    pub fn with_thread_scope(thread_scope: CodexThreadScope) -> Self {
        let gateway = Arc::new(Gateway::default());
        let codex_desktop = Arc::new(CodexDesktopCompanionAdapter::spawn_scoped(
            gateway.event_sink(),
            thread_scope.clone(),
        ));
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
            thread_scope,
        }
    }

    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex_desktop: None,
            thread_scope: CodexThreadScope::default(),
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
    let transport = state.transport().clone();
    Ok(transport.request(request).await)
}

#[tauri::command]
pub fn runtime_gateway_replay(
    state: tauri::State<'_, RuntimeGatewayState>,
    after_event_sequence: Option<EventSequence>,
) -> Result<Vec<ProtocolEvent>, ProtocolError> {
    state.transport().replay(after_event_sequence)
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

pub fn start_runtime_gateway_event_bridge(
    app: AppHandle,
    state: &RuntimeGatewayState,
) -> Result<(), ProtocolError> {
    start_local_event_bridge(
        app,
        state.gateway(),
        state.transport(),
        RUNTIME_GATEWAY_EVENT,
        "runtime_gateway",
    )
}

pub fn start_codex_desktop_companion_event_bridge(
    app: AppHandle,
    state: &CodexDesktopCompanionState,
) -> Result<(), ProtocolError> {
    let remote_threads = state.thread_scope.subscribe_remote_threads();
    let codex_desktop = state.codex_desktop.clone();
    start_local_event_bridge(
        app.clone(),
        state.gateway(),
        state.transport(),
        CODEX_DESKTOP_COMPANION_EVENT,
        "codex_desktop_companion",
    )?;
    std::thread::spawn(move || {
        while let Ok(conversation_id) = remote_threads.recv() {
            if let Some(adapter) = &codex_desktop {
                adapter.exclude_remote_thread(&conversation_id);
            }
            let _ = app.emit(
                CODEX_DESKTOP_COMPANION_THREAD_EXCLUDED_EVENT,
                CodexDesktopCompanionThreadExcluded { conversation_id },
            );
        }
    });
    Ok(())
}

fn start_local_event_bridge(
    app: AppHandle,
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
