use crate::{
    HostError, HostResult, ProviderGatewayService, RemoteAccessManager, RemoteCredential,
};
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use codepet_gateway_sdk as gateway;
use codepet_lan_channel_sdk as lan;
use futures_util::{SinkExt, StreamExt};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{
    broadcast, mpsc, watch, Notify, OwnedSemaphorePermit, Semaphore,
};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

const PAIRING_EXCHANGE_PATH: &str = "/remote/v1/pairings/:pairing_id/exchange";
const GATEWAY_PATH: &str = "/remote/v2/gateway";
const CURRENT_CREDENTIAL_PATH: &str = "/remote/v1/credentials/current";
const MAX_REST_BODY_BYTES: usize = 64 * 1024;
const MAX_WEBSOCKET_FRAME_BYTES: usize = 256 * 1024;
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 * 1024;
const OUTBOUND_QUEUE_CAPACITY: usize = 64;
const REQUEST_QUEUE_CAPACITY: usize = 64;
const MAX_CONCURRENT_REQUESTS_PER_SESSION: usize = 8;
const MAX_CONCURRENT_WEBSOCKET_SESSIONS: usize = 32;
const WEBSOCKET_SEND_TIMEOUT: Duration = Duration::from_secs(15);
const OUTBOUND_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECTION_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
static NEXT_REMOTE_LAN_LISTENER_ID: AtomicU64 = AtomicU64::new(1);

/// Socket binding and client-visible host for the TLS-only Remote Gateway listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteLanServerConfig {
    pub bind_addr: SocketAddr,
    pub advertised_host: Option<String>,
}

impl Default for RemoteLanServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            advertised_host: None,
        }
    }
}

impl RemoteLanServerConfig {
    pub fn loopback() -> Self {
        Self {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            advertised_host: None,
        }
    }

    pub fn with_advertised_host(mut self, advertised_host: impl Into<String>) -> Self {
        self.advertised_host = Some(advertised_host.into());
        self
    }
}

/// Starts one HTTPS/WSS listener backed by the retained RemoteAccessManager identity.
pub struct RemoteLanServer;

impl RemoteLanServer {
    pub async fn start(
        config: RemoteLanServerConfig,
        remote_access: Arc<RemoteAccessManager>,
        gateway: Arc<ProviderGatewayService>,
    ) -> HostResult<RemoteLanServerHandle> {
        let remote_identity = remote_access.remote_host_identity();
        let gateway_identity = gateway::GatewayHostIdentity {
            device_id: remote_identity.device_id.clone(),
            descriptor: remote_identity.descriptor.clone(),
        };
        if gateway.remote_host_identity() != Some(&gateway_identity) {
            return Err(HostError::new(
                "remote_gateway_identity_mismatch",
                "Remote Gateway service identity must match the LAN TLS and Host identity",
            ));
        }
        let advertised_host = resolve_advertised_host(
            config.bind_addr.ip(),
            config.advertised_host.as_deref(),
        )?;

        let listener = TcpListener::bind(config.bind_addr).map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::AddrInUse {
                "remote_lan_listener_address_in_use"
            } else {
                "remote_lan_listener_bind_failed"
            };
            HostError::new(code, format!("bind Remote LAN listener: {error}"))
                .retryable(true)
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            HostError::new(
                "remote_lan_listener_bind_failed",
                format!("configure Remote LAN listener: {error}"),
            )
            .retryable(true)
        })?;
        let local_addr = listener.local_addr().map_err(|error| {
            HostError::new(
                "remote_lan_listener_bind_failed",
                format!("read Remote LAN listener address: {error}"),
            )
            .retryable(true)
        })?;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls_identity = remote_access.tls_identity();
        let tls_config = RustlsConfig::from_der(
            vec![tls_identity.certificate_der().to_vec()],
            tls_identity.private_key_der().to_vec(),
        )
        .await
        .map_err(|error| {
            HostError::new(
                "remote_lan_tls_config_failed",
                format!("configure Remote LAN TLS identity: {error}"),
            )
        })?;

        let listener_id = NEXT_REMOTE_LAN_LISTENER_ID.fetch_add(1, Ordering::Relaxed);
        let advertised_endpoint = RemoteLanAdvertisedEndpoint::new(
            listener_id,
            advertised_host,
            local_addr.port(),
        );
        let advertised_endpoints = Arc::new(Mutex::new(RemoteLanAdvertisedEndpointState {
            current: Some(advertised_endpoint),
            pending: None,
            next_transition_id: 1,
        }));
        let sessions = Arc::new(SessionRegistry::new(MAX_CONCURRENT_WEBSOCKET_SESSIONS));
        let state = Arc::new(RemoteLanState {
            remote_access,
            gateway,
            remote_identity: remote_identity.clone(),
            advertised_endpoints: advertised_endpoints.clone(),
            sessions: sessions.clone(),
        });
        let app = Router::new()
            .route(PAIRING_EXCHANGE_PATH, post(pairing_exchange))
            .route(GATEWAY_PATH, get(gateway_websocket))
            .route(CURRENT_CREDENTIAL_PATH, delete(delete_current_credential))
            .layer(DefaultBodyLimit::max(MAX_REST_BODY_BYTES))
            .with_state(state);
        let server_handle = axum_server::Handle::new();
        let running_handle = server_handle.clone();
        let server_task = tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls_config)
                .map(|acceptor| acceptor.handshake_timeout(TLS_HANDSHAKE_TIMEOUT))
                .handle(running_handle)
                .serve(app.into_make_service())
                .await
        });

        Ok(RemoteLanServerHandle {
            listener_id,
            local_addr,
            remote_identity,
            advertised_endpoints,
            sessions,
            server_handle,
            server_task: Some(server_task),
        })
    }
}

