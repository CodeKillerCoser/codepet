use super::*;
use crate::{JsonRpcResponse, JsonRpcResponsePayload, ProtocolMethod, ProtocolRequest,
    ProviderMux, MuxIncoming, MuxDriver, ProviderWireMessage, JsonRpcInboundRequest, default_transport_limits};
// Test the transport contract without Provider DTOs, JSON, compression or RPC errors.
// Production profile selection always uses ProviderMessageCodec.
struct BytesCodec;

impl MessageCodec for BytesCodec {
    type Message = Vec<u8>;
    type Error = TransportError;
    const MIN_ENCODED_BYTES: usize = 1;

    fn classify(message: &Vec<u8>, small_message_bytes: usize) -> Result<Class, TransportError> {
        Ok(if message.len() <= small_message_bytes { Class::Small } else { Class::Normal })
    }

    fn encode(message: &Vec<u8>, encoded_cap: usize, decoded_cap: usize) -> Result<(Vec<u8>, usize), TransportError> {
        if message.len() > encoded_cap.min(decoded_cap) { return Err(TransportError::new("test payload too large")); }
        Ok((message.clone(), message.len()))
    }

    fn decode(frame: &[u8], decoded: usize, _: Class) -> Result<Vec<u8>, TransportError> {
        if frame.len() != decoded { return Err(TransportError::new("test payload length mismatch")); }
        Ok(frame.to_vec())
    }

    fn response_class(_: &Vec<u8>) -> Option<Class> { None }
}

#[tokio::test]
async fn transport_accepts_an_independent_codec_and_retains_receive_permits() {
    let (a, b) = tokio::io::duplex(64);
    let (ar, aw) = tokio::io::split(a);
    let (br, bw) = tokio::io::split(b);
    let (host, server) = tokio::join!(
        Connection::<BytesCodec>::connect(ar, aw, true, default_transport_limits()),
        Connection::<BytesCodec>::connect(br, bw, false, default_transport_limits()),
    );
    let (host, _, _host_driver) = host.unwrap();
    let (server, mut incoming, _server_driver) = server.unwrap();
    let caller = host.clone();
    let task = tokio::spawn(async move { caller.exchange(vec![0, 255, 42]).await });
    let mut stream = incoming.recv().await.unwrap();
    let message = stream.receive().await.unwrap();
    assert_eq!(message.message, [0, 255, 42]);
    assert_eq!(server.receive.lanes[1].bytes.available_permits(), 1024 * 1024 - 6);
    drop(message);
    assert_eq!(server.receive.lanes[1].bytes.available_permits(), 1024 * 1024);
    stream.respond(vec![255, 0]).await.unwrap();
    let response = task.await.unwrap().unwrap();
    assert_eq!(response.message, [255, 0]);
    assert_eq!(host.receive.lanes[1].bytes.available_permits(), 1024 * 1024 - 4);
    drop(response);
    assert_eq!(host.receive.lanes[1].bytes.available_permits(), 1024 * 1024);
}

fn reply(value: serde_json::Value) -> ProviderWireMessage {
    ProviderWireMessage::Response(JsonRpcResponse { jsonrpc: "2.0".into(), id: Some("test".into()), response: JsonRpcResponsePayload::Ok { result: value } })
}
fn small() -> ProviderWireMessage {
    ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(ProtocolRequest::from_method_params(ProtocolMethod::ProviderDescribe, "test".into(), serde_json::json!({})).unwrap()))
}
fn entropy() -> String {
    let mut n = 17u64;
    (0..2*1024*1024).map(|_| { n ^= n << 13; n ^= n >> 7; n ^= n << 17; (b'!' + (n % 90) as u8) as char }).collect()
}
async fn pair() -> ((ProviderMux, mpsc::Receiver<MuxIncoming>, MuxDriver), (ProviderMux, mpsc::Receiver<MuxIncoming>, MuxDriver)) {
    let (a, b) = tokio::io::duplex(1024);
    let (ar, aw) = tokio::io::split(a); let (br, bw) = tokio::io::split(b);
    let (a, b) = tokio::join!(ProviderMux::connect(ar, aw, true, default_transport_limits()), ProviderMux::connect(br, bw, false, default_transport_limits()));
    (a.unwrap(), b.unwrap())
}

#[tokio::test]
async fn control_exchange_progresses_with_normal_and_small_send_budgets_exhausted() {
    let ((host, _, _hd), (_server, mut incoming, _sd)) = pair().await;
    let _bulk = host.send.lanes[0].bytes.clone().acquire_many_owned(256*1024*1024).await.unwrap();
    let _small = host.send.lanes[1].bytes.clone().acquire_many_owned(1024*1024).await.unwrap();
    let request = ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(ProtocolRequest::from_method_params(
        ProtocolMethod::ProviderPing, "test".into(), serde_json::json!({"sequence":1,"hostSessionId":"test","clients":{"revision":1,"connections":[]},"instances":[]})
    ).unwrap()));
    let task = tokio::spawn(async move { host.exchange(request).await });
    let mut stream = incoming.recv().await.unwrap(); let _message = stream.receive().await.unwrap();
    stream.respond(reply(serde_json::json!(true))).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().is_ok());
}

#[tokio::test]
async fn partial_message_timeout_releases_reserved_receive_bytes() {
    let ((host, _, _hd), (mut server, mut incoming, _sd)) = pair().await;
    // A short local timeout makes an incomplete body deterministic without slowing the test suite.
    let mut limits = default_transport_limits(); limits.idle_timeout_ms = 100;
    server.receive = Arc::new(Resources::new(limits));
    let mut raw = host.open_stream().await.unwrap();
    let mut header = [0; HEADER_BYTES]; header[0] = 1;
    header[2..6].copy_from_slice(&1024u32.to_be_bytes()); header[6..10].copy_from_slice(&1024u32.to_be_bytes());
    raw.write_all(&header).await.unwrap(); raw.flush().await.unwrap();
    let mut stream = incoming.recv().await.unwrap(); stream.peer = server.clone();
    assert!(matches!(stream.receive().await, Err(e) if e.message.contains("idle timeout")));
    assert_eq!(server.receive.lanes[0].bytes.available_permits(), 256*1024*1024);
}

