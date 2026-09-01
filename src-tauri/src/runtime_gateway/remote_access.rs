use base64::Engine;
use codepet_gateway_sdk::PROTOCOL_VERSION;
use codepet_lan_channel_sdk::{DeviceDescriptor, PairingQrPayload};
use codepet_host::{
    select_remote_lan_ipv4, HostError, PairingStatus, PairingStatusKind,
    ProviderGatewayService,
    RemoteAccessManager, RemoteCredential, RemoteLanAdvertisedEndpoint,
    RemoteLanAdvertisementSource, RemoteLanMdnsAdvertiser, RemoteLanServer,
    RemoteLanServerConfig, RemoteLanServerHandle,
};
use qrcode::render::svg;
use qrcode::QrCode;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::{watch, Mutex};
use tokio::time::timeout;

pub const REMOTE_ADVERTISED_HOST_ENV: &str = "CODEPET_REMOTE_ADVERTISED_HOST";
const REMOTE_LAN_STABLE_PORT: u16 = 47_622;
const PAIRING_MONITOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const MDNS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const NETWORK_MONITOR_INTERVAL: Duration = Duration::from_secs(2);
const NETWORK_MONITOR_DEBOUNCE_OBSERVATIONS: usize = 2;

trait RemoteDiscoveryPublisher: Send {
    fn replace(
        &mut self,
        endpoint: &RemoteLanAdvertisedEndpoint,
        pairing_available: bool,
    ) -> Result<(), HostError>;
    fn shutdown(&mut self) -> Result<(), HostError>;
}

impl RemoteDiscoveryPublisher for RemoteLanMdnsAdvertiser {
    fn replace(
        &mut self,
        endpoint: &RemoteLanAdvertisedEndpoint,
        pairing_available: bool,
    ) -> Result<(), HostError> {
        self.replace_advertised_endpoint(endpoint, pairing_available)
    }

    fn shutdown(&mut self) -> Result<(), HostError> {
        RemoteLanMdnsAdvertiser::shutdown(self)
    }
}