/// One complete client-visible endpoint generation for a running listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteLanAdvertisedEndpoint {
    listener_id: u64,
    advertised_host: String,
    authority: String,
    https_base_url: String,
    gateway_url: String,
}

impl RemoteLanAdvertisedEndpoint {
    fn new(listener_id: u64, advertised_host: String, port: u16) -> Self {
        let authority = advertised_url_authority(&advertised_host, port);
        Self {
            listener_id,
            advertised_host,
            https_base_url: format!("https://{authority}"),
            gateway_url: format!("wss://{authority}{GATEWAY_PATH}"),
            authority,
        }
    }

    pub fn advertised_host(&self) -> &str {
        &self.advertised_host
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn https_base_url(&self) -> &str {
        &self.https_base_url
    }

    pub fn gateway_url(&self) -> &str {
        &self.gateway_url
    }

    pub(crate) fn listener_id(&self) -> u64 {
        self.listener_id
    }
}

#[derive(Clone, Debug)]
pub struct RemoteLanAdvertisementSource {
    listener_id: u64,
    local_addr: SocketAddr,
    remote_identity: lan::LanHostIdentity,
}

impl RemoteLanAdvertisementSource {
    pub(crate) fn listener_id(&self) -> u64 {
        self.listener_id
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub(crate) fn remote_host_identity(&self) -> &lan::LanHostIdentity {
        &self.remote_identity
    }
}

struct RemoteLanAdvertisedEndpointState {
    current: Option<RemoteLanAdvertisedEndpoint>,
    pending: Option<(u64, RemoteLanAdvertisedEndpoint)>,
    next_transition_id: u64,
}

/// A staged endpoint generation. Dropping it aborts only its own pending generation.
pub struct RemoteLanAdvertisedEndpointTransition {
    state: Arc<Mutex<RemoteLanAdvertisedEndpointState>>,
    transition_id: u64,
    endpoint: RemoteLanAdvertisedEndpoint,
    completed: bool,
}

impl RemoteLanAdvertisedEndpointTransition {
    pub fn endpoint(&self) -> &RemoteLanAdvertisedEndpoint {
        &self.endpoint
    }

    pub fn commit(mut self) -> HostResult<RemoteLanAdvertisedEndpoint> {
        let mut state = self.state.lock().map_err(|_| {
            HostError::new(
                "remote_lan_advertised_endpoint_unavailable",
                "Remote LAN advertised endpoint state is unavailable",
            )
            .retryable(true)
        })?;
        if !state
            .pending
            .as_ref()
            .is_some_and(|(transition_id, endpoint)| {
                *transition_id == self.transition_id && endpoint == &self.endpoint
            })
        {
            return Err(HostError::new(
                "remote_lan_advertised_generation_stale",
                "Remote LAN advertised endpoint generation is no longer current",
            )
            .retryable(true));
        }
        state.current = Some(self.endpoint.clone());
        state.pending = None;
        self.completed = true;
        Ok(self.endpoint.clone())
    }

    /// Clears both the pending endpoint and the obsolete committed endpoint.
    pub fn fail_closed(mut self) {
        if let Ok(mut state) = self.state.lock() {
            if state
                .pending
                .as_ref()
                .is_some_and(|(transition_id, _)| *transition_id == self.transition_id)
            {
                state.current = None;
                state.pending = None;
            }
        }
        self.completed = true;
    }
}

impl Drop for RemoteLanAdvertisedEndpointTransition {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            if state
                .pending
                .as_ref()
                .is_some_and(|(transition_id, _)| *transition_id == self.transition_id)
            {
                state.pending = None;
            }
        }
    }
}

/// Running listener metadata plus bounded shutdown ownership.
pub struct RemoteLanServerHandle {
    listener_id: u64,
    local_addr: SocketAddr,
    remote_identity: lan::LanHostIdentity,
    advertised_endpoints: Arc<Mutex<RemoteLanAdvertisedEndpointState>>,
    sessions: Arc<SessionRegistry>,
    server_handle: axum_server::Handle,
    server_task: Option<JoinHandle<std::io::Result<()>>>,
}

