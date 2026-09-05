use std::io::Cursor;

use codepet_provider_sdk::{
    decode_request, decode_wire_message, JsonRpcInboundRequest,
    JsonRpcNotification, ProtocolClient, ProtocolDispatchLane, ProtocolError, ProtocolRequest,
    ProtocolInboundFuture, ProtocolMethod, ProtocolTransport, ProtocolTransportFuture,
    ProviderFrameCodec, ProviderFrameEncoding, ProviderWireMessage, MAX_PROVIDER_FRAME_BYTES,
    PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES, PROVIDER_FRAME_HEADER_BYTES,
    PROVIDER_FRAME_MAGIC, PROVIDER_FRAME_VERSION, JSON_RPC_INVALID_PARAMS, JSON_RPC_INVALID_REQUEST,
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
    assert_eq!(request.method(), ProtocolMethod::ProviderInitialize);
    assert_eq!(request.method().dispatch_lane(), ProtocolDispatchLane::Normal);
    assert_eq!(request.id(), "request-1");

    let ProtocolRequest::ProviderInitialize { id, params, .. } = request else {
        panic!("expected typed initialize request");
    };
    assert_eq!(id, "request-1");
    assert_eq!(params.host_device_id, "device-test");
}

#[test]
fn generated_dispatch_lane_keeps_lifecycle_control_available_under_load() {
    assert_eq!(
        ProtocolMethod::InstanceStop.dispatch_lane(),
        ProtocolDispatchLane::Control
    );
    assert_eq!(
        ProtocolMethod::InstanceDestroy.dispatch_lane(),
        ProtocolDispatchLane::Control
    );
    assert_eq!(
        ProtocolMethod::ProviderShutdown.dispatch_lane(),
        ProtocolDispatchLane::Control
    );
}

#[test]
fn provider_frame_v1_round_trips_small_json_without_compression() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/initialize-request.json"
    ))
    .unwrap();
    let message = ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request));
    let codec = ProviderFrameCodec::default();
    let mut output = Vec::new();
    codec.write_message(&mut output, &message).unwrap();
    assert_eq!(&output[..4], &PROVIDER_FRAME_MAGIC);
    assert_eq!(output[4], PROVIDER_FRAME_VERSION);
    assert_eq!(output[5], ProviderFrameEncoding::RawJson as u8);
    assert_eq!(
        u32::from_be_bytes(output[6..10].try_into().unwrap()) as usize,
        output.len() - PROVIDER_FRAME_HEADER_BYTES
    );

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
fn provider_frame_v1_compresses_large_json_and_does_not_limit_decoded_size() {
    let text = "x".repeat(MAX_PROVIDER_FRAME_BYTES + 1024);
    let message = ProviderWireMessage::Notification(JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "provider.log".to_string(),
        params: serde_json::json!({ "message": text }),
    });
    let codec = ProviderFrameCodec::default();
    let encoded = codec.encode_message_with_metrics(&message).unwrap();
    let frame = encoded.frame;
    assert_eq!(frame[5], ProviderFrameEncoding::ZstdJson as u8);
    assert!(frame.len() < PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES);
    assert_eq!(encoded.metrics.encoding, ProviderFrameEncoding::ZstdJson);
    assert_eq!(encoded.metrics.frame_bytes, frame.len());
    assert_eq!(
        encoded.metrics.encoded_payload_bytes,
        frame.len() - PROVIDER_FRAME_HEADER_BYTES
    );
    assert!(encoded.metrics.json_bytes > MAX_PROVIDER_FRAME_BYTES);
    assert!(encoded.metrics.encoded_payload_bytes < encoded.metrics.json_bytes);

    let decoded = codec.decode_frame(&frame).unwrap();
    let ProviderWireMessage::Notification(notification) = decoded else {
        panic!("expected notification");
    };
    assert_eq!(
        notification.params["message"].as_str().unwrap().len(),
        MAX_PROVIDER_FRAME_BYTES + 1024
    );
}

#[test]
fn provider_frame_v1_limits_final_header_plus_encoded_payload() {
    let message = ProviderWireMessage::Notification(JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "provider.log".to_string(),
        params: serde_json::json!({ "message": "x".repeat(2048) }),
    });
    let codec = ProviderFrameCodec::new(1024).unwrap();
    let error = codec.encode_message(&message).unwrap_err();
    assert_eq!(error.code, "provider_frame_too_large");
    let details = error.details.expect("oversized frame metrics");
    assert_eq!(details["maxFrameBytes"], 1024);
    assert!(details["jsonBytes"].as_u64().unwrap() > 2048);
    assert_eq!(details["encoding"], "raw-json");
}

#[test]
fn provider_frame_v1_rejects_invalid_and_truncated_headers() {
    let codec = ProviderFrameCodec::default();
    let mut invalid_magic = [0_u8; PROVIDER_FRAME_HEADER_BYTES];
    invalid_magic[4] = PROVIDER_FRAME_VERSION;
    let error = codec.decode_frame(&invalid_magic).unwrap_err();
    assert_eq!(error.error.code, JSON_RPC_INVALID_REQUEST);
    assert!(error.error.message.contains("magic"));

    let mut truncated = Cursor::new(&PROVIDER_FRAME_MAGIC[..2]);
    let error = codec.read_message(&mut truncated).unwrap_err();
    assert_eq!(error.error.code, JSON_RPC_INVALID_REQUEST);
    assert!(error.error.message.contains("truncated"));
}

#[test]
fn provider_frame_v1_header_decoder_is_the_public_size_and_version_gate() {
    let codec = ProviderFrameCodec::new(1024).unwrap();
    let mut header = [0_u8; PROVIDER_FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(&PROVIDER_FRAME_MAGIC);
    header[4] = PROVIDER_FRAME_VERSION;
    header[5] = ProviderFrameEncoding::RawJson as u8;
    header[6..10].copy_from_slice(&1014_u32.to_be_bytes());

    let decoded = codec.decode_header(&header).unwrap();
    assert_eq!(decoded.payload_length, 1014);

    header[6..10].copy_from_slice(&1015_u32.to_be_bytes());
    let oversized = codec.decode_header(&header).unwrap_err();
    assert_eq!(oversized.error.code, JSON_RPC_INVALID_REQUEST);
    assert!(oversized.error.message.contains("exceeds 1024 bytes"));

    header[6..10].copy_from_slice(&0_u32.to_be_bytes());
    header[4] = PROVIDER_FRAME_VERSION + 1;
    let unsupported_version = codec.decode_header(&header).unwrap_err();
    assert!(unsupported_version.error.message.contains("version"));

    header[4] = PROVIDER_FRAME_VERSION;
    header[5] = 255;
    let unsupported_encoding = codec.decode_header(&header).unwrap_err();
    assert!(unsupported_encoding.error.message.contains("encoding"));
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