type RemoteDiscoveryPublisherFactory = dyn Fn(
        RemoteLanAdvertisementSource,
        RemoteLanAdvertisedEndpoint,
        bool,
    ) -> Result<Box<dyn RemoteDiscoveryPublisher>, HostError>
    + Send
    + Sync;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteAccessRuntimePhase {
    Starting,
    Available,
    Unavailable,
    Stopping,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessDiagnosticView {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl From<HostError> for RemoteAccessDiagnosticView {
    fn from(error: HostError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessStatusView {
    pub phase: RemoteAccessRuntimePhase,
    pub host_device_id: Option<String>,
    pub display_name: Option<String>,
    pub advertised_host: Option<String>,
    pub https_base_url: Option<String>,
    pub active_session_count: usize,
    pub pairing_available: bool,
    pub diagnostic: Option<RemoteAccessDiagnosticView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingStartView {
    pub pairing_id: String,
    pub expires_at: u64,
    pub qr_svg_data_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingStatusView {
    pub pairing_id: String,
    pub state: PairingStatusKind,
    pub expires_at: u64,
    pub qr_svg_data_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteClientView {
    pub credential_id: String,
    pub remote_client_id: String,
    pub descriptor: DeviceDescriptor,
    pub created_at: u64,
    pub last_seen_at: u64,
    pub revoked_at: Option<u64>,
    pub online_session_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCredentialRevokeView {
    pub credential_id: String,
    pub revoked_at: Option<u64>,
    pub disconnected_session_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommandError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl From<HostError> for RemoteCommandError {
    fn from(error: HostError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        }
    }
}

struct RemoteAccessRuntimeInner {
    phase: RemoteAccessRuntimePhase,
    listener: Option<RemoteLanServerHandle>,
    mdns: Option<Box<dyn RemoteDiscoveryPublisher>>,
    pairing_available: bool,
    listener_diagnostic: Option<RemoteAccessDiagnosticView>,
    discovery_diagnostic: Option<RemoteAccessDiagnosticView>,
}

struct ActivePairingPayload {
    pairing_id: String,
    json: String,
    qr_svg_data_url: Option<String>,
}

struct EncodedPairingPayload {
    json: String,
    qr_svg_data_url: String,
}

#[derive(Clone)]
pub struct RemoteAccessRuntime {
    manager: Option<Arc<RemoteAccessManager>>,
    gateway: Option<Arc<ProviderGatewayService>>,
    inner: Arc<Mutex<RemoteAccessRuntimeInner>>,
    active_pairing_payload: Arc<StdMutex<Option<ActivePairingPayload>>>,
    lifecycle: Arc<Mutex<()>>,
    pairing_monitor: Arc<StdMutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    pairing_shutdown: watch::Sender<bool>,
    network_monitor: Arc<StdMutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    network_shutdown: watch::Sender<bool>,
    shutdown_completed: Arc<AtomicBool>,
    advertised_host_resolver:
        Arc<dyn Fn() -> Result<std::net::Ipv4Addr, HostError> + Send + Sync>,
    preferred_port: u16,
    mdns_enabled: bool,
    network_monitor_interval: Option<Duration>,
    discovery_publisher_factory: Arc<RemoteDiscoveryPublisherFactory>,
}

impl RemoteAccessRuntime {
    pub fn new(
        manager: Arc<RemoteAccessManager>,
        gateway: Arc<ProviderGatewayService>,
    ) -> Self {
        let (pairing_shutdown, _) = watch::channel(false);
        let (network_shutdown, _) = watch::channel(false);
        Self {
            manager: Some(manager),
            gateway: Some(gateway),
            inner: Arc::new(Mutex::new(RemoteAccessRuntimeInner {
                phase: RemoteAccessRuntimePhase::Unavailable,
                listener: None,
                mdns: None,
                pairing_available: false,
                listener_diagnostic: None,
                discovery_diagnostic: Some(RemoteAccessDiagnosticView {
                    code: "remote_access_not_started".to_string(),
                    message: "Remote LAN access has not started yet".to_string(),
                    retryable: true,
                }),
            })),
            active_pairing_payload: Arc::new(StdMutex::new(None)),
            lifecycle: Arc::new(Mutex::new(())),
            pairing_monitor: Arc::new(StdMutex::new(None)),
            pairing_shutdown,
            network_monitor: Arc::new(StdMutex::new(None)),
            network_shutdown,
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            advertised_host_resolver: Arc::new(select_advertised_host_from_environment),
            preferred_port: REMOTE_LAN_STABLE_PORT,
            mdns_enabled: true,
            network_monitor_interval: Some(NETWORK_MONITOR_INTERVAL),
            discovery_publisher_factory: Arc::new(|source, endpoint, pairing_available| {
                RemoteLanMdnsAdvertiser::start_for_endpoint(
                    source,
                    endpoint,
                    pairing_available,
                )
                .map(|publisher| {
                    Box::new(publisher) as Box<dyn RemoteDiscoveryPublisher>
                })
            }),
        }
    }

    pub fn unavailable(error: HostError) -> Self {
        let (pairing_shutdown, _) = watch::channel(false);
        let (network_shutdown, _) = watch::channel(false);
        Self {
            manager: None,
            gateway: None,
            inner: Arc::new(Mutex::new(RemoteAccessRuntimeInner {
                phase: RemoteAccessRuntimePhase::Unavailable,
                listener: None,
                mdns: None,
                pairing_available: false,
                listener_diagnostic: None,
                discovery_diagnostic: Some(error.into()),
            })),
            active_pairing_payload: Arc::new(StdMutex::new(None)),
            lifecycle: Arc::new(Mutex::new(())),
            pairing_monitor: Arc::new(StdMutex::new(None)),
            pairing_shutdown,
            network_monitor: Arc::new(StdMutex::new(None)),
            network_shutdown,
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            advertised_host_resolver: Arc::new(|| {
                Err(HostError::new(
                    "remote_access_core_unavailable",
                    "Remote access cannot run because the shared Provider Host core is unavailable",
                ))
            }),
            preferred_port: REMOTE_LAN_STABLE_PORT,
            mdns_enabled: true,
            network_monitor_interval: Some(NETWORK_MONITOR_INTERVAL),
            discovery_publisher_factory: Arc::new(|source, endpoint, pairing_available| {
                RemoteLanMdnsAdvertiser::start_for_endpoint(
                    source,
                    endpoint,
                    pairing_available,
                )
                .map(|publisher| {
                    Box::new(publisher) as Box<dyn RemoteDiscoveryPublisher>
                })
            }),
        }
    }

    #[cfg(test)]
    fn new_for_test(
        manager: Arc<RemoteAccessManager>,
        gateway: Arc<ProviderGatewayService>,
        advertised_host_resolver: Arc<
            dyn Fn() -> Result<std::net::Ipv4Addr, HostError> + Send + Sync,
        >,
        mdns_enabled: bool,
    ) -> Self {
        let mut runtime = Self::new(manager, gateway);
        runtime.advertised_host_resolver = advertised_host_resolver;
        runtime.preferred_port = 0;
        runtime.mdns_enabled = mdns_enabled;
        runtime.network_monitor_interval = None;
        runtime
    }

    pub fn start_in_background(&self) {
        self.ensure_pairing_monitor();
        self.ensure_network_monitor();
        let runtime = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = runtime.retry().await {
                crate::app_log::error(
                    "remote_access",
                    &format!(
                        "Remote LAN access startup failed code={} message={}",
                        error.code, error.message
                    ),
                );
            }
        });
    }

    pub async fn status(&self) -> RemoteAccessStatusView {
        let inner = self.inner.lock().await;
        let identity = self.manager.as_ref().map(|manager| manager.remote_host_identity());
        RemoteAccessStatusView {
            phase: inner.phase,
            host_device_id: identity.as_ref().map(|identity| identity.device_id.clone()),
            display_name: identity
                .as_ref()
                .map(|identity| identity.descriptor.device_name.clone()),
            advertised_host: inner
                .listener
                .as_ref()
                .and_then(RemoteLanServerHandle::advertised_host),
            https_base_url: inner
                .listener
                .as_ref()
                .and_then(RemoteLanServerHandle::https_base_url),
            active_session_count: inner
                .listener
                .as_ref()
                .map(RemoteLanServerHandle::active_session_count)
                .unwrap_or(0),
            pairing_available: inner.pairing_available,
            diagnostic: Self::diagnostic_locked(&inner),
        }
    }

    pub async fn retry(&self) -> Result<RemoteAccessStatusView, RemoteCommandError> {
        self.ensure_pairing_monitor();
        self.ensure_network_monitor();
        let _lifecycle = self.lifecycle.lock().await;
        let already_available = {
            let inner = self.inner.lock().await;
            match inner.phase {
                RemoteAccessRuntimePhase::Available => true,
                RemoteAccessRuntimePhase::Stopping | RemoteAccessRuntimePhase::Stopped => {
                    return Err(runtime_stopped_error())
                }
                RemoteAccessRuntimePhase::Starting | RemoteAccessRuntimePhase::Unavailable => false,
            }
        };
        if already_available {
            self.reconcile_network_observation_locked(
                (self.advertised_host_resolver)(),
            )
            .await?;
            let inner = self.inner.lock().await;
            return Ok(self.status_locked(&inner));
        }
        let manager = self.manager.clone().ok_or_else(runtime_core_unavailable)?;
        let gateway = self.gateway.clone().ok_or_else(runtime_core_unavailable)?;
        {
            let mut inner = self.inner.lock().await;
            inner.phase = RemoteAccessRuntimePhase::Starting;
            inner.discovery_diagnostic = None;
        }

        let advertised_host = match (self.advertised_host_resolver)() {
            Ok(host) => host,
            Err(error) => return self.fail_start(error).await,
        };
        let config = RemoteLanServerConfig {
            bind_addr: std::net::SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
                self.preferred_port,
            ),
            advertised_host: Some(advertised_host.to_string()),
        };
        let (listener, listener_diagnostic) = match RemoteLanServer::start(
            config,
            manager.clone(),
            gateway.clone(),
        )
        .await
        {
            Ok(listener) => (listener, None),
            Err(error)
                if self.preferred_port != 0
                    && error.code == "remote_lan_listener_address_in_use" =>
            {
                let diagnostic = RemoteAccessDiagnosticView {
                    code: "remote_lan_stable_port_unavailable".to_string(),
                    message: format!(
                        "Remote LAN port {} is occupied; this launch uses an ephemeral port advertised through QR and mDNS",
                        self.preferred_port
                    ),
                    retryable: true,
                };
                crate::app_log::warn("remote_access", &diagnostic.message);
                let fallback = RemoteLanServerConfig::default()
                    .with_advertised_host(advertised_host.to_string());
                match RemoteLanServer::start(fallback, manager.clone(), gateway).await {
                    Ok(listener) => (listener, Some(diagnostic)),
                    Err(error) => return self.fail_start(error).await,
                }
            }
            Err(error) => return self.fail_start(error).await,
        };
        let pairing_available = manager
            .subscribe_pairing_state()
            .borrow()
            .pairing_available;
        let mut discovery_diagnostic = None;
        let mdns = if self.mdns_enabled {
            let endpoint = listener.advertised_endpoint().ok_or_else(|| {
                RemoteCommandError::from(HostError::new(
                    "remote_lan_discovery_unavailable",
                    "Remote LAN listener has no advertised endpoint",
                )
                .retryable(true))
            })?;
            let source = listener.advertisement_source();
            let factory = self.discovery_publisher_factory.clone();
            match tauri::async_runtime::spawn_blocking(move || {
                factory(source, endpoint, pairing_available)
            })
            .await
            {
                Ok(Ok(mdns)) => Some(mdns),
                Ok(Err(error)) => {
                    listener.withdraw_advertised_endpoint();
                    crate::app_log::error(
                        "remote_access",
                        &format!(
                            "Remote LAN discovery startup failed without stopping the listener code={} message={}",
                            error.code, error.message
                        ),
                    );
                    discovery_diagnostic = Some(error.into());
                    None
                }
                Err(error) => {
                    listener.withdraw_advertised_endpoint();
                    let error = HostError::new(
                        "remote_lan_mdns_task_failed",
                        format!("Remote LAN mDNS startup task failed: {error}"),
                    )
                    .retryable(true);
                    crate::app_log::error("remote_access", &error.message);
                    discovery_diagnostic = Some(error.into());
                    None
                }
            }
        } else {
            None
        };

        let mut inner = self.inner.lock().await;
        inner.phase = RemoteAccessRuntimePhase::Available;
        inner.listener = Some(listener);
        inner.mdns = mdns;
        inner.pairing_available = inner.mdns.is_some().then_some(pairing_available).unwrap_or(
            if self.mdns_enabled { false } else { pairing_available },
        );
        inner.listener_diagnostic = listener_diagnostic;
        inner.discovery_diagnostic = discovery_diagnostic;
        Ok(self.status_locked(&inner))
    }

    pub async fn start_pairing(&self) -> Result<RemotePairingStartView, RemoteCommandError> {
        let (https_base_url, identity) = {
            let inner = self.inner.lock().await;
            if inner.phase != RemoteAccessRuntimePhase::Available {
                return Err(runtime_unavailable_error(&inner));
            }
            let listener = inner.listener.as_ref().ok_or_else(runtime_core_unavailable)?;
            let manager = self.manager.as_ref().ok_or_else(runtime_core_unavailable)?;
            (
                listener
                    .https_base_url()
                    .ok_or_else(|| runtime_unavailable_error(&inner))?,
                manager.remote_host_identity(),
            )
        };
        let manager = self.manager.as_ref().ok_or_else(runtime_core_unavailable)?;
        let pairing = manager.begin_pairing().map_err(RemoteCommandError::from)?;
        let payload = PairingQrPayload {
            version: u64::from(PROTOCOL_VERSION),
            host_device_id: identity.device_id,
            display_name: identity.descriptor.device_name,
            https_base_url,
            cert_sha256: identity.identity_fingerprint,
            pairing_id: pairing.pairing_id.clone(),
            pairing_secret: pairing.pairing_secret,
            expires_at: pairing.expires_at,
        };
        let encoded = match encode_pairing_payload(&payload) {
            Ok(value) => value,
            Err(error) => {
                let _ = manager.cancel_pairing(&pairing.pairing_id);
                return Err(error);
            }
        };
        if self.store_pairing_payload(&pairing.pairing_id, &encoded).is_err() {
            let _ = manager.cancel_pairing(&pairing.pairing_id);
            return Err(pairing_payload_unavailable());
        }
        if let Err(error) = self.sync_pairing_available().await {
            let _ = manager.cancel_pairing(&pairing.pairing_id);
            self.clear_pairing_payload(&pairing.pairing_id);
            return Err(error);
        }
        let qr_svg_data_url = self.active_pairing_qr(&pairing.pairing_id)?;
        Ok(RemotePairingStartView {
            pairing_id: pairing.pairing_id,
            expires_at: pairing.expires_at,
            qr_svg_data_url,
        })
    }

    pub async fn pairing_status(
        &self,
        pairing_id: &str,
    ) -> Result<RemotePairingStatusView, RemoteCommandError> {
        let _generation = self.inner.lock().await;
        let status = self.manager
            .as_ref()
            .ok_or_else(runtime_core_unavailable)?
            .pairing_status(pairing_id)
            .map_err(RemoteCommandError::from)?;
        let qr_svg_data_url = (status.state == PairingStatusKind::Active)
            .then(|| {
                self.active_pairing_payload
                    .lock()
                    .ok()
                    .and_then(|payload| {
                        payload
                            .as_ref()
                            .filter(|payload| payload.pairing_id == pairing_id)
                            .and_then(|payload| payload.qr_svg_data_url.clone())
                    })
            })
            .flatten();
        Ok(RemotePairingStatusView {
            pairing_id: status.pairing_id,
            state: status.state,
            expires_at: status.expires_at,
            qr_svg_data_url,
        })
    }

    pub fn copy_pairing_json(
        &self,
        pairing_id: &str,
        write: impl FnOnce(&str) -> Result<(), RemoteCommandError>,
    ) -> Result<(), RemoteCommandError> {
        let json = self.pairing_payload(pairing_id)?;
        let manager = self.manager.as_ref().ok_or_else(runtime_core_unavailable)?;
        match manager.run_while_pairing_active(pairing_id, || write(&json)) {
            Ok(result) => result,
            Err(_) => {
                self.clear_pairing_payload(pairing_id);
                Err(pairing_payload_unavailable())
            }
        }
    }

    pub async fn cancel_pairing(
        &self,
        pairing_id: &str,
    ) -> Result<PairingStatus, RemoteCommandError> {
        let status = self
            .manager
            .as_ref()
            .ok_or_else(runtime_core_unavailable)?
            .cancel_pairing(pairing_id)
            .map_err(RemoteCommandError::from)?;
        self.clear_pairing_payload(pairing_id);
        self.sync_pairing_available().await?;
        Ok(status)
    }

    pub async fn list_clients(&self) -> Result<Vec<RemoteClientView>, RemoteCommandError> {
        let credentials = self
            .manager
            .as_ref()
            .ok_or_else(runtime_core_unavailable)?
            .list_credentials()
            .map_err(RemoteCommandError::from)?;
        let inner = self.inner.lock().await;
        Ok(project_remote_clients(credentials, |credential_id| {
            inner
                .listener
                .as_ref()
                .map(|listener| {
                    listener.active_session_count_for_credential(credential_id)
                })
                .unwrap_or(0)
        }))
    }

    pub async fn revoke_credential(
        &self,
        credential_id: &str,
    ) -> Result<RemoteCredentialRevokeView, RemoteCommandError> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(runtime_core_unavailable)?;
        let credentials = manager
            .list_credentials()
            .map_err(RemoteCommandError::from)?;
        let target = credentials
            .iter()
            .find(|credential| credential.credential_id == credential_id)
            .ok_or_else(remote_credential_not_found)?;
        let client_id = target.client_id.clone();
        manager
            .revoke_client(&client_id)
            .map_err(RemoteCommandError::from)?;
        let client_credentials = manager
            .list_credentials()
            .map_err(RemoteCommandError::from)?
            .into_iter()
            .filter(|credential| credential.client_id == client_id)
            .collect::<Vec<_>>();
        let revoked_at = client_credentials
            .iter()
            .find(|credential| credential.credential_id == credential_id)
            .and_then(|credential| credential.revoked_at);
        let disconnected_session_count = {
            let inner = self.inner.lock().await;
            match inner.listener.as_ref() {
                Some(listener) => listener
                    .disconnect_credentials(
                        client_credentials
                            .iter()
                            .map(|credential| credential.credential_id.as_str()),
                    )
                    .await
                    .map_err(RemoteCommandError::from)?,
                None => 0,
            }
        };
        Ok(RemoteCredentialRevokeView {
            credential_id: credential_id.to_string(),
            revoked_at,
            disconnected_session_count,
        })
    }

    pub fn shutdown_completed(&self) -> bool {
        self.shutdown_completed.load(Ordering::SeqCst)
    }

    pub async fn shutdown_once(&self) -> bool {
        let _lifecycle = self.lifecycle.lock().await;
        if self.shutdown_completed() {
            return true;
        }
        self.pairing_shutdown.send_replace(true);
        self.network_shutdown.send_replace(true);
        self.clear_all_pairing_payload();
        let monitor = self
            .pairing_monitor
            .lock()
            .ok()
            .and_then(|mut monitor| monitor.take());
        if let Some(mut monitor) = monitor {
            if timeout(PAIRING_MONITOR_SHUTDOWN_TIMEOUT, &mut monitor)
                .await
                .is_err()
            {
                monitor.abort();
                let _ = monitor.await;
            }
        }
        let network_monitor = self
            .network_monitor
            .lock()
            .ok()
            .and_then(|mut monitor| monitor.take());
        if let Some(mut monitor) = network_monitor {
            if timeout(PAIRING_MONITOR_SHUTDOWN_TIMEOUT, &mut monitor)
                .await
                .is_err()
            {
                monitor.abort();
                let _ = monitor.await;
            }
        }
        let (listener, mdns) = {
            let mut inner = self.inner.lock().await;
            inner.phase = RemoteAccessRuntimePhase::Stopping;
            inner.pairing_available = false;
            (inner.listener.take(), inner.mdns.take())
        };
        if let Some(mut mdns) = mdns {
            let shutdown = tauri::async_runtime::spawn_blocking(move || mdns.shutdown());
            if timeout(MDNS_SHUTDOWN_TIMEOUT, shutdown).await.is_err() {
                crate::app_log::error(
                    "remote_access",
                    "Remote LAN mDNS exceeded its bounded shutdown window",
                );
            }
        }
        if let Some(listener) = listener {
            if let Err(error) = listener.shutdown().await {
                crate::app_log::error(
                    "remote_access",
                    &format!("Remote LAN listener shutdown failed error={error:?}"),
                );
            }
        }
        let mut inner = self.inner.lock().await;
        inner.phase = RemoteAccessRuntimePhase::Stopped;
        inner.listener_diagnostic = None;
        inner.discovery_diagnostic = None;
        self.shutdown_completed.store(true, Ordering::SeqCst);
        true
    }

    fn ensure_pairing_monitor(&self) {
        if self.shutdown_completed() || *self.pairing_shutdown.borrow() {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let Ok(mut monitor) = self.pairing_monitor.lock() else {
            return;
        };
        if monitor.is_some() {
            return;
        }
        let runtime = self.clone();
        let mut pairing = manager.subscribe_pairing_state();
        let mut shutdown = self.pairing_shutdown.subscribe();
        *monitor = Some(tauri::async_runtime::spawn(async move {
            loop {
                if *shutdown.borrow() {
                    break;
                }
                let snapshot = pairing.borrow().clone();
                let deadline = snapshot.deadline;
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                    changed = pairing.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        runtime.clear_inactive_pairing_payload();
                        if let Err(error) = runtime.sync_pairing_available().await {
                            crate::app_log::error(
                                "remote_access",
                                &format!("failed to synchronize pairing discovery code={} message={}", error.code, error.message),
                            );
                        }
                    }
                    _ = async {
                        if let Some(deadline) = deadline {
                            tokio::time::sleep_until(deadline.into()).await;
                        }
                    }, if deadline.is_some() => {
                        if let Some(pairing_id) = snapshot.pairing_id.as_deref() {
                            let _ = manager.expire_pairing(pairing_id);
                        }
                    }
                }
            }
        }));
    }

    fn ensure_network_monitor(&self) {
        let Some(interval) = self.network_monitor_interval else {
            return;
        };
        if self.manager.is_none() || self.gateway.is_none() {
            return;
        }
        if self.shutdown_completed() || *self.network_shutdown.borrow() {
            return;
        }
        let Ok(mut monitor) = self.network_monitor.lock() else {
            return;
        };
        if monitor.is_some() {
            return;
        }
        let runtime = self.clone();
        let resolver = self.advertised_host_resolver.clone();
        let mut shutdown = self.network_shutdown.subscribe();
        *monitor = Some(tauri::async_runtime::spawn(async move {
            let mut observed = None;
            let mut repeated = 0_usize;
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(interval) => {}
                }
                if *shutdown.borrow() {
                    break;
                }
                let observation = resolver();
                let key = observation.as_ref().ok().copied();
                if observed == Some(key) {
                    repeated = repeated.saturating_add(1);
                } else {
                    observed = Some(key);
                    repeated = 1;
                }
                if repeated < NETWORK_MONITOR_DEBOUNCE_OBSERVATIONS {
                    continue;
                }

                let phase = runtime.inner.lock().await.phase;
                match phase {
                    RemoteAccessRuntimePhase::Available => {
                        if let Err(error) = runtime
                            .reconcile_network_observation(observation)
                            .await
                        {
                            crate::app_log::warn(
                                "remote_access",
                                &format!(
                                    "Remote LAN discovery will retry code={} message={}",
                                    error.code, error.message
                                ),
                            );
                        }
                    }
                    RemoteAccessRuntimePhase::Unavailable if observation.is_ok() => {
                        if let Err(error) = runtime.retry().await {
                            crate::app_log::warn(
                                "remote_access",
                                &format!(
                                    "Remote LAN listener recovery will retry code={} message={}",
                                    error.code, error.message
                                ),
                            );
                        }
                    }
                    RemoteAccessRuntimePhase::Starting
                    | RemoteAccessRuntimePhase::Unavailable
                    | RemoteAccessRuntimePhase::Stopping
                    | RemoteAccessRuntimePhase::Stopped => {}
                }
            }
        }));
    }

    async fn reconcile_network_observation(
        &self,
        observation: Result<Ipv4Addr, HostError>,
    ) -> Result<(), RemoteCommandError> {
        let _lifecycle = self.lifecycle.lock().await;
        self.reconcile_network_observation_locked(observation).await
    }

    async fn reconcile_network_observation_locked(
        &self,
        observation: Result<Ipv4Addr, HostError>,
    ) -> Result<(), RemoteCommandError> {
        match observation {
            Ok(advertised_host) => {
                let pairing_available = self.current_pairing_available()?;
                self.publish_generation_locked(advertised_host, pairing_available)
                    .await
            }
            Err(error) => self.withdraw_advertisement_locked(error).await,
        }
    }

    async fn withdraw_advertisement_locked(
        &self,
        error: HostError,
    ) -> Result<(), RemoteCommandError> {
        let error = error.retryable(true);
        let (publisher, had_endpoint, already_reported) = {
            let mut inner = self.inner.lock().await;
            if inner.phase != RemoteAccessRuntimePhase::Available {
                return Err(runtime_unavailable_error(&inner));
            }
            let had_endpoint = inner
                .listener
                .as_ref()
                .and_then(RemoteLanServerHandle::advertised_endpoint)
                .is_some();
            let already_reported = !had_endpoint
                && inner.mdns.is_none()
                && inner
                    .discovery_diagnostic
                    .as_ref()
                    .is_some_and(|diagnostic| diagnostic.code == error.code);
            (inner.mdns.take(), had_endpoint, already_reported)
        };
        if already_reported {
            return Ok(());
        }
        if let Some(mut publisher) = publisher {
            let shutdown = tauri::async_runtime::spawn_blocking(move || {
                publisher.shutdown()
            });
            match timeout(MDNS_SHUTDOWN_TIMEOUT, shutdown).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(shutdown_error))) => crate::app_log::warn(
                    "remote_access",
                    &format!(
                        "Remote LAN discovery withdrawal reported code={} message={}",
                        shutdown_error.code, shutdown_error.message
                    ),
                ),
                Ok(Err(join_error)) => crate::app_log::warn(
                    "remote_access",
                    &format!("Remote LAN discovery withdrawal task failed: {join_error}"),
                ),
                Err(_) => crate::app_log::warn(
                    "remote_access",
                    "Remote LAN discovery withdrawal exceeded its bounded window",
                ),
            }
        }

        let mut inner = self.inner.lock().await;
        if let Some(listener) = inner.listener.as_ref() {
            listener.withdraw_advertised_endpoint();
        }
        if let Err(payload_error) = self.update_active_pairing_endpoint(None) {
            crate::app_log::warn("remote_access", &payload_error.message);
        }
        inner.mdns = None;
        inner.pairing_available = false;
        inner.discovery_diagnostic = Some(error.clone().into());
        if had_endpoint {
            crate::app_log::warn(
                "remote_access",
                &format!(
                    "Remote LAN advertised endpoint withdrawn; listener remains active code={} message={}",
                    error.code, error.message
                ),
            );
        }
        Err(error.into())
    }

    async fn discovery_unavailable_error(&self) -> RemoteCommandError {
        let inner = self.inner.lock().await;
        inner
            .discovery_diagnostic
            .as_ref()
            .map(|diagnostic| RemoteCommandError {
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                retryable: true,
            })
            .unwrap_or_else(|| RemoteCommandError {
                code: "remote_lan_discovery_unavailable".to_string(),
                message: "Remote LAN discovery has no current advertised endpoint"
                    .to_string(),
                retryable: true,
            })
    }

    fn current_pairing_available(&self) -> Result<bool, RemoteCommandError> {
        let manager = self.manager.as_ref().ok_or_else(runtime_core_unavailable)?;
        let pairing = manager.subscribe_pairing_state();
        let pairing_available = pairing.borrow().pairing_available;
        Ok(pairing_available)
    }

    fn store_pairing_payload(
        &self,
        pairing_id: &str,
        encoded: &EncodedPairingPayload,
    ) -> Result<(), RemoteCommandError> {
        let mut payload = self
            .active_pairing_payload
            .lock()
            .map_err(|_| pairing_payload_unavailable())?;
        *payload = Some(ActivePairingPayload {
            pairing_id: pairing_id.to_string(),
            json: encoded.json.clone(),
            qr_svg_data_url: Some(encoded.qr_svg_data_url.clone()),
        });
        Ok(())
    }

    fn pairing_payload(&self, pairing_id: &str) -> Result<String, RemoteCommandError> {
        self.active_pairing_payload
            .lock()
            .map_err(|_| pairing_payload_unavailable())?
            .as_ref()
            .filter(|payload| {
                payload.pairing_id == pairing_id
                    && payload.qr_svg_data_url.is_some()
            })
            .map(|payload| payload.json.clone())
            .ok_or_else(pairing_payload_unavailable)
    }

    fn active_pairing_qr(
        &self,
        pairing_id: &str,
    ) -> Result<String, RemoteCommandError> {
        self.active_pairing_payload
            .lock()
            .map_err(|_| pairing_payload_unavailable())?
            .as_ref()
            .filter(|payload| payload.pairing_id == pairing_id)
            .and_then(|payload| payload.qr_svg_data_url.clone())
            .ok_or_else(pairing_payload_unavailable)
    }

    fn update_active_pairing_endpoint(
        &self,
        https_base_url: Option<&str>,
    ) -> Result<(), RemoteCommandError> {
        let mut active = self
            .active_pairing_payload
            .lock()
            .map_err(|_| pairing_payload_unavailable())?;
        let Some(active) = active.as_mut() else {
            return Ok(());
        };
        let Some(https_base_url) = https_base_url else {
            active.qr_svg_data_url = None;
            return Ok(());
        };
        let mut payload = serde_json::from_str::<PairingQrPayload>(&active.json)
            .map_err(|_| pairing_payload_unavailable())?;
        payload.https_base_url = https_base_url.to_string();
        let encoded = encode_pairing_payload(&payload)?;
        active.json = encoded.json;
        active.qr_svg_data_url = Some(encoded.qr_svg_data_url);
        Ok(())
    }

    fn clear_pairing_payload(&self, pairing_id: &str) {
        let Ok(mut payload) = self.active_pairing_payload.lock() else {
            return;
        };
        if payload
            .as_ref()
            .is_some_and(|payload| payload.pairing_id == pairing_id)
        {
            *payload = None;
        }
    }

    fn clear_all_pairing_payload(&self) {
        if let Ok(mut payload) = self.active_pairing_payload.lock() {
            *payload = None;
        }
    }

    fn clear_inactive_pairing_payload(&self) {
        let active_pairing_id = self.manager.as_ref().and_then(|manager| {
            let pairing = manager.subscribe_pairing_state();
            let snapshot = pairing.borrow().clone();
            snapshot.pairing_available.then_some(snapshot.pairing_id).flatten()
        });
        let Ok(mut payload) = self.active_pairing_payload.lock() else {
            return;
        };
        if payload
            .as_ref()
            .is_some_and(|payload| Some(&payload.pairing_id) != active_pairing_id.as_ref())
        {
            *payload = None;
        }
    }

    async fn sync_pairing_available(&self) -> Result<(), RemoteCommandError> {
        let _lifecycle = self.lifecycle.lock().await;
        loop {
            let pairing_available = self.current_pairing_available()?;
            let advertised_host = {
                let inner = self.inner.lock().await;
                if inner.phase != RemoteAccessRuntimePhase::Available {
                    return if pairing_available {
                        Err(runtime_unavailable_error(&inner))
                    } else {
                        Ok(())
                    };
                }
                inner
                    .listener
                    .as_ref()
                    .and_then(RemoteLanServerHandle::advertised_host)
            };
            let Some(advertised_host) = advertised_host else {
                return if pairing_available {
                    Err(self.discovery_unavailable_error().await)
                } else {
                    let mut inner = self.inner.lock().await;
                    inner.pairing_available = false;
                    Ok(())
                };
            };
            let advertised_host = advertised_host.parse::<Ipv4Addr>().map_err(|_| {
                RemoteCommandError::from(HostError::new(
                    "invalid_remote_lan_advertised_host",
                    "Remote LAN runtime advertised endpoint must be one IPv4 address",
                ))
            })?;
            self.publish_generation_locked(advertised_host, pairing_available)
                .await?;
            if self.current_pairing_available()? == pairing_available {
                return Ok(());
            }
        }
    }

    async fn publish_generation_locked(
        &self,
        advertised_host: Ipv4Addr,
        pairing_available: bool,
    ) -> Result<(), RemoteCommandError> {
        let advertised_host = advertised_host.to_string();
        let (source, endpoint, transition, publisher) = {
            let mut inner = self.inner.lock().await;
            if inner.phase != RemoteAccessRuntimePhase::Available {
                return Err(runtime_unavailable_error(&inner));
            }
            let listener = inner.listener.as_ref().ok_or_else(runtime_core_unavailable)?;
            let current = listener.advertised_endpoint();
            let endpoint_matches = current
                .as_ref()
                .is_some_and(|endpoint| endpoint.advertised_host() == advertised_host);
            let publisher_is_current = if self.mdns_enabled {
                inner.mdns.is_some()
            } else {
                true
            };
            if endpoint_matches
                && publisher_is_current
                && inner.pairing_available == pairing_available
            {
                return Ok(());
            }
            let transition = if endpoint_matches {
                None
            } else {
                Some(listener.stage_advertised_host(&advertised_host)?)
            };
            let endpoint = transition
                .as_ref()
                .map(|transition| transition.endpoint().clone())
                .or(current)
                .ok_or_else(runtime_core_unavailable)?;
            let source = listener.advertisement_source();
            let publisher = inner.mdns.take();
            (source, endpoint, transition, publisher)
        };

        if !self.mdns_enabled {
            let mut inner = self.inner.lock().await;
            if let Err(error) = self.update_active_pairing_endpoint(
                Some(endpoint.https_base_url()),
            ) {
                self.clear_all_pairing_payload();
                crate::app_log::error("remote_access", &error.message);
            }
            if let Some(transition) = transition {
                transition.commit().map_err(RemoteCommandError::from)?;
            }
            inner.pairing_available = pairing_available;
            inner.discovery_diagnostic = None;
            return Ok(());
        }

        let factory = self.discovery_publisher_factory.clone();
        let published_endpoint = endpoint.clone();
        let published = tauri::async_runtime::spawn_blocking(move || {
            match publisher {
                Some(mut publisher) => match publisher.replace(
                    &published_endpoint,
                    pairing_available,
                ) {
                    Ok(()) => Ok(publisher),
                    Err(error) => {
                        let _ = publisher.shutdown();
                        Err(error)
                    }
                },
                None => factory(source, published_endpoint, pairing_available),
            }
        })
        .await
        .map_err(|error| {
            RemoteCommandError::from(HostError::new(
                "remote_lan_mdns_task_failed",
                format!("Remote LAN mDNS generation task failed: {error}"),
            )
            .retryable(true))
        });

        let mut inner = self.inner.lock().await;
        match published {
            Ok(Ok(publisher)) => {
                if let Err(error) = self.update_active_pairing_endpoint(
                    Some(endpoint.https_base_url()),
                ) {
                    self.clear_all_pairing_payload();
                    crate::app_log::error("remote_access", &error.message);
                }
                if let Some(transition) = transition {
                    if let Err(error) = transition.commit() {
                        drop(inner);
                        let mut publisher = publisher;
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            publisher.shutdown()
                        })
                        .await;
                        return Err(error.into());
                    }
                }
                inner.mdns = Some(publisher);
                inner.pairing_available = pairing_available;
                inner.discovery_diagnostic = None;
                Ok(())
            }
            Ok(Err(error)) => {
                if let Some(transition) = transition {
                    transition.fail_closed();
                }
                inner.mdns = None;
                inner.pairing_available = false;
                inner.discovery_diagnostic = Some(error.clone().into());
                crate::app_log::error(
                    "remote_access",
                    &format!(
                        "Remote LAN discovery generation failed; listener remains active code={} message={}",
                        error.code, error.message
                    ),
                );
                Err(error.into())
            }
            Err(error) => {
                if let Some(transition) = transition {
                    transition.fail_closed();
                }
                inner.mdns = None;
                inner.pairing_available = false;
                inner.discovery_diagnostic = Some(RemoteAccessDiagnosticView {
                    code: error.code.clone(),
                    message: error.message.clone(),
                    retryable: error.retryable,
                });
                crate::app_log::error("remote_access", &error.message);
                Err(error)
            }
        }
    }

    async fn fail_start<T>(&self, error: HostError) -> Result<T, RemoteCommandError> {
        self.clear_all_pairing_payload();
        let diagnostic = RemoteAccessDiagnosticView::from(error.clone());
        let mut inner = self.inner.lock().await;
        inner.phase = RemoteAccessRuntimePhase::Unavailable;
        inner.listener = None;
        inner.mdns = None;
        inner.pairing_available = false;
        inner.discovery_diagnostic = Some(diagnostic);
        Err(error.into())
    }

    fn status_locked(&self, inner: &RemoteAccessRuntimeInner) -> RemoteAccessStatusView {
        let identity = self.manager.as_ref().map(|manager| manager.remote_host_identity());
        RemoteAccessStatusView {
            phase: inner.phase,
            host_device_id: identity.as_ref().map(|identity| identity.device_id.clone()),
            display_name: identity
                .as_ref()
                .map(|identity| identity.descriptor.device_name.clone()),
            advertised_host: inner
                .listener
                .as_ref()
                .and_then(RemoteLanServerHandle::advertised_host),
            https_base_url: inner
                .listener
                .as_ref()
                .and_then(RemoteLanServerHandle::https_base_url),
            active_session_count: inner
                .listener
                .as_ref()
                .map(RemoteLanServerHandle::active_session_count)
                .unwrap_or(0),
            pairing_available: inner.pairing_available,
            diagnostic: Self::diagnostic_locked(inner),
        }
    }

    fn diagnostic_locked(
        inner: &RemoteAccessRuntimeInner,
    ) -> Option<RemoteAccessDiagnosticView> {
        inner
            .discovery_diagnostic
            .clone()
            .or_else(|| inner.listener_diagnostic.clone())
    }
}