impl RemoteLanServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }

    pub fn advertised_endpoint(&self) -> Option<RemoteLanAdvertisedEndpoint> {
        self.advertised_endpoints
            .lock()
            .ok()
            .and_then(|state| state.current.clone())
    }

    /// Returns the immutable Host identity validated when this listener started.
    pub fn remote_host_identity(&self) -> &lan::LanHostIdentity {
        &self.remote_identity
    }

    pub fn advertisement_source(&self) -> RemoteLanAdvertisementSource {
        RemoteLanAdvertisementSource {
            listener_id: self.listener_id,
            local_addr: self.local_addr,
            remote_identity: self.remote_identity.clone(),
        }
    }

    pub fn advertised_host(&self) -> Option<String> {
        self.advertised_endpoint()
            .map(|endpoint| endpoint.advertised_host().to_string())
    }

    pub fn https_base_url(&self) -> Option<String> {
        self.advertised_endpoint()
            .map(|endpoint| endpoint.https_base_url().to_string())
    }

    pub fn gateway_url(&self) -> Option<String> {
        self.advertised_endpoint()
            .map(|endpoint| endpoint.gateway_url().to_string())
    }

    /// Stages a client-visible endpoint without exposing it through status/QR yet.
    ///
    /// Pairing exchange requests whose `Host` authority matches the staged endpoint
    /// already receive its WSS URL. This closes the short interval between the mDNS
    /// daemon sending a new Announce and the runtime committing that generation.
    pub fn stage_advertised_host(
        &self,
        advertised_host: impl AsRef<str>,
    ) -> HostResult<RemoteLanAdvertisedEndpointTransition> {
        let advertised_host = resolve_advertised_host(
            self.local_addr.ip(),
            Some(advertised_host.as_ref()),
        )?;
        let endpoint = RemoteLanAdvertisedEndpoint::new(
            self.listener_id,
            advertised_host,
            self.port(),
        );
        let mut state = self.advertised_endpoints.lock().map_err(|_| {
            HostError::new(
                "remote_lan_advertised_endpoint_unavailable",
                "Remote LAN advertised endpoint state is unavailable",
            )
            .retryable(true)
        })?;
        let transition_id = state.next_transition_id;
        state.next_transition_id = state.next_transition_id.wrapping_add(1).max(1);
        state.pending = Some((transition_id, endpoint.clone()));
        Ok(RemoteLanAdvertisedEndpointTransition {
            state: self.advertised_endpoints.clone(),
            transition_id,
            endpoint,
            completed: false,
        })
    }

    /// Removes the committed and staged advertised endpoint while keeping the listener.
    pub fn withdraw_advertised_endpoint(&self) {
        if let Ok(mut state) = self.advertised_endpoints.lock() {
            state.current = None;
            state.pending = None;
        }
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions.active_count()
    }

    pub fn active_session_count_for_credential(&self, credential_id: &str) -> usize {
        self.sessions.credential_active_count(credential_id)
    }

    /// Cancels every socket authenticated by one credential and waits a bounded interval.
    pub async fn disconnect_credential(&self, credential_id: &str) -> HostResult<usize> {
        self.disconnect_credentials([credential_id]).await
    }

    /// Cancels every socket in a credential group before waiting on one shared deadline.
    pub async fn disconnect_credentials<I, S>(&self, credential_ids: I) -> HostResult<usize>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let credential_ids = credential_ids
            .into_iter()
            .map(|credential_id| credential_id.as_ref().to_string())
            .collect::<BTreeSet<_>>();
        let outcome = self
            .sessions
            .cancel_and_wait_credentials(&credential_ids, CONNECTION_CLOSE_TIMEOUT)
            .await;
        if outcome.remaining == 0 {
            Ok(outcome.active)
        } else {
            Err(HostError::new(
                "remote_lan_credential_disconnect_timeout",
                "Remote credential sessions did not close within the shared bounded deadline",
            )
            .retryable(true)
            .with_detail("credentialCount", credential_ids.len() as u64)
            .with_detail("activeSessionCount", outcome.active as u64)
            .with_detail("remainingCredentialCount", outcome.remaining as u64))
        }
    }

    pub async fn shutdown(mut self) -> HostResult<()> {
        self.sessions.shutdown();
        self.server_handle
            .graceful_shutdown(Some(SERVER_SHUTDOWN_TIMEOUT));
        let sessions_closed = self.sessions.wait_empty(SERVER_SHUTDOWN_TIMEOUT).await;
        if !sessions_closed {
            self.server_handle.shutdown();
        }

        let Some(mut server_task) = self.server_task.take() else {
            return Ok(());
        };
        match timeout(SERVER_SHUTDOWN_TIMEOUT, &mut server_task).await {
            Ok(Ok(Ok(()))) if self.sessions.active_count() == 0 => Ok(()),
            Ok(Ok(Ok(()))) => Err(HostError::new(
                "remote_lan_session_shutdown_timeout",
                "Remote LAN sessions did not shut down within the bounded deadline",
            )
            .retryable(true)),
            Ok(Ok(Err(error))) => Err(HostError::new(
                "remote_lan_listener_failed",
                format!("Remote LAN listener failed: {error}"),
            )
            .retryable(true)),
            Ok(Err(error)) => Err(HostError::new(
                "remote_lan_listener_task_failed",
                format!("Remote LAN listener task failed: {error}"),
            )),
            Err(_) => {
                self.server_handle.shutdown();
                server_task.abort();
                let _ = server_task.await;
                Err(HostError::new(
                    "remote_lan_listener_shutdown_timeout",
                    "Remote LAN listener did not shut down within the bounded deadline",
                )
                .retryable(true))
            }
        }
    }
}

impl Debug for RemoteLanServerHandle {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteLanServerHandle")
            .field("local_addr", &self.local_addr)
            .field("advertised_endpoint", &self.advertised_endpoint())
            .field("active_session_count", &self.active_session_count())
            .finish()
    }
}

impl Drop for RemoteLanServerHandle {
    fn drop(&mut self) {
        self.sessions.shutdown();
        self.server_handle.shutdown();
        if let Some(server_task) = self.server_task.take() {
            server_task.abort();
        }
    }
}

struct RemoteLanState {
    remote_access: Arc<RemoteAccessManager>,
    gateway: Arc<ProviderGatewayService>,
    remote_identity: lan::LanHostIdentity,
    advertised_endpoints: Arc<Mutex<RemoteLanAdvertisedEndpointState>>,
    sessions: Arc<SessionRegistry>,
}

