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
use futures_util::{SinkExt, StreamExt};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{
    broadcast, mpsc, watch, Notify, OwnedSemaphorePermit, Semaphore,
};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const PAIRING_EXCHANGE_PATH: &str = "/remote/v1/pairings/:pairing_id/exchange";
const GATEWAY_PATH: &str = "/remote/v1/gateway";
const CURRENT_CREDENTIAL_PATH: &str = "/remote/v1/credentials/current";
const MAX_REST_BODY_BYTES: usize = 64 * 1024;
const MAX_WEBSOCKET_FRAME_BYTES: usize = 256 * 1024;
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 * 1024;
const OUTBOUND_QUEUE_CAPACITY: usize = 64;
const MAX_CONCURRENT_WEBSOCKET_SESSIONS: usize = 32;
const WEBSOCKET_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const OUTBOUND_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECTION_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

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
        if gateway.remote_host_identity() != Some(&remote_identity) {
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
            HostError::new(
                "remote_lan_listener_bind_failed",
                format!("bind Remote LAN listener: {error}"),
            )
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

        let advertised_authority =
            advertised_url_authority(&advertised_host, local_addr.port());
        let https_base_url = format!("https://{advertised_authority}");
        let gateway_url = format!("wss://{advertised_authority}{GATEWAY_PATH}");
        let sessions = Arc::new(SessionRegistry::new(MAX_CONCURRENT_WEBSOCKET_SESSIONS));
        let state = Arc::new(RemoteLanState {
            remote_access,
            gateway,
            remote_identity,
            gateway_url: gateway_url.clone(),
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
            local_addr,
            advertised_host,
            https_base_url,
            gateway_url,
            sessions,
            server_handle,
            server_task: Some(server_task),
        })
    }
}

/// Running listener metadata plus bounded shutdown ownership.
pub struct RemoteLanServerHandle {
    local_addr: SocketAddr,
    advertised_host: String,
    https_base_url: String,
    gateway_url: String,
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

    pub fn advertised_host(&self) -> &str {
        &self.advertised_host
    }

    pub fn https_base_url(&self) -> &str {
        &self.https_base_url
    }

