use codepet_gateway_sdk as gateway;
use codepet_lan_channel_sdk as lan;
use codepet_host::{
    DeviceRegistry, PluginCatalog, PluginCatalogConfig, PluginDescriptor,
    PluginInstanceConfig, PluginManager, PluginManagerConfig, PluginProcessOptions,
    ProviderGatewayService, ProviderInstanceRegistry, RemoteAccessConfig,
    RemoteAccessManager, RemoteLanServer, RemoteLanServerConfig,
};
use codepet_provider_sdk::JsonObject;
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::{CertificateDer, ServerName};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type TestWebSocket = WebSocketStream<TlsStream<TcpStream>>;

struct TestHost {
    _directory: TempDir,
    manager: Arc<PluginManager>,
    remote_access: Arc<RemoteAccessManager>,
    gateway: Arc<ProviderGatewayService>,
}

impl TestHost {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let device_path = directory.path().join("device.json");
        std::fs::write(
            &device_path,
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "deviceId": "device-lan-listener",
                "displayName": "LAN Listener Test Host",
                "createdAt": 1
            }))
            .unwrap(),
        )
        .unwrap();
        let device = Arc::new(
            DeviceRegistry::open(device_path, "LAN Listener Test Host").unwrap(),
        );
        let plugin_directory = directory.path().join("providers/fake");
        std::fs::create_dir_all(&plugin_directory).unwrap();
        let descriptor = fake_plugin();
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
        let catalog = PluginCatalog::discover(
            PluginCatalogConfig::default()
                .with_directory(directory.path().join("providers")),
        );
        let instances = ProviderInstanceRegistry::open(
            directory.path().join("instances.json"),
            device.identity().device_id.clone(),
        )
        .unwrap();
        let manager = Arc::new(
            PluginManager::with_device_registry(
                device.clone(),
                catalog,
                instances,
                PluginManagerConfig {
                    process: PluginProcessOptions {
                        request_timeout: Duration::from_secs(2),
                        shutdown_timeout: Duration::from_secs(2),
                        ..PluginProcessOptions::default()
                    },
                    ..PluginManagerConfig::default()
                },
            )
            .unwrap(),
        );
        let remote_directory = directory.path().join("remote");
        std::fs::create_dir_all(&remote_directory).unwrap();
        let remote_access = Arc::new(
            RemoteAccessManager::open(
                RemoteAccessConfig::for_data_directory(remote_directory),
                device,
                gateway::DeviceDescriptor {
                    device_name: "LAN Listener Test Host".to_string(),
                    operating_system: "TestOS".to_string(),
                    system_version: "1.0".to_string(),
                },
            )
            .unwrap(),
        );
        let gateway = Arc::new(
            ProviderGatewayService::with_remote_identity(
                manager.clone(),
                codepet_host::RemoteHostIdentity {
                    device_id: remote_access.remote_host_identity().device_id,
                    descriptor: remote_access.remote_host_identity().descriptor,
                },
            )
            .unwrap(),
        );
        assert!(gateway.start_event_forwarding());
        let outcomes = manager.start_enabled().await;
        assert_eq!(outcomes.len(), 1);
        outcomes[0].1.as_ref().unwrap();
        Self {
            _directory: directory,
            manager,
            remote_access,
            gateway,
        }
    }
}

fn fake_plugin() -> PluginDescriptor {
    PluginDescriptor {
        plugin_id: "dev.codepet.lan-listener".to_string(),
        display_name: "LAN Listener Provider".to_string(),
        icon: None,
        executable: env!("CARGO_BIN_EXE_codepet-host-fake-provider").into(),
        args: Vec::new(),
        env: BTreeMap::from([(
            "CODEPET_FAKE_PLUGIN_ID".to_string(),
            "dev.codepet.lan-listener".to_string(),
        )]),
        enabled: true,
        instances: vec![PluginInstanceConfig {
            instance_id: Some("instance-lan-listener".to_string()),
            instance_kind: "fake".to_string(),
            display_name: "LAN Listener Instance".to_string(),
            settings: JsonObject::new(),
            enabled: true,
        }],
    }
}

struct PinnedTlsClient {
    address: std::net::SocketAddr,
    certificate_der: Vec<u8>,
    config: Arc<rustls::ClientConfig>,
}

impl PinnedTlsClient {
    fn new(address: std::net::SocketAddr, certificate_der: Vec<u8>) -> Self {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(certificate_der.clone()))
            .unwrap();
        let mut config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Self {
            address,
            certificate_der,
            config: Arc::new(config),
        }
    }

    async fn connect_tls(&self) -> TlsStream<TcpStream> {
        let tcp = TcpStream::connect(self.address).await.unwrap();
        let server_name = ServerName::try_from("localhost").unwrap();
        let tls = TlsConnector::from(self.config.clone())
            .connect(server_name, tcp)
            .await
            .unwrap();
        let peer = tls
            .get_ref()
            .1
            .peer_certificates()
            .unwrap()
            .first()
            .unwrap();
        assert_eq!(peer.as_ref(), self.certificate_der.as_slice());
        tls
    }

    async fn json_request(
        &self,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        body: Option<&str>,
    ) -> (u16, serde_json::Value) {
        self.json_request_with_host(
            method,
            path,
            bearer,
            body,
            &format!("localhost:{}", self.address.port()),
        )
        .await
    }

    async fn json_request_with_host(
        &self,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        body: Option<&str>,
        host: &str,
    ) -> (u16, serde_json::Value) {
        let body = body.unwrap_or("");
        let authorization = bearer
            .map(|bearer| format!("Authorization: Bearer {bearer}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{authorization}Connection: close\r\n\r\n{body}",
            body.len(),
        );
        let mut tls = self.connect_tls().await;
        tls.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tls.read_to_end(&mut response).await.unwrap();
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&response[..header_end]).unwrap();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap();
        let body = serde_json::from_slice(&response[header_end + 4..]).unwrap();
        (status, body)
    }

    async fn connect_websocket(
        &self,
        bearer: &str,
    ) -> Result<TestWebSocket, WebSocketError> {
        let tls = self.connect_tls().await;
        let mut request = format!(
            "wss://localhost:{}/remote/v1/gateway",
            self.address.port()
        )
        .into_client_request()
        .unwrap();
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap(),
        );
        client_async(request, tls).await.map(|(socket, _)| socket)
    }

    async fn websocket_upgrade_headers(
        &self,
        bearer: &str,
        extensions: &str,
    ) -> String {
        let request = format!(
            "GET /remote/v1/gateway HTTP/1.1\r\nHost: localhost:{}\r\nAuthorization: Bearer {bearer}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Extensions: {extensions}\r\n\r\n",
            self.address.port(),
        );
        let mut tls = self.connect_tls().await;
        tls.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        let header_end = timeout(Duration::from_secs(2), async {
            loop {
                if let Some(position) = response
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                {
                    return position;
                }
                let mut buffer = [0_u8; 1024];
                let read = tls.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0, "WebSocket upgrade ended before headers");
                response.extend_from_slice(&buffer[..read]);
            }
        })
        .await
        .unwrap();
        std::str::from_utf8(&response[..header_end])
            .unwrap()
            .to_ascii_lowercase()
    }
}

