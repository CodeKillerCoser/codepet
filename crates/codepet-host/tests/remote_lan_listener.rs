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
                gateway::GatewayHostIdentity {
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
            "wss://localhost:{}/remote/v2/gateway",
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
        device_id: "device-lan-listener".to_string(),
        provider_plugin_id: "dev.codepet.lan-listener".to_string(),
        provider_instance_id: "instance-lan-listener".to_string(),
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

async fn collect_response_and_events(
    socket: &mut TestWebSocket,
    response_id: &str,
    event_count: usize,
) -> (gateway::JsonRpcResponse, Vec<gateway::ProtocolEvent>) {
    let mut response = None;
    let mut events = Vec::new();
    while response.is_none() || events.len() < event_count {
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
        panic!("expected a WebSocket close frame");
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
            "wss://listener.local:{}/remote/v2/gateway",
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
        Some(format!("wss://127.0.0.1:{}/remote/v2/gateway", server.port()))
    );
    let address = server.local_addr();
    let gateway_url = server.gateway_url().unwrap();
    let certificate_der = host.remote_access.tls_identity().certificate_der().to_vec();
    let client = PinnedTlsClient::new(address, certificate_der.clone());

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
        format!("wss://127.0.0.2:{}/remote/v2/gateway", server.port())
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

    let pairing_a = pair_client(host.remote_access.as_ref(), &client, "client-a").await;
    assert_eq!(pairing_a.device, host.remote_access.remote_host_identity());
    assert_eq!(pairing_a.gateway_url, gateway_url);
    let mut socket_a = client
        .connect_websocket(&pairing_a.credential)
        .await
        .unwrap();
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
    assert_eq!(handshake.device.device_id, pairing_a.device.device_id);
    assert_eq!(handshake.device.descriptor, pairing_a.device.descriptor);
    assert_eq!(
        handshake.device.descriptor,
        gateway::DeviceDescriptor {
            device_name: "LAN Listener Test Host".to_string(),
            operating_system: "TestOS".to_string(),
            system_version: "1.0".to_string(),
        }
    );
    assert_eq!(
        handshake.devices[0].display_name,
        handshake.device.descriptor.device_name
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
        gateway::ProtocolRequest::DeviceList {
            jsonrpc: "2.0".to_string(),
            id: "before-handshake".to_string(),
            params: gateway::DeviceListRequest {},
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
    oversized
        .send(Message::Text("x".repeat(300 * 1024)))
        .await
        .unwrap();
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
        let request = gateway::ProtocolRequest::DeviceList {
            jsonrpc: "2.0".to_string(),
            id: format!("backpressure-{sequence}-{large_id_tail}"),
            params: gateway::DeviceListRequest {},
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
        gateway::ProtocolRequest::DeviceList {
            jsonrpc: "2.0".to_string(),
            id: "healthy-during-backpressure".to_string(),
            params: gateway::DeviceListRequest {},
        },
    )
    .await;
    let healthy_during_backpressure =
        next_response(&mut socket_a, "healthy-during-backpressure").await;
    let _: gateway::DeviceListResponse = response_result(healthy_during_backpressure);
    wait_for_active_sessions(&server, 1).await;
    drop(stalled);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationList {
            jsonrpc: "2.0".to_string(),
            id: "list-a".to_string(),
            params: gateway::ConversationListRequest {
                route: Some(gateway::GatewayProviderRoute {
                    device_id: "device-lan-listener".to_string(),
                    provider_plugin_id: "dev.codepet.lan-listener".to_string(),
                    provider_instance_id: "instance-lan-listener".to_string(),
                }),
                cursor: None,
                limit: Some(10),
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
    let replay_one: gateway::ProtocolEvent = serde_json::from_value(next_value(&mut socket_a).await).unwrap();
    let replay_two: gateway::ProtocolEvent = serde_json::from_value(next_value(&mut socket_a).await).unwrap();
    let replay_sequences = [
        cursor_sequence(event_cursor(&replay_one)),
        cursor_sequence(event_cursor(&replay_two)),
    ];
    assert!(replay_sequences[0] < replay_sequences[1]);

    send_request(
        &mut socket_a,
        gateway::ProtocolRequest::ConversationGet {
            jsonrpc: "2.0".to_string(),
            id: "live-event-a".to_string(),
            params: gateway::ConversationGetRequest {
                conversation: conversation_resource("event-first"),
            },
        },
    )
    .await;
    let (_, live_events) = collect_response_and_events(&mut socket_a, "live-event-a", 2).await;
    let live_sequences = live_events
        .iter()
        .map(|event| cursor_sequence(event_cursor(event)))
        .collect::<Vec<_>>();
    assert!(replay_sequences[1] < live_sequences[0]);
    assert!(live_sequences[0] < live_sequences[1]);

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
    let duplicate = next_response(&mut socket_a, "subscribe-a-again").await;
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
            },
        },
    )
    .await;
    let (_, client_a_events) =
        collect_response_and_events(&mut socket_a, "client-a-only", 2).await;
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
            },
        },
    )
    .await;
    let (_, events_a) =
        collect_response_and_events(&mut socket_a, "shared-event-source-a", 2).await;
    let event_b_one: gateway::ProtocolEvent =
        serde_json::from_value(next_value(&mut socket_b).await).unwrap();
    let event_b_two: gateway::ProtocolEvent =
        serde_json::from_value(next_value(&mut socket_b).await).unwrap();
    assert_eq!(event_cursor(&events_a[0]), event_cursor(&event_b_one));
    assert_eq!(event_cursor(&events_a[1]), event_cursor(&event_b_two));

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
        gateway::ProtocolRequest::DeviceList {
            jsonrpc: "2.0".to_string(),
            id: "client-b-still-active".to_string(),
            params: gateway::DeviceListRequest {},
        },
    )
    .await;
    let client_b_active = next_response(&mut socket_b, "client-b-still-active").await;
    let _: gateway::DeviceListResponse = response_result(client_b_active);

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