    pub fn gateway_url(&self) -> &str {
        &self.gateway_url
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions.active_count()
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
            .field("advertised_host", &self.advertised_host)
            .field("https_base_url", &self.https_base_url)
            .field("gateway_url", &self.gateway_url)
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
    remote_identity: gateway::RemoteHostIdentity,
    gateway_url: String,
    sessions: Arc<SessionRegistry>,
}

async fn pairing_exchange(
    State(state): State<Arc<RemoteLanState>>,
    Path(pairing_id): Path<String>,
    request: Result<Json<gateway::PairingExchangeRequest>, JsonRejection>,
) -> Result<Json<gateway::PairingExchangeResponse>, RestError> {
    let Json(request) = request.map_err(RestError::invalid_json)?;
    let issued = state
        .remote_access
        .complete_pairing(&pairing_id, request)
        .map_err(RestError::pairing)?;
    Ok(Json(gateway::PairingExchangeResponse {
        device: state.remote_identity.clone(),
        gateway_url: state.gateway_url.clone(),
        credential: issued.bearer_token,
    }))
}

async fn delete_current_credential(
    State(state): State<Arc<RemoteLanState>>,
    headers: HeaderMap,
) -> Result<Json<gateway::CurrentCredentialDeleteResponse>, RestError> {
    let bearer = bearer_from_headers(&headers)?;
    let revoked = state
        .remote_access
        .revoke_current_credential(bearer)
        .map_err(RestError::authorization)?;
    state.sessions.cancel_credential(&revoked.credential_id);
    Ok(Json(gateway::CurrentCredentialDeleteResponse { revoked: true }))
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
    Ok(websocket
        .max_frame_size(MAX_WEBSOCKET_FRAME_BYTES)
        .max_message_size(MAX_WEBSOCKET_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_gateway_socket(socket, gateway, credential, registration)))
}

async fn run_gateway_socket(
    socket: WebSocket,
    gateway: Arc<ProviderGatewayService>,
    credential: RemoteCredential,
    mut registration: SessionRegistration,
) {
    let (mut sink, mut source) = socket.split();
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Message>(OUTBOUND_QUEUE_CAPACITY);
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

    let mut handshaken = false;
    let mut subscribed = false;
    let mut event_task: Option<JoinHandle<()>> = None;
    let mut close_frame = None;

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
        let request = match serde_json::from_str::<gateway::ProtocolRequest>(&text) {
            Ok(request) => request,
            Err(_) => {
                close_frame = Some(close_message(1007, "invalid_gateway_json"));
                break;
            }
        };

        if !handshaken {
            let gateway::ProtocolRequest::ProtocolHandshake {
                protocol_version,
                id,
                params,
            } = request
            else {
                close_frame = Some(close_message(1008, "protocol_handshake_required"));
                break;
            };
            if params.client_id != credential.client_id {
                let response = gateway::ProtocolResponse::ProtocolHandshake {
                    protocol_version,
                    id,
                    response: gateway::ResponsePayload::Error {
                        error: protocol_error(
                            "gateway_client_identity_mismatch",
                            "Handshake clientId does not match the authenticated credential",
                            false,
                        ),
                    },
                };
                if !queue_json(&outbound_tx, &response, &mut registration).await {
                    break;
                }
                close_frame = Some(close_message(1008, "gateway_client_identity_mismatch"));
                break;
            }
            let request = gateway::ProtocolRequest::ProtocolHandshake {
                protocol_version,
                id,
                params,
            };
            let response = tokio::select! {
                cancellation = registration.cancelled() => {
                    close_frame = Some(cancellation_close(cancellation));
                    break;
                }
                response = gateway::dispatch(gateway.as_ref(), request) => response,
            };
            let succeeded = matches!(
                &response,
                gateway::ProtocolResponse::ProtocolHandshake {
                    response: gateway::ResponsePayload::Ok { .. },
                    ..
                }
            );
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            if !succeeded {
                close_frame = Some(close_message(1008, "protocol_handshake_rejected"));
                break;
            }
            handshaken = true;
            continue;
        }

        if let gateway::ProtocolRequest::ProtocolHandshake {
            protocol_version,
            id,
            ..
        } = &request
        {
            let response = gateway::ProtocolResponse::ProtocolHandshake {
                protocol_version: *protocol_version,
                id: id.clone(),
                response: gateway::ResponsePayload::Error {
                    error: protocol_error(
                        "gateway_handshake_already_completed",
                        "protocol.handshake may succeed only once per socket",
                        false,
                    ),
                },
            };
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            continue;
        }

        if let gateway::ProtocolRequest::EventSubscribe {
            protocol_version,
            id,
            params,
        } = &request
        {
            if subscribed {
                let response = gateway::ProtocolResponse::EventSubscribe {
                    protocol_version: *protocol_version,
                    id: id.clone(),
                    response: gateway::ResponsePayload::Error {
                        error: protocol_error(
                            "gateway_event_already_subscribed",
                            "event.subscribe may succeed only once per socket",
                            false,
                        ),
                    },
                };
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

        let response = tokio::select! {
            cancellation = registration.cancelled() => {
                close_frame = Some(cancellation_close(cancellation));
                break;
            }
            response = gateway::dispatch(gateway.as_ref(), request) => response,
        };
        if !queue_json(&outbound_tx, &response, &mut registration).await {
            break;
        }
    }

    let _ = stop_tx.send(true);
    if let Some(close_frame) = close_frame {
        let _ = outbound_tx.try_send(close_frame);
    }
    if let Some(mut event_task) = event_task {
        if timeout(CONNECTION_CLOSE_TIMEOUT, &mut event_task).await.is_err() {
            event_task.abort();
            let _ = event_task.await;
        }
    }
    drop(outbound_tx);
    let mut writer = writer;
    if timeout(CONNECTION_CLOSE_TIMEOUT, &mut writer).await.is_err() {
        writer.abort();
        let _ = writer.await;
    }
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
            SessionGroup { sender, active: 0 }
        });
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

    fn cancel_credential(&self, credential_id: &str) {
        let sender = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.groups.remove(credential_id))
            .map(|group| group.sender);
        if let Some(sender) = sender {
            let _ = sender.send(SessionCancellation::CredentialRevoked);
        }
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
        let notify = match self.state.lock() {
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
                state.active == 0
            }
            Err(_) => false,
        };
        if notify {
            self.empty.notify_waiters();
        }
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
    use super::{RestError, SessionRegistrationError, SessionRegistry};
    use axum::http::StatusCode;
    use std::sync::Arc;

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
}