async fn pair_client(
    remote_access: &RemoteAccessManager,
    client: &PinnedTlsClient,
    client_id: &str,
) -> lan::PairingExchangeResponse {
    let pairing = remote_access.begin_pairing().unwrap();
    let pairing_id = pairing.pairing_id.clone();
    let pairing_watch = remote_access.subscribe_pairing_state();
    assert!(pairing_watch.borrow().pairing_available);
    let request = lan::PairingExchangeRequest {
        pairing_secret: pairing.pairing_secret,
        client_id: client_id.to_string(),
        device: client_descriptor(client_id, "1.0"),
    };
    let body = serde_json::to_string(&request).unwrap();
    let path = format!("/remote/v1/pairings/{}/exchange", pairing.pairing_id);
    let (status, body) = client
        .json_request("POST", &path, None, Some(&body))
        .await;
    assert_eq!(status, 200);
    assert!(!pairing_watch.borrow().pairing_available);
    assert_eq!(
        remote_access.pairing_status(&pairing_id).unwrap().state,
        codepet_host::PairingStatusKind::Succeeded
    );
    serde_json::from_value(body).unwrap()
}

fn handshake_request(id: &str, client_id: &str) -> gateway::ProtocolRequest {
    handshake_request_with_descriptor(id, client_id, client_descriptor(client_id, "1.0"))
}

fn handshake_request_with_descriptor(
    id: &str,
    client_id: &str,
    device: gateway::DeviceDescriptor,
) -> gateway::ProtocolRequest {
    gateway::ProtocolRequest::ProtocolHandshake {
        jsonrpc: "2.0".to_string(),
        id: id.to_string(),
        params: gateway::HandshakeRequest {
            client_id: client_id.to_string(),
            device,
            client_version: "1.0.0".to_string(),
            supported_versions: gateway::VersionRange {
                min_version: gateway::PROTOCOL_VERSION,
                max_version: gateway::PROTOCOL_VERSION,
            },
            last_event_cursor: None,
        },
    }
}

fn client_descriptor(client_id: &str, system_version: &str) -> gateway::DeviceDescriptor {
    gateway::DeviceDescriptor {
        device_name: format!("Client {client_id}"),
        operating_system: "TestOS".to_string(),
        system_version: system_version.to_string(),
    }
}

fn conversation_resource(native_id: &str) -> gateway::RoutedResourceId {
    gateway::RoutedResourceId {
        provider_id: "instance-lan-listener".to_string(),
        native_resource_id: native_id.to_string(),
    }
}

async fn send_request(socket: &mut TestWebSocket, request: gateway::ProtocolRequest) {
    socket
        .send(Message::Text(serde_json::to_string(&request).unwrap()))
        .await
        .unwrap();
}

async fn next_value(socket: &mut TestWebSocket) -> serde_json::Value {
    let message = timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = message else {
        panic!("expected a text Gateway frame");
    };
    serde_json::from_str(&text).unwrap()
}

async fn next_response(
    socket: &mut TestWebSocket,
    expected_id: &str,
) -> gateway::JsonRpcResponse {
    let value = next_value(socket).await;
    assert_eq!(value.get("id").and_then(|id| id.as_str()), Some(expected_id));
    serde_json::from_value(value).unwrap()
}

async fn next_response_after_events(
    socket: &mut TestWebSocket,
    expected_id: &str,
) -> gateway::JsonRpcResponse {
    loop {
        let value = next_value(socket).await;
        if value.get("id").and_then(|id| id.as_str()) == Some(expected_id) {
            return serde_json::from_value(value).unwrap();
        }
        let _: gateway::ProtocolEvent = serde_json::from_value(value).unwrap();
    }
}

fn response_result<T: serde::de::DeserializeOwned>(response: gateway::JsonRpcResponse) -> T {
    let gateway::JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected successful JSON-RPC response");
    };
    serde_json::from_value(result).unwrap()
}

fn response_error_code(response: gateway::JsonRpcResponse) -> String {
    let gateway::JsonRpcResponsePayload::Error { error } = response.response else {
        panic!("expected JSON-RPC error response");
    };
    error
        .data
        .and_then(|data| data.get("code").and_then(serde_json::Value::as_str).map(str::to_string))
        .unwrap_or_else(|| error.code.to_string())
}

fn event_cursor(event: &gateway::ProtocolEvent) -> &str {
    event.event_cursor()
}

fn cursor_sequence(cursor: &str) -> u64 {
    cursor.strip_prefix("event-").unwrap().parse().unwrap()
}

fn fixture_event_batch_complete(events: &[gateway::ProtocolEvent]) -> bool {
    events.windows(2).any(|pair| matches!(pair,
        [gateway::ProtocolEvent::TurnOutputDelta { .. }, gateway::ProtocolEvent::ConversationActivityChanged { .. }]))
}

async fn collect_response_and_events(
    socket: &mut TestWebSocket,
    response_id: &str,
) -> (gateway::JsonRpcResponse, Vec<gateway::ProtocolEvent>) {
    let mut response = None;
    let mut events = Vec::new();
    // Mux response/event streams can complete in either order. Drain the fixture's
    // final delta and the activity event that Gateway publishes immediately after it.
    while response.is_none() || !fixture_event_batch_complete(&events) {
        let value = next_value(socket).await;
        if value.get("id").is_some() {
            assert_eq!(value.get("id").and_then(|id| id.as_str()), Some(response_id));
            response = Some(serde_json::from_value(value).unwrap());
        } else {
            events.push(serde_json::from_value(value).unwrap());
        }
    }
    (response.unwrap(), events)
}

async fn assert_close_reason(socket: &mut TestWebSocket, expected: &str) {
    let message = timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Close(Some(frame)) = message else {
        panic!("expected a WebSocket close frame ({expected}), got {message:?}");
    };
    assert_eq!(frame.reason, expected);
}

async fn assert_socket_ends_safely(socket: &mut TestWebSocket) {
    timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(message)) => panic!("unexpected frame before socket close: {message:?}"),
            }
        }
    })
    .await
    .unwrap();
}

