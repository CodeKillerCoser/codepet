use crate::providers::catalog::{CatalogDiagnostic, PluginCatalog, PluginDescriptor};
use crate::{
    DeviceRegistry, HostError, HostResult, PluginProcess, PluginProcessExit,
    PluginProcessOptions, ProviderInstanceRecord, ProviderInstanceRegistry, StderrDiagnostic,
};
use codepet_provider_sdk::{
    ApprovalResolveRequest, ApprovalResolveResponse, ClientId, ConversationCreateRequest,
    ConversationAcquireInteractionRequest, ConversationAcquireInteractionResponse,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse,
    ConversationSearchRequest, ConversationSearchResponse, InstanceCapabilitiesRequest,
    InstanceCapabilitiesResponse, InstanceCreateRequest, InstanceStartRequest, InstanceStatus,
    InstanceStopRequest, ProjectCreateRequest, ProjectCreateResponse, ProjectDeleteRequest,
    ProjectDeleteResponse, ProjectGetRequest, ProjectGetResponse, ProjectListRequest,
    ProjectListResponse, ProjectUpdateRequest, ProjectUpdateResponse, ProtocolEvent, ProtocolMethod,
    ProviderDescribeRequest, ProviderInitializeRequest, ProviderInstance, ProviderInstanceRoute,
    ProviderPluginDescriptor, ProviderResourceId, ProviderWireMessage, RoutedResourceId,
    TurnInterruptRequest,
    RuntimeCandidate, RuntimeGetInstalledRequest, RuntimeGetInstalledResponse, RuntimeSelectRequest,
    RuntimeSelectResponse, TurnInterruptResponse, TurnStartRequest, TurnStartResponse, TurnSteerRequest,
    TurnSteerResponse, VersionRange, PROTOCOL_VERSION,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginRuntimeState {
    Starting,
    Ready,
    Stopped,
    Crashed,
}

#[derive(Clone, Debug)]
pub struct ProviderInstanceRuntimeSnapshot {
    pub record: ProviderInstanceRecord,
    pub instance: Option<ProviderInstance>,
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeSnapshot {
    pub catalog: PluginDescriptor,
    pub reported: Option<ProviderPluginDescriptor>,
    pub state: PluginRuntimeState,
    pub diagnostic: Option<HostError>,
    pub generation: u64,
    pub connection_status: codepet_provider_sdk::ConnectionStatus,
    pub process_exit: Option<PluginProcessExit>,
    pub stderr_diagnostics: Vec<StderrDiagnostic>,
    pub instances: Vec<ProviderInstanceRuntimeSnapshot>,
}

#[derive(Clone, Debug)]
pub(crate) enum HostUpdate {
    PluginStateChanged {
        snapshot: PluginRuntimeSnapshot,
        previous_state: PluginRuntimeState,
    },
    InstanceChanged {
        snapshot: PluginRuntimeSnapshot,
        instance_id: String,
        previous_status: Option<InstanceStatus>,
    },
    ProviderEvent(ProtocolEvent),
}

#[derive(Clone, Debug)]
pub struct PluginManagerConfig {
    pub provider_data_root: Option<std::path::PathBuf>,
    pub host_client_id: ClientId,
    pub host_version: String,
    pub supported_versions: VersionRange,
    pub process: PluginProcessOptions,
    pub event_capacity: usize,
    pub runtime_selections: BTreeMap<String, RuntimeCandidate>,
}

impl Default for PluginManagerConfig {
    fn default() -> Self {
        Self {
            provider_data_root: None,
            host_client_id: format!("client-host-{}", Uuid::new_v4()),
            host_version: env!("CARGO_PKG_VERSION").to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
            process: PluginProcessOptions::default(),
            event_capacity: 256,
            runtime_selections: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct ManagedInstance {
    record: ProviderInstanceRecord,
    instance: Option<ProviderInstance>,
    diagnostic: Option<HostError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoricalRouteRecovery {
    Plugin,
    Instance,
}

struct PluginEntry {
    runtime_inventory: Option<RuntimeGetInstalledResponse>,
    catalog: PluginDescriptor,
    reported: Option<ProviderPluginDescriptor>,
    state: PluginRuntimeState,
    diagnostic: Option<HostError>,
    generation: u64,
    connection_status: codepet_provider_sdk::ConnectionStatus,
    process: Option<Arc<PluginProcess>>,
    process_exit: Option<PluginProcessExit>,
    stderr_diagnostics: Vec<StderrDiagnostic>,
    instances: BTreeMap<String, ManagedInstance>,
}

impl PluginEntry {
    fn snapshot(&self) -> PluginRuntimeSnapshot {
        PluginRuntimeSnapshot {
            catalog: self.catalog.clone(),
            reported: self.reported.clone(),
            state: self.state,
            diagnostic: self.diagnostic.clone(),
            generation: self.generation,
            connection_status: match self.state {
                PluginRuntimeState::Stopped | PluginRuntimeState::Crashed => codepet_provider_sdk::ConnectionStatus::Offline,
                PluginRuntimeState::Starting => codepet_provider_sdk::ConnectionStatus::Connecting,
                PluginRuntimeState::Ready => self.connection_status,
            },
            process_exit: self.process_exit.clone(),
            stderr_diagnostics: self.stderr_diagnostics.clone(),
            instances: self
                .instances
                .values()
                .map(|instance| ProviderInstanceRuntimeSnapshot {
                    record: instance.record.clone(),
                    instance: instance.instance.clone(),
                })
                .collect(),
        }
    }
}

struct PluginManagerInner {
    conversation_state_path: StdMutex<Option<std::path::PathBuf>>,
    event_journal: StdMutex<Option<Arc<crate::event_journal::EventJournal>>>,
    device: Arc<DeviceRegistry>,
    instances: ProviderInstanceRegistry,
    plugins: RwLock<BTreeMap<String, PluginEntry>>,
    plugin_operations: BTreeMap<String, Arc<Mutex<()>>>,
    subscriptions: StdMutex<BTreeMap<String, subscriptions::SubscriptionHub>>,
    subscription_operations: tokio::sync::Mutex<()>,
    updates: mpsc::Sender<HostUpdate>,
    update_receiver: StdMutex<Option<mpsc::Receiver<HostUpdate>>>,
    shutting_down: AtomicBool,
    remote_connections: Arc<crate::RemoteConnections>,
    status_changes: tokio::sync::watch::Sender<u64>,
    runtime_changes: tokio::sync::watch::Sender<u64>,
    heartbeat_enabled: AtomicBool,
    config: PluginManagerConfig,
    catalog_diagnostics: Vec<CatalogDiagnostic>,
    runtime_selections: RwLock<BTreeMap<String, RuntimeCandidate>>,
}

#[derive(Clone)]
pub struct PluginManager {
    inner: Arc<PluginManagerInner>,
}

mod heartbeat;
mod subscriptions;
pub use subscriptions::ProviderEventSubscription;

impl PluginManager {
    pub fn new(
        device: DeviceRegistry,
        catalog: PluginCatalog,
        instances: ProviderInstanceRegistry,
        config: PluginManagerConfig,
    ) -> HostResult<Self> {
        Self::with_device_registry(Arc::new(device), catalog, instances, config)
    }

    /// Builds a Provider Manager around the App's shared device identity registry.
    pub fn with_device_registry(
        device: Arc<DeviceRegistry>,
        catalog: PluginCatalog,
        instances: ProviderInstanceRegistry,
        config: PluginManagerConfig,
    ) -> HostResult<Self> {
        if device.identity().device_id != instances.device_id() {
            return Err(HostError::new(
                "provider_host_device_mismatch",
                "device identity and Provider instance registry do not match",
            )
            .with_detail("identityDeviceId", device.identity().device_id.clone())
            .with_detail("registryDeviceId", instances.device_id().to_string()));
        }
        let synchronized = instances.synchronize_catalog(&catalog)?;
        let mut instances_by_plugin = BTreeMap::<String, Vec<ProviderInstanceRecord>>::new();
        for record in synchronized {
            instances_by_plugin
                .entry(record.plugin_id.clone())
                .or_default()
                .push(record);
        }
        let mut plugins = BTreeMap::new();
        for descriptor in catalog.descriptors() {
            let managed_instances = instances_by_plugin
                .remove(&descriptor.plugin_id)
                .unwrap_or_default()
                .into_iter()
                .map(|record| {
                    (
                        record.instance_id.clone(),
                        ManagedInstance {
                            record,
                            instance: None,
                            diagnostic: None,
                        },
                    )
                })
                .collect();
            plugins.insert(
                descriptor.plugin_id.clone(),
                PluginEntry {
                    runtime_inventory: None,
                    catalog: descriptor.clone(),
                    reported: None,
                    state: PluginRuntimeState::Stopped,
                    diagnostic: None,
                    generation: 0,
                    connection_status: codepet_provider_sdk::ConnectionStatus::Offline,
                    process: None,
                    process_exit: None,
                    stderr_diagnostics: Vec::new(),
                    instances: managed_instances,
                },
            );
        }
        let plugin_operations = plugins
            .keys()
            .map(|plugin_id| (plugin_id.clone(), Arc::new(Mutex::new(()))))
            .collect();
        let (updates, update_receiver) = mpsc::channel(config.event_capacity.max(1));
        Ok(Self {
            inner: Arc::new(PluginManagerInner {
                conversation_state_path: StdMutex::new(None),
                event_journal: StdMutex::new(None),
                device,
                instances,
                plugins: RwLock::new(plugins),
                plugin_operations,
                subscriptions: StdMutex::new(BTreeMap::new()),
                subscription_operations: tokio::sync::Mutex::new(()),
                updates,
                update_receiver: StdMutex::new(Some(update_receiver)),
                shutting_down: AtomicBool::new(false),
                remote_connections: Arc::new(crate::RemoteConnections::default()),
                status_changes: tokio::sync::watch::channel(0).0,
                runtime_changes: tokio::sync::watch::channel(0).0,
                heartbeat_enabled: AtomicBool::new(false),
                catalog_diagnostics: catalog.diagnostics().to_vec(),
                runtime_selections: RwLock::new(config.runtime_selections.clone()),
                config,
            }),
        })
    }

    pub fn set_event_journal(&self, journal: Arc<crate::event_journal::EventJournal>) {
        *self.inner.event_journal.lock().unwrap() = Some(journal);
    }

    /// Called while constructing the Gateway, before starting Provider processes.
    /// Every participant uses one locked document; a late path switch is rejected.
    pub(crate) fn set_conversation_state_path(&self, path: &std::path::Path) -> HostResult<()> {
        if !path.is_absolute() {
            return Err(HostError::new("conversation_state_unavailable", "Shared state path must be absolute"));
        }
        let plugins = self.inner.plugins.try_read().map_err(|_| HostError::new(
            "conversation_state_unavailable", "Configure shared state before starting Providers"))?;
        if plugins.values().any(|plugin| plugin.generation != 0) {
            return Err(HostError::new("conversation_state_unavailable", "Cannot change shared state after Provider startup"));
        }
        let mut configured = self.inner.conversation_state_path.lock().map_err(|_| HostError::new(
            "conversation_state_unavailable", "Shared state configuration lock is poisoned"))?;
        if configured.as_deref().is_some_and(|old| old != path) {
            return Err(HostError::new("conversation_state_unavailable", "Shared state path was already configured"));
        }
        *configured = Some(path.to_owned());
        Ok(())
    }

    pub fn subscribe_runtime_changes(&self) -> tokio::sync::watch::Receiver<u64> { self.inner.runtime_changes.subscribe() }
    pub fn subscribe_status_changes(&self) -> tokio::sync::watch::Receiver<u64> { self.inner.status_changes.subscribe() }

    pub fn remote_connections(&self) -> Arc<crate::RemoteConnections> {
        self.inner.remote_connections.clone()
    }

    pub fn enable_connection_heartbeats(&self) {
        self.inner.heartbeat_enabled.store(true, Ordering::SeqCst);
    }

    pub(crate) fn device(&self) -> &DeviceRegistry {
        &self.inner.device
    }

    pub fn device_registry(&self) -> Arc<DeviceRegistry> {
        self.inner.device.clone()
    }

    pub fn catalog_diagnostics(&self) -> &[CatalogDiagnostic] {
        &self.inner.catalog_diagnostics
    }

    pub fn shutdown_timeout(&self) -> Duration {
        self.inner.config.process.shutdown_timeout
    }

    pub(crate) fn event_capacity(&self) -> usize {
        self.inner.config.event_capacity
    }

    pub(crate) fn take_updates(&self) -> HostResult<mpsc::Receiver<HostUpdate>> {
        self.inner
            .update_receiver
            .lock()
            .map_err(|_| update_channel_error())?
            .take()
            .ok_or_else(|| {
                HostError::new(
                    "provider_update_consumer_exists",
                    "Provider Manager updates already have a Gateway consumer",
                )
            })
    }

    pub async fn snapshots(&self) -> Vec<PluginRuntimeSnapshot> {
        self.inner
            .plugins
            .read()
            .await
            .values()
            .map(PluginEntry::snapshot)
            .collect()
    }

    pub async fn snapshot(&self, plugin_id: &str) -> HostResult<PluginRuntimeSnapshot> {
        self.inner
            .plugins
            .read()
            .await
            .get(plugin_id)
            .map(PluginEntry::snapshot)
            .ok_or_else(|| unknown_plugin(plugin_id))
    }

    pub async fn start_enabled(&self) -> Vec<(String, HostResult<()>)> {
        let plugin_ids = self
            .inner
            .plugins
            .read()
            .await
            .values()
            .filter(|entry| entry.catalog.enabled)
            .map(|entry| entry.catalog.plugin_id.clone())
            .collect::<Vec<_>>();
        let mut outcomes = Vec::new();
        for plugin_id in plugin_ids {
            let outcome = match self.plugin_operation(&plugin_id) {
                Ok(operation) => {
                    let _operation = operation.lock().await;
                    async {
                        self.start_plugin(&plugin_id).await?;
                        self.start_manifest_instances(&plugin_id).await
                    }
                    .await
                }
                Err(error) => Err(error),
            };
            outcomes.push((plugin_id, outcome));
        }
        outcomes
    }

    pub async fn replace_instance_setting(
        &self,
        plugin_id: &str,
        instance_kind: &str,
        key: &str,
        value: Option<Value>,
    ) -> HostResult<usize> {
        let records = self.inner.instances.replace_setting_for_kind(
            plugin_id,
            instance_kind,
            key,
            value,
        )?;
        if records.is_empty() {
            return Ok(0);
        }
        let mut plugins = self.inner.plugins.write().await;
        let entry = plugins
            .get_mut(plugin_id)
            .ok_or_else(|| unknown_plugin(plugin_id))?;
        for record in &records {
            let runtime = entry.instances.get_mut(&record.instance_id).ok_or_else(|| {
                HostError::new(
                    "unknown_provider_instance",
                    "Provider instance setting targets an unknown manifest instance",
                )
            })?;
            runtime.record = record.clone();
            runtime.diagnostic = None;
        }
        Ok(records.len())
    }

    pub async fn restart_plugin(&self, plugin_id: &str) -> HostResult<()> {
        let operation = self.plugin_operation(plugin_id)?;
        let _operation = operation.lock().await;
        self.restart_plugin_locked(plugin_id).await
    }

    async fn restart_plugin_locked(&self, plugin_id: &str) -> HostResult<()> {
        let stop_error = self.stop_plugin(plugin_id).await.err();
        if let Some(error) = stop_error.as_ref() {
            let terminated = self
                .inner
                .plugins
                .read()
                .await
                .get(plugin_id)
                .is_some_and(|entry| entry.process.is_none());
            if !terminated {
                return Err(error.clone());
            }
            eprintln!(
                "Provider {plugin_id} graceful stop failed before restart; the old process was force-terminated: {error}"
            );
        }
        if let Err(error) = self.start_plugin(plugin_id).await {
            return Err(with_restart_stop_diagnostic(error, stop_error.as_ref()));
        }
        self.start_manifest_instances(plugin_id)
            .await
            .map_err(|error| with_restart_stop_diagnostic(error, stop_error.as_ref()))
    }

    fn plugin_operation(&self, plugin_id: &str) -> HostResult<Arc<Mutex<()>>> {
        self.inner
            .plugin_operations
            .get(plugin_id)
            .cloned()
            .ok_or_else(|| unknown_plugin(plugin_id))
    }

    async fn start_manifest_instances(&self, plugin_id: &str) -> HostResult<()> {
        let mut first_error = None;
        for record in self
            .inner
            .instances
            .list_for_plugin(plugin_id)?
            .into_iter()
            .filter(|record| record.enabled)
        {
            let result = if self.inner.heartbeat_enabled.load(Ordering::SeqCst) {
                let created = self.inner.plugins.read().await.get(plugin_id)
                    .and_then(|entry| entry.instances.get(&record.instance_id))
                    .is_some_and(|runtime| runtime.instance.is_some());
                if created { Ok(()) } else { self.create_instance_record(&record).await.map(|_| ()) }
            } else {
                self.recover_instance_record(&record).await
            };
            if first_error.is_none() {
                first_error = result.err().filter(|error|error.code != "runtime_scanning");
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn start_plugin(&self, plugin_id: &str) -> HostResult<()> {
        let provider_directories = self.provider_directories(plugin_id)?;
        let saved_candidate = self.inner.runtime_selections.read().await.get(plugin_id).cloned();
        let preparation = {
            let mut plugins = self.inner.plugins.write().await;
            if self.inner.shutting_down.load(Ordering::SeqCst) {
                return Err(HostError::new(
                    "provider_manager_shutting_down",
                    "Provider Manager is shutting down",
                )
                .retryable(true));
            }
            let entry = plugins
                .get_mut(plugin_id)
                .ok_or_else(|| unknown_plugin(plugin_id))?;
            if !entry.catalog.enabled {
                return Err(HostError::new(
                    "provider_plugin_disabled",
                    format!("Provider plugin is disabled: {plugin_id}"),
                ));
            }
            if entry.state == PluginRuntimeState::Ready && entry.process.is_some() {
                return Ok(());
            }
            if entry.process.is_some() {
                return Err(HostError::new(
                    "provider_plugin_starting",
                    format!("Provider plugin already has a live process: {plugin_id}"),
                )
                .retryable(true));
            }
            let previous_state = entry.state;
            entry.generation = entry.generation.saturating_add(1);
            entry.state = PluginRuntimeState::Starting;
            entry.connection_status = codepet_provider_sdk::ConnectionStatus::Connecting;
            entry.diagnostic = None;
            entry.process_exit = None;
            entry.stderr_diagnostics.clear();
            for instance in entry.instances.values_mut() {
                instance.instance = None;
            }
            let mut descriptor = entry.catalog.clone();
            if let Some(dirs) = &provider_directories {
                descriptor.env.insert("CODEPET_PROVIDER_LOG_DIRECTORY".into(), dirs.logs.clone());
            }
            if let Some(path) = self.inner.conversation_state_path.lock().map_err(|_| {
                HostError::new("conversation_state_unavailable", "Shared state configuration lock is poisoned")
            })?.as_ref() {
                let path = path.to_str().ok_or_else(|| HostError::new(
                    "conversation_state_unavailable", "Shared state path must be UTF-8"))?;
                descriptor.env.insert("CODEPET_CONVERSATION_STATE_DATABASE".into(), path.to_owned());
            }
            if let Some(dirs) = &provider_directories {
                descriptor.env.insert("CODEPET_CONVERSATION_STATE_DATABASE".into(), dirs.database_path.clone());
            }
            if let Some(candidate) = saved_candidate {
                descriptor.env.insert("CODEPET_RUNTIME_EXECUTABLE".into(), candidate.executable_path);
            }
            match PluginProcess::spawn(&descriptor, self.inner.config.process.clone()) {
                Ok(process) => {
                    let process = Arc::new(process);
                    entry.process = Some(process.clone());
                    Ok((
                        descriptor,
                        entry.generation,
                        process,
                        HostUpdate::PluginStateChanged {
                            snapshot: entry.snapshot(),
                            previous_state,
                        },
                    ))
                }
                Err(error) => {
                    entry.state = PluginRuntimeState::Crashed;
                    entry.diagnostic = Some(error.clone());
                    Err((
                        error,
                        HostUpdate::PluginStateChanged {
                            snapshot: entry.snapshot(),
                            previous_state,
                        },
                    ))
                }
            }
        };
        let (descriptor, generation, process, starting_update) = match preparation {
            Ok(prepared) => prepared,
            Err((error, update)) => {
                if let Err(publish_error) = self.send_update(update).await {
                    eprintln!("Provider Host failed to publish spawn failure: {publish_error}");
                }
                return Err(error);
            }
        };
        if let Err(error) = self.send_update(starting_update).await {
            self.fail_started_process(plugin_id, generation, process, error.clone())
                .await;
            return Err(error);
        }

        let initialization = process
            .client()
            .provider_initialize(ProviderInitializeRequest { directories: provider_directories,
                host_client_id: self.inner.config.host_client_id.clone(),
                host_device_id: self.inner.device.identity().device_id.clone(),
                host_version: self.inner.config.host_version.clone(),
                supported_versions: self.inner.config.supported_versions.clone(),
            })
            .await
            .map_err(HostError::from)
            .and_then(|response| {
                validate_negotiated_descriptor(
                    &descriptor,
                    &self.inner.config.supported_versions,
                    response.selected_version,
                    &response.plugin,
                )?;
                Ok(response.plugin)
            });
        let initialized_descriptor = match initialization {
            Ok(descriptor) => descriptor,
            Err(error) => {
                self.fail_started_process(plugin_id, generation, process, error.clone())
                    .await;
                return Err(error);
            }
        };
        if !self.startup_is_current(plugin_id, generation, &process).await {
            let error = start_cancelled();
            self.fail_started_process(plugin_id, generation, process, error.clone())
                .await;
            return Err(error);
        }
        let described = process
            .client()
            .provider_describe(ProviderDescribeRequest {})
            .await
            .map_err(HostError::from)
            .and_then(|response| {
                validate_describe_descriptor(&initialized_descriptor, &response.plugin)?;
                Ok(response.plugin)
            });
        let reported = match described {
            Ok(reported) => reported,
            Err(error) => {
                self.fail_started_process(plugin_id, generation, process, error.clone())
                    .await;
                return Err(error);
            }
        };
        let inbound = match process.take_inbound().await {
            Ok(inbound) => inbound,
            Err(error) => {
                self.fail_started_process(plugin_id, generation, process, error.clone())
                    .await;
                return Err(error);
            }
        };
        let exit = process.exit_receiver();
        let diagnostics = process.diagnostics_handle();
        let process_identity = Arc::as_ptr(&process) as usize;

        let previous_state = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins
                .get_mut(plugin_id)
                .ok_or_else(|| unknown_plugin(plugin_id))?;
            if self.inner.shutting_down.load(Ordering::SeqCst)
                || entry.generation != generation
                || !entry
                    .process
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &process))
            {
                drop(plugins);
                let error = start_cancelled();
                self.fail_started_process(plugin_id, generation, process, error.clone())
                    .await;
                return Err(error);
            }
            let previous_state = entry.state;
            entry.reported = Some(reported);
            entry.process = Some(process.clone());
            entry.state = PluginRuntimeState::Ready;
            entry.runtime_inventory = None;
            entry.diagnostic = None;
            previous_state
        };
        if let Err(error) = self.publish_state(plugin_id, previous_state).await {
            self.fail_started_process(plugin_id, generation, process, error.clone())
                .await;
            return Err(error);
        }
        self.spawn_process_tasks(
            plugin_id.to_string(),
            generation,
            process_identity,
            Arc::downgrade(&process),
            inbound,
            exit,
            diagnostics,
        );
        if self.inner.heartbeat_enabled.load(Ordering::SeqCst) {
            self.spawn_heartbeat(plugin_id.to_string(), generation, Arc::downgrade(&process));
        }
        Ok(())
    }

    pub async fn stop_plugin(&self, plugin_id: &str) -> HostResult<()> {
        let (process, previous_state) = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins
                .get_mut(plugin_id)
                .ok_or_else(|| unknown_plugin(plugin_id))?;
            if entry.state == PluginRuntimeState::Stopped && entry.process.is_none() {
                return Ok(());
            }
            let previous_state = entry.state;
            entry.generation = entry.generation.saturating_add(1);
            entry.state = PluginRuntimeState::Stopped;
            (entry.process.clone(), previous_state)
        };
        self.publish_state(plugin_id, previous_state).await?;
        if let Some(process) = process {
            let outcome = process.shutdown().await;
            let stderr_diagnostics = process.stderr_diagnostics();
            match outcome {
                Ok(exit) => {
                    let mut plugins = self.inner.plugins.write().await;
                    if let Some(entry) = plugins.get_mut(plugin_id) {
                        entry.process = None;
                        entry.process_exit = Some(exit);
                        entry.stderr_diagnostics = stderr_diagnostics;
                    }
                }
                Err(error) => {
                    let forced = process
                        .force_kill("Provider graceful shutdown failed")
                        .await;
                    let mut plugins = self.inner.plugins.write().await;
                    if let Some(entry) = plugins.get_mut(plugin_id) {
                        if let Ok(exit) = forced.as_ref() {
                            if entry
                                .process
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &process))
                            {
                                entry.process = None;
                            }
                            entry.process_exit = Some(exit.clone());
                        }
                        entry.stderr_diagnostics = stderr_diagnostics;
                    }
                    drop(plugins);
                    let diagnostic = match forced {
                        Ok(_) => error.clone(),
                        Err(kill_error) => HostError::new(
                            "provider_force_kill_failed",
                            "Provider graceful shutdown and forced termination both failed",
                        )
                        .retryable(true)
                        .with_detail("gracefulStopCode", error.code.clone())
                        .with_detail("gracefulStopMessage", error.message.clone())
                        .with_detail("forceKillCode", kill_error.code.clone())
                        .with_detail("forceKillMessage", kill_error.message.clone()),
                    };
                    self.set_plugin_state(
                        plugin_id,
                        PluginRuntimeState::Crashed,
                        Some(diagnostic.clone()),
                    )
                    .await?;
                    return Err(diagnostic);
                }
            }
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Vec<(String, HostResult<()>)> {
        let plugin_ids = {
            let plugins = self.inner.plugins.write().await;
            self.inner.shutting_down.store(true, Ordering::SeqCst);
            plugins.keys().cloned().collect::<Vec<_>>()
        };
        let mut outcomes = Vec::new();
        for plugin_id in plugin_ids {
            let outcome = self.stop_plugin(&plugin_id).await;
            outcomes.push((plugin_id, outcome));
        }
        outcomes
    }

    pub async fn force_kill_all(
        &self,
        reason: &str,
    ) -> Vec<(String, HostResult<PluginProcessExit>)> {
        let processes = {
            let mut plugins = self.inner.plugins.write().await;
            self.inner.shutting_down.store(true, Ordering::SeqCst);
            plugins
                .iter_mut()
                .filter_map(|(plugin_id, entry)| {
                    entry.process.clone().map(|process| {
                        let previous_state = entry.state;
                        entry.generation = entry.generation.saturating_add(1);
                        entry.state = PluginRuntimeState::Stopped;
                        let update = (previous_state != PluginRuntimeState::Stopped).then(|| {
                            HostUpdate::PluginStateChanged {
                                snapshot: entry.snapshot(),
                                previous_state,
                            }
                        });
                        (plugin_id.clone(), process, update)
                    })
                })
                .collect::<Vec<_>>()
        };
        let mut outcomes = Vec::new();
        for (plugin_id, process, update) in processes {
            if let Some(update) = update {
                if let Err(error) = self.send_update(update).await {
                    eprintln!("Provider Host failed to publish forced stop: {error}");
                }
            }
            let outcome = process
                .force_kill(format!("{reason}: {plugin_id}"))
                .await;
            let stderr_diagnostics = process.stderr_diagnostics();
            let mut plugins = self.inner.plugins.write().await;
            if let Some(entry) = plugins.get_mut(&plugin_id) {
                if entry
                    .process
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &process))
                {
                    entry.process = None;
                }
                entry.stderr_diagnostics = stderr_diagnostics;
                match &outcome {
                    Ok(exit) => entry.process_exit = Some(exit.clone()),
                    Err(error) => {
                        entry.state = PluginRuntimeState::Crashed;
                        entry.diagnostic = Some(error.clone());
                    }
                }
            }
            outcomes.push((plugin_id, outcome));
        }
        outcomes
    }

    async fn ensure_historical_route_ready(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<()> {
        if self.inner.heartbeat_enabled.load(Ordering::SeqCst)
            && self.inner.remote_connections.snapshot().connections.is_empty() {
            return Err(HostError::new("no_connected_clients", "No authenticated client is connected").retryable(true));
        }
        validate_route_identity(route)?;
        let record = self.inner.instances.resolve_route(route, None)?;
        if !record.enabled {
            return Err(provider_instance_disabled(&record));
        }
        let operation = self.plugin_operation(&record.plugin_id)?;
        let _operation = operation.lock().await;
        let recovery = self.historical_route_recovery(&record).await?;
        match recovery {
            None => return Ok(()),
            Some(HistoricalRouteRecovery::Plugin) => {
                let recovery_error = self.restart_plugin_locked(&record.plugin_id).await.err();
                if self.historical_route_is_ready(&record).await {
                    return Ok(());
                }
                if let Some(error) = self.instance_diagnostic(&record).await? {
                    return Err(error);
                }
                if let Some(error) = recovery_error {
                    return Err(error);
                }
            }
            Some(HistoricalRouteRecovery::Instance) => {
                self.recover_instance_record(&record).await?;
            }
        }
        if self.historical_route_is_ready(&record).await {
            return Ok(());
        }
        Err(HostError::new(
            "provider_instance_unavailable",
            format!(
                "Provider instance did not become ready after recovery: {}",
                record.instance_id
            ),
        )
        .retryable(true)
        .with_detail("pluginId", record.plugin_id)
        .with_detail("providerInstanceId", record.instance_id))
    }

    async fn historical_route_recovery(
        &self,
        record: &ProviderInstanceRecord,
    ) -> HostResult<Option<HistoricalRouteRecovery>> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(provider_manager_shutting_down());
        }
        let plugins = self.inner.plugins.read().await;
        let entry = plugins
            .get(&record.plugin_id)
            .ok_or_else(|| unknown_plugin(&record.plugin_id))?;
        if !entry.catalog.enabled {
            return Err(provider_plugin_disabled(&record.plugin_id));
        }
        let runtime = entry.instances.get(&record.instance_id).ok_or_else(|| {
            HostError::new(
                "unknown_provider_instance",
                "Provider instance is not configured by the current plugin catalog",
            )
            .with_detail("pluginId", record.plugin_id.clone())
            .with_detail("providerInstanceId", record.instance_id.clone())
        })?;
        match entry.state {
            PluginRuntimeState::Stopped => Ok(Some(HistoricalRouteRecovery::Plugin)),
            PluginRuntimeState::Starting => Err(HostError::new(
                "provider_plugin_starting",
                format!("Provider plugin is still starting: {}", record.plugin_id),
            )
            .retryable(true)),
            PluginRuntimeState::Crashed => match entry.diagnostic.as_ref() {
                Some(error) if !error.retryable => Err(error.clone()),
                _ => Ok(Some(HistoricalRouteRecovery::Plugin)),
            },
            PluginRuntimeState::Ready => {
                if !entry
                    .process
                    .as_ref()
                    .is_some_and(|process| process.is_available())
                {
                    return Ok(Some(HistoricalRouteRecovery::Plugin));
                }
                if runtime
                    .instance
                    .as_ref()
                    .is_some_and(|instance| instance.status == InstanceStatus::Ready)
                {
                    return Ok(None);
                }
                if let Some(error) = runtime
                    .diagnostic
                    .as_ref()
                    .filter(|error| !error.retryable)
                {
                    return Err(error.clone());
                }
                Ok(Some(HistoricalRouteRecovery::Instance))
            }
        }
    }

    async fn historical_route_is_ready(&self, record: &ProviderInstanceRecord) -> bool {
        let plugins = self.inner.plugins.read().await;
        let Some(entry) = plugins.get(&record.plugin_id) else {
            return false;
        };
        entry.state == PluginRuntimeState::Ready
            && entry.process.as_ref().is_some_and(|process| process.is_available())
            && entry
                .instances
                .get(&record.instance_id)
                .and_then(|runtime| runtime.instance.as_ref())
                .is_some_and(|instance| instance.status == InstanceStatus::Ready)
    }

    async fn create_instance_record(
        &self,
        record: &ProviderInstanceRecord,
    ) -> HostResult<ProviderInstance> {
        let result: HostResult<ProviderInstance> = async {
            let process = self.process_for_plugin(&record.plugin_id).await?;
            let reported = self
                .snapshot(&record.plugin_id)
                .await?
                .reported
                .ok_or_else(|| {
                    HostError::new(
                        "provider_not_initialized",
                        format!("Provider plugin is not initialized: {}", record.plugin_id),
                    )
                })?;
            reported
                .validate_instance_kind(&record.instance_kind)
                .map_err(HostError::from)?;
            let response = process
                .client()
                .instance_create(InstanceCreateRequest {
                    route: record.route(),
                    instance_kind: record.instance_kind.clone(),
                    display_name: record.display_name.clone(),
                    settings: record.settings.clone(),
                })
                .await
                .map_err(HostError::from)?;
            validate_instance_response(record, &response.instance)?;
            self.set_runtime_instance(&record.plugin_id, response.instance.clone())
                .await?;
            Ok(response.instance)
        }
        .await;
        if let Err(error) = result.as_ref() {
            self.set_instance_diagnostic(record, Some(error.clone())).await?;
        }
        result
    }

    pub async fn start_instance(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<ProviderInstance> {
        let (record, process, before) = self.instance_context(route).await?;
        let result: HostResult<ProviderInstance> = async {
            let response = process
                .client()
                .instance_start(InstanceStartRequest {
                    route: route.clone(),
                })
                .await
                .map_err(HostError::from)?;
            validate_instance_response(&record, &response.instance)?;
            self.apply_start_response(&record, &before, response.instance).await
        }
        .await;
        if let Err(error) = result.as_ref() {
            self.set_instance_diagnostic(&record, Some(error.clone())).await?;
        }
        result
    }

    async fn recover_instance_record(&self, record: &ProviderInstanceRecord) -> HostResult<()> {
        if let Some(error) = self
            .instance_diagnostic(record)
            .await?
            .filter(|error| !error.retryable)
        {
            return Err(error);
        }
        let needs_create = self
            .inner
            .plugins
            .read()
            .await
            .get(&record.plugin_id)
            .and_then(|entry| entry.instances.get(&record.instance_id))
            .and_then(|runtime| runtime.instance.as_ref())
            .is_none();
        if needs_create {
            self.create_instance_record(record).await?;
        }
        self.start_instance(&record.route()).await?;
        Ok(())
    }

    pub async fn stop_instance(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<ProviderInstance> {
        let (record, process, _) = self.instance_context(route).await?;
        let response = process
            .client()
            .instance_stop(InstanceStopRequest {
                route: route.clone(),
            })
            .await
            .map_err(HostError::from)?;
        validate_instance_response(&record, &response.instance)?;
        self.set_runtime_instance(&record.plugin_id, response.instance.clone())
            .await?;
        Ok(response.instance)
    }

    pub async fn instance_capabilities(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<InstanceCapabilitiesResponse> {
        let (record, process, mut instance) = self.instance_context(route).await?;
        let response = process
            .client()
            .instance_capabilities(InstanceCapabilitiesRequest {
                route: route.clone(),
            })
            .await
            .map_err(HostError::from)?;
        instance.capabilities = response.capabilities.clone();
        self.set_runtime_instance(&record.plugin_id, instance).await?;
        Ok(response)
    }

    pub async fn runtime_get_installed(
        &self,
        plugin_id: &str,
        request: RuntimeGetInstalledRequest,
    ) -> HostResult<RuntimeGetInstalledResponse> {
        let process = self.process_for_plugin(plugin_id).await?;
        if request.refresh == Some(true) {
            if let Some(entry) = self.inner.plugins.write().await.get_mut(plugin_id) {
                entry.runtime_inventory = None;
            }
        } else if let Some(inventory) = self.inner.plugins.read().await.get(plugin_id).and_then(|entry| entry.runtime_inventory.clone()) {
            return Ok(inventory);
        }
        process.client().runtime_get_installed(request).await.map_err(HostError::from)
    }

    pub async fn runtime_select(
        &self,
        plugin_id: &str,
        request: RuntimeSelectRequest,
    ) -> HostResult<RuntimeSelectResponse> {
        self.process_for_plugin(plugin_id)
            .await?
            .client()
            .runtime_select(request)
            .await
            .map_err(HostError::from)
    }

    pub async fn remember_runtime_selection(
        &self,
        plugin_id: &str,
        candidate: RuntimeCandidate,
    ) -> HostResult<()> {
        if !self.inner.plugins.read().await.contains_key(plugin_id) {
            return Err(unknown_plugin(plugin_id));
        }
        self.inner.runtime_selections.write().await.insert(plugin_id.to_string(), candidate);
        Ok(())
    }

    pub async fn usage_query(&self, request: codepet_provider_sdk::UsageQueryRequest) -> HostResult<codepet_provider_sdk::UsageQueryResponse> {
        self.ensure_historical_route_ready(&request.route).await?;
        let (_, process, instance) = self.routing_context(&request.route).await?;
        ensure_capability(&instance, ProtocolMethod::UsageQuery)?;
        process.client().usage_query(request).await.map_err(HostError::from)
    }

    pub(crate) fn provider_state_paths(&self) -> HostResult<Vec<(String,std::path::PathBuf)>> {
        let mut paths=Vec::new();
        for record in self.inner.instances.list()? {
            if let Some(dirs)=self.provider_directories(&record.plugin_id)? {paths.push((record.instance_id,std::path::PathBuf::from(dirs.database_path)));}
        }
        Ok(paths)
    }

    fn provider_directories(&self, plugin_id: &str) -> HostResult<Option<codepet_provider_sdk::ProviderDirectories>> {
        let Some(root) = &self.inner.config.provider_data_root else { return Ok(None) };
        if !root.is_absolute() || plugin_id.is_empty() || !plugin_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) || plugin_id=="." || plugin_id==".." {
            return Err(HostError::new("invalid_provider_directories", "Invalid Provider root or plugin ID"));
        }
        let base=root.join(plugin_id); let data=base.join("data"); let logs=base.join("logs");
        std::fs::create_dir_all(&data)?; std::fs::create_dir_all(&logs)?;
        Ok(Some(codepet_provider_sdk::ProviderDirectories { data:data.to_string_lossy().into_owned(), logs:logs.to_string_lossy().into_owned(), database_path:data.join("provider.sqlite").to_string_lossy().into_owned() }))
    }

    pub async fn conversation_list(
        &self,
        request: ConversationListRequest,
    ) -> HostResult<ConversationListResponse> {
        let route = request.route.clone();
        if let codepet_provider_sdk::ConversationProjectFilter::ConversationProjectFilterProject(filter) =
            &request.project_filter
        {
            validate_provider_resource_identity(&filter.project)?;
            validate_provider_resource_route(&filter.project, &route)?;
        }
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationList)?;
        if request.reader_scope.is_some() {
            ensure_capability(&instance, ProtocolMethod::ConversationUnreadList)?;
            if request.reader_scope.as_deref().is_some_and(|scope| scope.trim().is_empty()) {
                return Err(HostError::new("invalid_request", "Reader scope must not be empty"));
            }
        }
        if request.query.is_some() {
            let modes = instance.capabilities.conversation_list_query.as_ref().ok_or_else(|| {
                HostError::new("unsupported", "Provider does not support scoped summary queries")
            })?;
            let allowed = match &request.query {
                Some(codepet_provider_sdk::ConversationListQuery::ConversationUpdatedAfterQuery(_)) => modes.updated_after,
                Some(codepet_provider_sdk::ConversationListQuery::ConversationIdsQuery(_)) => modes.ids,
                None => modes.updated_after || modes.ids,
            };
            if !allowed { return Err(HostError::new("unsupported", "Provider does not support this summary query")); }
            if let Some(codepet_provider_sdk::ConversationListQuery::ConversationIdsQuery(query)) = &request.query {
                if query.ids.is_empty() || query.ids.len() > 100
                    || query.ids.iter().any(|id| id.trim().is_empty())
                    || query.ids.iter().collect::<BTreeSet<_>>().len() != query.ids.len()
                    || !matches!(request.project_filter, codepet_provider_sdk::ConversationProjectFilter::ConversationProjectFilterAll(_)) {
                    return Err(HostError::new("invalid_request", "IDs query requires 1..100 distinct IDs and all project scope"));
                }
            }
        }
        if matches!(
            &request.project_filter,
            codepet_provider_sdk::ConversationProjectFilter::ConversationProjectFilterProject(_)
        ) {
            ensure_capability(&instance, ProtocolMethod::ProjectList)?;
        }
        let response = process
            .client()
            .conversation_list(request)
            .await
            .map_err(HostError::from)?;
        for conversation in &response.conversations {
            validate_conversation_routes(conversation, &route)?;
        }
        Ok(response)
    }

    pub async fn conversation_active_list(&self, request: codepet_provider_sdk::ConversationActiveListRequest)
        -> HostResult<codepet_provider_sdk::ConversationActiveListResponse> {
        let route = request.route.clone();
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationActiveList)?;
        let response = process.client().conversation_active_list(request).await.map_err(HostError::from)?;
        for row in &response.conversations {
            validate_provider_resource_identity(&row.conversation)?;
            validate_provider_resource_route(&row.conversation, &route)?;
            if row.activity_version.trim().is_empty() {
                return Err(HostError::new("provider_response_invalid", "Active entry requires an activity version"));
            }
        }
        Ok(response)
    }

    pub async fn conversation_unread_list(&self, request: codepet_provider_sdk::ConversationUnreadListRequest)
        -> HostResult<codepet_provider_sdk::ConversationUnreadListResponse> {
        let route = request.route.clone();
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationUnreadList)?;
        let response = process.client().conversation_unread_list(request).await.map_err(HostError::from)?;
        for row in &response.conversations {
            validate_provider_resource_identity(&row.conversation)?;
            validate_provider_resource_route(&row.conversation, &route)?;
            if row.read_state.activity_version.trim().is_empty() || !row.read_state.unread {
                return Err(HostError::new("provider_response_invalid", "Unread entry requires unread state and an activity version"));
            }
        }
        Ok(response)
    }

    pub async fn conversation_mark_read(&self, request: codepet_provider_sdk::ConversationMarkReadRequest)
        -> HostResult<codepet_provider_sdk::ConversationMarkReadResponse> {
        validate_provider_resource_identity(&request.conversation)?;
        let route = route_from_provider_resource(&request.conversation);
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationMarkRead)?;
        process.client().conversation_mark_read(request).await.map_err(HostError::from)
    }

    pub async fn project_list(&self, request: ProjectListRequest) -> HostResult<ProjectListResponse> {
        let route = request.route.clone();
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ProjectList)?;
        let response = process
            .client()
            .project_list(request)
            .await
            .map_err(HostError::from)?;
        for project in &response.projects {
            validate_project_routes(project, &route)?;
        }
        Ok(response)
    }

    pub async fn project_get(&self, request: ProjectGetRequest) -> HostResult<ProjectGetResponse> {
        validate_provider_resource_identity(&request.project)?;
        let expected = request.project.clone();
        let route = route_from_provider_resource(&expected);
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ProjectGet)?;
        let response = process
            .client()
            .project_get(request)
            .await
            .map_err(HostError::from)?;
        validate_project_routes(&response.project, &route)?;
        validate_provider_response_resource(&response.project.resource, &expected, "project.get")?;
        Ok(response)
    }

    pub async fn project_create(
        &self,
        request: ProjectCreateRequest,
    ) -> HostResult<ProjectCreateResponse> {
        validate_route_identity(&request.route)?;
        let route = request.route.clone();
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ProjectCreate)?;
        let response = process
            .client()
            .project_create(request)
            .await
            .map_err(HostError::from)?;
        validate_project_routes(&response.project, &route)?;
        Ok(response)
    }

    pub async fn project_update(
        &self,
        request: ProjectUpdateRequest,
    ) -> HostResult<ProjectUpdateResponse> {
        validate_provider_resource_identity(&request.project)?;
        let expected = request.project.clone();
        let route = route_from_provider_resource(&expected);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ProjectUpdate)?;
        let response = process
            .client()
            .project_update(request)
            .await
            .map_err(HostError::from)?;
        validate_project_routes(&response.project, &route)?;
        validate_provider_response_resource(&response.project.resource, &expected, "project.update")?;
        Ok(response)
    }

    pub async fn project_delete(
        &self,
        request: ProjectDeleteRequest,
    ) -> HostResult<ProjectDeleteResponse> {
        validate_provider_resource_identity(&request.project)?;
        let route = route_from_provider_resource(&request.project);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ProjectDelete)?;
        process
            .client()
            .project_delete(request)
            .await
            .map_err(HostError::from)
    }

    pub async fn conversation_search(
        &self,
        request: ConversationSearchRequest,
    ) -> HostResult<ConversationSearchResponse> {
        let route = request.route.clone();
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationSearch)?;
        let response = process
            .client()
            .conversation_search(request)
            .await
            .map_err(HostError::from)?;
        for conversation in &response.conversations {
            validate_conversation_routes(conversation, &route)?;
        }
        Ok(response)
    }

    pub async fn conversation_get(
        &self,
        request: ConversationGetRequest,
    ) -> HostResult<ConversationGetResponse> {
        validate_provider_resource_identity(&request.conversation)?;
        let expected = request.conversation.clone();
        let route = route_from_provider_resource(&expected);
        self.ensure_historical_route_ready(&route).await?;
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationGet)?;
        let response = process
            .client()
            .conversation_get(request)
            .await
            .map_err(HostError::from)?;
        validate_conversation_routes(&response.conversation, &route)?;
        validate_provider_response_resource(
            &response.conversation.resource,
            &expected,
            "conversation.get",
        )?;
        let expected_agent_resource = agent_resource_from_provider(&expected);
        validate_conversation_items(&response.items, &expected_agent_resource, &route)?;
        Ok(response)
    }

    pub async fn conversation_acquire_interaction(
        &self,
        request: ConversationAcquireInteractionRequest,
    ) -> HostResult<ConversationAcquireInteractionResponse> {
        validate_provider_resource_identity(&request.conversation)?;
        let route = route_from_provider_resource(&request.conversation);
        let (_, process, _) = self.routing_context(&route).await?;
        process
            .client()
            .conversation_acquire_interaction(request)
            .await
            .map_err(HostError::from)
    }

    pub async fn conversation_create(
        &self,
        request: ConversationCreateRequest,
    ) -> HostResult<ConversationCreateResponse> {
        validate_route_identity(&request.route)?;
        let route = request.route.clone();
        if let Some(project) = request.project.as_ref() {
            validate_provider_resource_identity(project)?;
            validate_provider_resource_route(project, &route)?;
        }
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationCreate)?;
        let response = process
            .client()
            .conversation_create(request)
            .await
            .map_err(HostError::from)?;
        validate_conversation_routes(&response.conversation, &route)?;
        Ok(response)
    }

    pub async fn turn_start(&self, request: TurnStartRequest) -> HostResult<TurnStartResponse> {
        validate_provider_resource_identity(&request.conversation)?;
        let expected_conversation = request.conversation.clone();
        let route = route_from_provider_resource(&expected_conversation);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::TurnStart)?;
        let response = process
            .client()
            .turn_start(request)
            .await
            .map_err(HostError::from)?;
        if !response.accepted {
            return Err(HostError::new(
                "provider_response_invalid",
                "Provider returned a successful turn.start response that was not accepted",
            ));
        }
        validate_turn_routes(&response.turn, &route)?;
        validate_provider_response_resource(
            &response.turn.conversation,
            &expected_conversation,
            "turn.start conversation",
        )?;
        if let Some(user_item) = response.user_item.as_ref() {
            let expected_agent_conversation = agent_resource_from_provider(&expected_conversation);
            validate_conversation_items(
                std::slice::from_ref(user_item),
                &expected_agent_conversation,
                &route,
            )?;
            let provider_user_item = match user_item {
                codepet_provider_sdk::ConversationItem::MessageConversationItem(item) => item,
                _ => {
                    return Err(HostError::new(
                        "provider_response_invalid",
                        "Provider turn.start userItem must be a canonical user message when present",
                    ));
                }
            };
            validate_exact_agent_resource(
                &provider_user_item.turn,
                &response.turn.resource,
                "turn.start user item turn",
            )?;
            if provider_user_item.role != codepet_provider_sdk::ConversationItemRole::User
                || provider_user_item.contents.is_empty()
            {
                return Err(HostError::new(
                    "provider_response_invalid",
                    "Provider turn.start userItem must be a canonical user message when present",
                ));
            }
        }
        Ok(response)
    }

    pub async fn turn_steer(&self, request: TurnSteerRequest) -> HostResult<TurnSteerResponse> {
        validate_provider_resource_identity(&request.conversation)?;
        validate_provider_resource_identity(&request.turn)?;
        validate_same_provider_resource_route(&request.conversation, &request.turn)?;
        let expected_conversation = request.conversation.clone();
        let expected_turn = request.turn.clone();
        let route = route_from_provider_resource(&expected_conversation);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::TurnSteer)?;
        let response = process
            .client()
            .turn_steer(request)
            .await
            .map_err(HostError::from)?;
        validate_turn_routes(&response.turn, &route)?;
        validate_provider_response_resource(&response.turn.resource, &expected_turn, "turn.steer")?;
        validate_provider_response_resource(
            &response.turn.conversation,
            &expected_conversation,
            "turn.steer conversation",
        )?;
        Ok(response)
    }

    pub async fn turn_interrupt(
        &self,
        request: TurnInterruptRequest,
    ) -> HostResult<TurnInterruptResponse> {
        validate_provider_resource_identity(&request.conversation)?;
        validate_provider_resource_identity(&request.turn)?;
        validate_same_provider_resource_route(&request.conversation, &request.turn)?;
        let expected_conversation = request.conversation.clone();
        let expected_turn = request.turn.clone();
        let route = route_from_provider_resource(&expected_conversation);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::TurnInterrupt)?;
        let response = process
            .client()
            .turn_interrupt(request)
            .await
            .map_err(HostError::from)?;
        validate_turn_routes(&response.turn, &route)?;
        validate_provider_response_resource(&response.turn.resource, &expected_turn, "turn.interrupt")?;
        validate_provider_response_resource(
            &response.turn.conversation,
            &expected_conversation,
            "turn.interrupt conversation",
        )?;
        Ok(response)
    }

    pub async fn approval_resolve(
        &self,
        request: ApprovalResolveRequest,
    ) -> HostResult<ApprovalResolveResponse> {
        validate_provider_resource_identity(&request.approval)?;
        let expected_approval = request.approval.clone();
        let route = route_from_provider_resource(&expected_approval);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ApprovalResolve)?;
        let response = process
            .client()
            .approval_resolve(request)
            .await
            .map_err(HostError::from)?;
        validate_approval_routes(&response.approval, &route)?;
        validate_provider_response_resource(
            &response.approval.resource,
            &expected_approval,
            "approval.resolve",
        )?;
        Ok(response)
    }

    async fn accept_provider_event(
        &self,
        plugin_id: &str,
        event: ProtocolEvent,
    ) -> HostResult<()> {
        if let Some(journal) = self.inner.event_journal.lock().unwrap().as_ref() {
            journal.provider(plugin_id, &event);
        }
        if let ProtocolEvent::RuntimeInventoryChanged {params,..} = event {
            let (first,generation)={
                let mut plugins=self.inner.plugins.write().await;
                let entry=plugins.get_mut(plugin_id).ok_or_else(||unknown_plugin(plugin_id))?;
                let first=entry.runtime_inventory.is_none();
                entry.runtime_inventory=Some(params.clone());(first,entry.generation)
            };
            self.inner.runtime_changes.send_modify(|version| *version = version.wrapping_add(1));
            if first && params.scanning != Some(true) {
                let manager=self.clone();let plugin_id=plugin_id.to_owned();
                tokio::spawn(async move {
                    let Ok(operation) = manager.plugin_operation(&plugin_id) else { return; };
                    let _operation = operation.lock().await;
                    if manager.inner.shutting_down.load(Ordering::SeqCst) || !manager.inner.plugins.read().await.get(&plugin_id)
                        .is_some_and(|entry| entry.generation == generation && entry.state == PluginRuntimeState::Ready) { return; }
                    let candidate = manager.inner.runtime_selections.read().await.get(&plugin_id).cloned();
                    if let Some(candidate)=candidate {
                        let _=manager.runtime_select(&plugin_id,RuntimeSelectRequest{candidate}).await;
                    }
                    let _=manager.start_manifest_instances(&plugin_id).await;
                });
            }
            return Ok(());
        }
        if let ProtocolEvent::EventNotification { params, .. } = event {
            return self.deliver_notification(plugin_id, params).await;
        }
        let record = match &event {
            ProtocolEvent::EventInstanceStatusChanged { params, .. } => self
                .inner
                .instances
                .resolve_route(&params.instance.route, Some(plugin_id))?,
            _ => self
                .inner
                .instances
                .resolve_provider_id(event_provider_id(&event)?, Some(plugin_id))?,
        };
        let route = record.route();
        validate_event_routes(&event, &route, &record)?;
        if let ProtocolEvent::EventInstanceStatusChanged { params, .. } = event {
            return self
                .set_runtime_instance(plugin_id, params.instance)
                .await;
        }
        self.send_update(HostUpdate::ProviderEvent(event)).await
    }

    async fn instance_context(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<(ProviderInstanceRecord, Arc<PluginProcess>, ProviderInstance)> {
        validate_route_identity(route)?;
        let record = self.inner.instances.resolve_route(route, None)?;
        let plugins = self.inner.plugins.read().await;
        let entry = plugins
            .get(&record.plugin_id)
            .ok_or_else(|| unknown_plugin(&record.plugin_id))?;
        if entry.state != PluginRuntimeState::Ready {
            return Err(HostError::new(
                "provider_plugin_unavailable",
                format!("Provider plugin is not ready: {}", record.plugin_id),
            )
            .retryable(true));
        }
        let process = entry.process.clone().ok_or_else(|| {
            HostError::new(
                "provider_process_unavailable",
                format!("Provider process is unavailable: {}", record.plugin_id),
            )
            .retryable(true)
        })?;
        let instance = entry
            .instances
            .get(&record.instance_id)
            .and_then(|runtime| runtime.instance.clone())
            .ok_or_else(|| {
                HostError::new(
                    "provider_instance_unavailable",
                    format!("Provider instance has not been created: {}", record.instance_id),
                )
                .retryable(true)
            })?;
        Ok((record, process, instance))
    }

    async fn routing_context(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<(ProviderInstanceRecord, Arc<PluginProcess>, ProviderInstance)> {
        let context = self.instance_context(route).await?;
        if context.2.status != InstanceStatus::Ready {
            return Err(HostError::new(
                "provider_instance_unavailable",
                format!(
                    "Provider instance is not ready: {}",
                    context.0.instance_id
                ),
            )
            .retryable(true));
        }
        Ok(context)
    }

    async fn process_for_plugin(&self, plugin_id: &str) -> HostResult<Arc<PluginProcess>> {
        let plugins = self.inner.plugins.read().await;
        let entry = plugins
            .get(plugin_id)
            .ok_or_else(|| unknown_plugin(plugin_id))?;
        if entry.state != PluginRuntimeState::Ready {
            return Err(HostError::new(
                "provider_plugin_unavailable",
                format!("Provider plugin is not ready: {plugin_id}"),
            )
            .retryable(true));
        }
        entry.process.clone().ok_or_else(|| {
            HostError::new(
                "provider_process_unavailable",
                format!("Provider process is unavailable: {plugin_id}"),
            )
            .retryable(true)
        })
    }

    async fn apply_start_response(
        &self,
        record: &ProviderInstanceRecord,
        before: &ProviderInstance,
        response: ProviderInstance,
    ) -> HostResult<ProviderInstance> {
        let (snapshot, previous_status) = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins
                .get_mut(&record.plugin_id)
                .ok_or_else(|| unknown_plugin(&record.plugin_id))?;
            let runtime = entry
                .instances
                .get_mut(&record.instance_id)
                .ok_or_else(|| {
                    HostError::new(
                        "unknown_provider_instance",
                        "Provider instance is not configured",
                    )
                })?;
            // A Ready metadata event (or a later stop/error) may beat the RPC response
            // on another mux stream. Preserve that newer observed state atomically.
            if let Some(current) = runtime.instance.as_ref() {
                if preserve_observed_start(before, current) || current == &response {
                    return Ok(current.clone());
                }
            }
            let previous_status = runtime.instance.as_ref().map(|instance| instance.status);
            runtime.instance = Some(response.clone());
            runtime.diagnostic = None;
            (entry.snapshot(), previous_status)
        };
        self.send_update(HostUpdate::InstanceChanged {
            snapshot,
            instance_id: record.instance_id.clone(),
            previous_status,
        })
        .await?;
        Ok(response)
    }

    async fn set_runtime_instance(
        &self,
        plugin_id: &str,
        instance: ProviderInstance,
    ) -> HostResult<()> {
        let record = self
            .inner
            .instances
            .resolve_route(&instance.route, Some(plugin_id))?;
        validate_instance_response(&record, &instance)?;
        self.set_runtime_instance_value(
            plugin_id,
            &record.instance_id,
            Some(instance),
        )
        .await
    }

    async fn set_runtime_instance_value(
        &self,
        plugin_id: &str,
        instance_id: &str,
        instance: Option<ProviderInstance>,
    ) -> HostResult<()> {
        let (snapshot, previous_status) = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins
                .get_mut(plugin_id)
                .ok_or_else(|| unknown_plugin(plugin_id))?;
            let runtime = entry.instances.get_mut(instance_id).ok_or_else(|| {
                HostError::new(
                    "unknown_provider_instance",
                    "Provider instance is not configured by the manifest",
                )
            })?;
            if runtime.instance == instance {
                return Ok(());
            }
            let previous_status = runtime.instance.as_ref().map(|instance| instance.status);
            runtime.instance = instance;
            if runtime.instance.is_some() {
                runtime.diagnostic = None;
            }
            (entry.snapshot(), previous_status)
        };
        self.send_update(HostUpdate::InstanceChanged {
            snapshot,
            instance_id: instance_id.to_string(),
            previous_status,
        })
        .await
    }

    async fn instance_diagnostic(
        &self,
        record: &ProviderInstanceRecord,
    ) -> HostResult<Option<HostError>> {
        let plugins = self.inner.plugins.read().await;
        let entry = plugins
            .get(&record.plugin_id)
            .ok_or_else(|| unknown_plugin(&record.plugin_id))?;
        entry
            .instances
            .get(&record.instance_id)
            .map(|runtime| runtime.diagnostic.clone())
            .ok_or_else(|| {
                HostError::new(
                    "unknown_provider_instance",
                    "Provider instance is not configured by the current plugin catalog",
                )
                .with_detail("pluginId", record.plugin_id.clone())
                .with_detail("providerInstanceId", record.instance_id.clone())
            })
    }

    async fn set_instance_diagnostic(
        &self,
        record: &ProviderInstanceRecord,
        diagnostic: Option<HostError>,
    ) -> HostResult<()> {
        let mut plugins = self.inner.plugins.write().await;
        let entry = plugins
            .get_mut(&record.plugin_id)
            .ok_or_else(|| unknown_plugin(&record.plugin_id))?;
        let runtime = entry.instances.get_mut(&record.instance_id).ok_or_else(|| {
            HostError::new(
                "unknown_provider_instance",
                "Provider instance is not configured by the current plugin catalog",
            )
            .with_detail("pluginId", record.plugin_id.clone())
            .with_detail("providerInstanceId", record.instance_id.clone())
        })?;
        runtime.diagnostic = diagnostic;
        Ok(())
    }

    async fn finish_start_failure(
        &self,
        plugin_id: &str,
        generation: u64,
        error: HostError,
        stderr_diagnostics: Vec<StderrDiagnostic>,
        process_exit: Option<PluginProcessExit>,
    ) {
        let update = {
            let mut plugins = self.inner.plugins.write().await;
            let Some(entry) = plugins.get_mut(plugin_id) else {
                return;
            };
            if entry.generation != generation {
                return;
            }
            let previous_state = entry.state;
            entry.state = PluginRuntimeState::Crashed;
            entry.diagnostic = Some(error);
            entry.process = None;
            entry.process_exit = process_exit;
            entry.stderr_diagnostics = stderr_diagnostics;
            HostUpdate::PluginStateChanged {
                snapshot: entry.snapshot(),
                previous_state,
            }
        };
        if let Err(error) = self.send_update(update).await {
            eprintln!("Provider Host failed to publish startup failure: {error}");
        }
    }

    async fn fail_started_process(
        &self,
        plugin_id: &str,
        generation: u64,
        process: Arc<PluginProcess>,
        error: HostError,
    ) {
        if !self.startup_is_current(plugin_id, generation, &process).await {
            return;
        }
        let process_exit = process
            .force_kill(format!("Provider startup failed: {}", error.message))
            .await
            .ok();
        self.finish_start_failure(
            plugin_id,
            generation,
            error,
            process.stderr_diagnostics(),
            process_exit,
        )
        .await;
    }

    async fn startup_is_current(
        &self,
        plugin_id: &str,
        generation: u64,
        process: &Arc<PluginProcess>,
    ) -> bool {
        let plugins = self.inner.plugins.read().await;
        !self.inner.shutting_down.load(Ordering::SeqCst)
            && plugins.get(plugin_id).is_some_and(|entry| {
                entry.generation == generation
                    && entry.state == PluginRuntimeState::Starting
                    && entry
                        .process
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, process))
            })
    }

    async fn set_plugin_state(
        &self,
        plugin_id: &str,
        state: PluginRuntimeState,
        diagnostic: Option<HostError>,
    ) -> HostResult<()> {
        let update = {
            let mut plugins = self.inner.plugins.write().await;
            let entry = plugins
                .get_mut(plugin_id)
                .ok_or_else(|| unknown_plugin(plugin_id))?;
            let previous_state = entry.state;
            entry.state = state;
            entry.diagnostic = diagnostic;
            HostUpdate::PluginStateChanged {
                snapshot: entry.snapshot(),
                previous_state,
            }
        };
        self.send_update(update).await
    }

    async fn publish_state(
        &self,
        plugin_id: &str,
        previous_state: PluginRuntimeState,
    ) -> HostResult<()> {
        let snapshot = self.snapshot(plugin_id).await?;
        self.send_update(HostUpdate::PluginStateChanged {
            snapshot,
            previous_state,
        })
        .await
    }

    async fn send_update(&self, update: HostUpdate) -> HostResult<()> {
        if matches!(&update, HostUpdate::PluginStateChanged { .. } | HostUpdate::InstanceChanged { .. }) {
            self.inner.status_changes.send_modify(|version| *version = version.wrapping_add(1));
        }
        self.inner
            .updates
            .send(update)
            .await
            .map_err(|_| update_channel_error())
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_process_tasks(
        &self,
        plugin_id: String,
        generation: u64,
        process_identity: usize,
        process: std::sync::Weak<PluginProcess>,
        mut inbound: mpsc::Receiver<ProviderWireMessage>,
        mut exit: tokio::sync::watch::Receiver<Option<PluginProcessExit>>,
        diagnostics: Arc<StdMutex<VecDeque<StderrDiagnostic>>>,
    ) {
        let manager = Arc::downgrade(&self.inner);
        let event_plugin_id = plugin_id.clone();
        tokio::spawn(async move {
            while let Some(message) = inbound.recv().await {
                let ProviderWireMessage::Event(event) = message else {
                    continue;
                };
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                let manager = PluginManager { inner: manager };
                if !manager
                    .is_current_process(&event_plugin_id, generation, process_identity)
                    .await
                {
                    return;
                }
                if let Err(error) = manager
                    .accept_provider_event(&event_plugin_id, event)
                    .await
                {
                    let _ = manager
                        .set_plugin_state(
                            &event_plugin_id,
                            PluginRuntimeState::Crashed,
                            Some(error.clone()),
                        )
                        .await;
                    if let Some(process) = process.upgrade() {
                        let _ = process
                            .force_kill(format!("invalid Provider event: {}", error.message))
                            .await;
                    }
                    return;
                }
            }
        });

        let manager = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                if exit.changed().await.is_err() {
                    return;
                }
                let Some(process_exit) = exit.borrow().clone() else {
                    continue;
                };
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                let manager = PluginManager { inner: manager };
                let stderr_diagnostics = diagnostics
                    .lock()
                    .map(|diagnostics| diagnostics.iter().cloned().collect())
                    .unwrap_or_default();
                let update = {
                    let mut plugins = manager.inner.plugins.write().await;
                    let Some(entry) = plugins.get_mut(&plugin_id) else {
                        return;
                    };
                    if entry.generation != generation {
                        return;
                    }
                    let previous_state = entry.state;
                    entry.process_exit = Some(process_exit.clone());
                    entry.process = None;
                    entry.stderr_diagnostics = stderr_diagnostics;
                    if entry.state == PluginRuntimeState::Stopped {
                        None
                    } else if entry.state == PluginRuntimeState::Crashed {
                        None
                    } else {
                        entry.state = PluginRuntimeState::Crashed;
                        entry.diagnostic = Some(
                            HostError::new(
                                "provider_process_crashed",
                                process_exit.reason.clone().unwrap_or_else(|| {
                                    "Provider process exited unexpectedly".to_string()
                                }),
                            )
                            .retryable(true),
                        );
                        Some(HostUpdate::PluginStateChanged {
                            snapshot: entry.snapshot(),
                            previous_state,
                        })
                    }
                };
                if let Some(update) = update {
                    if let Err(error) = manager.send_update(update).await {
                        eprintln!("Provider Host failed to publish process exit: {error}");
                    }
                }
                return;
            }
        });
    }

    async fn is_current_process(
        &self,
        plugin_id: &str,
        generation: u64,
        process_identity: usize,
    ) -> bool {
        let plugins = self.inner.plugins.read().await;
        plugins.get(plugin_id).is_some_and(|entry| {
            entry.generation == generation
                && entry.state == PluginRuntimeState::Ready
                && entry
                    .process
                    .as_ref()
                    .is_some_and(|current| Arc::as_ptr(current) as usize == process_identity)
        })
    }
}