async fn pairing_exchange(
    State(state): State<Arc<RemoteLanState>>,
    Path(pairing_id): Path<String>,
    headers: HeaderMap,
    request: Result<Json<lan::PairingExchangeRequest>, JsonRejection>,
) -> Result<Json<lan::PairingExchangeResponse>, RestError> {
    let gateway_url = pairing_exchange_gateway_url(&state.advertised_endpoints, &headers)?;
    let Json(request) = request.map_err(RestError::invalid_json)?;
    let issued = state
        .remote_access
        .complete_pairing(&pairing_id, request)
        .map_err(RestError::pairing)?;
    Ok(Json(lan::PairingExchangeResponse {
        device: state.remote_identity.clone(),
        gateway_url,
        credential: issued.bearer_token,
    }))
}

async fn delete_current_credential(
    State(state): State<Arc<RemoteLanState>>,
    headers: HeaderMap,
) -> Result<Json<lan::CurrentCredentialDeleteResponse>, RestError> {
    let bearer = bearer_from_headers(&headers)?;
    let revoked = state
        .remote_access
        .revoke_current_credential(bearer)
        .map_err(RestError::authorization)?;
    state.sessions.cancel_credential(&revoked.credential_id);
    Ok(Json(lan::CurrentCredentialDeleteResponse { revoked: true }))
}

async fn gateway_websocket(
    State(state): State<Arc<RemoteLanState>>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, RestError> {
    let bearer = bearer_from_headers(&headers)?;
    let credential = state
        .remote_access
        .validate_bearer(bearer)
        .map_err(RestError::authorization)?;
    let registration = state
        .sessions
        .register(&credential.credential_id)
        .map_err(RestError::session_registration)?;
    state
        .remote_access
        .validate_bearer(bearer)
        .map_err(RestError::authorization)?;
    let gateway = state.gateway.clone();
    let remote_access = state.remote_access.clone();
    Ok(websocket
        .max_frame_size(MAX_WEBSOCKET_FRAME_BYTES)
        .max_message_size(MAX_WEBSOCKET_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            run_gateway_socket(
                socket,
                gateway,
                remote_access,
                credential,
                registration,
            )
        }))
}

