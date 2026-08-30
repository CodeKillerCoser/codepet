use crate::catalog::{CatalogDiagnostic, PluginCatalog, PluginDescriptor};
use crate::{
    DeviceRegistry, HostError, HostResult, PluginProcess, PluginProcessExit,
    PluginProcessOptions, ProviderInstanceRecord, ProviderInstanceRegistry, StderrDiagnostic,
};
use codepet_provider_sdk::{
    ApprovalResolveRequest, ApprovalResolveResponse, ClientId, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, InstanceCapabilitiesRequest,
    InstanceCapabilitiesResponse, InstanceCreateRequest, InstanceStartRequest, InstanceStatus,
    InstanceStopRequest, ProtocolEvent, ProtocolMethod,
    ProviderDescribeRequest, ProviderInitializeRequest, ProviderInstance, ProviderInstanceRoute,
    ProviderPluginDescriptor, ProviderWireMessage, RoutedResourceId, TurnInterruptRequest,
    TurnInterruptResponse, TurnStartRequest, TurnStartResponse, TurnSteerRequest,
    TurnSteerResponse, VersionRange, PROTOCOL_VERSION,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
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
    pub host_client_id: ClientId,
    pub host_version: String,
    pub supported_versions: VersionRange,
    pub process: PluginProcessOptions,
    pub event_capacity: usize,
}

impl Default for PluginManagerConfig {
    fn default() -> Self {
        Self {
            host_client_id: format!("client-host-{}", Uuid::new_v4()),
            host_version: env!("CARGO_PKG_VERSION").to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
            process: PluginProcessOptions::default(),
            event_capacity: 256,
        }
    }
}

#[derive(Clone, Debug)]
struct ManagedInstance {
    record: ProviderInstanceRecord,
    instance: Option<ProviderInstance>,
}

struct PluginEntry {
    catalog: PluginDescriptor,
    reported: Option<ProviderPluginDescriptor>,
    state: PluginRuntimeState,
    diagnostic: Option<HostError>,
    generation: u64,
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
    device: DeviceRegistry,
    instances: ProviderInstanceRegistry,
    plugins: RwLock<BTreeMap<String, PluginEntry>>,
    updates: mpsc::Sender<HostUpdate>,
    update_receiver: StdMutex<Option<mpsc::Receiver<HostUpdate>>>,
    shutting_down: AtomicBool,
    config: PluginManagerConfig,
    catalog_diagnostics: Vec<CatalogDiagnostic>,
}

#[derive(Clone)]
pub struct PluginManager {
    inner: Arc<PluginManagerInner>,
}