fn validate_negotiated_descriptor(
    catalog: &PluginDescriptor,
    supported: &VersionRange,
    selected_version: u32,
    reported: &ProviderPluginDescriptor,
) -> HostResult<()> {
    if selected_version < supported.min_version || selected_version > supported.max_version {
        return Err(HostError::new(
            "provider_protocol_version_mismatch",
            format!("Provider selected unsupported protocol version: {selected_version}"),
        ));
    }
    if reported.supported_versions.min_version > reported.supported_versions.max_version
        || selected_version < reported.supported_versions.min_version
        || selected_version > reported.supported_versions.max_version
    {
        return Err(HostError::new(
            "provider_protocol_version_mismatch",
            "Provider selected a version outside its reported supported range",
        )
        .with_detail("selectedVersion", selected_version)
        .with_detail(
            "providerMinVersion",
            reported.supported_versions.min_version,
        )
        .with_detail(
            "providerMaxVersion",
            reported.supported_versions.max_version,
        ));
    }
    if reported.plugin_id != catalog.plugin_id {
        return Err(HostError::new(
            "provider_plugin_id_mismatch",
            "Provider initialize response does not match the catalog plugin id",
        )
        .with_detail("catalogPluginId", catalog.plugin_id.clone())
        .with_detail("reportedPluginId", reported.plugin_id.clone()));
    }
    reported.validate_instance_kinds().map_err(HostError::from)
}

