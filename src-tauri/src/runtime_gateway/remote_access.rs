use base64::Engine;
use codepet_gateway_sdk::{PairingQrPayload, PROTOCOL_VERSION};
use codepet_host::{
    select_remote_lan_ipv4, HostError, PairingStatus, ProviderGatewayService,
    RemoteAccessManager, RemoteLanMdnsAdvertiser, RemoteLanServer,
    RemoteLanServerConfig, RemoteLanServerHandle,
};
use qrcode::render::svg;
use qrcode::QrCode;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tauri::State;
use tokio::sync::{watch, Mutex};
use tokio::time::timeout;

pub const REMOTE_ADVERTISED_HOST_ENV: &str = "CODEPET_REMOTE_ADVERTISED_HOST";
const PAIRING_MONITOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const MDNS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

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
pub struct RemoteClientView {
    pub credential_id: String,
    pub remote_client_id: String,
    pub client_name: String,
    pub platform: String,
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
    mdns: Option<RemoteLanMdnsAdvertiser>,
    pairing_available: bool,
    diagnostic: Option<RemoteAccessDiagnosticView>,
}

#[derive(Clone)]
pub struct RemoteAccessRuntime {
    manager: Option<Arc<RemoteAccessManager>>,
    gateway: Option<Arc<ProviderGatewayService>>,
    inner: Arc<Mutex<RemoteAccessRuntimeInner>>,
    lifecycle: Arc<Mutex<()>>,
    pairing_monitor: Arc<StdMutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    pairing_shutdown: watch::Sender<bool>,
    shutdown_completed: Arc<AtomicBool>,
    advertised_host_resolver:
        Arc<dyn Fn() -> Result<std::net::Ipv4Addr, HostError> + Send + Sync>,
    mdns_enabled: bool,
}

impl RemoteAccessRuntime {
    pub fn new(
        manager: Arc<RemoteAccessManager>,
        gateway: Arc<ProviderGatewayService>,
    ) -> Self {
        let (pairing_shutdown, _) = watch::channel(false);
        Self {
            manager: Some(manager),
            gateway: Some(gateway),
            inner: Arc::new(Mutex::new(RemoteAccessRuntimeInner {
                phase: RemoteAccessRuntimePhase::Unavailable,
                listener: None,
                mdns: None,
                pairing_available: false,
                diagnostic: Some(RemoteAccessDiagnosticView {
                    code: "remote_access_not_started".to_string(),
                    message: "Remote LAN access has not started yet".to_string(),
                    retryable: true,
                }),
            })),
            lifecycle: Arc::new(Mutex::new(())),
            pairing_monitor: Arc::new(StdMutex::new(None)),
            pairing_shutdown,
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            advertised_host_resolver: Arc::new(select_advertised_host_from_environment),
            mdns_enabled: true,
        }
    }

    pub fn unavailable(error: HostError) -> Self {
        let (pairing_shutdown, _) = watch::channel(false);
        Self {
            manager: None,
            gateway: None,
            inner: Arc::new(Mutex::new(RemoteAccessRuntimeInner {
                phase: RemoteAccessRuntimePhase::Unavailable,
                listener: None,
                mdns: None,
                pairing_available: false,
                diagnostic: Some(error.into()),
            })),
            lifecycle: Arc::new(Mutex::new(())),
            pairing_monitor: Arc::new(StdMutex::new(None)),
            pairing_shutdown,
            shutdown_completed: Arc::new(AtomicBool::new(false)),
            advertised_host_resolver: Arc::new(|| {
                Err(HostError::new(
                    "remote_access_core_unavailable",
                    "Remote access cannot run because the shared Provider Host core is unavailable",
                ))
            }),
            mdns_enabled: true,
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
        runtime.mdns_enabled = mdns_enabled;
        runtime
    }

    pub fn start_in_background(&self) {
        self.ensure_pairing_monitor();
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
            display_name: identity.as_ref().map(|identity| identity.display_name.clone()),
            advertised_host: inner
                .listener
                .as_ref()
                .map(|listener| listener.advertised_host().to_string()),
            https_base_url: inner
                .listener
                .as_ref()
                .map(|listener| listener.https_base_url().to_string()),
            active_session_count: inner
                .listener
                .as_ref()
                .map(RemoteLanServerHandle::active_session_count)
                .unwrap_or(0),
            pairing_available: inner.pairing_available,
            diagnostic: inner.diagnostic.clone(),
        }
    }

