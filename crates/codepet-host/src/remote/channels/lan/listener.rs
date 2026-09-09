#[cfg(test)]
use super::super::session::SessionCancellation;
use super::super::session::{
    protocol_error, run_gateway_channel, ChannelFuture, GatewayChannel, GatewayFrame, GatewaySink,
    GatewaySource, SessionRegistrationError, SessionRegistry, CONNECTION_CLOSE_TIMEOUT,
};
use crate::{HostError, HostResult, ProviderGatewayService, RemoteAccessManager};
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use codepet_gateway_sdk as gateway;
use codepet_lan_channel_sdk as lan;
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeSet;
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use yawc::close::CloseCode;
use yawc::frame::{FrameView as Message, OpCode};
use yawc::{CompressionLevel, IncomingUpgrade, Options, WebSocket};

const PAIRING_EXCHANGE_PATH: &str = "/remote/v1/pairings/:pairing_id/exchange";
const PAIRING_REQUEST_CREATE_PATH: &str = "/remote/v1/pairing-requests";
const PAIRING_REQUEST_STATUS_PATH: &str = "/remote/v1/pairing-requests/:request_id";
const DISCOVERY_PATH: &str = "/remote/v1/discovery";
const GATEWAY_PATH: &str = "/remote/v1/gateway";
const CURRENT_CREDENTIAL_PATH: &str = "/remote/v1/credentials/current";
const MAX_REST_BODY_BYTES: usize = 64 * 1024;
const MAX_WEBSOCKET_FRAME_BYTES: usize = 256 * 1024;
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_CONCURRENT_WEBSOCKET_SESSIONS: usize = 32;
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
        let gateway_identity = crate::RemoteHostIdentity {
            device_id: remote_identity.device_id.clone(),
            descriptor: remote_identity.descriptor.clone(),
        };
        if gateway.remote_host_identity() != Some(&gateway_identity) {
            return Err(HostError::new(
                "remote_gateway_identity_mismatch",
                "Remote Gateway service identity must match the LAN TLS and Host identity",
            ));
        }
        let advertised_host =
            resolve_advertised_host(config.bind_addr.ip(), config.advertised_host.as_deref())?;

        let listener = TcpListener::bind(config.bind_addr).map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::AddrInUse {
                "remote_lan_listener_address_in_use"
            } else {
                "remote_lan_listener_bind_failed"
            };
            HostError::new(code, format!("bind Remote LAN listener: {error}")).retryable(true)
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
        let advertised_endpoint =
            RemoteLanAdvertisedEndpoint::new(listener_id, advertised_host, local_addr.port());
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
            .route(DISCOVERY_PATH, get(discovery))
            .route(PAIRING_EXCHANGE_PATH, post(pairing_exchange))
            .route(PAIRING_REQUEST_CREATE_PATH, post(create_pairing_request))
            .route(PAIRING_REQUEST_STATUS_PATH, get(pairing_request_status))
            .route(GATEWAY_PATH, get(gateway_websocket))
            .route("/remote/v1/webrtc/offer", post(gateway_rtc_offer))
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
        let advertised_host =
            resolve_advertised_host(self.local_addr.ip(), Some(advertised_host.as_ref()))?;
        let endpoint =
            RemoteLanAdvertisedEndpoint::new(self.listener_id, advertised_host, self.port());
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

#[derive(serde::Serialize)]
struct RemoteLanDiscoveryResponse {
    id: String,
    name: String,
    fp: String,
    vmin: u64,
    vmax: u64,
    pair: u8,
}