fn project_remote_clients(
    credentials: Vec<RemoteCredential>,
    active_session_count: impl Fn(&str) -> usize,
) -> Vec<RemoteClientView> {
    let mut grouped = BTreeMap::<String, Vec<RemoteCredential>>::new();
    for credential in credentials {
        grouped
            .entry(credential.client_id.clone())
            .or_default()
            .push(credential);
    }

    let mut clients = grouped
        .into_iter()
        .filter_map(|(client_id, credentials)| {
            let representative = credentials.iter().max_by(|left, right| {
                left.revoked_at
                    .is_none()
                    .cmp(&right.revoked_at.is_none())
                    .then_with(|| left.created_at.cmp(&right.created_at))
                    .then_with(|| left.last_seen_at.cmp(&right.last_seen_at))
                    .then_with(|| left.credential_id.cmp(&right.credential_id))
            })?;
            let created_at = credentials
                .iter()
                .map(|credential| credential.created_at)
                .min()?;
            let last_seen_at = credentials
                .iter()
                .filter(|credential| credential.last_seen_at > credential.created_at)
                .map(|credential| credential.last_seen_at)
                .max()
                .unwrap_or(created_at);
            let revoked_at = credentials
                .iter()
                .all(|credential| credential.revoked_at.is_some())
                .then(|| {
                    credentials
                        .iter()
                        .filter_map(|credential| credential.revoked_at)
                        .max()
                })
                .flatten();
            let online_session_count = credentials.iter().fold(0_usize, |total, credential| {
                total.saturating_add(active_session_count(&credential.credential_id))
            });
            Some(RemoteClientView {
                credential_id: representative.credential_id.clone(),
                remote_client_id: client_id,
                descriptor: representative.descriptor.clone(),
                created_at,
                last_seen_at,
                revoked_at,
                online_session_count,
            })
        })
        .collect::<Vec<_>>();
    clients.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.remote_client_id.cmp(&right.remote_client_id))
    });
    clients
}