fn validate_describe_descriptor(
    initialized: &ProviderPluginDescriptor,
    described: &ProviderPluginDescriptor,
) -> HostResult<()> {
    described.validate_instance_kinds().map_err(HostError::from)?;
    if initialized != described {
        return Err(HostError::new(
            "provider_descriptor_changed",
            "Provider describe response differs from initialize response",
        ));
    }
    Ok(())
}

fn validate_instance_response(
    record: &ProviderInstanceRecord,
    instance: &ProviderInstance,
) -> HostResult<()> {
    validate_route_identity(&instance.route)?;
    if instance.route != record.route()
        || instance.plugin_id != record.plugin_id
        || instance.instance_kind != record.instance_kind
    {
        return Err(HostError::new(
            "provider_instance_response_mismatch",
            "Provider returned an instance for a different route, plugin, or kind",
        )
        .with_detail("expectedPluginId", record.plugin_id.clone())
        .with_detail("expectedInstanceId", record.instance_id.clone()));
    }
    Ok(())
}

fn ensure_capability(instance: &ProviderInstance, method: ProtocolMethod) -> HostResult<()> {
    let Some(capability) = method.capability() else {
        return Ok(());
    };
    if instance.capabilities.methods.contains(&capability) {
        return Ok(());
    }
    Err(HostError::new(
        "provider_capability_unsupported",
        format!("Provider instance does not support method: {}", method.as_str()),
    )
    .with_detail(
        "providerInstanceId",
        instance.route.provider_instance_id.clone(),
    )
    .with_detail("method", method.as_str().to_string()))
}