async fn discovery(State(state): State<Arc<RemoteLanState>>) -> Json<RemoteLanDiscoveryResponse> {
    let pairing_available = state
        .remote_access
        .subscribe_pairing_state()
        .borrow()
        .pairing_available;
    Json(RemoteLanDiscoveryResponse {
        id: state.remote_identity.device_id.clone(),
        name: state.remote_identity.descriptor.device_name.clone(),
        fp: state.remote_identity.identity_fingerprint.clone(),
        vmin: lan::CHANNEL_LAN_SCHEMA_VERSION,
        vmax: lan::CHANNEL_LAN_SCHEMA_VERSION,
        pair: u8::from(pairing_available),
    })
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

async fn create_pairing_request(
    State(state): State<Arc<RemoteLanState>>,
    headers: HeaderMap,
    request: Result<Json<lan::PairingRequestCreateRequest>, JsonRejection>,
) -> Result<Json<lan::PairingRequestStatusResponse>, RestError> {
    let gateway_url = pairing_exchange_gateway_url(&state.advertised_endpoints, &headers)?;
    let Json(request) = request.map_err(RestError::invalid_json)?;
    let pairing_request = state
        .remote_access
        .create_pairing_request(request)
        .map_err(RestError::pairing)?;
    Ok(Json(pairing_request_response(
        &state,
        pairing_request,
        gateway_url,
    )))
}

async fn pairing_request_status(
    State(state): State<Arc<RemoteLanState>>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<lan::PairingRequestStatusResponse>, RestError> {
    let gateway_url = pairing_exchange_gateway_url(&state.advertised_endpoints, &headers)?;
    let pairing_request = state
        .remote_access
        .pairing_request_status(&request_id)
        .map_err(RestError::pairing)?;
    Ok(Json(pairing_request_response(
        &state,
        pairing_request,
        gateway_url,
    )))
}

fn pairing_request_response(
    state: &RemoteLanState,
    request: crate::RemotePairingRequest,
    gateway_url: String,
) -> lan::PairingRequestStatusResponse {
    let accepted = request.state == lan::PairingRequestState::Accepted;
    let credential = request.bearer_token().map(str::to_string);
    lan::PairingRequestStatusResponse {
        request_id: request.request_id,
        state: request.state,
        device: state.remote_identity.clone(),
        expires_at: request.expires_at,
        confirmation_code: request.confirmation_code,
        gateway_url: accepted.then_some(gateway_url),
        credential,
    }
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
    websocket: IncomingUpgrade,
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
    let (response, upgrade) = websocket
        .upgrade(gateway_websocket_options())
        .map_err(RestError::websocket_upgrade)?;
    tokio::spawn(async move {
        if let Ok(socket) = upgrade.await {
            run_gateway_channel(
                websocket_channel(socket),
                gateway,
                remote_access,
                credential,
                registration,
            )
            .await;
        }
    });
    Ok(response.into_response())
}

async fn gateway_rtc_offer(
    State(state): State<Arc<RemoteLanState>>,
    headers: HeaderMap,
    request: Result<
        Json<webrtc::peer_connection::sdp::session_description::RTCSessionDescription>,
        JsonRejection,
    >,
) -> Result<Json<webrtc::peer_connection::sdp::session_description::RTCSessionDescription>, RestError>
{
    let bearer = bearer_from_headers(&headers)?;
    let credential = state
        .remote_access
        .validate_bearer(&bearer)
        .map_err(RestError::authorization)?;
    let registration = state
        .sessions
        .register(&credential.credential_id)
        .map_err(RestError::session_registration)?;
    state
        .remote_access
        .validate_bearer(&bearer)
        .map_err(RestError::authorization)?;
    let Json(offer) = request.map_err(|_| RestError {
        status: StatusCode::BAD_REQUEST,
        error: protocol_error("invalid_rtc_offer", "Expected an SDP offer", false),
    })?;
    super::super::webrtc::answer_offer(
        offer,
        state.gateway.clone(),
        state.remote_access.clone(),
        credential,
        registration,
    )
    .await
    .map(Json)
    .map_err(|error| RestError {
        status: StatusCode::BAD_REQUEST,
        error: protocol_error(error.code, error.message, error.retryable),
    })
}

fn gateway_websocket_options() -> Options {
    Options::default()
        .with_compression_level(CompressionLevel::default())
        .server_no_context_takeover()
        .client_no_context_takeover()
        .with_max_payload_read(MAX_WEBSOCKET_MESSAGE_BYTES)
        .with_max_read_buffer(MAX_WEBSOCKET_FRAME_BYTES * 2)
        .with_utf8()
}

fn websocket_channel(socket: WebSocket) -> GatewayChannel {
    let (sink, source) = socket.split();
    GatewayChannel {
        sink: Box::new(WebSocketSink(sink)),
        source: Box::new(WebSocketSource(source)),
    }
}

struct WebSocketSink(futures_util::stream::SplitSink<WebSocket, Message>);

impl GatewaySink for WebSocketSink {
    fn send(&mut self, frame: GatewayFrame) -> ChannelFuture<'_, bool> {
        Box::pin(async move {
            let message = match frame {
                GatewayFrame::Text(text) => match String::from_utf8(text) {
                    Ok(text) => Message::text(text),
                    Err(_) => return false,
                },
                GatewayFrame::Close { code, reason } => {
                    Message::close(CloseCode::from(code), reason)
                }
                _ => return false,
            };
            self.0.send(message).await.is_ok()
        })
    }
}

struct WebSocketSource(futures_util::stream::SplitStream<WebSocket>);

impl GatewaySource for WebSocketSource {
    fn recv(&mut self) -> ChannelFuture<'_, Option<GatewayFrame>> {
        Box::pin(async move {
            self.0.next().await.map(|message| match message.opcode {
                OpCode::Text => GatewayFrame::Text(message.payload.to_vec()),
                OpCode::Ping | OpCode::Pong => GatewayFrame::KeepAlive,
                OpCode::Close => GatewayFrame::Close {
                    code: 1000,
                    reason: String::new(),
                },
                _ => GatewayFrame::Invalid,
            })
        })
    }
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

fn resolve_advertised_host(bind_ip: IpAddr, configured_host: Option<&str>) -> HostResult<String> {
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

struct RestError {
    status: StatusCode,
    error: gateway::ProtocolError,
}

impl RestError {
    fn websocket_upgrade(_error: yawc::WebSocketError) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: protocol_error(
                "invalid_websocket_upgrade",
                "Remote Gateway request must be a valid WebSocket upgrade",
                false,
            ),
        }
    }

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
            "pairing_request_not_found" => StatusCode::NOT_FOUND,
            "invalid_remote_client_identity"
            | "invalid_remote_access_id"
            | "invalid_pairing_request"
            | "pairing_host_identity_mismatch" => StatusCode::BAD_REQUEST,
            "remote_credential_limit_reached" | "pairing_request_limit_reached" => {
                StatusCode::CONFLICT
            }
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

#[cfg(test)]
mod tests {
    use super::{RestError, SessionCancellation, SessionRegistrationError, SessionRegistry};
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
        let limit_error = RestError::session_registration(SessionRegistrationError::LimitReached);
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
        let credential_ids = ["credential-a".to_string(), "credential-b".to_string()]
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