fn select_advertised_host_from_environment() -> Result<std::net::Ipv4Addr, HostError> {
    let configured = std::env::var_os(REMOTE_ADVERTISED_HOST_ENV);
    let configured = configured
        .as_ref()
        .map(|value| {
            value.to_str().ok_or_else(|| {
                HostError::new(
                    "invalid_remote_lan_advertised_host",
                    "CODEPET_REMOTE_ADVERTISED_HOST must contain a UTF-8 IPv4 address",
                )
            })
        })
        .transpose()?;
    select_remote_lan_ipv4(configured)
}

fn encode_pairing_payload(
    payload: &PairingQrPayload,
) -> Result<EncodedPairingPayload, RemoteCommandError> {
    let json = serde_json::to_string(payload).map_err(|error| RemoteCommandError {
        code: "remote_pairing_qr_encoding_failed".to_string(),
        message: format!("encode the generated pairing QR payload: {error}"),
        retryable: false,
    })?;
    let code = QrCode::new(json.as_bytes()).map_err(|error| RemoteCommandError {
        code: "remote_pairing_qr_encoding_failed".to_string(),
        message: format!("encode the generated pairing QR code: {error}"),
        retryable: false,
    })?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .build();
    let encoded = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    Ok(EncodedPairingPayload {
        json,
        qr_svg_data_url: format!("data:image/svg+xml;base64,{encoded}"),
    })
}