fn event_provider_id(event: &ProtocolEvent) -> HostResult<&str> {
    let provider_id = match event {
        ProtocolEvent::EventConversationActiveChanged { params, .. } => params.conversation.provider_instance_id.as_str(),
        ProtocolEvent::EventConversationUnreadChanged { params, .. } => params.conversation.provider_instance_id.as_str(),
        ProtocolEvent::EventConversationDeleted { params, .. } => params.conversation.provider_instance_id.as_str(),
        ProtocolEvent::EventNotification { .. } | ProtocolEvent::RuntimeInventoryChanged { .. } => return Err(HostError::new("invalid_provider_event", "Notification uses a subscription")),
        ProtocolEvent::EventInstanceStatusChanged { .. } => {
            return Err(HostError::new(
                "invalid_provider_event",
                "Instance status events use their explicit Provider route",
            ));
        }
        ProtocolEvent::EventProjectChanged { params, .. } => params.project.provider_instance_id.as_str(),
        ProtocolEvent::EventConversationUpserted { params, .. } => {
            params.conversation.resource.provider_id.as_str()
        }
        ProtocolEvent::EventConversationItemUpserted { params, .. } => {
            conversation_item_resource(&params.item).provider_id.as_str()
        }
        ProtocolEvent::EventTurnUpserted { params, .. } => params.turn.resource.provider_id.as_str(),
        ProtocolEvent::EventTurnOutputDelta { params, .. } => params.turn.provider_instance_id.as_str(),
        ProtocolEvent::EventApprovalRequested { params, .. } => {
            params.approval.resource.provider_id.as_str()
        }
        ProtocolEvent::EventApprovalResolved { params, .. } => {
            params.approval.resource.provider_id.as_str()
        }
    };
    if provider_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_provider_resource",
            "Provider event resource providerId must not be empty",
        ));
    }
    Ok(provider_id)
}