async fn wait_for_active_sessions(server: &codepet_host::RemoteLanServerHandle, expected: usize) {
    timeout(Duration::from_secs(5), async {
        while server.active_session_count() != expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loopback_tls_wss_listener_enforces_identity_subscription_isolation_and_shutdown() {
    assert_eq!(
        RemoteLanServerConfig::default().bind_addr,
        "0.0.0.0:0".parse().unwrap()
    );
    assert!(RemoteLanServerConfig::default().advertised_host.is_none());
    let host = TestHost::start().await;
    let wildcard_error = RemoteLanServer::start(
        RemoteLanServerConfig::default(),
        host.remote_access.clone(),
        host.gateway.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        wildcard_error.code,
        "remote_lan_advertised_host_required"
    );
    let wildcard_advertised_error = RemoteLanServer::start(
        RemoteLanServerConfig::default().with_advertised_host("0.0.0.0"),
        host.remote_access.clone(),
        host.gateway.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        wildcard_advertised_error.code,
        "invalid_remote_lan_advertised_host"
    );
    let wildcard_server = RemoteLanServer::start(
        RemoteLanServerConfig::default().with_advertised_host("listener.local"),
        host.remote_access.clone(),
        host.gateway.clone(),
    )
    .await
    .unwrap();
    assert!(wildcard_server.local_addr().ip().is_unspecified());
    assert_eq!(
        wildcard_server.advertised_host().as_deref(),
        Some("listener.local")
    );
    assert_eq!(
        wildcard_server.https_base_url(),
        Some(format!("https://listener.local:{}", wildcard_server.port()))
    );
    assert_eq!(
        wildcard_server.gateway_url(),
        Some(format!(
            "wss://listener.local:{}/remote/v1/gateway",
            wildcard_server.port()
        ))
    );
    wildcard_server.shutdown().await.unwrap();
    let server = RemoteLanServer::start(
        RemoteLanServerConfig::loopback().with_advertised_host("127.0.0.1"),
        host.remote_access.clone(),
        host.gateway.clone(),
    )
    .await
    .unwrap();
    assert_ne!(server.port(), 0);
    assert_eq!(server.advertised_host().as_deref(), Some("127.0.0.1"));
    assert_eq!(
        server.remote_host_identity(),
        &host.remote_access.remote_host_identity()
    );
    assert_eq!(
        server.https_base_url(),
        Some(format!("https://127.0.0.1:{}", server.port()))
    );
    assert_eq!(
        server.gateway_url(),
        Some(format!("wss://127.0.0.1:{}/remote/v1/gateway", server.port()))
    );
    let address = server.local_addr();
    let gateway_url = server.gateway_url().unwrap();
    let certificate_der = host.remote_access.tls_identity().certificate_der().to_vec();
    let client = PinnedTlsClient::new(address, certificate_der.clone());

    let (discovery_status, discovery_body) = client
        .json_request("GET", "/remote/v1/discovery", None, None)
        .await;
    assert_eq!(discovery_status, 200);
    assert_eq!(discovery_body["id"], "device-lan-listener");
    assert_eq!(discovery_body["name"], "LAN Listener Test Host");
    assert_eq!(
        discovery_body["fp"],
        host.remote_access
            .tls_identity()
            .certificate_fingerprint()
    );
    assert_eq!(discovery_body["vmin"], lan::CHANNEL_LAN_SCHEMA_VERSION);
    assert_eq!(discovery_body["vmax"], lan::CHANNEL_LAN_SCHEMA_VERSION);
    assert_eq!(discovery_body["pair"], 0);

    let discovery_pairing = host.remote_access.begin_pairing().unwrap();
    let (_, pairing_discovery_body) = client
        .json_request("GET", "/remote/v1/discovery", None, None)
        .await;
    assert_eq!(pairing_discovery_body["pair"], 1);
    host.remote_access
        .cancel_pairing(&discovery_pairing.pairing_id)
        .unwrap();

    let staged = server.stage_advertised_host("127.0.0.2").unwrap();
    assert_eq!(server.advertised_host().as_deref(), Some("127.0.0.1"));
    let staged_pairing = host.remote_access.begin_pairing().unwrap();
    let staged_request = lan::PairingExchangeRequest {
        pairing_secret: staged_pairing.pairing_secret,
        client_id: "client-generation-test".to_string(),
        device: client_descriptor("client-generation-test", "1.0"),
    };
    let staged_path = format!(
        "/remote/v1/pairings/{}/exchange",
        staged_pairing.pairing_id
    );
    let (staged_status, staged_response) = client
        .json_request_with_host(
            "POST",
            &staged_path,
            None,
            Some(&serde_json::to_string(&staged_request).unwrap()),
            &format!("127.0.0.2:{}", server.port()),
        )
        .await;
    assert_eq!(staged_status, 200);
    let staged_response: lan::PairingExchangeResponse =
        serde_json::from_value(staged_response).unwrap();
    assert_eq!(
        staged_response.gateway_url,
        format!("wss://127.0.0.2:{}/remote/v1/gateway", server.port())
    );
    assert_eq!(server.advertised_host().as_deref(), Some("127.0.0.1"));
    staged.commit().unwrap();
    assert_eq!(server.advertised_host().as_deref(), Some("127.0.0.2"));
    server
        .stage_advertised_host("127.0.0.1")
        .unwrap()
        .commit()
        .unwrap();

    let failed = server.stage_advertised_host("127.0.0.2").unwrap();
    failed.fail_closed();
    assert_eq!(server.advertised_endpoint(), None);
    let unavailable_pairing = host.remote_access.begin_pairing().unwrap();
    let unavailable_path = format!(
        "/remote/v1/pairings/{}/exchange",
        unavailable_pairing.pairing_id
    );
    let unavailable_request = lan::PairingExchangeRequest {
        pairing_secret: unavailable_pairing.pairing_secret,
        client_id: "client-unavailable".to_string(),
        device: client_descriptor("client-unavailable", "1.0"),
    };
    let (unavailable_status, unavailable_response) = client
        .json_request(
            "POST",
            &unavailable_path,
            None,
            Some(&serde_json::to_string(&unavailable_request).unwrap()),
        )
        .await;
    assert_eq!(unavailable_status, 503);
    assert_eq!(
        unavailable_response["code"],
        "remote_lan_discovery_unavailable"
    );
    assert!(host
        .remote_access
        .subscribe_pairing_state()
        .borrow()
        .pairing_available);
    host.remote_access
        .cancel_pairing(&unavailable_pairing.pairing_id)
        .unwrap();
    server
        .stage_advertised_host("127.0.0.1")
        .unwrap()
        .commit()
        .unwrap();

    let invalid_pairing = host.remote_access.begin_pairing().unwrap();
    let invalid_path = format!(
        "/remote/v1/pairings/{}/exchange",
        invalid_pairing.pairing_id
    );
    let (invalid_status, invalid_body) = client
        .json_request("POST", &invalid_path, None, Some("{"))
        .await;
    assert_eq!(invalid_status, 400);
    let invalid_error: gateway::ProtocolError =
        serde_json::from_value(invalid_body).unwrap();
    assert_eq!(invalid_error.code, "invalid_remote_request");
    let oversized_body = "x".repeat(65 * 1024);
    let (oversized_status, oversized_response) = client
        .json_request("POST", &invalid_path, None, Some(&oversized_body))
        .await;
    assert_eq!(oversized_status, 413);
    let oversized_error: gateway::ProtocolError =
        serde_json::from_value(oversized_response).unwrap();
    assert_eq!(oversized_error.code, "remote_request_too_large");
    host.remote_access
        .cancel_pairing(&invalid_pairing.pairing_id)
        .unwrap();

    let pairing_request = lan::PairingRequestCreateRequest {
        host_device_id: host.remote_access.remote_host_identity().device_id,
        client_id: "client-confirmed".to_string(),
        device: client_descriptor("client-confirmed", "1.0"),
        client_nonce: "ab".repeat(32),
    };
    let (request_status, request_body) = client
        .json_request(
            "POST",
            "/remote/v1/pairing-requests",
            None,
            Some(&serde_json::to_string(&pairing_request).unwrap()),
        )
        .await;
    assert_eq!(request_status, 200);
    let pending: lan::PairingRequestStatusResponse =
        serde_json::from_value(request_body).unwrap();
    assert_eq!(pending.state, lan::PairingRequestState::Pending);
    assert!(pending.credential.is_none());
    assert!(pending.gateway_url.is_none());
    assert_eq!(host.remote_access.pending_pairing_requests().unwrap().len(), 1);
    host.remote_access
        .resolve_pairing_request(&pending.request_id, true)
        .unwrap();
    let (accepted_status, accepted_body) = client
        .json_request(
            "GET",
            &format!("/remote/v1/pairing-requests/{}", pending.request_id),
            None,
            None,
        )
        .await;
    assert_eq!(accepted_status, 200);
    let accepted: lan::PairingRequestStatusResponse =
        serde_json::from_value(accepted_body).unwrap();
    assert_eq!(accepted.state, lan::PairingRequestState::Accepted);
    assert!(accepted.gateway_url.as_deref().is_some_and(|url| {
        url == server.gateway_url().unwrap()
    }));
    assert!(host
        .remote_access
        .validate_bearer(accepted.credential.as_deref().unwrap())
        .is_ok());

    let pairing_a = pair_client(host.remote_access.as_ref(), &client, "client-a").await;
    assert_eq!(pairing_a.device, host.remote_access.remote_host_identity());
    assert_eq!(pairing_a.gateway_url, gateway_url);
    let compressed_upgrade = client
        .websocket_upgrade_headers(
            &pairing_a.credential,
            "permessage-deflate; client_no_context_takeover; server_no_context_takeover",
        )
        .await;
    assert!(compressed_upgrade.starts_with("http/1.1 101 "));
    assert!(compressed_upgrade.contains("sec-websocket-extensions: permessage-deflate"));
    assert!(compressed_upgrade.contains("client_no_context_takeover"));
    assert!(compressed_upgrade.contains("server_no_context_takeover"));
    let mut socket_a = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    socket_a
        .send(Message::Ping(vec![1_u8, 2_u8, 3_u8]))
        .await
        .unwrap();
    let pong = timeout(Duration::from_secs(1), socket_a.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(pong, Message::Pong(vec![1_u8, 2_u8, 3_u8]));
    let refreshed_client_a = client_descriptor("client-a", "2.0");
    send_request(
        &mut socket_a,
        handshake_request_with_descriptor(
            "handshake-a",
            "client-a",
            refreshed_client_a.clone(),
        ),
    )
    .await;
    let handshake: gateway::HandshakeResponse =
        response_result(next_response(&mut socket_a, "handshake-a").await);
    let certificate_fingerprint = hex_sha256(&certificate_der);
    assert_eq!(pairing_a.device.identity_fingerprint, certificate_fingerprint);
    assert_eq!(handshake.device.name, pairing_a.device.descriptor.device_name);
    assert_eq!(handshake.device.operating_system, pairing_a.device.descriptor.operating_system);
    assert_eq!(handshake.device.system_version, pairing_a.device.descriptor.system_version);
    assert_eq!(
        handshake.device,
        gateway::GatewayDevice {
            name: "LAN Listener Test Host".to_string(),
            operating_system: "TestOS".to_string(),
            system_version: "1.0".to_string(),
        }
    );
    assert_eq!(
        host.remote_access
            .list_credentials()
            .unwrap()
            .into_iter()
            .find(|credential| credential.client_id == "client-a")
            .unwrap()
            .descriptor,
        refreshed_client_a
    );
    let initial_cursor = handshake.event_cursor.clone();

    let mut heartbeat_while_loading = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    send_request(
        &mut heartbeat_while_loading,
        handshake_request("handshake-heartbeat", "client-a"),
    )
    .await;
    next_response(&mut heartbeat_while_loading, "handshake-heartbeat").await;
    send_request(
        &mut heartbeat_while_loading,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "get-that-does-not-complete".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("timeout"),
                cursor: None,
                limit: None,
            },
        },
    )
    .await;
    heartbeat_while_loading
        .send(Message::Ping(vec![4_u8, 5_u8, 6_u8]))
        .await
        .unwrap();
    let pong = timeout(Duration::from_secs(1), heartbeat_while_loading.next())
        .await
        .expect("conversation.get must not block WebSocket control frames")
        .unwrap()
        .unwrap();
    assert_eq!(pong, Message::Pong(vec![4_u8, 5_u8, 6_u8]));
    send_request(&mut heartbeat_while_loading, gateway::ProtocolRequest::ProtocolPing {
        jsonrpc: "2.0".into(), id: "app-heartbeat".into(),
        params: gateway::PingRequest { sequence: 1 },
    }).await;
    let pong = timeout(Duration::from_secs(1), next_response(&mut heartbeat_while_loading, "app-heartbeat"))
        .await.expect("pending get must not block application heartbeat");
    let pong: gateway::PingResponse = response_result(pong);
    assert_eq!(pong.sequence, 1);
    assert!(!pong.providers.is_empty());
    assert_eq!(host.gateway.remote_connections().snapshot().connections.len(), 2);
    drop(heartbeat_while_loading);
    wait_for_active_sessions(&server, 1).await;

    let mut malformed = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    malformed
        .send(Message::Text("{".to_string()))
        .await
        .unwrap();
    let malformed_response: gateway::JsonRpcResponse =
        serde_json::from_value(next_value(&mut malformed).await).unwrap();
    let gateway::JsonRpcResponsePayload::Error { error } = malformed_response.response else {
        panic!("expected JSON-RPC parse error");
    };
    assert_eq!(error.code, gateway::JSON_RPC_PARSE_ERROR);
    assert_close_reason(&mut malformed, "protocol_handshake_required").await;

    let mut missing_handshake = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    send_request(
        &mut missing_handshake,
        gateway::ProtocolRequest::ProviderList {
            jsonrpc: "2.0".to_string(),
            id: "before-handshake".to_string(),
            params: gateway::ProviderListRequest {},
        },
    )
    .await;
    assert_close_reason(&mut missing_handshake, "protocol_handshake_required").await;

    wait_for_active_sessions(&server, 1).await;
    let mut binary = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    binary
        .send(Message::Binary(vec![0_u8, 1_u8, 2_u8]))
        .await
        .unwrap();
    assert_close_reason(&mut binary, "text_frames_required").await;

    wait_for_active_sessions(&server, 1).await;
    let mut oversized = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    // The server can reject the advertised frame size before the client has
    // flushed its payload. Windows may report that rejection from send itself.
    match timeout(
        Duration::from_secs(2),
        oversized.send(Message::Text("x".repeat(300 * 1024))),
    )
    .await
    .expect("oversized frame send must finish or be rejected")
    {
        Ok(()) | Err(WebSocketError::ConnectionClosed) => {}
        Err(WebSocketError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            ) => {}
        Err(error) => panic!("unexpected oversized frame send failure: {error:?}"),
    }
    assert_socket_ends_safely(&mut oversized).await;

    wait_for_active_sessions(&server, 1).await;
    let mut stalled = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    send_request(
        &mut stalled,
        handshake_request("handshake-stalled", "client-a"),
    )
    .await;
    let stalled_handshake = next_response(&mut stalled, "handshake-stalled").await;
    let _: gateway::HandshakeResponse = response_result(stalled_handshake);
    wait_for_active_sessions(&server, 2).await;
    let large_id_tail = "x".repeat(248 * 1024);
    let mut backpressure_requests_sent = 0;
    for sequence in 0..128 {
        let request = gateway::ProtocolRequest::ProviderList {
            jsonrpc: "2.0".to_string(),
            id: format!("backpressure-{sequence}-{large_id_tail}"),
            params: gateway::ProviderListRequest {},
        };
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.len() < 256 * 1024);
        match timeout(Duration::from_secs(1), stalled.send(Message::Text(text))).await {
            Ok(Ok(())) => backpressure_requests_sent += 1,
            Ok(Err(_)) | Err(_) => break,
        }
    }
    assert!(backpressure_requests_sent > 0);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ProviderList {
            jsonrpc: "2.0".to_string(),
            id: "healthy-during-backpressure".to_string(),
            params: gateway::ProviderListRequest {},
        },
    )
    .await;
    let healthy_during_backpressure =
        next_response(&mut socket_a, "healthy-during-backpressure").await;
    let _: gateway::ProviderListResponse = response_result(healthy_during_backpressure);
    wait_for_active_sessions(&server, 1).await;
    drop(stalled);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationList {
            jsonrpc: "2.0".to_string(),
            id: "list-a".to_string(),
            params: gateway::ConversationListRequest {
                provider_id: "instance-lan-listener".to_string(),
                cursor: None,
                limit: Some(10),
                project_filter: gateway::ConversationProjectFilter::ConversationProjectFilterAll(
                    gateway::ConversationProjectFilterAll {
                        kind: gateway::ConversationProjectFilterAllKind::All,
                    },
                ),
            },
        },
    )
    .await;
    let list: gateway::ConversationListResponse =
        response_result(next_response(&mut socket_a, "list-a").await);
    assert_eq!(list.conversations.len(), 1);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "get-a".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("ordinary"),
                cursor: None,
                limit: None,
            },
        },
    )
    .await;
    let get: gateway::ConversationGetResponse =
        response_result(next_response(&mut socket_a, "get-a").await);
    assert_eq!(get.conversation.resource.native_resource_id, "ordinary");

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "pre-subscribe-event".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("event-first"),
                cursor: None,
                limit: None,
            },
        },
    )
    .await;
    next_response(&mut socket_a, "pre-subscribe-event").await;
    assert!(timeout(Duration::from_millis(150), socket_a.next())
        .await
        .is_err());

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::EventSubscribe {
            jsonrpc: "2.0".to_string(),
            id: "subscribe-a".to_string(),
            params: gateway::EventSubscribeRequest {
                after_cursor: initial_cursor,
            },
        },
    )
    .await;
    let subscribed = next_response(&mut socket_a, "subscribe-a").await;
    let _: gateway::EventSubscribeResponse = response_result(subscribed);
    let mut replay_events = Vec::new();
    while !fixture_event_batch_complete(&replay_events) {
        replay_events.push(serde_json::from_value(next_value(&mut socket_a).await).unwrap());
    }
    let replay_sequences = replay_events.iter().map(|event| cursor_sequence(event_cursor(event))).collect::<Vec<_>>();
    assert!(replay_sequences.len() >= 3);
    assert!(replay_sequences.windows(2).all(|pair| pair[0] < pair[1]));

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "live-event-a".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("event-first"),
                cursor: None,
                limit: None,
            },
        },
    )
    .await;
    let (_, live_events) = collect_response_and_events(&mut socket_a, "live-event-a").await;
    let live_sequences = live_events
        .iter()
        .map(|event| cursor_sequence(event_cursor(event)))
        .collect::<Vec<_>>();
    assert!(*replay_sequences.last().unwrap() < live_sequences[0]);
    assert!(live_sequences.windows(2).all(|pair| pair[0] < pair[1]));

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::EventSubscribe {
            jsonrpc: "2.0".to_string(),
            id: "subscribe-a-again".to_string(),
            params: gateway::EventSubscribeRequest {
                after_cursor: event_cursor(live_events.last().unwrap()).to_string(),
            },
        },
    )
    .await;
    let duplicate = next_response_after_events(&mut socket_a, "subscribe-a-again").await;
    assert_eq!(response_error_code(duplicate), "gateway_event_already_subscribed");

    let descriptor_before_mismatch = host.remote_access
        .list_credentials()
        .unwrap()
        .into_iter()
        .find(|credential| credential.client_id == "client-a")
        .unwrap()
        .descriptor;
    let mut mismatch = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    send_request(
        &mut mismatch,
        handshake_request("handshake-mismatch", "different-client"),
    )
    .await;
    let mismatch_response = next_response(&mut mismatch, "handshake-mismatch").await;
    assert_eq!(response_error_code(mismatch_response), "gateway_client_identity_mismatch");
    assert_close_reason(&mut mismatch, "gateway_client_identity_mismatch").await;
    assert_eq!(
        host.remote_access
            .list_credentials()
            .unwrap()
            .into_iter()
            .find(|credential| credential.client_id == "client-a")
            .unwrap()
            .descriptor,
        descriptor_before_mismatch
    );

    let pairing_b = pair_client(host.remote_access.as_ref(), &client, "client-b").await;
    let mut socket_b = client
        .connect_websocket(&pairing_b.credential)
        .await
        .unwrap();
    send_request(&mut socket_b, handshake_request("handshake-b", "client-b")).await;
    let handshake_b = next_response(&mut socket_b, "handshake-b").await;
    let _: gateway::HandshakeResponse = response_result(handshake_b);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "client-a-only".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("event-first"),
                cursor: None,
                limit: None,
            },
        },
    )
    .await;
    let (_, client_a_events) =
        collect_response_and_events(&mut socket_a, "client-a-only").await;
    assert!(timeout(Duration::from_millis(150), socket_b.next())
        .await
        .is_err());
    let latest_cursor = event_cursor(client_a_events.last().unwrap()).to_string();

    send_request(
        &mut socket_b,
        gateway::ProtocolRequest::EventSubscribe {
            jsonrpc: "2.0".to_string(),
            id: "subscribe-b".to_string(),
            params: gateway::EventSubscribeRequest {
                after_cursor: latest_cursor,
            },
        },
    )
    .await;
    let subscribed_b = next_response(&mut socket_b, "subscribe-b").await;
    let _: gateway::EventSubscribeResponse = response_result(subscribed_b);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "shared-event-source-a".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("event-first"),
                cursor: None,
                limit: None,
            },
        },
    )
    .await;
    let (_, events_a) =
        collect_response_and_events(&mut socket_a, "shared-event-source-a").await;
    let mut events_b = Vec::new();
    for _ in 0..events_a.len() {
        events_b.push(serde_json::from_value::<gateway::ProtocolEvent>(next_value(&mut socket_b).await).unwrap());
    }
    assert!(events_a
        .iter()
        .zip(events_b.iter())
        .all(|(event_a, event_b)| event_cursor(event_a) == event_cursor(event_b)));

    let mut socket_a_second = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
    send_request(
        &mut socket_a_second,
        handshake_request("handshake-a-second", "client-a"),
    )
    .await;
    let handshake_a_second = next_response(&mut socket_a_second, "handshake-a-second").await;
    let _: gateway::HandshakeResponse = response_result(handshake_a_second);
    wait_for_active_sessions(&server, 3).await;
    let credential_a = host
        .remote_access
        .validate_bearer(&pairing_a.credential)
        .unwrap();
    let credential_b = host
        .remote_access
        .validate_bearer(&pairing_b.credential)
        .unwrap();
    assert_eq!(
        server.active_session_count_for_credential(&credential_a.credential_id),
        2
    );
    assert_eq!(
        server.active_session_count_for_credential(&credential_b.credential_id),
        1
    );
    let (delete_status, delete_body) = client
        .json_request(
            "DELETE",
            "/remote/v1/credentials/current",
            Some(&pairing_a.credential),
            None,
        )
        .await;
    assert_eq!(delete_status, 200);
    let deleted: lan::CurrentCredentialDeleteResponse =
        serde_json::from_value(delete_body).unwrap();
    assert!(deleted.revoked);
    assert_close_reason(&mut socket_a, "credential_revoked").await;
    assert_close_reason(&mut socket_a_second, "credential_revoked").await;
    wait_for_active_sessions(&server, 1).await;
    assert_eq!(
        server.active_session_count_for_credential(&credential_a.credential_id),
        0
    );
    assert_eq!(
        server.active_session_count_for_credential(&credential_b.credential_id),
        1
    );

    let reconnect = client.connect_websocket(&pairing_a.credential).await;
    let Err(WebSocketError::Http(response)) = reconnect else {
        panic!("revoked bearer must not reconnect");
    };
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    send_request(
        &mut socket_b,
        gateway::ProtocolRequest::ProviderList {
            jsonrpc: "2.0".to_string(),
            id: "client-b-still-active".to_string(),
            params: gateway::ProviderListRequest {},
        },
    )
    .await;
    let client_b_active = next_response_after_events(&mut socket_b, "client-b-still-active").await;
    let _: gateway::ProviderListResponse = response_result(client_b_active);

    let pairing_c = pair_client(host.remote_access.as_ref(), &client, "client-c").await;
    let mut socket_c_first = client
        .connect_websocket(&pairing_c.credential)
        .await
        .unwrap();
    let mut socket_c_second = client
        .connect_websocket(&pairing_c.credential)
        .await
        .unwrap();
    send_request(
        &mut socket_c_first,
        handshake_request("handshake-c-first", "client-c"),
    )
    .await;
    next_response(&mut socket_c_first, "handshake-c-first").await;
    send_request(
        &mut socket_c_second,
        handshake_request("handshake-c-second", "client-c"),
    )
    .await;
    next_response(&mut socket_c_second, "handshake-c-second").await;
    wait_for_active_sessions(&server, 3).await;
    let credential_c = host
        .remote_access
        .validate_bearer(&pairing_c.credential)
        .unwrap();
    host.remote_access
        .revoke_credential(&credential_c.credential_id)
        .unwrap();
    assert_eq!(
        server
            .disconnect_credential(&credential_c.credential_id)
            .await
            .unwrap(),
        2
    );
    assert_close_reason(&mut socket_c_first, "credential_revoked").await;
    assert_close_reason(&mut socket_c_second, "credential_revoked").await;
    wait_for_active_sessions(&server, 1).await;
    assert_eq!(
        server.active_session_count_for_credential(&credential_b.credential_id),
        1
    );

    let mut stalled_tls_handshake = TcpStream::connect(address).await.unwrap();
    timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(1), socket_b.next()).await,
        Ok(Some(Ok(Message::Close(_)))) | Ok(None)
    ));
    let mut closed = [0_u8; 1];
    assert!(matches!(
        timeout(Duration::from_secs(1), stalled_tls_handshake.read(&mut closed)).await,
        Ok(Ok(0)) | Ok(Err(_))
    ));
    assert!(TcpStream::connect(address).await.is_err());
    host.manager.shutdown().await;
}