async fn run_gateway_socket(
    socket: WebSocket,
    gateway: Arc<ProviderGatewayService>,
    remote_access: Arc<RemoteAccessManager>,
    credential: RemoteCredential,
    mut registration: SessionRegistration,
) {
    let (mut sink, mut source) = socket.split();
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Message>(OUTBOUND_QUEUE_CAPACITY);
    let (request_tx, request_rx) =
        mpsc::channel::<gateway::ProtocolRequest>(REQUEST_QUEUE_CAPACITY);
    let (stop_tx, _) = watch::channel(false);
    let (writer_done_tx, mut writer_done) = watch::channel(false);
    let (transport_failed_tx, mut transport_failed) = watch::channel(false);
    let writer_failed = transport_failed_tx.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            let closing = matches!(message, Message::Close(_));
            let sent = timeout(WEBSOCKET_SEND_TIMEOUT, sink.send(message)).await;
            if !matches!(sent, Ok(Ok(()))) {
                let _ = writer_failed.send(true);
                break;
            }
            if closing {
                break;
            }
        }
        let _ = writer_done_tx.send(true);
    });
    let request_dispatcher = tokio::spawn(run_gateway_requests(
        request_rx,
        gateway.clone(),
        format!("remote-client:{}", credential.client_id),
        outbound_tx.clone(),
        transport_failed_tx.clone(),
        stop_tx.subscribe(),
    ));

    let mut handshaken = false;
    let mut subscribed = false;
    let mut event_task: Option<JoinHandle<()>> = None;
    let mut close_frame = None;
    let caller_scope = format!("remote-client:{}", credential.client_id);

    loop {
        let next = tokio::select! {
            cancellation = registration.cancelled() => {
                close_frame = Some(match cancellation {
                    SessionCancellation::CredentialRevoked => close_message(1008, "credential_revoked"),
                    SessionCancellation::ServerShutdown => close_message(1001, "server_shutdown"),
                });
                break;
            }
            changed = writer_done.changed() => {
                let _ = changed;
                break;
            }
            changed = transport_failed.changed() => {
                if changed.is_err() || *transport_failed.borrow() {
                    break;
                }
                continue;
            }
            next = source.next() => next,
        };
        let Some(message) = next else {
            break;
        };
        let message = match message {
            Ok(message) => message,
            Err(_) => {
                close_frame = Some(close_message(1002, "websocket_protocol_error"));
                break;
            }
        };
        let text = match message {
            Message::Text(text) => text,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => break,
            Message::Binary(_) => {
                close_frame = Some(close_message(1003, "text_frames_required"));
                break;
            }
        };
        let request = match gateway::decode_wire_message(text.as_bytes()) {
            Ok(gateway::ProviderWireMessage::Request(
                gateway::JsonRpcInboundRequest::Typed(request),
            )) => request,
            Ok(gateway::ProviderWireMessage::Request(
                gateway::JsonRpcInboundRequest::Rejected(rejection),
            )) => {
                if !queue_json(&outbound_tx, &rejection.into_response(), &mut registration).await {
                    break;
                }
                if !handshaken {
                    close_frame = Some(close_message(1008, "protocol_handshake_required"));
                    break;
                }
                continue;
            }
            Err(error) => {
                if !queue_json(&outbound_tx, &error.into_response(), &mut registration).await {
                    break;
                }
                if !handshaken {
                    close_frame = Some(close_message(1008, "protocol_handshake_required"));
                    break;
                }
                continue;
            }
            Ok(_) => {
                close_frame = Some(close_message(1008, "gateway_requests_required"));
                break;
            }
        };

        if !handshaken {
            let gateway::ProtocolRequest::ProtocolHandshake {
                jsonrpc,
                id,
                params,
            } = request
            else {
                close_frame = Some(close_message(1008, "protocol_handshake_required"));
                break;
            };
            if params.client_id != credential.client_id {
                let response = json_rpc_error_response(
                    jsonrpc,
                    Some(id),
                    protocol_error(
                        "gateway_client_identity_mismatch",
                        "Handshake clientId does not match the authenticated credential",
                        false,
                    ),
                );
                if !queue_json(&outbound_tx, &response, &mut registration).await {
                    break;
                }
                close_frame = Some(close_message(1008, "gateway_client_identity_mismatch"));
                break;
            }
            let device_descriptor = params.device.clone();
            let response_id = id.clone();
            let request = gateway::ProtocolRequest::ProtocolHandshake {
                jsonrpc: jsonrpc.clone(),
                id,
                params,
            };
            let mut response = tokio::select! {
                cancellation = registration.cancelled() => {
                    close_frame = Some(cancellation_close(cancellation));
                    break;
                }
                response = gateway.dispatch_for_caller_scope(&caller_scope, request) => response,
            };
            let mut succeeded = matches!(
                &response.response,
                gateway::JsonRpcResponsePayload::Ok { .. }
            );
            if succeeded {
                if let Err(error) = remote_access.update_credential_descriptor(
                    &credential.credential_id,
                    device_descriptor,
                ) {
                    response = json_rpc_error_response(
                        jsonrpc,
                        Some(response_id),
                        error.into_protocol_error(),
                    );
                    succeeded = false;
                    close_frame = Some(close_message(
                        1008,
                        "gateway_client_descriptor_persistence_failed",
                    ));
                }
            }
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            if !succeeded {
                if close_frame.is_none() {
                    close_frame = Some(close_message(1008, "protocol_handshake_rejected"));
                }
                break;
            }
            handshaken = true;
            continue;
        }

        if let gateway::ProtocolRequest::ProtocolHandshake {
            jsonrpc,
            id,
            ..
        } = &request
        {
            let response = json_rpc_error_response(
                jsonrpc.clone(),
                Some(id.clone()),
                protocol_error(
                    "gateway_handshake_already_completed",
                    "protocol.handshake may succeed only once per socket",
                    false,
                ),
            );
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            continue;
        }

        if let gateway::ProtocolRequest::EventSubscribe {
            jsonrpc,
            id,
            params,
        } = &request
        {
            if subscribed {
                let response = json_rpc_error_response(
                    jsonrpc.clone(),
                    Some(id.clone()),
                    protocol_error(
                        "gateway_event_already_subscribed",
                        "event.subscribe may succeed only once per socket",
                        false,
                    ),
                );
                if !queue_json(&outbound_tx, &response, &mut registration).await {
                    break;
                }
                continue;
            }
            let subscription = gateway.subscribe_events(Some(&params.after_cursor));
            let dispatcher = EventSubscribeDispatcher {
                response: subscription
                    .as_ref()
                    .map(|_| gateway::EventSubscribeResponse {
                        subscribed_after_cursor: params.after_cursor.clone(),
                    })
                    .map_err(|error| error.clone()),
            };
            let response = tokio::select! {
                cancellation = registration.cancelled() => {
                    close_frame = Some(cancellation_close(cancellation));
                    break;
                }
                response = gateway::dispatch(&dispatcher, request) => response,
            };
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            let Ok(mut subscription) = subscription else {
                continue;
            };
            subscribed = true;
            let event_outbound = outbound_tx.clone();
            let event_transport_failed = transport_failed_tx.clone();
            let mut event_stop = stop_tx.subscribe();
            event_task = Some(tokio::spawn(async move {
                loop {
                    let event = tokio::select! {
                        changed = event_stop.changed() => {
                            if changed.is_err() || *event_stop.borrow() {
                                break;
                            }
                            continue;
                        }
                        event = subscription.next_event() => event,
                    };
                    let event = match event {
                        Ok(event) => event,
                        Err(error) => {
                            if !enqueue_outbound(
                                &event_outbound,
                                close_message(1011, error.code),
                            )
                            .await
                            {
                                let _ = event_transport_failed.send(true);
                            }
                            break;
                        }
                    };
                    let text = match serde_json::to_string(&event) {
                        Ok(text) => text,
                        Err(_) => {
                            if !enqueue_outbound(
                                &event_outbound,
                                close_message(1011, "gateway_event_encoding_failed"),
                            )
                            .await
                            {
                                let _ = event_transport_failed.send(true);
                            }
                            break;
                        }
                    };
                    if !enqueue_outbound(&event_outbound, Message::Text(text)).await {
                        let _ = event_transport_failed.send(true);
                        break;
                    }
                }
            }));
            continue;
        }

        match request_tx.try_send(request) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                close_frame = Some(close_message(1013, "gateway_request_queue_full"));
                break;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }

    let _ = stop_tx.send(true);
    drop(request_tx);
    if let Some(close_frame) = close_frame {
        let _ = outbound_tx.try_send(close_frame);
    }
    if let Some(mut event_task) = event_task {
        if timeout(CONNECTION_CLOSE_TIMEOUT, &mut event_task).await.is_err() {
            event_task.abort();
            let _ = event_task.await;
        }
    }
    let mut request_dispatcher = request_dispatcher;
    if timeout(CONNECTION_CLOSE_TIMEOUT, &mut request_dispatcher)
        .await
        .is_err()
    {
        request_dispatcher.abort();
        let _ = request_dispatcher.await;
    }
    drop(outbound_tx);
    let mut writer = writer;
    if timeout(CONNECTION_CLOSE_TIMEOUT, &mut writer).await.is_err() {
        writer.abort();
        let _ = writer.await;
    }
}