fn validate_event_routes(
    event: &ProtocolEvent,
    route: &ProviderInstanceRoute,
    record: &ProviderInstanceRecord,
) -> HostResult<()> {
    match event {
        ProtocolEvent::EventConversationActiveChanged { params, .. } => {
            validate_provider_resource_route(&params.conversation, route)
        }
        ProtocolEvent::EventConversationUnreadChanged { params, .. } => {
            if params.reader_scope.trim().is_empty() {
                return Err(HostError::new("invalid_provider_event", "Unread event requires a reader scope"));
            }
            validate_provider_resource_route(&params.conversation, route)
        }
        ProtocolEvent::EventConversationDeleted { params, .. } => {
            validate_provider_resource_route(&params.conversation, route)
        }
        ProtocolEvent::EventNotification { .. } | ProtocolEvent::RuntimeInventoryChanged { .. } => Err(HostError::new("invalid_provider_event", "Notification uses a subscription")),
        ProtocolEvent::EventInstanceStatusChanged { params, .. } => {
            validate_instance_response(record, &params.instance)
        }
        ProtocolEvent::EventProjectChanged { params, .. } => {
            validate_provider_resource_route(&params.project, route)
        }
        ProtocolEvent::EventConversationUpserted { params, .. } => {
            validate_conversation_routes(&params.conversation, route)
        }
        ProtocolEvent::EventConversationItemUpserted { params, .. } => {
            validate_conversation_items(
                std::slice::from_ref(&params.item),
                conversation_item_conversation(&params.item),
                route,
            )
        }
        ProtocolEvent::EventTurnUpserted { params, .. } => {
            validate_turn_routes(&params.turn, route)
        }
        ProtocolEvent::EventTurnOutputDelta { params, .. } => {
            validate_provider_resource_route(&params.turn, route)?;
            validate_provider_resource_route(&params.conversation, route)?;
            if params.item_id.trim().is_empty() || params.content_id.trim().is_empty() {
                return Err(HostError::new(
                    "invalid_turn_output_delta",
                    "Provider output deltas require non-empty item and content identities",
                ));
            }
            Ok(())
        }
        ProtocolEvent::EventApprovalRequested { params, .. } => {
            validate_approval_routes(&params.approval, route)
        }
        ProtocolEvent::EventApprovalResolved { params, .. } => {
            validate_approval_routes(&params.approval, route)
        }
    }
}