impl PluginManager {
    pub fn new(
        device: DeviceRegistry,
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
                        },
                    )
                })
                .collect();
            plugins.insert(
                descriptor.plugin_id.clone(),
                PluginEntry {
                    catalog: descriptor.clone(),
                    reported: None,
                    state: PluginRuntimeState::Stopped,
                    diagnostic: None,
                    generation: 0,
                    process: None,
                    process_exit: None,
                    stderr_diagnostics: Vec::new(),
                    instances: managed_instances,
                },
            );
        }
        let (updates, update_receiver) = mpsc::channel(config.event_capacity.max(1));
        Ok(Self {
            inner: Arc::new(PluginManagerInner {
                device,
                instances,
                plugins: RwLock::new(plugins),
                updates,
                update_receiver: StdMutex::new(Some(update_receiver)),
                shutting_down: AtomicBool::new(false),
                config,
                catalog_diagnostics: catalog.diagnostics().to_vec(),
            }),
        })
    }

    pub(crate) fn device(&self) -> &DeviceRegistry {
        &self.inner.device
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
            let outcome = async {
                self.start_plugin(&plugin_id).await?;
                let mut first_error = None;
                for record in self
                    .inner
                    .instances
                    .list_for_plugin(&plugin_id)?
                    .into_iter()
                    .filter(|record| record.enabled)
                {
                    let result = async {
                        self.create_instance_record(&record).await?;
                        self.start_instance(&record.route()).await?;
                        Ok::<(), HostError>(())
                    }
                    .await;
                    if first_error.is_none() {
                        first_error = result.err();
                    }
                }
                if let Some(error) = first_error {
                    return Err(error);
                }
                Ok(())
            }
            .await;
            outcomes.push((plugin_id, outcome));
        }
        outcomes
    }

    pub async fn start_plugin(&self, plugin_id: &str) -> HostResult<()> {
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
            entry.diagnostic = None;
            entry.process_exit = None;
            entry.stderr_diagnostics.clear();
            for instance in entry.instances.values_mut() {
                instance.instance = None;
            }
            let descriptor = entry.catalog.clone();
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
            .provider_initialize(ProviderInitializeRequest {
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
                    let process_exit = process
                        .force_kill("Provider graceful shutdown failed")
                        .await
                        .ok();
                    let mut plugins = self.inner.plugins.write().await;
                    if let Some(entry) = plugins.get_mut(plugin_id) {
                        if entry
                            .process
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &process))
                        {
                            entry.process = None;
                        }
                        entry.process_exit = process_exit;
                        entry.stderr_diagnostics = stderr_diagnostics;
                    }
                    drop(plugins);
                    self.set_plugin_state(
                        plugin_id,
                        PluginRuntimeState::Crashed,
                        Some(error.clone()),
                    )
                    .await?;
                    return Err(error);
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

    async fn create_instance_record(
        &self,
        record: &ProviderInstanceRecord,
    ) -> HostResult<ProviderInstance> {
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

    pub async fn start_instance(
        &self,
        route: &ProviderInstanceRoute,
    ) -> HostResult<ProviderInstance> {
        let (record, process, _) = self.instance_context(route).await?;
        let response = process
            .client()
            .instance_start(InstanceStartRequest {
                route: route.clone(),
            })
            .await
            .map_err(HostError::from)?;
        validate_instance_response(&record, &response.instance)?;
        self.set_runtime_instance(&record.plugin_id, response.instance.clone())
            .await?;
        Ok(response.instance)
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

    pub async fn conversation_list(
        &self,
        request: ConversationListRequest,
    ) -> HostResult<ConversationListResponse> {
        let route = request.route.clone();
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationList)?;
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

    pub async fn conversation_get(
        &self,
        request: ConversationGetRequest,
    ) -> HostResult<ConversationGetResponse> {
        validate_resource_identity(&request.conversation)?;
        let expected = request.conversation.clone();
        let route = route_from_resource(&expected);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ConversationGet)?;
        let response = process
            .client()
            .conversation_get(request)
            .await
            .map_err(HostError::from)?;
        validate_conversation_routes(&response.conversation, &route)?;
        validate_exact_resource(&response.conversation.resource, &expected, "conversation.get")?;
        Ok(response)
    }

    pub async fn conversation_create(
        &self,
        request: ConversationCreateRequest,
    ) -> HostResult<ConversationCreateResponse> {
        validate_route_identity(&request.route)?;
        let route = request.route.clone();
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
        validate_resource_identity(&request.conversation)?;
        let expected_conversation = request.conversation.clone();
        let route = route_from_resource(&expected_conversation);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::TurnStart)?;
        let response = process
            .client()
            .turn_start(request)
            .await
            .map_err(HostError::from)?;
        validate_turn_routes(&response.turn, &route)?;
        validate_exact_resource(
            &response.turn.conversation,
            &expected_conversation,
            "turn.start conversation",
        )?;
        Ok(response)
    }

    pub async fn turn_steer(&self, request: TurnSteerRequest) -> HostResult<TurnSteerResponse> {
        validate_resource_identity(&request.turn)?;
        let expected_turn = request.turn.clone();
        let route = route_from_resource(&expected_turn);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::TurnSteer)?;
        let response = process
            .client()
            .turn_steer(request)
            .await
            .map_err(HostError::from)?;
        validate_turn_routes(&response.turn, &route)?;
        validate_exact_resource(&response.turn.resource, &expected_turn, "turn.steer")?;
        Ok(response)
    }

    pub async fn turn_interrupt(
        &self,
        request: TurnInterruptRequest,
    ) -> HostResult<TurnInterruptResponse> {
        validate_resource_identity(&request.turn)?;
        let expected_turn = request.turn.clone();
        let route = route_from_resource(&expected_turn);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::TurnInterrupt)?;
        let response = process
            .client()
            .turn_interrupt(request)
            .await
            .map_err(HostError::from)?;
        validate_turn_routes(&response.turn, &route)?;
        validate_exact_resource(&response.turn.resource, &expected_turn, "turn.interrupt")?;
        Ok(response)
    }

    pub async fn approval_resolve(
        &self,
        request: ApprovalResolveRequest,
    ) -> HostResult<ApprovalResolveResponse> {
        validate_resource_identity(&request.approval)?;
        let expected_approval = request.approval.clone();
        let route = route_from_resource(&expected_approval);
        let (_, process, instance) = self.routing_context(&route).await?;
        ensure_capability(&instance, ProtocolMethod::ApprovalResolve)?;
        let response = process
            .client()
            .approval_resolve(request)
            .await
            .map_err(HostError::from)?;
        validate_approval_routes(&response.approval, &route)?;
        validate_exact_resource(
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
        let route = event_route(&event)?;
        let record = self
            .inner
            .instances
            .resolve_route(&route, Some(plugin_id))?;
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
            let previous_status = runtime.instance.as_ref().map(|instance| instance.status);
            runtime.instance = instance;
            (entry.snapshot(), previous_status)
        };
        self.send_update(HostUpdate::InstanceChanged {
            snapshot,
            instance_id: instance_id.to_string(),
            previous_status,
        })
        .await
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

fn event_route(event: &ProtocolEvent) -> HostResult<ProviderInstanceRoute> {
    let route = match event {
        ProtocolEvent::EventInstanceStatusChanged { params, .. } => params.instance.route.clone(),
        ProtocolEvent::EventConversationUpserted { params, .. } => {
            route_from_resource(&params.conversation.resource)
        }
        ProtocolEvent::EventTurnUpserted { params, .. } => {
            route_from_resource(&params.turn.resource)
        }
        ProtocolEvent::EventTurnOutputDelta { params, .. } => route_from_resource(&params.turn),
        ProtocolEvent::EventApprovalRequested { params, .. } => {
            route_from_resource(&params.approval.resource)
        }
        ProtocolEvent::EventApprovalResolved { params, .. } => {
            route_from_resource(&params.approval.resource)
        }
    };
    validate_route_identity(&route)?;
    Ok(route)
}

fn validate_event_routes(
    event: &ProtocolEvent,
    route: &ProviderInstanceRoute,
    record: &ProviderInstanceRecord,
) -> HostResult<()> {
    match event {
        ProtocolEvent::EventInstanceStatusChanged { params, .. } => {
            validate_instance_response(record, &params.instance)
        }
        ProtocolEvent::EventConversationUpserted { params, .. } => {
            validate_conversation_routes(&params.conversation, route)
        }
        ProtocolEvent::EventTurnUpserted { params, .. } => {
            validate_turn_routes(&params.turn, route)
        }
        ProtocolEvent::EventTurnOutputDelta { params, .. } => {
            validate_resource_route(&params.turn, route)
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
    conversation: &codepet_provider_sdk::ProviderConversation,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_resource_route(&conversation.resource, route)?;
    if let Some(turn) = conversation.active_turn.as_ref() {
        validate_turn_routes(turn, route)?;
        validate_exact_resource(
            &turn.conversation,
            &conversation.resource,
            "conversation active turn",
        )?;
    }
    Ok(())
}

fn validate_turn_routes(
    turn: &codepet_provider_sdk::ProviderTurn,
    route: &ProviderInstanceRoute,
) -> HostResult<()> {
    validate_resource_route(&turn.resource, route)?;
    validate_resource_route(&turn.conversation, route)
}

fn validate_approval_routes(
    approval: &codepet_provider_sdk::ProviderApproval,
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
    validate_resource_identity(resource)?;
    if resource.device_id != route.device_id
        || resource.provider_instance_id != route.provider_instance_id
    {
        return Err(HostError::new(
            "provider_resource_route_mismatch",
            "Provider returned a resource for a different device or instance",
        )
        .with_detail("expectedDeviceId", route.device_id.clone())
        .with_detail(
            "expectedProviderInstanceId",
            route.provider_instance_id.clone(),
        )
        .with_detail("actualDeviceId", resource.device_id.clone())
        .with_detail(
            "actualProviderInstanceId",
            resource.provider_instance_id.clone(),
        ));
    }
    Ok(())
}

fn validate_route_identity(route: &ProviderInstanceRoute) -> HostResult<()> {
    if route.device_id.trim().is_empty() || route.provider_instance_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_provider_route",
            "Provider route deviceId and providerInstanceId must not be empty",
        )
        .with_detail("deviceId", route.device_id.clone())
        .with_detail(
            "providerInstanceId",
            route.provider_instance_id.clone(),
        ));
    }
    Ok(())
}

fn validate_resource_identity(resource: &RoutedResourceId) -> HostResult<()> {
    validate_route_identity(&route_from_resource(resource))?;
    if resource.native_resource_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_provider_resource",
            "Provider nativeResourceId must not be empty",
        )
        .with_detail("deviceId", resource.device_id.clone())
        .with_detail(
            "providerInstanceId",
            resource.provider_instance_id.clone(),
        ));
    }
    Ok(())
}

fn validate_exact_resource(
    actual: &RoutedResourceId,
    expected: &RoutedResourceId,
    operation: &str,
) -> HostResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(HostError::new(
        "provider_resource_identity_mismatch",
        format!("Provider {operation} response changed the requested resource identity"),
    )
    .with_detail("expectedNativeResourceId", expected.native_resource_id.clone())
    .with_detail("actualNativeResourceId", actual.native_resource_id.clone()))
}

fn route_from_resource(resource: &RoutedResourceId) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
        provider_instance_id: resource.provider_instance_id.clone(),
    }
}

fn start_cancelled() -> HostError {
    HostError::new(
        "provider_start_cancelled",
        "Provider startup was superseded by shutdown",
    )
    .retryable(true)
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
