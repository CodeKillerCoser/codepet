use codepet_gateway_sdk::{
    decode_event, decode_observed_wire_message, decode_request, decode_response,
    encode_event_with_trace, GatewayCapability, HandshakeResponse, JsonRpcResponsePayload,
    ModelCatalog, ModelSelection, ProtocolEvent, ProtocolMethod, ProtocolRequest, TraceContext,
    TurnSendResponse,
};
use serde::de::DeserializeOwned;

fn decode_success<T: DeserializeOwned>(bytes: &[u8]) -> T {
    let response = decode_response(bytes).unwrap();
    assert_eq!(response.jsonrpc, "2.0");
    let JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected successful JSON-RPC response");
    };
    serde_json::from_value(result).unwrap()
}

#[test]
fn gateway_handshake_uses_json_rpc_and_business_identity_only() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/handshake-request.json"
    ))
    .unwrap();
    let ProtocolRequest::ProtocolHandshake {
        jsonrpc, params, ..
    } = request
    else {
        panic!("expected protocol.handshake request");
    };
    assert_eq!(jsonrpc, "2.0");
    assert_eq!(params.supported_versions.min_version, 1);
    assert_eq!(params.device.device_name, "Alice's Pixel");

    let result: HandshakeResponse = decode_success(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/handshake-response.json"
    ));
    assert_eq!(result.protocol.version, 1);
    assert_eq!(result.device.name, "MacBook");
    assert_eq!(result.device.operating_system, "macOS");
    let identity = serde_json::to_value(&result.device).unwrap();
    assert!(identity.get("identityFingerprint").is_none());
}

#[test]
fn gateway_turn_send_derives_route_from_conversation() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/turn-send-request.json"
    ))
    .unwrap();
    let ProtocolRequest::TurnSend { params, .. } = request else {
        panic!("expected turn.send request");
    };
    assert_eq!(params.client_request_id, "remote-turn-01");
    assert_eq!(params.conversation.provider_id, "codex-work");
    let encoded = serde_json::to_value(&params).unwrap();
    assert!(encoded.get("route").is_none());

    let result: TurnSendResponse = decode_success(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/turn-send-response.json"
    ));
    assert!(result.accepted);
    assert!(result.user_item.is_none());
    let Some(ModelSelection::FlatModelSelection(model)) = result.effective_selection.model else {
        panic!("expected effective flat model selection");
    };
    assert_eq!(model.model_id, "gpt-5");
}

#[test]
fn model_catalogs_retain_discriminated_shapes() {
    let flat: ModelCatalog = serde_json::from_value(serde_json::json!({
        "kind": "flat",
        "models": [{"id": "gpt-5", "displayName": "GPT-5"}],
        "defaultSelection": {"kind": "flat", "modelId": "gpt-5"}
    }))
    .unwrap();
    let ModelCatalog::FlatModelCatalog(flat) = flat else {
        panic!("expected flat catalog");
    };
    assert_eq!(flat.models[0].id, "gpt-5");

    let grouped: ModelCatalog = serde_json::from_value(serde_json::json!({
        "kind": "grouped",
        "providers": [
            {"id": "openai", "displayName": "OpenAI", "models": [{"id": "gpt-5", "displayName": "GPT-5"}]},
            {"id": "anthropic", "displayName": "Anthropic", "models": [{"id": "claude-sonnet", "displayName": "Claude Sonnet"}]}
        ],
        "defaultSelection": {"kind": "grouped", "providerId": "openai", "modelId": "gpt-5"}
    }))
    .unwrap();
    let ModelCatalog::GroupedModelCatalog(grouped) = grouped else {
        panic!("expected grouped catalog");
    };
    assert_eq!(grouped.providers[1].id, "anthropic");
}

#[test]
fn gateway_event_wraps_cursor_inside_json_rpc_params() {
    let event = decode_event(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-upserted-event.json"
    ))
    .unwrap();
    let ProtocolEvent::ConversationUpserted { jsonrpc, params } = event else {
        panic!("expected conversation.upserted");
    };
    assert_eq!(jsonrpc, "2.0");
    assert_eq!(params.event_cursor, "event-41");
    assert_eq!(
        params.payload.conversation.resource.provider_id,
        "codex-work"
    );
}

#[test]
fn gateway_capability_metadata_is_typed() {
    assert_eq!(
        ProtocolMethod::ConversationSearch.capability(),
        Some(GatewayCapability::ConversationSearch)
    );
    assert_eq!(
        ProtocolMethod::TurnSend.capability(),
        Some(GatewayCapability::TurnSend)
    );
}

#[test]
fn trace_context_is_observed_without_entering_business_payloads() {
    let request = br#"{"jsonrpc":"2.0","id":"request-1","method":"protocol.describe","params":{},"meta":{"traceparent":"00-0123456789abcdef0123456789abcdef-0123456789abcdef-01"}}"#;
    let observed = decode_observed_wire_message(request).unwrap();
    assert_eq!(
        observed.trace_context.unwrap().traceparent,
        "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01"
    );

    let event = decode_event(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-upserted-event.json"
    ))
    .unwrap();
    let encoded = encode_event_with_trace(
        &event,
        Some(&TraceContext {
            traceparent:
                "00-0123456789abcdef0123456789abcdef-fedcba9876543210-01".to_string(),
            tracestate: None,
        }),
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        json["meta"]["traceparent"],
        "00-0123456789abcdef0123456789abcdef-fedcba9876543210-01"
    );
}