fn validate_conversation_routes(
    conversation: &codepet_provider_sdk::Conversation,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_resource_route(&conversation.resource, route)?;
    if let Some(project) = conversation.project.as_ref() {
        validate_resource_route(project, route)?;
    }
    if let Some(turn) = conversation.active_turn.as_ref() {
        validate_turn_routes(turn, route)?;
        validate_exact_agent_resource(
            &turn.conversation,
            &conversation.resource,
            "conversation active turn",
        )?;
    }
    Ok(())
}

fn validate_project_routes(
    project: &codepet_provider_sdk::Project,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_resource_route(&project.resource, route)?;
    if project.name.trim().is_empty()
        || project.roots.is_empty()
        || project.roots.iter().any(|root| root.path.trim().is_empty())
        || !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&project.position)
        || project.created_at > 9_007_199_254_740_991
        || project.updated_at > 9_007_199_254_740_991
    {
        return Err(HostError::new(
            "provider_response_invalid",
            "Provider returned a project with invalid fields",
        ));
    }
    Ok(())
}

fn validate_conversation_items(
    items: &[codepet_provider_sdk::ConversationItem],
    conversation: &RoutedResourceId,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    let mut item_ids = BTreeSet::new();
    let mut content_ids = BTreeSet::new();
    for item in items {
        let resource = conversation_item_resource(item);
        let turn = conversation_item_turn(item);
        let item_conversation = conversation_item_conversation(item);
        validate_resource_route(resource, route)?;
        validate_resource_route(turn, route)?;
        validate_resource_route(item_conversation, route)?;
        validate_exact_agent_resource(
            item_conversation,
            conversation,
            "conversation history item",
        )?;
        if !item_ids.insert(resource.native_resource_id.as_str()) {
            return Err(HostError::new(
                "duplicate_conversation_item",
                "Provider conversation history contains a duplicate item identity",
            ));
        }
        for content in conversation_item_contents(item) {
            let content_id = conversation_content_id(content);
            if content_id.trim().is_empty() {
                return Err(HostError::new(
                    "invalid_conversation_content",
                    "Provider conversation history contains an empty content identity",
                ));
            }
            if !content_ids.insert(content_id) {
                return Err(HostError::new(
                    "duplicate_conversation_content",
                    "Provider conversation history contains a duplicate content identity",
                ));
            }
        }
        if let Some(related_item) = conversation_item_related_item(item) {
            validate_resource_route(related_item, route)?;
        }
        if let Some(approval) = conversation_item_approval(item) {
            validate_approval_routes(approval, route)?;
            validate_exact_agent_resource(&approval.resource, resource, "history approval")?;
            validate_exact_agent_resource(&approval.turn, turn, "history approval turn")?;
            validate_exact_agent_resource(
                &approval.conversation,
                conversation,
                "history approval conversation",
            )?;
        }
    }
    Ok(())
}