async fn run_gateway_requests(
    mut requests: mpsc::Receiver<gateway::ProtocolRequest>,
    gateway: Arc<ProviderGatewayService>,
    caller_scope: String,
    outbound: mpsc::Sender<Message>,
    transport_failed: watch::Sender<bool>,
    mut stop: watch::Receiver<bool>,
) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if completed.is_some_and(|result| result.is_err()) {
                    let _ = transport_failed.send(true);
                    break;
                }
            }
            request = requests.recv(), if tasks.len() < MAX_CONCURRENT_REQUESTS_PER_SESSION => {
                let Some(request) = request else {
                    break;
                };
                let request_gateway = gateway.clone();
                let request_scope = caller_scope.clone();
                let request_outbound = outbound.clone();
                let request_transport_failed = transport_failed.clone();
                let mut request_stop = stop.clone();
                tasks.spawn(async move {
                    let response = tokio::select! {
                        biased;
                        changed = request_stop.changed() => {
                            let _ = changed;
                            return;
                        }
                        response = request_gateway.dispatch_for_caller_scope(&request_scope, request) => response,
                    };
                    let text = match serde_json::to_string(&response) {
                        Ok(text) => text,
                        Err(_) => {
                            let _ = request_transport_failed.send(true);
                            return;
                        }
                    };
                    if !enqueue_outbound(&request_outbound, Message::Text(text)).await {
                        let _ = request_transport_failed.send(true);
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn queue_json<T: serde::Serialize>(
    outbound: &mpsc::Sender<Message>,
    value: &T,
    registration: &mut SessionRegistration,
) -> bool {
    let text = match serde_json::to_string(value) {
        Ok(text) => text,
        Err(_) => return false,
    };
    tokio::select! {
        cancellation = registration.cancelled() => {
            let _ = cancellation;
            false
        }
        sent = enqueue_outbound(outbound, Message::Text(text)) => sent,
    }
}

async fn enqueue_outbound(outbound: &mpsc::Sender<Message>, message: Message) -> bool {
    matches!(
        timeout(OUTBOUND_ENQUEUE_TIMEOUT, outbound.send(message)).await,
        Ok(Ok(()))
    )
}

fn bearer_from_headers(headers: &HeaderMap) -> Result<&str, RestError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(RestError::missing_authorization)?;
    let bearer = value
        .strip_prefix("Bearer ")
        .filter(|bearer| !bearer.is_empty() && !bearer.contains(char::is_whitespace))
        .ok_or_else(RestError::missing_authorization)?;
    Ok(bearer)
}

fn pairing_exchange_gateway_url(
    endpoints: &Mutex<RemoteLanAdvertisedEndpointState>,
    headers: &HeaderMap,
) -> Result<String, RestError> {
    let state = endpoints
        .lock()
        .map_err(|_| RestError::discovery_unavailable())?;
    let requested_authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if let (Some(requested_authority), Some((_, pending))) =
        (requested_authority, state.pending.as_ref())
    {
        if requested_authority.eq_ignore_ascii_case(pending.authority()) {
            return Ok(pending.gateway_url().to_string());
        }
    }
    state
        .current
        .as_ref()
        .map(|endpoint| endpoint.gateway_url().to_string())
        .ok_or_else(RestError::discovery_unavailable)
}

fn resolve_advertised_host(
    bind_ip: IpAddr,
    configured_host: Option<&str>,
) -> HostResult<String> {
    let host = match configured_host {
        Some(host) => host.trim(),
        None if bind_ip.is_unspecified() => {
            return Err(HostError::new(
                "remote_lan_advertised_host_required",
                "Remote LAN wildcard bindings require an explicit advertised host",
            ));
        }
        None => return Ok(bind_ip.to_string()),
    };
    if host.is_empty() {
        return Err(invalid_advertised_host());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip.is_unspecified() {
            return Err(invalid_advertised_host());
        }
        return Ok(ip.to_string());
    }
    if host.len() > 253
        || !host.is_ascii()
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(invalid_advertised_host());
    }
    Ok(host.to_string())
}

fn invalid_advertised_host() -> HostError {
    HostError::new(
        "invalid_remote_lan_advertised_host",
        "Remote LAN advertised host must be a concrete IP address or DNS host without a port",
    )
}

fn advertised_url_authority(host: &str, port: u16) -> String {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V6(ip)) => format!("[{ip}]:{port}"),
        _ => format!("{host}:{port}"),
    }
}