    pub async fn retry(&self) -> Result<RemoteAccessStatusView, RemoteCommandError> {
        self.ensure_pairing_monitor();
        let _lifecycle = self.lifecycle.lock().await;
        {
            let inner = self.inner.lock().await;
            match inner.phase {
                RemoteAccessRuntimePhase::Available => return Ok(self.status_locked(&inner)),
                RemoteAccessRuntimePhase::Stopping | RemoteAccessRuntimePhase::Stopped => {
                    return Err(runtime_stopped_error())
                }
                RemoteAccessRuntimePhase::Starting | RemoteAccessRuntimePhase::Unavailable => {}
            }
        }
        let manager = self.manager.clone().ok_or_else(runtime_core_unavailable)?;
        let gateway = self.gateway.clone().ok_or_else(runtime_core_unavailable)?;
        {
            let mut inner = self.inner.lock().await;
            inner.phase = RemoteAccessRuntimePhase::Starting;
            inner.diagnostic = None;
        }

        let advertised_host = match (self.advertised_host_resolver)() {
            Ok(host) => host,
            Err(error) => return self.fail_start(error).await,
        };
        let config = RemoteLanServerConfig::default()
            .with_advertised_host(advertised_host.to_string());
        let listener = match RemoteLanServer::start(config, manager.clone(), gateway).await {
            Ok(listener) => listener,
            Err(error) => return self.fail_start(error).await,
        };
        let pairing_available = manager
            .subscribe_pairing_state()
            .borrow()
            .pairing_available;
        let (listener, mdns) = if self.mdns_enabled {
            match tauri::async_runtime::spawn_blocking(move || {
                let mdns = RemoteLanMdnsAdvertiser::start(&listener, pairing_available);
                (listener, mdns)
            })
            .await
            {
                Ok((listener, Ok(mdns))) => (listener, Some(mdns)),
                Ok((listener, Err(error))) => {
                    let _ = listener.shutdown().await;
                    return self.fail_start(error).await;
                }
                Err(error) => {
                    return self
                        .fail_start(HostError::new(
                            "remote_lan_mdns_task_failed",
                            format!("Remote LAN mDNS startup task failed: {error}"),
                        ))
                        .await;
                }
            }
        } else {
            (listener, None)
        };

        let mut inner = self.inner.lock().await;
        inner.phase = RemoteAccessRuntimePhase::Available;
        inner.listener = Some(listener);
        inner.mdns = mdns;
        inner.pairing_available = pairing_available;
        inner.diagnostic = None;
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
                listener.https_base_url().to_string(),
                manager.remote_host_identity(),
            )
        };
        let manager = self.manager.as_ref().ok_or_else(runtime_core_unavailable)?;
        let pairing = manager.begin_pairing().map_err(RemoteCommandError::from)?;
        let payload = PairingQrPayload {
            version: u64::from(PROTOCOL_VERSION),
            host_device_id: identity.device_id,
            display_name: identity.display_name,
            https_base_url,
            cert_sha256: identity.identity_fingerprint,
            pairing_id: pairing.pairing_id.clone(),
            pairing_secret: pairing.pairing_secret,
            expires_at: pairing.expires_at,
        };
        let qr_svg_data_url = match encode_pairing_qr(&payload) {
            Ok(value) => value,
            Err(error) => {
                let _ = manager.cancel_pairing(&pairing.pairing_id);
                return Err(error);
            }
        };
        if let Err(error) = self.sync_pairing_available().await {
            let _ = manager.cancel_pairing(&pairing.pairing_id);
            return Err(error);
        }
        Ok(RemotePairingStartView {
            pairing_id: pairing.pairing_id,
            expires_at: pairing.expires_at,
            qr_svg_data_url,
        })
    }

    pub fn pairing_status(&self, pairing_id: &str) -> Result<PairingStatus, RemoteCommandError> {
        self.manager
            .as_ref()
            .ok_or_else(runtime_core_unavailable)?
            .pairing_status(pairing_id)
            .map_err(RemoteCommandError::from)
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
        Ok(credentials
            .into_iter()
            .map(|credential| RemoteClientView {
                online_session_count: inner
                    .listener
                    .as_ref()
                    .map(|listener| {
                        listener.active_session_count_for_credential(&credential.credential_id)
                    })
                    .unwrap_or(0),
                credential_id: credential.credential_id,
                remote_client_id: credential.client_id,
                client_name: credential.client_name,
                platform: credential.platform,
                created_at: credential.created_at,
                last_seen_at: credential.last_seen_at,
                revoked_at: credential.revoked_at,
            })
            .collect())
    }

    pub async fn revoke_credential(
        &self,
        credential_id: &str,
    ) -> Result<RemoteCredentialRevokeView, RemoteCommandError> {
        let credential = self
            .manager
            .as_ref()
            .ok_or_else(runtime_core_unavailable)?
            .revoke_credential(credential_id)
            .map_err(RemoteCommandError::from)?;
        let disconnected_session_count = {
            let inner = self.inner.lock().await;
            match inner.listener.as_ref() {
                Some(listener) => listener
                    .disconnect_credential(credential_id)
                    .await
                    .map_err(RemoteCommandError::from)?,
                None => 0,
            }
        };
        Ok(RemoteCredentialRevokeView {
            credential_id: credential.credential_id,
            revoked_at: credential.revoked_at,
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
        inner.diagnostic = None;
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

    fn current_pairing_available(&self) -> Result<bool, RemoteCommandError> {
        let manager = self.manager.as_ref().ok_or_else(runtime_core_unavailable)?;
        let pairing = manager.subscribe_pairing_state();
        let pairing_available = pairing.borrow().pairing_available;
        Ok(pairing_available)
    }

    async fn sync_pairing_available(&self) -> Result<(), RemoteCommandError> {
        let _lifecycle = self.lifecycle.lock().await;
        loop {
            let pairing_available = self.current_pairing_available()?;
            let mut mdns = {
                let mut inner = self.inner.lock().await;
                if inner.phase != RemoteAccessRuntimePhase::Available {
                    return if pairing_available {
                        Err(runtime_unavailable_error(&inner))
                    } else {
                        Ok(())
                    };
                }
                if inner.pairing_available == pairing_available {
                    return Ok(());
                }
                if !self.mdns_enabled {
                    inner.pairing_available = pairing_available;
                    return Ok(());
                }
                inner.mdns.take().ok_or_else(runtime_core_unavailable)?
            };
            let updated = tauri::async_runtime::spawn_blocking(move || {
                let result = mdns.update_pairing_available(pairing_available);
                (mdns, result)
            })
            .await;
            let (returned, result) = match updated {
                Ok(updated) => updated,
                Err(error) => {
                    let command_error = RemoteCommandError {
                        code: "remote_lan_mdns_task_failed".to_string(),
                        message: format!("Remote LAN mDNS update task failed: {error}"),
                        retryable: true,
                    };
                    self.fail_running(command_error.clone()).await;
                    return Err(command_error);
                }
            };
            let mut inner = self.inner.lock().await;
            match result {
                Ok(()) => {
                    inner.mdns = Some(returned);
                    inner.pairing_available = pairing_available;
                    drop(inner);
                    if self.current_pairing_available()? == pairing_available {
                        return Ok(());
                    }
                }
                Err(error) => {
                    drop(inner);
                    drop(returned);
                    self.fail_running(RemoteCommandError::from(error.clone()))
                        .await;
                    return Err(error.into());
                }
            }
        }
    }

    async fn fail_running(&self, error: RemoteCommandError) {
        let listener = {
            let mut inner = self.inner.lock().await;
            inner.phase = RemoteAccessRuntimePhase::Unavailable;
            inner.pairing_available = false;
            inner.mdns = None;
            inner.diagnostic = Some(RemoteAccessDiagnosticView {
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            });
            inner.listener.take()
        };
        if let Some(listener) = listener {
            if let Err(error) = listener.shutdown().await {
                crate::app_log::error(
                    "remote_access",
                    &format!("Remote LAN listener fail-closed shutdown failed error={error:?}"),
                );
            }
        }
    }

    async fn fail_start<T>(&self, error: HostError) -> Result<T, RemoteCommandError> {
        let diagnostic = RemoteAccessDiagnosticView::from(error.clone());
        let mut inner = self.inner.lock().await;
        inner.phase = RemoteAccessRuntimePhase::Unavailable;
        inner.listener = None;
        inner.mdns = None;
        inner.pairing_available = false;
        inner.diagnostic = Some(diagnostic);
        Err(error.into())
    }

    fn status_locked(&self, inner: &RemoteAccessRuntimeInner) -> RemoteAccessStatusView {
        let identity = self.manager.as_ref().map(|manager| manager.remote_host_identity());
        RemoteAccessStatusView {
            phase: inner.phase,
            host_device_id: identity.as_ref().map(|identity| identity.device_id.clone()),
            display_name: identity.as_ref().map(|identity| identity.display_name.clone()),
            advertised_host: inner
                .listener
                .as_ref()
                .map(|listener| listener.advertised_host().to_string()),
            https_base_url: inner
                .listener
                .as_ref()
                .map(|listener| listener.https_base_url().to_string()),
            active_session_count: inner
                .listener
                .as_ref()
                .map(RemoteLanServerHandle::active_session_count)
                .unwrap_or(0),
            pairing_available: inner.pairing_available,
            diagnostic: inner.diagnostic.clone(),
        }
    }
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

fn encode_pairing_qr(payload: &PairingQrPayload) -> Result<String, RemoteCommandError> {
    let json = serde_json::to_vec(payload).map_err(|error| RemoteCommandError {
        code: "remote_pairing_qr_encoding_failed".to_string(),
        message: format!("encode the generated pairing QR payload: {error}"),
        retryable: false,
    })?;
    let code = QrCode::new(json).map_err(|error| RemoteCommandError {
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
    Ok(format!("data:image/svg+xml;base64,{encoded}"))
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
        .diagnostic
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
pub fn get_remote_pairing_status(
    state: State<'_, RemoteAccessRuntime>,
    pairing_id: String,
) -> Result<PairingStatus, RemoteCommandError> {
    state.pairing_status(&pairing_id)
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
    use codepet_gateway_sdk::PairingExchangeRequest;
    use codepet_host::{
        DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginManager,
        PluginManagerConfig, ProviderInstanceRegistry, RemoteAccessConfig,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    struct TestRuntime {
        _directory: TempDir,
        provider_manager: Arc<PluginManager>,
        remote_manager: Arc<RemoteAccessManager>,
        runtime: RemoteAccessRuntime,
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
                remote_manager.remote_host_identity(),
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
        assert_eq!(attempts.load(Ordering::SeqCst), 2);

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
    async fn qr_response_pairing_watch_client_listing_and_revoke_are_secret_safe() {
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
                .pairing_status(&started.pairing_id)
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
                .cancel_pairing(&started.pairing_id)
                .await
                .unwrap()
                .state,
            codepet_host::PairingStatusKind::Cancelled
        );

        let pairing = test.remote_manager.begin_pairing().unwrap();
        let pairing_id = pairing.pairing_id.clone();
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
            test.runtime.pairing_status(&pairing_id).unwrap().state,
            codepet_host::PairingStatusKind::Active
        );
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
                pairing_secret: pairing.pairing_secret,
                client_id: "runtime-client".to_string(),
                client_name: "Runtime Client".to_string(),
                platform: "test".to_string(),
            })
            .send()
            .await
            .unwrap();
        assert!(exchange.status().is_success());
        let exchange: codepet_gateway_sdk::PairingExchangeResponse =
            exchange.json().await.unwrap();
        wait_for_pairing_advertisement(&test.runtime, false).await;
        assert_eq!(
            test.runtime.pairing_status(&pairing_id).unwrap().state,
            codepet_host::PairingStatusKind::Succeeded
        );
        let clients = test.runtime.list_clients().await.unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].remote_client_id, "runtime-client");
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
}