fn conversation_item_resource(item: &codepet_provider_sdk::ConversationItem) -> &RoutedResourceId {
    match item {
        codepet_provider_sdk::ConversationItem::MessageConversationItem(item) => &item.resource,
        codepet_provider_sdk::ConversationItem::ReasoningConversationItem(item) => &item.resource,
        codepet_provider_sdk::ConversationItem::CommandConversationItem(item) => &item.resource,
        codepet_provider_sdk::ConversationItem::FileChangeConversationItem(item) => &item.resource,
        codepet_provider_sdk::ConversationItem::ToolConversationItem(item) => &item.resource,
        codepet_provider_sdk::ConversationItem::ApprovalConversationItem(item) => &item.resource,
        codepet_provider_sdk::ConversationItem::UnknownConversationItem(item) => &item.resource,
    }
}

fn conversation_item_turn(item: &codepet_provider_sdk::ConversationItem) -> &RoutedResourceId {
    match item {
        codepet_provider_sdk::ConversationItem::MessageConversationItem(item) => &item.turn,
        codepet_provider_sdk::ConversationItem::ReasoningConversationItem(item) => &item.turn,
        codepet_provider_sdk::ConversationItem::CommandConversationItem(item) => &item.turn,
        codepet_provider_sdk::ConversationItem::FileChangeConversationItem(item) => &item.turn,
        codepet_provider_sdk::ConversationItem::ToolConversationItem(item) => &item.turn,
        codepet_provider_sdk::ConversationItem::ApprovalConversationItem(item) => &item.turn,
        codepet_provider_sdk::ConversationItem::UnknownConversationItem(item) => &item.turn,
    }
}

fn conversation_item_conversation(item: &codepet_provider_sdk::ConversationItem) -> &RoutedResourceId {
    match item {
        codepet_provider_sdk::ConversationItem::MessageConversationItem(item) => &item.conversation,
        codepet_provider_sdk::ConversationItem::ReasoningConversationItem(item) => &item.conversation,
        codepet_provider_sdk::ConversationItem::CommandConversationItem(item) => &item.conversation,
        codepet_provider_sdk::ConversationItem::FileChangeConversationItem(item) => &item.conversation,
        codepet_provider_sdk::ConversationItem::ToolConversationItem(item) => &item.conversation,
        codepet_provider_sdk::ConversationItem::ApprovalConversationItem(item) => &item.conversation,
        codepet_provider_sdk::ConversationItem::UnknownConversationItem(item) => &item.conversation,
    }
}

fn conversation_item_contents(item: &codepet_provider_sdk::ConversationItem) -> &[codepet_provider_sdk::ContentBlock] {
    match item {
        codepet_provider_sdk::ConversationItem::MessageConversationItem(item) => &item.contents,
        codepet_provider_sdk::ConversationItem::ReasoningConversationItem(item) => &item.contents,
        codepet_provider_sdk::ConversationItem::FileChangeConversationItem(item) => &item.contents,
        codepet_provider_sdk::ConversationItem::CommandConversationItem(item) => tool_outcome_contents(&item.tool),
        codepet_provider_sdk::ConversationItem::ToolConversationItem(item) => tool_outcome_contents(&item.tool),
        codepet_provider_sdk::ConversationItem::ApprovalConversationItem(_)
        | codepet_provider_sdk::ConversationItem::UnknownConversationItem(_) => &[],
    }
}