#[tokio::test]
async fn negotiated_frame_bound_is_checked_before_body_allocation() {
    let mut header = [0;12]; header[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut io = FrameBoundedIo::new(futures::io::Cursor::new(header), 16384, Duration::from_secs(1));
    let mut out = [0;12];
    assert!(io.read_exact(&mut out).await.unwrap_err().to_string().contains("negotiated limit"));
}

#[tokio::test]
async fn incomplete_physical_frame_times_out_without_killing_an_idle_connection() {
    let (mut writer, reader) = tokio::io::duplex(64);
    let mut io = FrameBoundedIo::new(reader.compat(), 16384, Duration::from_millis(30));
    let mut header = [0;12];
    assert!(tokio::time::timeout(Duration::from_millis(50), io.read_exact(&mut header)).await.is_err());
    tokio::io::AsyncWriteExt::write_all(&mut writer, &[0;3]).await.unwrap();
    let e = tokio::time::timeout(Duration::from_secs(1), io.read_exact(&mut header)).await.unwrap().unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn asymmetric_limits_return_a_small_error_for_oversized_response() {
    let (a,b) = tokio::io::duplex(1024);
    let (ar,aw) = tokio::io::split(a); let (br,bw) = tokio::io::split(b);
    let mut limits = default_transport_limits(); limits.max_decoded_message_bytes = 1024;
    limits.max_encoded_message_bytes = 2048; limits.small_message_bytes = 1024; limits.control_message_bytes = 1024;
    let (host, server) = tokio::join!(ProviderMux::connect(ar,aw,true,limits), ProviderMux::connect(br,bw,false,default_transport_limits()));
    let (host,_,_hd) = host.unwrap(); let (_, mut incoming,_sd) = server.unwrap();
    let task = tokio::spawn(async move { host.exchange(small()).await });
    let mut stream = incoming.recv().await.unwrap(); let _message = stream.receive().await.unwrap();
    stream.respond_with_fallback(reply(serde_json::json!("x".repeat(4096))), Some("test".into())).await.unwrap();
    let result = task.await.unwrap().unwrap();
    assert!(matches!(result.message, ProviderWireMessage::Response(JsonRpcResponse { response: JsonRpcResponsePayload::Error { .. }, .. })));
}

#[tokio::test]
async fn blocked_large_stream_does_not_block_small_stream_and_cancel_releases_capacity() {
    let ((host, _, _hd), (_server, mut incoming, _sd)) = pair().await;
    let h = host.clone();
    let large = tokio::spawn(async move { h.exchange(reply(serde_json::json!(entropy()))).await });
    let mut blocked = incoming.recv().await.unwrap();
    // Admit the body, then stop reading this stream. Its own Yamux window fills.
    let mut header = [0; HEADER_BYTES]; blocked.stream.read_exact(&mut header).await.unwrap();
    blocked.stream.write_all(&[1]).await.unwrap(); blocked.stream.flush().await.unwrap();
    let h = host.clone();
    let second_large = tokio::spawn(async move { h.exchange(reply(serde_json::json!(entropy()))).await });
    let mut second_blocked = tokio::time::timeout(Duration::from_secs(2), incoming.recv()).await.unwrap().unwrap();
    second_blocked.stream.read_exact(&mut header).await.unwrap();
    second_blocked.stream.write_all(&[1]).await.unwrap(); second_blocked.stream.flush().await.unwrap();
    let h = host.clone(); let small_request = tokio::spawn(async move { h.exchange(small()).await });
    let mut stream = tokio::time::timeout(Duration::from_secs(2), incoming.recv()).await.unwrap().unwrap();
    let message = stream.receive().await.unwrap(); assert!(matches!(message.message, ProviderWireMessage::Request(_)));
    stream.respond(reply(serde_json::json!("ok"))).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), small_request).await.unwrap().unwrap().is_ok());
    assert!(!large.is_finished(), "large stream must still be blocked");
    assert!(!second_large.is_finished());
    large.abort(); second_large.abort(); let _ = large.await; let _ = second_large.await;
    drop(blocked); drop(second_blocked);
    assert_eq!(host.send.lanes[0].slots.available_permits(), 24);
    assert_eq!(host.send.lanes[0].bytes.available_permits(), 256 * 1024 * 1024);
}

#[tokio::test]
async fn receiver_enforces_declared_budget_and_connection_recovers() {
    let ((host, _, _hd), (server, mut incoming, _sd)) = pair().await;
    let mut raw = host.open_stream().await.unwrap();
    let mut header = [0; HEADER_BYTES]; header[0] = 1;
    header[2..6].copy_from_slice(&1024u32.to_be_bytes());
    header[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
    raw.write_all(&header).await.unwrap(); raw.flush().await.unwrap();
    let mut stream = incoming.recv().await.unwrap();
    assert!(stream.receive().await.unwrap_err().message.contains("receive limits"));
    drop(stream); drop(raw);
    assert_eq!(server.receive.lanes[0].bytes.available_permits(), 256 * 1024 * 1024);
    let h = host.clone(); let task = tokio::spawn(async move { h.exchange(small()).await });
    let mut stream = incoming.recv().await.unwrap(); let _message = stream.receive().await.unwrap();
    stream.respond(reply(serde_json::json!(true))).await.unwrap();
    assert!(task.await.unwrap().is_ok());
}