fn pairing_payload_unavailable() -> RemoteCommandError {
    RemoteCommandError {
        code: "remote_pairing_payload_unavailable".to_string(),
        message: "Pairing JSON is only available while the matching pairing is active"
            .to_string(),
        retryable: false,
    }
}

fn pairing_copy_window_not_allowed() -> RemoteCommandError {
    RemoteCommandError {
        code: "remote_pairing_copy_window_not_allowed".to_string(),
        message: "Pairing JSON can only be copied from the main Code Pet window".to_string(),
        retryable: false,
    }
}

fn pairing_clipboard_write_failed() -> RemoteCommandError {
    RemoteCommandError {
        code: "remote_pairing_clipboard_write_failed".to_string(),
        message: "Code Pet could not write the active pairing JSON to the system clipboard"
            .to_string(),
        retryable: true,
    }
}

fn remote_credential_not_found() -> RemoteCommandError {
    RemoteCommandError {
        code: "remote_credential_not_found".to_string(),
        message: "Remote credential does not exist".to_string(),
        retryable: false,
    }
}

fn runtime_core_unavailable() -> RemoteCommandError {
    RemoteCommandError {
        code: "remote_access_core_unavailable".to_string(),
        message: "Remote access cannot run because the shared Provider Host core is unavailable"
            .to_string(),
        retryable: false,
    }
}

fn runtime_stopped_error() -> RemoteCommandError {
    RemoteCommandError {
        code: "remote_access_stopped".to_string(),
        message: "Remote access is shutting down or has stopped".to_string(),
        retryable: false,
    }
}

fn runtime_unavailable_error(inner: &RemoteAccessRuntimeInner) -> RemoteCommandError {
    if inner.phase == RemoteAccessRuntimePhase::Starting {
        return RemoteCommandError {
            code: "remote_access_starting".to_string(),
            message: "Remote LAN access is still starting".to_string(),
            retryable: true,
        };
    }
    if matches!(
        inner.phase,
        RemoteAccessRuntimePhase::Stopping | RemoteAccessRuntimePhase::Stopped
    ) {
        return runtime_stopped_error();
    }
    inner
        .discovery_diagnostic
        .as_ref()
        .map(|diagnostic| RemoteCommandError {
            code: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
            retryable: diagnostic.retryable,
        })
        .unwrap_or_else(runtime_core_unavailable)
}

#[tauri::command]
pub async fn remote_access_status(
    state: State<'_, RemoteAccessRuntime>,
) -> Result<RemoteAccessStatusView, RemoteCommandError> {
    Ok(state.status().await)
}

#[tauri::command]
pub async fn retry_remote_access(
    state: State<'_, RemoteAccessRuntime>,
) -> Result<RemoteAccessStatusView, RemoteCommandError> {
    state.retry().await
}

#[tauri::command]
pub async fn list_remote_clients(
    state: State<'_, RemoteAccessRuntime>,
) -> Result<Vec<RemoteClientView>, RemoteCommandError> {
    state.list_clients().await
}

#[tauri::command]
pub async fn start_remote_pairing(
    state: State<'_, RemoteAccessRuntime>,
) -> Result<RemotePairingStartView, RemoteCommandError> {
    state.start_pairing().await
}

#[tauri::command]
pub async fn get_remote_pairing_status(
    state: State<'_, RemoteAccessRuntime>,
    pairing_id: String,
) -> Result<RemotePairingStatusView, RemoteCommandError> {
    state.pairing_status(&pairing_id).await
}

#[tauri::command]
pub fn copy_remote_pairing_json(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, RemoteAccessRuntime>,
    pairing_id: String,
) -> Result<(), RemoteCommandError> {
    if window.label() != "main" {
        return Err(pairing_copy_window_not_allowed());
    }
    state.copy_pairing_json(&pairing_id, |json| {
        app.clipboard()
            .write_text(json.to_string())
            .map_err(|_| pairing_clipboard_write_failed())
    })
}

#[tauri::command]
pub async fn cancel_remote_pairing(
    state: State<'_, RemoteAccessRuntime>,
    pairing_id: String,
) -> Result<PairingStatus, RemoteCommandError> {
    state.cancel_pairing(&pairing_id).await
}

