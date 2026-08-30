use std::io::Cursor;

use codepet_provider_sdk::{
    decode_request, decode_wire_message, JsonLineCodec, JsonRpcInboundRequest,
    JsonRpcNotification, ProtocolClient, ProtocolError, ProtocolRequest,
    ProtocolInboundFuture, ProtocolMethod, ProtocolTransport, ProtocolTransportFuture,
    ProviderWireMessage, JSON_RPC_INVALID_PARAMS, JSON_RPC_INVALID_REQUEST,
    JSON_RPC_METHOD_NOT_FOUND, JSON_RPC_PARSE_ERROR,
};

#[test]
fn generated_sdk_builds_a_typed_wire_request_from_method_and_params() {
    let params = serde_json::json!({
        "hostClientId": "client-test",
        "hostDeviceId": "device-test",
        "hostVersion": "0.1.0",
        "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
    });
    let request = ProtocolRequest::from_method_params(
        ProtocolMethod::ProviderInitialize,
        "request-1".to_string(),
        params,
    )
    .unwrap();

    let ProtocolRequest::ProviderInitialize { id, params, .. } = request else {
        panic!("expected typed initialize request");
    };
    assert_eq!(id, "request-1");
    assert_eq!(params.host_device_id, "device-test");
}

#[test]
fn json_line_codec_round_trips_one_bounded_request_frame() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/initialize-request.json"
    ))
    .unwrap();
    let message = ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request));
    let codec = JsonLineCodec::new(4096).unwrap();
    let mut output = Vec::new();
    codec.write_message(&mut output, &message).unwrap();
    assert_eq!(output.last(), Some(&b'\n'));

    let decoded = codec
        .read_message(&mut Cursor::new(output))
        .unwrap()
        .expect("one framed message");
    assert!(matches!(
        decoded,
        ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(_))
    ));
}

#[test]
fn json_line_codec_rejects_oversized_frames_without_unbounded_reads() {
    let codec = JsonLineCodec::new(16).unwrap();
    let error = codec
        .decode_line(br#"{"jsonrpc":"2.0","method":"provider.log","params":{}}"#)
        .unwrap_err();
    assert_eq!(error.error.code, JSON_RPC_INVALID_REQUEST);
    assert!(error.error.message.contains("exceeds 16 bytes"));

    let codec = JsonLineCodec::new(64).unwrap();
    let mut framed = vec![b'x'; 128];
    framed.extend_from_slice(
        b"\n{\"jsonrpc\":\"2.0\",\"method\":\"provider.log\",\"params\":{}}\n",
    );
    let mut input = Cursor::new(framed);
    let error = codec.read_message(&mut input).unwrap_err();
    assert_eq!(error.error.code, JSON_RPC_INVALID_REQUEST);
    let message = codec.read_message(&mut input).unwrap().unwrap();
    assert!(matches!(message, ProviderWireMessage::Notification(_)));
}

#[test]
fn malformed_and_invalid_messages_use_standard_json_rpc_errors() {
    let parse_error = decode_wire_message(b"{").unwrap_err();
    assert_eq!(parse_error.error.code, JSON_RPC_PARSE_ERROR);

    let invalid_request = decode_wire_message(br#"{"jsonrpc":"2.0"}"#).unwrap_err();
    assert_eq!(invalid_request.error.code, JSON_RPC_INVALID_REQUEST);
}

#[test]
fn unknown_method_and_invalid_params_preserve_request_id() {
    let unknown = decode_wire_message(
        br#"{"jsonrpc":"2.0","id":"unknown-1","method":"missing.call","params":{}}"#,
    )
    .unwrap();
    let ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(unknown)) = unknown else {
        panic!("expected rejected request");
    };
    assert_eq!(unknown.id, "unknown-1");
    assert_eq!(unknown.error.code, JSON_RPC_METHOD_NOT_FOUND);
    let response = unknown.into_response();
    assert_eq!(response.id.as_deref(), Some("unknown-1"));

    let invalid = decode_wire_message(
        br#"{"jsonrpc":"2.0","id":"invalid-1","method":"provider.initialize","params":{}}"#,
    )
    .unwrap();
    let ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(invalid)) = invalid else {
        panic!("expected rejected params");
    };
    assert_eq!(invalid.id, "invalid-1");
    assert_eq!(invalid.error.code, JSON_RPC_INVALID_PARAMS);
    let response = invalid.into_response();
    assert_eq!(response.id.as_deref(), Some("invalid-1"));
    let codepet_provider_sdk::JsonRpcResponsePayload::Error { error } = response.response else {
        panic!("expected invalid params response");
    };
    assert_eq!(error.code, JSON_RPC_INVALID_PARAMS);
}

#[test]
fn response_requires_exactly_one_of_result_or_error() {
    let both = decode_wire_message(
        br#"{"jsonrpc":"2.0","id":"response-1","result":{},"error":{"code":-32603,"message":"bad"}}"#,
    )
    .unwrap_err();
    assert_eq!(both.id.as_deref(), Some("response-1"));
    assert_eq!(both.error.code, JSON_RPC_INVALID_REQUEST);

    let neither = decode_wire_message(br#"{"jsonrpc":"2.0","id":"response-2"}"#)
        .unwrap_err();
    assert_eq!(neither.id.as_deref(), Some("response-2"));
    assert_eq!(neither.error.code, JSON_RPC_INVALID_REQUEST);
}

#[test]
fn inbound_classifier_distinguishes_events_and_other_notifications() {
    let event = decode_wire_message(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/conversation-upserted-event.json"
    ))
    .unwrap();
    assert!(matches!(event, ProviderWireMessage::Event(_)));

    let notification = decode_wire_message(
        br#"{"jsonrpc":"2.0","method":"provider.log","params":{"message":"ready"}}"#,
    )
    .unwrap();
    let ProviderWireMessage::Notification(notification) = notification else {
        panic!("expected notification");
    };
    assert_eq!(notification.method, "provider.log");
}

struct InboundTransport;

impl ProtocolTransport for InboundTransport {
    fn request<'a>(
        &'a self,
        _method: ProtocolMethod,
        _params: serde_json::Value,
    ) -> ProtocolTransportFuture<'a> {
        Box::pin(async {
            Err(ProtocolError {
                code: "unused".to_string(),
                message: "unused in inbound test".to_string(),
                retryable: false,
                details: None,
            })
        })
    }

    fn next_message<'a>(&'a self) -> ProtocolInboundFuture<'a> {
        Box::pin(async {
            Ok(ProviderWireMessage::Notification(JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "provider.log".to_string(),
                params: serde_json::json!({ "message": "ready" }),
            }))
        })
    }
}

#[tokio::test]
async fn transport_exposes_notification_and_event_inbound_messages() {
    let client = ProtocolClient::new(InboundTransport);
    let message = client.next_message().await.unwrap();
    let ProviderWireMessage::Notification(notification) = message else {
        panic!("expected inbound notification");
    };
    assert_eq!(notification.params["message"], "ready");
}