fn tool_outcome_contents(tool: &codepet_provider_sdk::ToolInvocation) -> &[codepet_provider_sdk::ContentBlock] {
    match tool.outcome.as_ref() {
        Some(codepet_provider_sdk::ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
        Some(codepet_provider_sdk::ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
        None => &[],
    }
}

fn conversation_content_id(content: &codepet_provider_sdk::ContentBlock) -> &str {
    match content {
        codepet_provider_sdk::ContentBlock::TextContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::ReasoningSummaryContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::OutputContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::ActivitySummaryContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::StructuredJsonContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::ImageContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::AudioContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::ResourceLinkContentBlock(content) => &content.content_id,
        codepet_provider_sdk::ContentBlock::EmbeddedResourceContentBlock(content) => &content.content_id,
    }
}

fn conversation_item_related_item(item: &codepet_provider_sdk::ConversationItem) -> Option<&RoutedResourceId> {
    match item {
        codepet_provider_sdk::ConversationItem::ApprovalConversationItem(item) => item.related_item.as_ref(),
        _ => None,
    }
}

fn conversation_item_approval(item: &codepet_provider_sdk::ConversationItem) -> Option<&codepet_provider_sdk::Approval> {
    match item {
        codepet_provider_sdk::ConversationItem::ApprovalConversationItem(item) => Some(&item.approval),
        _ => None,
    }
}

fn validate_turn_routes(
    turn: &codepet_provider_sdk::TurnTask,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_resource_route(&turn.resource, route)?;
    validate_resource_route(&turn.conversation, route)
}

fn validate_approval_routes(
    approval: &codepet_provider_sdk::Approval,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_resource_route(&approval.resource, route)?;
    validate_resource_route(&approval.conversation, route)?;
    validate_resource_route(&approval.turn, route)
}

fn validate_resource_route(
    resource: &RoutedResourceId,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_agent_resource_identity(resource)?;
    if resource.provider_id != route.provider_instance_id {
        return Err(HostError::new(
            "provider_resource_route_mismatch",
            "Provider returned a resource owned by a different Provider instance",
        )
        .with_detail("expectedProviderId", route.provider_instance_id.clone())
        .with_detail("actualProviderId", resource.provider_id.clone()));
    }
    Ok(())
}

fn validate_provider_resource_route(
    resource: &ProviderResourceId,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_provider_resource_identity(resource)?;
    if resource.device_id != route.device_id
        || resource.provider_plugin_id != route.provider_plugin_id
        || resource.provider_instance_id != route.provider_instance_id
    {
        return Err(HostError::new(
            "provider_resource_route_mismatch",
            "Provider request resource targets a different device, plugin, or instance",
        ));
    }
    Ok(())
}

fn validate_route_identity(route: &ProviderInstanceRoute) -> HostResult<()> {
    if route.device_id.trim().is_empty()
        || route.provider_plugin_id.trim().is_empty()
        || route.provider_instance_id.trim().is_empty()
    {
        return Err(HostError::new(
            "invalid_provider_route",
            "Provider route deviceId, providerPluginId, and providerInstanceId must not be empty",
        )
        .with_detail("deviceId", route.device_id.clone())
        .with_detail("providerPluginId", route.provider_plugin_id.clone())
        .with_detail(
            "providerInstanceId",
            route.provider_instance_id.clone(),
        ));
    }
    Ok(())
}

fn validate_provider_resource_identity(resource: &ProviderResourceId) -> HostResult<()> {
    validate_route_identity(&route_from_provider_resource(resource))?;
    if resource.native_resource_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_provider_resource",
            "Provider nativeResourceId must not be empty",
        )
        .with_detail("deviceId", resource.device_id.clone())
        .with_detail("providerPluginId", resource.provider_plugin_id.clone())
        .with_detail(
            "providerInstanceId",
            resource.provider_instance_id.clone(),
        ));
    }
    Ok(())
}

fn validate_agent_resource_identity(resource: &RoutedResourceId) -> HostResult<()> {
    if resource.provider_id.trim().is_empty() || resource.native_resource_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_provider_resource",
            "Provider response resource requires non-empty providerId and nativeResourceId",
        )
        .with_detail("providerId", resource.provider_id.clone())
        .with_detail("nativeResourceId", resource.native_resource_id.clone()));
    }
    Ok(())
}

fn validate_provider_response_resource(
    actual: &RoutedResourceId,
    expected: &ProviderResourceId,
    operation: &str,
) -> HostResult<()> {
    if actual.provider_id == expected.provider_instance_id
        && actual.native_resource_id == expected.native_resource_id
    {
        return Ok(());
    }
    Err(HostError::new(
        "provider_resource_identity_mismatch",
        format!("Provider {operation} response changed the requested resource identity"),
    )
    .with_detail("expectedNativeResourceId", expected.native_resource_id.clone())
    .with_detail("actualNativeResourceId", actual.native_resource_id.clone()))
}

fn validate_exact_agent_resource(
    actual: &RoutedResourceId,
    expected: &RoutedResourceId,
    operation: &str,
) -> HostResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(HostError::new(
        "provider_resource_identity_mismatch",
        format!("Provider {operation} response changed the resource identity"),
    )
    .with_detail("expectedNativeResourceId", expected.native_resource_id.clone())
    .with_detail("actualNativeResourceId", actual.native_resource_id.clone()))
}

fn route_from_provider_resource(resource: &ProviderResourceId) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
        provider_plugin_id: resource.provider_plugin_id.clone(),
        provider_instance_id: resource.provider_instance_id.clone(),
    }
}

fn agent_resource_from_provider(resource: &ProviderResourceId) -> RoutedResourceId {
    RoutedResourceId {
        provider_id: resource.provider_instance_id.clone(),
        native_resource_id: resource.native_resource_id.clone(),
    }
}

fn validate_same_provider_resource_route(
    left: &ProviderResourceId,
    right: &ProviderResourceId,
) -> HostResult<()> {
    if left.device_id == right.device_id
        && left.provider_plugin_id == right.provider_plugin_id
        && left.provider_instance_id == right.provider_instance_id
    {
        return Ok(());
    }
    Err(HostError::new(
        "provider_resource_route_mismatch",
        "Provider resources target different device, plugin, or instance routes",
    ))
}

fn with_restart_stop_diagnostic(
    error: HostError,
    stop_error: Option<&HostError>,
) -> HostError {
    let Some(stop_error) = stop_error else {
        return error;
    };
    error
        .with_detail("gracefulStopCode", stop_error.code.clone())
        .with_detail("gracefulStopMessage", stop_error.message.clone())
}

fn start_cancelled() -> HostError {
    HostError::new(
        "provider_start_cancelled",
        "Provider startup was superseded by shutdown",
    )
    .retryable(true)
}

fn provider_manager_shutting_down() -> HostError {
    HostError::new(
        "provider_manager_shutting_down",
        "Provider Manager is shutting down",
    )
    .retryable(true)
}

fn provider_plugin_disabled(plugin_id: &str) -> HostError {
    HostError::new(
        "provider_plugin_disabled",
        format!("Provider plugin is disabled: {plugin_id}"),
    )
    .with_detail("pluginId", plugin_id.to_string())
}

fn provider_instance_disabled(record: &ProviderInstanceRecord) -> HostError {
    HostError::new(
        "provider_instance_disabled",
        format!("Provider instance is disabled: {}", record.instance_id),
    )
    .with_detail("pluginId", record.plugin_id.clone())
    .with_detail("providerInstanceId", record.instance_id.clone())
}

fn unknown_plugin(plugin_id: &str) -> HostError {
    HostError::new(
        "unknown_provider_plugin",
        format!("Provider plugin is not registered: {plugin_id}"),
    )
    .with_detail("pluginId", plugin_id.to_string())
}

fn update_channel_error() -> HostError {
    HostError::new(
        "provider_gateway_update_unavailable",
        "Provider Gateway update consumer is unavailable",
    )
    .retryable(true)
}

#[cfg(test)]
mod conversation_item_validation_tests {
    use super::*;
    use serde_json::json;

    fn route() -> ProviderInstanceRoute {
        ProviderInstanceRoute {
            device_id: "device-test".to_string(),
            provider_plugin_id: "dev.codepet.test".to_string(),
            provider_instance_id: "instance-test".to_string(),
        }
    }

    fn resource(native_id: &str) -> serde_json::Value {
        json!({
            "providerId": "instance-test", "nativeResourceId": native_id,
        })
    }

    fn item_values() -> Vec<serde_json::Value> {
        vec![
            json!({"resource": resource("message"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "message", "status": "completed", "role": "assistant", "contents": [{"contentId": "message:text", "kind": "text", "text": "text"}]}),
            json!({"resource": resource("reasoning"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "reasoning", "status": "completed", "contents": [{"contentId": "reasoning:summary", "kind": "reasoning-summary", "text": "summary"}]}),
            json!({"resource": resource("command"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "command", "status": "completed", "tool": {"callId": "call-command", "name": "shell", "category": "command", "origin": {"kind": "builtin"}, "input": {"kind": "command", "command": "pwd"}, "outcome": {"kind": "success", "content": [{"contentId": "command:output", "kind": "output", "text": "out"}]}}}),
            json!({"resource": resource("file"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "file-change", "status": "completed", "contents": [{"contentId": "file:output", "kind": "output", "text": "diff"}]}),
            json!({"resource": resource("tool"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "tool", "status": "failed", "tool": {"callId": "call-tool", "name": "lookup", "category": "search", "origin": {"kind": "mcp"}, "input": {"kind": "structured", "value": {"q": "x"}}, "outcome": {"kind": "failure", "content": [{"contentId": "tool:output", "kind": "output", "text": "out"}], "error": {"message": "failed"}}}}),
            json!({"resource": resource("approval"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "approval", "status": "pending", "approval": {"resource": resource("approval"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "command", "title": "approve", "status": "pending", "decisions": ["approve", "deny"]}}),
            json!({"resource": resource("unknown"), "turn": resource("turn"), "conversation": resource("conversation"), "kind": "unknown", "status": "unknown"}),
        ]
    }

    #[test]
    fn validates_routes_for_every_item_variant() {
        let expected_route = route();
        let conversation: RoutedResourceId = serde_json::from_value(resource("conversation")).unwrap();
        let valid: Vec<codepet_provider_sdk::ConversationItem> = item_values().into_iter().map(|item| serde_json::from_value(item).unwrap()).collect();
        validate_conversation_items(&valid, &conversation, &expected_route).unwrap();

        for mut value in item_values() {
            value["turn"]["providerId"] = json!("wrong-provider");
            let item = serde_json::from_value(value).unwrap();
            assert_eq!(
                validate_conversation_items(&[item], &conversation, &expected_route).unwrap_err().code,
                "provider_resource_route_mismatch"
            );
        }
    }

    #[test]
    fn validates_canonical_content_ids_inside_tool_outcomes() {
        let expected_route = route();
        let conversation: RoutedResourceId = serde_json::from_value(resource("conversation")).unwrap();
        let mut values = item_values();
        values[4]["tool"]["outcome"]["content"][0]["contentId"] = json!("message:text");
        let items = values.into_iter().map(|item| serde_json::from_value(item).unwrap()).collect::<Vec<_>>();
        assert_eq!(
            validate_conversation_items(&items, &conversation, &expected_route).unwrap_err().code,
            "duplicate_conversation_content"
        );

        let mut values = item_values();
        values[2]["tool"]["outcome"]["content"][0]["contentId"] = json!("");
        let items = values.into_iter().map(|item| serde_json::from_value(item).unwrap()).collect::<Vec<_>>();
        assert_eq!(
            validate_conversation_items(&items, &conversation, &expected_route).unwrap_err().code,
            "invalid_conversation_content"
        );
    }
}

// Starting is an intermediate notification; any different terminal/Ready snapshot
// observed during this RPC is more recent than its initial response snapshot.
fn preserve_observed_start(before: &ProviderInstance, current: &ProviderInstance) -> bool {
    before != current && current.status != InstanceStatus::Starting
}

#[cfg(test)]
mod startup_snapshot_tests {
    use super::*;
    #[test]
    fn late_start_response_preserves_observed_metadata_and_stop() {
        let before: ProviderInstance = serde_json::from_value(serde_json::json!({
            "route":{"deviceId":"device","providerPluginId":"plugin","providerInstanceId":"instance"},
            "pluginId":"plugin","instanceKind":"fixture","displayName":"Fixture",
            "harness":{"id":"fixture","displayName":"Fixture"},"status":"created",
            "capabilities":{"revision":"pending","methods":[],"extensions":[]}
        })).unwrap();
        let mut current = before.clone();
        assert!(!preserve_observed_start(&before, &current));
        current.status = InstanceStatus::Starting;
        assert!(!preserve_observed_start(&before, &current));
        current.status = InstanceStatus::Ready;
        current.harness.version = Some("discovered".into());
        assert!(preserve_observed_start(&before, &current));
        current.status = InstanceStatus::Stopped;
        assert!(preserve_observed_start(&before, &current));
    }
}