#[tauri::command]
pub async fn revoke_remote_credential(
    state: State<'_, RemoteAccessRuntime>,
    credential_id: String,
) -> Result<RemoteCredentialRevokeView, RemoteCommandError> {
    state.revoke_credential(&credential_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use codepet_lan_channel_sdk::PairingExchangeRequest;
    use codepet_host::{
        DeviceRegistry, IssuedRemoteCredential, PluginCatalog,
        PluginCatalogConfig, PluginManager, PluginManagerConfig,
        ProviderInstanceRegistry, RemoteAccessConfig,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    struct TestRuntime {
        _directory: TempDir,
        provider_manager: Arc<PluginManager>,
        remote_manager: Arc<RemoteAccessManager>,
        runtime: RemoteAccessRuntime,
    }

    #[derive(Default)]
    struct FakeDiscoveryState {
        generations: Vec<(String, bool)>,
        shutdown_count: usize,
        fail_next_publish: bool,
    }

    struct FakeDiscoveryPublisher {
        state: Arc<StdMutex<FakeDiscoveryState>>,
    }

    impl RemoteDiscoveryPublisher for FakeDiscoveryPublisher {
        fn replace(
            &mut self,
            endpoint: &RemoteLanAdvertisedEndpoint,
            pairing_available: bool,
        ) -> Result<(), HostError> {
            record_fake_generation(
                &self.state,
                endpoint,
                pairing_available,
            )
        }

        fn shutdown(&mut self) -> Result<(), HostError> {
            self.state.lock().unwrap().shutdown_count += 1;
            Ok(())
        }
    }

    fn record_fake_generation(
        state: &Arc<StdMutex<FakeDiscoveryState>>,
        endpoint: &RemoteLanAdvertisedEndpoint,
        pairing_available: bool,
    ) -> Result<(), HostError> {
        let mut state = state.lock().unwrap();
        if state.fail_next_publish {
            state.fail_next_publish = false;
            return Err(HostError::new(
                "test_mdns_announce_failed",
                "test mDNS Announce failed",
            )
            .retryable(true));
        }
        state.generations.push((
            endpoint.advertised_host().to_string(),
            pairing_available,
        ));
        Ok(())
    }

    fn fake_discovery_factory(
        state: Arc<StdMutex<FakeDiscoveryState>>,
    ) -> Arc<RemoteDiscoveryPublisherFactory> {
        Arc::new(move |_source, endpoint, pairing_available| {
            record_fake_generation(&state, &endpoint, pairing_available)?;
            Ok(Box::new(FakeDiscoveryPublisher {
                state: state.clone(),
            }))
        })
    }

    fn test_runtime(
        resolver: Arc<dyn Fn() -> Result<std::net::Ipv4Addr, HostError> + Send + Sync>,
        mdns_enabled: bool,
    ) -> TestRuntime {
        let directory = tempfile::tempdir().unwrap();
        let device = Arc::new(
            DeviceRegistry::open(directory.path().join("device.json"), "Runtime Test")
                .unwrap(),
        );
        let instances = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            device.identity().device_id.clone(),
        )
        .unwrap();
        let provider_manager = Arc::new(
            PluginManager::with_device_registry(
                device.clone(),
                PluginCatalog::discover(PluginCatalogConfig::default()),
                instances,
                PluginManagerConfig::default(),
            )
            .unwrap(),
        );
        let remote_manager = Arc::new(
            RemoteAccessManager::open(
                RemoteAccessConfig::for_data_directory(directory.path().join("remote")),
                device.clone(),
                DeviceDescriptor {
                    device_name: "Runtime Test".to_string(),
                    operating_system: "TestOS".to_string(),
                    system_version: "1.0".to_string(),
                },
            )
            .unwrap(),
        );
        assert!(Arc::ptr_eq(
            &provider_manager.device_registry(),
            &device
        ));
        let gateway = Arc::new(
            ProviderGatewayService::with_remote_identity(
                provider_manager.clone(),
                {
                    let identity = remote_manager.remote_host_identity();
                    codepet_gateway_sdk::GatewayHostIdentity {
                        device_id: identity.device_id,
                        descriptor: identity.descriptor,
                    }
                },
            )
            .unwrap(),
        );
        let runtime = RemoteAccessRuntime::new_for_test(
            remote_manager.clone(),
            gateway,
            resolver,
            mdns_enabled,
        );
        TestRuntime {
            _directory: directory,
            provider_manager,
            remote_manager,
            runtime,
        }
    }

    fn pair_remote_client(
        manager: &RemoteAccessManager,
        client_id: &str,
        device_name: &str,
    ) -> IssuedRemoteCredential {
        let pairing = manager.begin_pairing().unwrap();
        manager
            .complete_pairing(
                &pairing.pairing_id,
                PairingExchangeRequest {
                    pairing_secret: pairing.pairing_secret,
                    client_id: client_id.to_string(),
                    device: DeviceDescriptor {
                        device_name: device_name.to_string(),
                        operating_system: "TestOS".to_string(),
                        system_version: "1.0".to_string(),
                    },
                },
            )
            .unwrap()
    }

    async fn wait_for_pairing_advertisement(
        runtime: &RemoteAccessRuntime,
        expected: bool,
    ) {
        timeout(Duration::from_secs(2), async {
            loop {
                if runtime.status().await.pairing_available == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn startup_failure_is_nonfatal_and_retry_uses_the_same_host_core() {
        let unavailable = RemoteAccessRuntime::unavailable(HostError::new(
            "provider_host_unavailable",
            "test Provider Host is unavailable",
        ));
        assert_eq!(
            unavailable.retry().await.unwrap_err().code,
            "remote_access_core_unavailable"
        );
        assert_eq!(
            unavailable.status().await.phase,
            RemoteAccessRuntimePhase::Unavailable
        );

        let attempts = Arc::new(AtomicUsize::new(0));
        let resolver_attempts = attempts.clone();
        let test = test_runtime(Arc::new(move || {
            if resolver_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(HostError::new(
                    "test_lan_unavailable",
                    "test route is unavailable",
                )
                .retryable(true))
            } else {
                Ok(std::net::Ipv4Addr::LOCALHOST)
            }
        }), false);

        let first = test.runtime.retry().await.unwrap_err();
        assert_eq!(first.code, "test_lan_unavailable");
        assert_eq!(
            test.runtime.status().await.phase,
            RemoteAccessRuntimePhase::Unavailable
        );
        assert!(test.remote_manager.list_credentials().unwrap().is_empty());

        let available = test.runtime.retry().await.unwrap();
        assert_eq!(available.phase, RemoteAccessRuntimePhase::Available);
        assert_eq!(available.advertised_host.as_deref(), Some("127.0.0.1"));
        let repeated = test.runtime.retry().await.unwrap();
        assert_eq!(repeated.https_base_url, available.https_base_url);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        assert!(test.runtime.shutdown_once().await);
        assert_eq!(
            test.runtime.status().await.phase,
            RemoteAccessRuntimePhase::Stopped
        );
        assert_eq!(
            test.runtime.retry().await.unwrap_err().code,
            "remote_access_stopped"
        );
        assert!(test.runtime.pairing_monitor.lock().unwrap().is_none());
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn preferred_listener_port_is_stable_and_address_in_use_falls_back() {
        assert_eq!(REMOTE_LAN_STABLE_PORT, 47_622);

        let reserved = std::net::TcpListener::bind((
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        ))
        .unwrap();
        let preferred_port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let mut stable = test_runtime(
            Arc::new(|| Ok(std::net::Ipv4Addr::LOCALHOST)),
            false,
        );
        stable.runtime.preferred_port = preferred_port;
        let stable_status = stable.runtime.retry().await.unwrap();
        assert_eq!(stable_status.phase, RemoteAccessRuntimePhase::Available);
        assert_eq!(stable_status.diagnostic, None);
        assert_eq!(
            stable
                .runtime
                .inner
                .lock()
                .await
                .listener
                .as_ref()
                .unwrap()
                .port(),
            preferred_port
        );
        stable.runtime.shutdown_once().await;
        stable.provider_manager.shutdown().await;

        let occupied = std::net::TcpListener::bind((
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        ))
        .unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();
        let mut fallback = test_runtime(
            Arc::new(|| Ok(std::net::Ipv4Addr::LOCALHOST)),
            false,
        );
        fallback.runtime.preferred_port = occupied_port;
        let fallback_status = fallback.runtime.retry().await.unwrap();
        assert_eq!(fallback_status.phase, RemoteAccessRuntimePhase::Available);
        assert_eq!(
            fallback_status
                .diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.code.as_str()),
            Some("remote_lan_stable_port_unavailable")
        );
        assert_ne!(
            fallback
                .runtime
                .inner
                .lock()
                .await
                .listener
                .as_ref()
                .unwrap()
                .port(),
            occupied_port
        );
        fallback.runtime.shutdown_once().await;
        fallback.provider_manager.shutdown().await;
        drop(occupied);
    }

    #[tokio::test]
    async fn address_generation_commits_mdns_status_qr_and_exchange_together() {
        let discovery = Arc::new(StdMutex::new(FakeDiscoveryState::default()));
        let mut test = test_runtime(
            Arc::new(|| Ok("192.168.10.20".parse().unwrap())),
            true,
        );
        test.runtime.discovery_publisher_factory =
            fake_discovery_factory(discovery.clone());
        let started_at_a = test.runtime.retry().await.unwrap();
        let port = test
            .runtime
            .inner
            .lock()
            .await
            .listener
            .as_ref()
            .unwrap()
            .port();
        assert_eq!(
            started_at_a.https_base_url,
            Some(format!("https://192.168.10.20:{port}"))
        );

        let pairing = test.runtime.start_pairing().await.unwrap();
        let qr_at_a = pairing.qr_svg_data_url.clone();
        test.runtime
            .reconcile_network_observation(Ok(
                "192.168.10.21".parse().unwrap(),
            ))
            .await
            .unwrap();

        let status_at_b = test.runtime.status().await;
        assert_eq!(status_at_b.phase, RemoteAccessRuntimePhase::Available);
        assert_eq!(
            status_at_b.advertised_host.as_deref(),
            Some("192.168.10.21")
        );
        assert_eq!(
            status_at_b.https_base_url,
            Some(format!("https://192.168.10.21:{port}"))
        );
        assert!(status_at_b.pairing_available);
        let pairing_status = test
            .runtime
            .pairing_status(&pairing.pairing_id)
            .await
            .unwrap();
        assert_ne!(
            pairing_status.qr_svg_data_url.as_deref(),
            Some(qr_at_a.as_str())
        );
        let pairing_json = test
            .runtime
            .active_pairing_payload
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .json
            .clone();
        let pairing_payload: PairingQrPayload =
            serde_json::from_str(&pairing_json).unwrap();
        assert_eq!(
            pairing_payload.https_base_url,
            format!("https://192.168.10.21:{port}")
        );
        assert_eq!(
            discovery.lock().unwrap().generations.last(),
            Some(&("192.168.10.21".to_string(), true))
        );

        let certificate = reqwest::Certificate::from_der(
            test.remote_manager.tls_identity().certificate_der(),
        )
        .unwrap();
        let client = reqwest::Client::builder()
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate)
            .build()
            .unwrap();
        let exchange = client
            .post(format!(
                "https://localhost:{port}/remote/v1/pairings/{}/exchange",
                pairing.pairing_id
            ))
            .json(&PairingExchangeRequest {
                pairing_secret: pairing_payload.pairing_secret,
                client_id: "generation-client".to_string(),
                device: DeviceDescriptor {
                    device_name: "Generation Client".to_string(),
                    operating_system: "TestOS".to_string(),
                    system_version: "1.0".to_string(),
                },
            })
            .send()
            .await
            .unwrap();
        assert!(exchange.status().is_success());
        let exchange: codepet_lan_channel_sdk::PairingExchangeResponse =
            exchange.json().await.unwrap();
        assert_eq!(
            exchange.gateway_url,
            format!("wss://192.168.10.21:{port}/remote/v2/gateway")
        );

        test.runtime.shutdown_once().await;
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn missing_address_and_announce_failure_withdraw_only_discovery_then_retry() {
        let discovery = Arc::new(StdMutex::new(FakeDiscoveryState::default()));
        let mut test = test_runtime(
            Arc::new(|| Ok("192.168.20.30".parse().unwrap())),
            true,
        );
        test.runtime.discovery_publisher_factory =
            fake_discovery_factory(discovery.clone());
        test.runtime.retry().await.unwrap();
        let listener_port = test
            .runtime
            .inner
            .lock()
            .await
            .listener
            .as_ref()
            .unwrap()
            .port();
        let tls_certificate = test
            .remote_manager
            .tls_identity()
            .certificate_der()
            .to_vec();
        let credential = pair_remote_client(
            test.remote_manager.as_ref(),
            "persistent-client",
            "Persistent Client",
        );

        let unavailable = HostError::new(
            "remote_lan_route_probe_failed",
            "test route unavailable",
        )
        .retryable(true);
        assert_eq!(
            test.runtime
                .reconcile_network_observation(Err(unavailable))
                .await
                .unwrap_err()
                .code,
            "remote_lan_route_probe_failed"
        );
        let withdrawn = test.runtime.status().await;
        assert_eq!(withdrawn.phase, RemoteAccessRuntimePhase::Available);
        assert_eq!(withdrawn.advertised_host, None);
        assert_eq!(withdrawn.https_base_url, None);
        assert!(withdrawn.diagnostic.as_ref().unwrap().retryable);
        assert_eq!(
            test.runtime
                .inner
                .lock()
                .await
                .listener
                .as_ref()
                .unwrap()
                .port(),
            listener_port
        );
        assert_eq!(
            test.remote_manager.tls_identity().certificate_der(),
            tls_certificate.as_slice()
        );
        assert!(test
            .remote_manager
            .validate_bearer(&credential.bearer_token)
            .is_ok());

        test.runtime
            .reconcile_network_observation(Ok(
                "192.168.20.31".parse().unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(
            test.runtime.status().await.advertised_host.as_deref(),
            Some("192.168.20.31")
        );
        discovery.lock().unwrap().fail_next_publish = true;
        assert_eq!(
            test.runtime
                .reconcile_network_observation(Ok(
                    "192.168.20.32".parse().unwrap(),
                ))
                .await
                .unwrap_err()
                .code,
            "test_mdns_announce_failed"
        );
        let failed = test.runtime.status().await;
        assert_eq!(failed.phase, RemoteAccessRuntimePhase::Available);
        assert_eq!(failed.advertised_host, None);
        assert!(test
            .remote_manager
            .validate_bearer(&credential.bearer_token)
            .is_ok());
        test.runtime
            .reconcile_network_observation(Ok(
                "192.168.20.32".parse().unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(
            test.runtime.status().await.advertised_host.as_deref(),
            Some("192.168.20.32")
        );
        assert_eq!(
            discovery
                .lock()
                .unwrap()
                .generations
                .iter()
                .filter(|(host, _)| host == "192.168.20.32")
                .count(),
            1
        );
        assert_eq!(
            test.runtime
                .inner
                .lock()
                .await
                .listener
                .as_ref()
                .unwrap()
                .port(),
            listener_port
        );
        assert_eq!(
            test.remote_manager.tls_identity().certificate_der(),
            tls_certificate.as_slice()
        );

        test.runtime.shutdown_once().await;
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn concurrent_address_and_pairing_changes_converge_to_latest_generation() {
        let discovery = Arc::new(StdMutex::new(FakeDiscoveryState::default()));
        let mut test = test_runtime(
            Arc::new(|| Ok("192.168.30.40".parse().unwrap())),
            true,
        );
        test.runtime.discovery_publisher_factory =
            fake_discovery_factory(discovery.clone());
        test.runtime.retry().await.unwrap();

        let refresh_runtime = test.runtime.clone();
        let refresh = tokio::spawn(async move {
            refresh_runtime
                .reconcile_network_observation(Ok(
                    "192.168.30.41".parse().unwrap(),
                ))
                .await
        });
        let pairing = test.remote_manager.begin_pairing().unwrap();
        refresh.await.unwrap().unwrap();
        timeout(Duration::from_secs(2), async {
            loop {
                let status = test.runtime.status().await;
                if status.advertised_host.as_deref()
                    == Some("192.168.30.41")
                    && status.pairing_available
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let generations = discovery.lock().unwrap().generations.clone();
        let first_new = generations
            .iter()
            .position(|(host, _)| host == "192.168.30.41")
            .unwrap();
        assert!(generations[first_new..]
            .iter()
            .all(|(host, _)| host == "192.168.30.41"));
        assert_eq!(
            generations.last(),
            Some(&("192.168.30.41".to_string(), true))
        );

        test.remote_manager
            .cancel_pairing(&pairing.pairing_id)
            .unwrap();
        wait_for_pairing_advertisement(&test.runtime, false).await;
        test.runtime.shutdown_once().await;
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_stops_the_network_monitor() {
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let calls = resolver_calls.clone();
        let mut test = test_runtime(
            Arc::new(move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Ipv4Addr::LOCALHOST)
            }),
            false,
        );
        test.runtime.network_monitor_interval =
            Some(Duration::from_millis(10));
        test.runtime.retry().await.unwrap();
        timeout(Duration::from_secs(1), async {
            while resolver_calls.load(Ordering::SeqCst) < 3 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        test.runtime.shutdown_once().await;
        assert!(test.runtime.network_monitor.lock().unwrap().is_none());
        let calls_after_shutdown = resolver_calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(
            resolver_calls.load(Ordering::SeqCst),
            calls_after_shutdown
        );
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn remote_clients_group_credentials_by_client_identity_and_revoke_the_group() {
        let descriptor = DeviceDescriptor {
            device_name: "Same Device Name".to_string(),
            operating_system: "TestOS".to_string(),
            system_version: "1.0".to_string(),
        };
        let projected = project_remote_clients(
            vec![
                RemoteCredential {
                    credential_id: "credential-first".to_string(),
                    client_id: "client-same".to_string(),
                    descriptor: descriptor.clone(),
                    created_at: 1_000,
                    last_seen_at: 1_500,
                    revoked_at: None,
                },
                RemoteCredential {
                    credential_id: "credential-second".to_string(),
                    client_id: "client-same".to_string(),
                    descriptor: descriptor.clone(),
                    created_at: 2_000,
                    last_seen_at: 2_000,
                    revoked_at: None,
                },
                RemoteCredential {
                    credential_id: "credential-other".to_string(),
                    client_id: "client-other".to_string(),
                    descriptor,
                    created_at: 3_000,
                    last_seen_at: 3_000,
                    revoked_at: None,
                },
            ],
            |credential_id| match credential_id {
                "credential-first" => 1,
                "credential-second" => 2,
                "credential-other" => 4,
                _ => 0,
            },
        );
        assert_eq!(projected.len(), 2);
        let same_client = projected
            .iter()
            .find(|client| client.remote_client_id == "client-same")
            .unwrap();
        assert_eq!(same_client.credential_id, "credential-second");
        assert_eq!(same_client.created_at, 1_000);
        assert_eq!(same_client.last_seen_at, 1_500);
        assert_eq!(same_client.online_session_count, 3);
        let other_client = projected
            .iter()
            .find(|client| client.remote_client_id == "client-other")
            .unwrap();
        assert_eq!(other_client.online_session_count, 4);

        let test = test_runtime(
            Arc::new(|| Ok(std::net::Ipv4Addr::LOCALHOST)),
            false,
        );
        let first = pair_remote_client(
            test.remote_manager.as_ref(),
            "client-same",
            "Same Device Name",
        );
        let second = pair_remote_client(
            test.remote_manager.as_ref(),
            "client-same",
            "Same Device Name",
        );
        let other = pair_remote_client(
            test.remote_manager.as_ref(),
            "client-other",
            "Same Device Name",
        );

        let clients = test.runtime.list_clients().await.unwrap();
        assert_eq!(clients.len(), 2);
        let same_client = clients
            .iter()
            .find(|client| client.remote_client_id == "client-same")
            .unwrap();
        test.runtime
            .revoke_credential(&same_client.credential_id)
            .await
            .unwrap();
        assert_eq!(
            test.remote_manager
                .validate_bearer(&first.bearer_token)
                .unwrap_err()
                .code,
            "invalid_remote_credential"
        );
        assert_eq!(
            test.remote_manager
                .validate_bearer(&second.bearer_token)
                .unwrap_err()
                .code,
            "invalid_remote_credential"
        );
        assert!(test
            .remote_manager
            .validate_bearer(&other.bearer_token)
            .is_ok());
        let clients = test.runtime.list_clients().await.unwrap();
        assert_eq!(clients.len(), 2);
        assert!(clients
            .iter()
            .find(|client| client.remote_client_id == "client-same")
            .unwrap()
            .revoked_at
            .is_some());
        assert_eq!(
            clients
                .iter()
                .find(|client| client.remote_client_id == "client-other")
                .unwrap()
                .revoked_at,
            None
        );

        test.runtime.shutdown_once().await;
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn pairing_json_is_active_only_and_remote_views_stay_secret_safe() {
        let test = test_runtime(
            Arc::new(|| Ok(std::net::Ipv4Addr::LOCALHOST)),
            true,
        );
        test.runtime.retry().await.unwrap();
        assert!(test.runtime.inner.lock().await.mdns.is_some());

        let started = test.runtime.start_pairing().await.unwrap();
        let response = serde_json::to_value(&started).unwrap();
        assert_eq!(
            response.as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
            vec!["expiresAt", "pairingId", "qrSvgDataUrl"]
        );
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("pairingSecret"));
        assert!(!serialized.contains("credential"));
        let encoded = started
            .qr_svg_data_url
            .strip_prefix("data:image/svg+xml;base64,")
            .unwrap();
        let svg = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        )
        .unwrap();
        assert!(svg.starts_with("<?xml"));
        assert!(!svg.contains("pairingSecret"));
        assert_eq!(
            test.runtime
                .copy_pairing_json(&started.pairing_id, |_| {
                    Err(pairing_clipboard_write_failed())
                })
                .unwrap_err()
                .code,
            "remote_pairing_clipboard_write_failed"
        );
        let remote_identity = test.remote_manager.remote_host_identity();
        test.runtime
            .copy_pairing_json(&started.pairing_id, |json| {
                let pairing_payload: PairingQrPayload = serde_json::from_str(json).unwrap();
                assert_eq!(pairing_payload.pairing_id, started.pairing_id);
                assert_eq!(pairing_payload.expires_at, started.expires_at);
                assert_eq!(pairing_payload.version, u64::from(PROTOCOL_VERSION));
                assert_eq!(pairing_payload.host_device_id, remote_identity.device_id);
                assert_eq!(
                    pairing_payload.display_name,
                    remote_identity.descriptor.device_name
                );
                assert_eq!(
                    pairing_payload.cert_sha256,
                    remote_identity.identity_fingerprint
                );
                assert_eq!(pairing_payload.pairing_secret.len(), 64);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            test.runtime
                .pairing_status(&started.pairing_id)
                .await
                .unwrap()
                .state,
            codepet_host::PairingStatusKind::Active
        );
        assert_eq!(
            test.runtime.start_pairing().await.unwrap_err().code,
            "pairing_session_active"
        );
        let cancelled = test
            .runtime
            .cancel_pairing(&started.pairing_id)
            .await
            .unwrap();
        assert_eq!(cancelled.state, codepet_host::PairingStatusKind::Cancelled);
        assert_eq!(
            test.runtime
                .copy_pairing_json(&started.pairing_id, |_| Ok(()))
                .unwrap_err()
                .code,
            "remote_pairing_payload_unavailable"
        );
        assert_eq!(
            test.runtime
                .cancel_pairing(&started.pairing_id)
                .await
                .unwrap()
                .state,
            codepet_host::PairingStatusKind::Cancelled
        );

        let pairing = test.runtime.start_pairing().await.unwrap();
        let pairing_id = pairing.pairing_id.clone();
        let pairing_json = {
            let payload = test.runtime.active_pairing_payload.lock().unwrap();
            payload.as_ref().unwrap().json.clone()
        };
        let pairing_payload: PairingQrPayload = serde_json::from_str(&pairing_json).unwrap();
        wait_for_pairing_advertisement(&test.runtime, true).await;
        assert_eq!(
            test.runtime
                .cancel_pairing(&started.pairing_id)
                .await
                .unwrap()
                .state,
            codepet_host::PairingStatusKind::Cancelled
        );
        assert_eq!(
            test.runtime
                .pairing_status(&pairing_id)
                .await
                .unwrap()
                .state,
            codepet_host::PairingStatusKind::Active
        );
        assert!(test
            .runtime
            .copy_pairing_json(&pairing_id, |_| Ok(()))
            .is_ok());
        assert!(test.runtime.status().await.pairing_available);
        let port = test
            .runtime
            .inner
            .lock()
            .await
            .listener
            .as_ref()
            .unwrap()
            .port();
        let certificate = reqwest::Certificate::from_der(
            test.remote_manager.tls_identity().certificate_der(),
        )
        .unwrap();
        let client = reqwest::Client::builder()
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate)
            .build()
            .unwrap();
        let exchange = client
            .post(format!(
                "https://localhost:{port}/remote/v1/pairings/{pairing_id}/exchange"
            ))
            .json(&PairingExchangeRequest {
                pairing_secret: pairing_payload.pairing_secret,
                client_id: "runtime-client".to_string(),
                device: DeviceDescriptor {
                    device_name: "Runtime Client".to_string(),
                    operating_system: "TestOS".to_string(),
                    system_version: "2.0".to_string(),
                },
            })
            .send()
            .await
            .unwrap();
        assert!(exchange.status().is_success());
        let exchange: codepet_lan_channel_sdk::PairingExchangeResponse =
            exchange.json().await.unwrap();
        wait_for_pairing_advertisement(&test.runtime, false).await;
        assert_eq!(
            test.runtime
                .pairing_status(&pairing_id)
                .await
                .unwrap()
                .state,
            codepet_host::PairingStatusKind::Succeeded
        );
        assert_eq!(
            test.runtime
                .copy_pairing_json(&pairing_id, |_| Ok(()))
                .unwrap_err()
                .code,
            "remote_pairing_payload_unavailable"
        );
        assert!(test.runtime.active_pairing_payload.lock().unwrap().is_none());
        let clients = test.runtime.list_clients().await.unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].remote_client_id, "runtime-client");
        assert_eq!(clients[0].descriptor.device_name, "Runtime Client");
        assert_eq!(clients[0].descriptor.operating_system, "TestOS");
        assert_eq!(clients[0].descriptor.system_version, "2.0");
        assert_eq!(clients[0].online_session_count, 0);
        assert_eq!(clients[0].revoked_at, None);
        assert!(!exchange.credential.is_empty());

        let revoked = test
            .runtime
            .revoke_credential(&clients[0].credential_id)
            .await
            .unwrap();
        assert_eq!(revoked.disconnected_session_count, 0);
        assert!(revoked.revoked_at.is_some());
        let repeated = test
            .runtime
            .revoke_credential(&clients[0].credential_id)
            .await
            .unwrap();
        assert_eq!(repeated.revoked_at, revoked.revoked_at);
        assert_eq!(
            test.runtime.list_clients().await.unwrap()[0].revoked_at,
            revoked.revoked_at
        );

        test.runtime.shutdown_once().await;
        test.provider_manager.shutdown().await;
    }

    #[tokio::test]
    async fn pairing_copy_finishes_before_concurrent_cancellation() {
        let test = test_runtime(
            Arc::new(|| Ok(std::net::Ipv4Addr::LOCALHOST)),
            false,
        );
        test.runtime.retry().await.unwrap();
        let pairing = test.runtime.start_pairing().await.unwrap();
        let pairing_id = pairing.pairing_id;

        let (copy_started_tx, copy_started_rx) = std::sync::mpsc::channel();
        let (release_copy_tx, release_copy_rx) = std::sync::mpsc::channel();
        let copy_runtime = test.runtime.clone();
        let copy_pairing_id = pairing_id.clone();
        let copy_thread = std::thread::spawn(move || {
            copy_runtime.copy_pairing_json(&copy_pairing_id, |_| {
                copy_started_tx.send(()).unwrap();
                release_copy_rx.recv().unwrap();
                Ok(())
            })
        });
        copy_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let (cancel_started_tx, cancel_started_rx) = std::sync::mpsc::channel();
        let (cancel_finished_tx, cancel_finished_rx) = std::sync::mpsc::channel();
        let cancel_manager = test.remote_manager.clone();
        let cancel_pairing_id = pairing_id.clone();
        let cancel_thread = std::thread::spawn(move || {
            cancel_started_tx.send(()).unwrap();
            let result = cancel_manager.cancel_pairing(&cancel_pairing_id);
            cancel_finished_tx.send(()).unwrap();
            result
        });
        cancel_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(cancel_finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());

        release_copy_tx.send(()).unwrap();
        copy_thread.join().unwrap().unwrap();
        assert_eq!(
            cancel_thread.join().unwrap().unwrap().state,
            codepet_host::PairingStatusKind::Cancelled
        );
        assert_eq!(
            test.runtime
                .copy_pairing_json(&pairing_id, |_| Ok(()))
                .unwrap_err()
                .code,
            "remote_pairing_payload_unavailable"
        );

        test.runtime.shutdown_once().await;
        test.provider_manager.shutdown().await;
    }
}