struct RtcClient {
    peer: Arc<webrtc::peer_connection::RTCPeerConnection>,
    channel: Arc<webrtc::data_channel::RTCDataChannel>,
    incoming: tokio::sync::mpsc::Receiver<bytes::Bytes>,
}

impl Drop for RtcClient {
    fn drop(&mut self) {
        let peer = self.peer.clone();
        tokio::spawn(async move { let _ = peer.close().await; });
    }
}

impl RtcClient {
    async fn connect(client: &PinnedTlsClient, credential: &str) -> Self {
        use webrtc::api::APIBuilder;
        use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
        use webrtc::peer_connection::configuration::RTCConfiguration;
        let peer = Arc::new(APIBuilder::new().build().new_peer_connection(RTCConfiguration::default()).await.unwrap());
        let channel = peer.create_data_channel("codepet.gateway.v1", Some(RTCDataChannelInit {
            ordered: Some(true), negotiated: Some(0),
            protocol: Some("codepet.gateway.cpg1".into()), ..Default::default()
        })).await.unwrap();
        let (tx, incoming) = tokio::sync::mpsc::channel(128);
        channel.on_message(Box::new(move |message| {
            let tx = tx.clone();
            Box::pin(async move {
                assert!(!message.is_string);
                tx.send(message.data).await.unwrap();
            })
        }));
        let offer = peer.create_offer(None).await.unwrap();
        let mut gathered = peer.gathering_complete_promise().await;
        peer.set_local_description(offer).await.unwrap();
        timeout(Duration::from_secs(8), gathered.recv()).await.unwrap();
        let offer = peer.local_description().await.unwrap();
        let (status, answer) = client.json_request(
            "POST", "/remote/v1/webrtc/offer", Some(credential),
            Some(&serde_json::to_string(&offer).unwrap()),
        ).await;
        assert_eq!(status, 200, "{answer}");
        peer.set_remote_description(serde_json::from_value(answer).unwrap()).await.unwrap();
        timeout(Duration::from_secs(15), async {
            while channel.ready_state() != webrtc::data_channel::data_channel_state::RTCDataChannelState::Open {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        Self { peer, channel, incoming }
    }

    async fn request(&self, request: gateway::ProtocolRequest) {
        let bytes = serde_json::to_vec(&request).unwrap();
        for (index, part) in bytes.chunks(16 * 1024 - 12).enumerate() {
            let mut frame = b"CPG1".to_vec();
            frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            frame.extend_from_slice(&((index * (16 * 1024 - 12)) as u32).to_be_bytes());
            frame.extend_from_slice(part);
            self.channel.send(&bytes::Bytes::from(frame)).await.unwrap();
        }
    }

    async fn next_json(&mut self) -> serde_json::Value {
        self.next_json_timeout(Duration::from_secs(5)).await
    }

    async fn next_json_timeout(&mut self, budget: Duration) -> serde_json::Value {
        timeout(budget, async {
            let mut message = Vec::new();
            let mut total = None;
            loop {
                let frame = self.incoming.recv().await.expect("RTC response");
                assert_eq!(&frame[..4], b"CPG1");
                let length = u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize;
                let offset = u32::from_be_bytes(frame[8..12].try_into().unwrap()) as usize;
                assert!(length > 0, "unexpected close: {frame:?}");
                assert_eq!(*total.get_or_insert(length), length);
                assert_eq!(offset, message.len());
                message.extend_from_slice(&frame[12..]);
                if message.len() == length { return serde_json::from_slice(&message).unwrap(); }
            }
        }).await.unwrap()
    }
}

#[tokio::test]
async fn rtc_and_lan_share_rpc_admission_large_messages_and_revocation() {
    let host = TestHost::start().await;
    let server = RemoteLanServer::start(
        RemoteLanServerConfig::loopback(), host.remote_access.clone(), host.gateway.clone(),
    ).await.unwrap();
    let client = PinnedTlsClient::new(server.local_addr(), host.remote_access.tls_identity().certificate_der().to_vec());
    let (status, _) = client.json_request("POST", "/remote/v1/webrtc/offer", None, Some(r#"{"type":"offer","sdp":""}"#)).await;
    assert_eq!(status, 401);
    assert_eq!(server.active_session_count(), 0);
    let pairing = pair_client(&host.remote_access, &client, "rtc-client").await;
    let (status, _) = client.json_request("POST", "/remote/v1/webrtc/offer", Some(&pairing.credential), Some(r#"{"type":"offer","sdp":"invalid"}"#)).await;
    assert_eq!(status, 400);
    wait_for_active_sessions(&server, 0).await;

    let mut rtc = RtcClient::connect(&client, &pairing.credential).await;
    rtc.request(handshake_request("rtc-handshake", "rtc-client")).await;
    let handshake = rtc.next_json().await;
    assert_eq!(handshake["id"], "rtc-handshake");
    assert!(handshake.get("result").is_some(), "{handshake}");

    // Both request and response cross several native message boundaries.
    let large_id = format!("large-{}", "界".repeat(24000));
    rtc.request(gateway::ProtocolRequest::ProviderList {
        jsonrpc: "2.0".into(), id: large_id.clone(), params: gateway::ProviderListRequest {},
    }).await;
    let response = rtc.next_json().await;
    assert_eq!(response["id"], large_id);
    assert!(response.get("result").is_some(), "{response}");

    let mut lan = client.connect_websocket(&pairing.credential).await.unwrap();
    send_request(&mut lan, handshake_request("lan-handshake", "rtc-client")).await;
    next_response(&mut lan, "lan-handshake").await;
    wait_for_active_sessions(&server, 2).await;
    let credential = host.remote_access.validate_bearer(&pairing.credential).unwrap();
    host.remote_access.revoke_credential(&credential.credential_id).unwrap();
    assert_eq!(server.disconnect_credential(&credential.credential_id).await.unwrap(), 2);
    wait_for_active_sessions(&server, 0).await;
    assert_socket_ends_safely(&mut lan).await;
    timeout(Duration::from_secs(5), async {
        loop {
            match rtc.incoming.recv().await {
                Some(frame) if frame.len() >= 14 && frame[4..8] == [0, 0, 0, 0] => {
                    assert_eq!(u16::from_be_bytes([frame[12], frame[13]]), 1008);
                    break;
                }
                _ if rtc.channel.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Closed => break,
                _ => {}
            }
        }
    }).await.unwrap();
    server.shutdown().await.unwrap();
}


#[tokio::test]
#[ignore = "manual Android native RTC probe; exports an ephemeral credential to the configured file"]
async fn rtc_android_probe_host() {
    let export = std::env::var("CODEPET_RTC_PROBE_EXPORT").expect("set CODEPET_RTC_PROBE_EXPORT");
    let host = TestHost::start().await;
    if let Ok(config) = std::env::var("CODEPET_RTC_CLOUD_PROBE_CONFIG") {
        std::fs::copy(config, host._directory.path().join("remote/rtc-cloud.json")).unwrap();
    }
    let server = RemoteLanServer::start(
        RemoteLanServerConfig {
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            advertised_host: Some("127.0.0.1".into()),
        },
        host.remote_access.clone(), host.gateway.clone(),
    ).await.unwrap();
    let address = std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), server.local_addr().port());
    let client = PinnedTlsClient::new(address, host.remote_access.tls_identity().certificate_der().to_vec());
    let pairing = pair_client(&host.remote_access, &client, "rtc-android-probe").await;
    std::fs::write(&export, serde_json::to_vec(&serde_json::json!({
        "RTC_GATEWAY_URI": format!("wss://127.0.0.1:{}/remote/v1/gateway", address.port()),
        "RTC_PIN": host.remote_access.tls_identity().certificate_fingerprint(),
        "RTC_CREDENTIAL": pairing.credential,
        "RTC_HANDSHAKE": serde_json::to_string(&handshake_request("probe-handshake", "rtc-android-probe")).unwrap(),
        "RTC_EVENT_REQUEST": serde_json::to_string(&gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".into(), id: "probe-events".into(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("event-first"), cursor: None, limit: None,
            },
        }).unwrap(),
    })).unwrap()).unwrap();
    // A stop file allows the runner to close all peers and delete the credential.
    let stop = format!("{export}.stop");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    while !std::path::Path::new(&stop).exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_file(export);
    let _ = std::fs::remove_file(stop);
}

fn hex_sha256(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = ring::digest::digest(&ring::digest::SHA256, value);
    let mut encoded = String::with_capacity(64);
    for byte in digest.as_ref() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[tokio::test]
#[ignore = "authorized live VPS probe; requires private CODEPET_RTC_CLOUD_PROBE_CONFIG"]
async fn rtc_public_relay_probe() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
    use serde_json::{json, Value};
    use webrtc::{api::APIBuilder, data_channel::data_channel_init::RTCDataChannelInit, peer_connection::{configuration::RTCConfiguration, policy::ice_transport_policy::RTCIceTransportPolicy}};
    let config = std::env::var("CODEPET_RTC_CLOUD_PROBE_CONFIG").unwrap();
    let host = TestHost::start().await;
    std::fs::copy(config, host._directory.path().join("remote/rtc-cloud.json")).unwrap();
    let server=RemoteLanServer::start(RemoteLanServerConfig::loopback(),host.remote_access.clone(),host.gateway.clone()).await.unwrap();
    let local=PinnedTlsClient::new(server.local_addr(),host.remote_access.tls_identity().certificate_der().to_vec());
    let pairing=pair_client(&host.remote_access,&local,"cloud-rust-probe").await;
    let key=Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap().as_ref()).unwrap();
    let (status,bootstrap)=local.json_request("POST","/remote/v1/channel-bootstrap",Some(&pairing.credential),Some(&json!({"publicKey":B64.encode(key.public_key().as_ref())}).to_string())).await;
    assert_eq!(status,200,"bootstrap failed");
    let http=reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap();
    let token=bootstrap["token"].as_str().unwrap();
    let base=bootstrap["serviceUrl"].as_str().unwrap();
    let mut ice:Value=http.get(format!("{base}/v1/ice")).bearer_auth(token).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    if let Ok(transport) = std::env::var("CODEPET_PROBE_TURN_TRANSPORT") {
        for server in ice["iceServers"].as_array_mut().unwrap() {
            server["urls"].as_array_mut().unwrap().retain(|url| {
                let url=url.as_str().unwrap();
                match transport.as_str() {
                    "tcp" => url.starts_with("turn:") && url.ends_with("transport=tcp"),
                    "tls" => url.starts_with("turns:"),
                    "udp" => url.starts_with("turn:") && url.ends_with("transport=udp"),
                    _ => panic!("unsupported probe TURN transport"),
                }
            });
        }
        ice["iceServers"].as_array_mut().unwrap().retain(|server| !server["urls"].as_array().unwrap().is_empty());
    }
    let peer=Arc::new(APIBuilder::new().build().new_peer_connection(RTCConfiguration {
        ice_servers:serde_json::from_value(ice["iceServers"].clone()).unwrap(),
        ice_transport_policy:RTCIceTransportPolicy::Relay,..Default::default()
    }).await.unwrap());
    let channel=peer.create_data_channel("codepet.gateway.v1",Some(RTCDataChannelInit{ordered:Some(true),negotiated:Some(0),protocol:Some("codepet.gateway.cpg1".into()),..Default::default()})).await.unwrap();
    let (tx,incoming)=tokio::sync::mpsc::channel(128);
    channel.on_message(Box::new(move |message|{let tx=tx.clone();Box::pin(async move {let _=tx.send(message.data).await;})}));
    let offer=peer.create_offer(None).await.unwrap();
    let mut gathering=peer.gathering_complete_promise().await;
    peer.set_local_description(offer).await.unwrap();
    timeout(Duration::from_secs(30),gathering.recv()).await.unwrap();
    let offer=peer.local_description().await.unwrap();
    assert!(offer.sdp.contains("typ relay"),"TURN did not yield a relay candidate");
    let attempt=uuid::Uuid::new_v4().to_string();
    let expires=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()+60;
    let payload=serde_json::to_vec(&json!({"v":1,"kind":"offer","host":bootstrap["host"],"client":bootstrap["client"],"attempt":attempt,"expires":expires,"description":offer})).unwrap();
    http.post(format!("{base}/v1/offers")).bearer_auth(token).json(&json!({"attempt":attempt,"envelope":{"payload":B64.encode(&payload),"signature":B64.encode(key.sign(&payload).as_ref())}})).send().await.unwrap().error_for_status().unwrap();
    let envelope:Value=timeout(Duration::from_secs(45),async {
        loop {
            let result:Value=http.get(format!("{base}/v1/answer?attempt={attempt}")).bearer_auth(token).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
            if !result["answer"].is_null() {break result["answer"].clone();}
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }).await.unwrap();
    let answer_bytes=B64.decode(envelope["payload"].as_str().unwrap()).unwrap();
    UnparsedPublicKey::new(&ED25519,B64.decode(bootstrap["hostPublicKey"].as_str().unwrap()).unwrap()).verify(&answer_bytes,&B64.decode(envelope["signature"].as_str().unwrap()).unwrap()).unwrap();
    let answer:Value=serde_json::from_slice(&answer_bytes).unwrap();
    assert_eq!(answer["offerHash"],hex_sha256(&payload));
    assert_eq!(answer["attempt"],attempt);
    peer.set_remote_description(serde_json::from_value(answer["description"].clone()).unwrap()).await.unwrap();
    timeout(Duration::from_secs(20),async {
        while channel.ready_state()!=webrtc::data_channel::data_channel_state::RTCDataChannelState::Open {tokio::time::sleep(Duration::from_millis(20)).await;}
    }).await.unwrap();
    let selected=peer.sctp().transport().ice_transport().get_selected_candidate_pair().await.unwrap();
    assert!(selected.to_string().contains("relay"),"selected ICE pair must include relay");
    eprintln!("Relay selected; requesting Gateway handshake");
    let mut rtc=RtcClient{peer,channel,incoming};
    rtc.request(handshake_request("cloud-handshake","cloud-rust-probe")).await;
    assert!(rtc.next_json().await.get("result").is_some());
    eprintln!("Relay handshake passed; requesting large RPC");
    let id=format!("cloud-{}","界".repeat(24000));
    rtc.request(gateway::ProtocolRequest::ProviderList{jsonrpc:"2.0".into(),id:id.clone(),params:gateway::ProviderListRequest{}}).await;
    // Match the production RPC deadline, rather than the LAN fixture's 5 seconds.
    assert_eq!(rtc.next_json_timeout(Duration::from_secs(15)).await["id"],id);
    eprintln!("Relay large RPC passed; revoking credential");
    let credential=host.remote_access.validate_bearer(&pairing.credential).unwrap();
    host.remote_access.revoke_credential(&credential.credential_id).unwrap();
    assert_eq!(server.disconnect_credential(&credential.credential_id).await.unwrap(),1);
    wait_for_active_sessions(&server,0).await;
    rtc.peer.close().await.unwrap();
    server.shutdown().await.unwrap();
    eprintln!("Public signaling + selected TURN relay + Gateway handshake + large RPC + revocation passed");
}