fn close_message(code: u16, reason: impl Into<Cow<'static, str>>) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

fn cancellation_close(cancellation: SessionCancellation) -> Message {
    match cancellation {
        SessionCancellation::CredentialRevoked => close_message(1008, "credential_revoked"),
        SessionCancellation::ServerShutdown => close_message(1001, "server_shutdown"),
    }
}

fn protocol_error(
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> gateway::ProtocolError {
    gateway::ProtocolError {
        code: code.into(),
        message: message.into(),
        retryable,
        details: None,
    }
}

fn json_rpc_error_response(
    jsonrpc: String,
    id: Option<gateway::RequestId>,
    error: gateway::ProtocolError,
) -> gateway::JsonRpcResponse {
    let mut data = error.details.unwrap_or_default();
    data.insert("code".to_string(), serde_json::Value::String(error.code));
    data.insert(
        "retryable".to_string(),
        serde_json::Value::Bool(error.retryable),
    );
    gateway::JsonRpcResponse {
        jsonrpc,
        id,
        response: gateway::JsonRpcResponsePayload::Error {
            error: gateway::RpcError {
                code: -32000,
                message: error.message,
                data: Some(data),
            },
        },
    }
}

struct EventSubscribeDispatcher {
    response: Result<gateway::EventSubscribeResponse, gateway::ProtocolError>,
}

impl gateway::ProtocolServer for EventSubscribeDispatcher {
    fn event_subscribe<'a>(
        &'a self,
        _request: gateway::EventSubscribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::EventSubscribeResponse> {
        let response = self.response.clone();
        Box::pin(async move { response })
    }
}

struct RestError {
    status: StatusCode,
    error: gateway::ProtocolError,
}

impl RestError {
    fn invalid_json(rejection: JsonRejection) -> Self {
        let status = if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        };
        Self {
            status,
            error: protocol_error(
                if status == StatusCode::PAYLOAD_TOO_LARGE {
                    "remote_request_too_large"
                } else {
                    "invalid_remote_request"
                },
                if status == StatusCode::PAYLOAD_TOO_LARGE {
                    "Remote request body exceeds the transport limit"
                } else {
                    "Remote request body must be valid Gateway JSON"
                },
                false,
            ),
        }
    }

    fn pairing(error: HostError) -> Self {
        let status = match error.code.as_str() {
            "invalid_pairing_session" | "pairing_session_expired" => StatusCode::UNAUTHORIZED,
            "invalid_remote_client_identity" | "invalid_remote_access_id" => {
                StatusCode::BAD_REQUEST
            }
            "remote_credential_limit_reached" => StatusCode::CONFLICT,
            _ if error.retryable => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            error: error.into_protocol_error(),
        }
    }

    fn discovery_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            error: protocol_error(
                "remote_lan_discovery_unavailable",
                "Remote LAN discovery has no current advertised endpoint",
                true,
            ),
        }
    }

    fn missing_authorization() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: protocol_error(
                "remote_authorization_required",
                "Authorization Bearer credential is required",
                false,
            ),
        }
    }

    fn authorization(_error: HostError) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: protocol_error(
                "invalid_remote_credential",
                "Remote bearer credential is invalid or revoked",
                false,
            ),
        }
    }

    fn session_registration(error: SessionRegistrationError) -> Self {
        match error {
            SessionRegistrationError::ShuttingDown => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                error: protocol_error(
                    "remote_lan_listener_shutting_down",
                    "Remote LAN listener is shutting down",
                    true,
                ),
            },
            SessionRegistrationError::LimitReached => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                error: protocol_error(
                    "remote_lan_session_limit_reached",
                    "Remote LAN listener session limit is reached",
                    true,
                ),
            },
            SessionRegistrationError::CredentialRevoked => Self {
                status: StatusCode::UNAUTHORIZED,
                error: protocol_error(
                    "invalid_remote_credential",
                    "Remote bearer credential is invalid or revoked",
                    false,
                ),
            },
        }
    }
}

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        (self.status, Json(self.error)).into_response()
    }
}

#[derive(Clone, Copy)]
enum SessionCancellation {
    CredentialRevoked,
    ServerShutdown,
}

struct SessionGroup {
    sender: broadcast::Sender<SessionCancellation>,
    active: usize,
    cancelled: bool,
}

struct SessionRegistryState {
    groups: BTreeMap<String, SessionGroup>,
    active: usize,
    shutting_down: bool,
}

struct SessionRegistry {
    state: Mutex<SessionRegistryState>,
    empty: Notify,
    slots: Arc<Semaphore>,
}

#[derive(Debug, PartialEq, Eq)]
struct CredentialGroupDisconnectOutcome {
    active: usize,
    remaining: usize,
}

impl SessionRegistry {
    fn new(max_sessions: usize) -> Self {
        Self {
            state: Mutex::new(SessionRegistryState {
                groups: BTreeMap::new(),
                active: 0,
                shutting_down: false,
            }),
            empty: Notify::new(),
            slots: Arc::new(Semaphore::new(max_sessions)),
        }
    }

    fn register(
        self: &Arc<Self>,
        credential_id: &str,
    ) -> Result<SessionRegistration, SessionRegistrationError> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionRegistrationError::LimitReached)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| SessionRegistrationError::ShuttingDown)?;
        if state.shutting_down {
            return Err(SessionRegistrationError::ShuttingDown);
        }
        let group = state.groups.entry(credential_id.to_string()).or_insert_with(|| {
            let (sender, _) = broadcast::channel(1);
            SessionGroup {
                sender,
                active: 0,
                cancelled: false,
            }
        });
        if group.cancelled {
            return Err(SessionRegistrationError::CredentialRevoked);
        }
        group.active += 1;
        let sender = group.sender.clone();
        let receiver = sender.subscribe();
        state.active += 1;
        Ok(SessionRegistration {
            registry: self.clone(),
            credential_id: credential_id.to_string(),
            sender,
            receiver,
            _permit: permit,
        })
    }

    fn cancel_credential(&self, credential_id: &str) -> usize {
        let cancelled = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| {
                state.groups.get_mut(credential_id).map(|group| {
                    group.cancelled = true;
                    (group.sender.clone(), group.active)
                })
            });
        let Some((sender, active)) = cancelled else {
            return 0;
        };
        if active > 0 {
            let _ = sender.send(SessionCancellation::CredentialRevoked);
        }
        active
    }

    fn shutdown(&self) {
        let senders = match self.state.lock() {
            Ok(mut state) => {
                if state.shutting_down {
                    return;
                }
                state.shutting_down = true;
                std::mem::take(&mut state.groups)
                    .into_values()
                    .map(|group| group.sender)
                    .collect::<Vec<_>>()
            }
            Err(_) => return,
        };
        for sender in senders {
            let _ = sender.send(SessionCancellation::ServerShutdown);
        }
    }

    fn active_count(&self) -> usize {
        self.state.lock().map(|state| state.active).unwrap_or(0)
    }

    fn credential_active_count(&self, credential_id: &str) -> usize {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.groups.get(credential_id).map(|group| group.active))
            .unwrap_or(0)
    }

    async fn cancel_and_wait_credentials(
        &self,
        credential_ids: &BTreeSet<String>,
        duration: Duration,
    ) -> CredentialGroupDisconnectOutcome {
        let active = credential_ids.iter().fold(0_usize, |total, credential_id| {
            total.saturating_add(self.cancel_credential(credential_id))
        });
        let drained = self
            .wait_credentials_empty(credential_ids, duration)
            .await;
        let remaining = if drained {
            0
        } else {
            credential_ids
                .iter()
                .filter(|credential_id| self.credential_active_count(credential_id) > 0)
                .count()
        };
        CredentialGroupDisconnectOutcome { active, remaining }
    }

    async fn wait_credentials_empty(
        &self,
        credential_ids: &BTreeSet<String>,
        duration: Duration,
    ) -> bool {
        if credential_ids
            .iter()
            .all(|credential_id| self.credential_active_count(credential_id) == 0)
        {
            return true;
        }
        let waited = timeout(duration, async {
            loop {
                self.empty.notified().await;
                if credential_ids
                    .iter()
                    .all(|credential_id| self.credential_active_count(credential_id) == 0)
                {
                    return;
                }
            }
        })
        .await;
        waited.is_ok()
            || credential_ids
                .iter()
                .all(|credential_id| self.credential_active_count(credential_id) == 0)
    }

    async fn wait_empty(&self, duration: Duration) -> bool {
        if self.active_count() == 0 {
            return true;
        }
        timeout(duration, async {
            loop {
                self.empty.notified().await;
                if self.active_count() == 0 {
                    return;
                }
            }
        })
        .await
        .is_ok()
    }

    fn unregister(&self, credential_id: &str, sender: &broadcast::Sender<SessionCancellation>) {
        match self.state.lock() {
            Ok(mut state) => {
                if state.active > 0 {
                    state.active -= 1;
                }
                let remove_group = state
                    .groups
                    .get_mut(credential_id)
                    .filter(|group| group.sender.same_channel(sender))
                    .map(|group| {
                        if group.active > 0 {
                            group.active -= 1;
                        }
                        group.active == 0
                    })
                    .unwrap_or(false);
                if remove_group {
                    state.groups.remove(credential_id);
                }
            }
            Err(_) => return,
        }
        self.empty.notify_waiters();
    }
}

struct SessionRegistration {
    registry: Arc<SessionRegistry>,
    credential_id: String,
    sender: broadcast::Sender<SessionCancellation>,
    receiver: broadcast::Receiver<SessionCancellation>,
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug)]
enum SessionRegistrationError {
    ShuttingDown,
    LimitReached,
    CredentialRevoked,
}

impl SessionRegistration {
    async fn cancelled(&mut self) -> SessionCancellation {
        match self.receiver.recv().await {
            Ok(cancellation) => cancellation,
            Err(_) => SessionCancellation::ServerShutdown,
        }
    }
}

impl Drop for SessionRegistration {
    fn drop(&mut self) {
        self.registry.unregister(&self.credential_id, &self.sender);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RestError, SessionCancellation, SessionRegistrationError, SessionRegistry,
    };
    use axum::http::StatusCode;
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn session_registry_enforces_the_global_limit() {
        let registry = Arc::new(SessionRegistry::new(1));
        let first = registry.register("credential-a").unwrap();
        assert!(matches!(
            registry.register("credential-b"),
            Err(SessionRegistrationError::LimitReached)
        ));
        drop(first);
        assert!(registry.register("credential-b").is_ok());
        let limit_error =
            RestError::session_registration(SessionRegistrationError::LimitReached);
        assert_eq!(limit_error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(limit_error.error.code, "remote_lan_session_limit_reached");
    }

    #[tokio::test]
    async fn credential_group_cancel_reaches_later_session_before_shared_timeout() {
        let registry = Arc::new(SessionRegistry::new(2));
        let stalled = registry.register("credential-a").unwrap();
        let mut later = registry.register("credential-b").unwrap();
        let later_cancelled = tokio::spawn(async move {
            matches!(
                later.cancelled().await,
                SessionCancellation::CredentialRevoked
            )
        });
        let credential_ids = [
            "credential-a".to_string(),
            "credential-b".to_string(),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();

        let outcome = registry
            .cancel_and_wait_credentials(&credential_ids, Duration::from_millis(25))
            .await;

        assert!(later_cancelled.await.unwrap());
        assert_eq!(outcome.active, 2);
        assert_eq!(outcome.remaining, 1);
        assert_eq!(registry.credential_active_count("credential-a"), 1);
        assert_eq!(registry.credential_active_count("credential-b"), 0);
        drop(stalled);
    }
}
