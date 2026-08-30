use codepet_gateway_sdk::{
    decode_event, decode_request, decode_response, CurrentCredentialDeleteResponse,
    PairingExchangeRequest, PairingExchangeResponse, PairingQrPayload, ProtocolEvent,
    ProtocolRequest, ProtocolResponse, ResponsePayload,
};

#[test]
fn gateway_handshake_returns_a_dedicated_remote_host_identity() {
    let response = decode_response(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/handshake-response.json"
    ))
    .unwrap();
    let ProtocolResponse::ProtocolHandshake {
        response: ResponsePayload::Ok { result },
        ..
    } = response
    else {
        panic!("expected successful protocol.handshake response");
    };
    assert_eq!(result.device.device_id, "device-macbook-1");
    assert_eq!(result.device.display_name, "MacBook");
    assert_eq!(result.device.identity_fingerprint.len(), 64);
    assert_eq!(result.devices[0].device_id, result.device.device_id);
}

#[test]
fn gateway_lan_rest_fixtures_use_the_generated_dtos() {
    let qr: PairingQrPayload = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-qr-payload.json"
    ))
    .unwrap();
    assert_eq!(qr.version, 1);
    assert_eq!(qr.host_device_id, "device-macbook-1");
    assert_eq!(qr.cert_sha256.len(), 64);

    let request: PairingExchangeRequest = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-exchange-request.json"
    ))
    .unwrap();
    assert_eq!(request.client_id, "remote-client-phone-1");
    assert_eq!(request.platform, "android");

    let response: PairingExchangeResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-exchange-response.json"
    ))
    .unwrap();
    assert_eq!(response.device.device_id, qr.host_device_id);
    assert_eq!(response.gateway_url, "wss://192.168.1.10:49152/remote/v1/gateway");

    let deleted: CurrentCredentialDeleteResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/current-credential-delete-response.json"
    ))
    .unwrap();
    assert!(deleted.revoked);
}

#[test]
fn gateway_lan_secret_fields_are_redacted_from_debug_output() {
    let qr: PairingQrPayload = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-qr-payload.json"
    ))
    .unwrap();
    let qr_debug = format!("{qr:?}");
    assert!(!qr_debug.contains(&qr.pairing_secret));
    assert!(qr_debug.contains("<redacted>"));

    let request: PairingExchangeRequest = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-exchange-request.json"
    ))
    .unwrap();
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains(&request.pairing_secret));
    assert!(request_debug.contains("<redacted>"));

    let response: PairingExchangeResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-exchange-response.json"
    ))
    .unwrap();
    let response_debug = format!("{response:?}");
    assert!(!response_debug.contains(&response.credential));
    assert!(response_debug.contains("<redacted>"));
}

#[test]
fn gateway_event_subscription_preserves_the_exact_opaque_cursor() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/event-subscribe-request.json"
    ))
    .unwrap();
    let ProtocolRequest::EventSubscribe { params, .. } = request else {
        panic!("expected event.subscribe request");
    };
    assert_eq!(params.after_cursor, "opaque-gateway-cursor-QkFTRQ");

    let response = decode_response(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/event-subscribe-response.json"
    ))
    .unwrap();
    let ProtocolResponse::EventSubscribe {
        response: ResponsePayload::Ok { result },
        ..
    } = response
    else {
        panic!("expected successful event.subscribe response");
    };
    assert_eq!(
        result.subscribed_after_cursor,
        "opaque-gateway-cursor-QkFTRQ"
    );
}

#[test]
fn gateway_request_routes_by_all_four_resource_dimensions() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-get-request.json"
    ))
    .unwrap();
    let ProtocolRequest::ConversationGet { params, .. } = request else {
        panic!("expected conversation.get");
    };
    assert_eq!(params.conversation.device_id, "device-macbook-1");
    assert_eq!(
        params.conversation.provider_plugin_id,
        "dev.codepet.codex"
    );
    assert_eq!(params.conversation.provider_instance_id, "codex-work");
    assert_eq!(params.conversation.native_resource_id, "thread-01");
}

#[test]
fn gateway_event_carries_an_opaque_cursor_and_full_resource_route() {
    let event = decode_event(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-upserted-event.json"
    ))
    .unwrap();
    let ProtocolEvent::ConversationUpserted {
        event_cursor,
        payload,
        ..
    } = event
    else {
        panic!("expected conversation.upserted");
    };
    assert_eq!(event_cursor, "event-41");
    assert_eq!(payload.conversation.resource.device_id, "device-macbook-1");
    assert_eq!(
        payload.conversation.resource.provider_plugin_id,
        "dev.codepet.codex"
    );
    assert_eq!(payload.conversation.resource.provider_instance_id, "codex-work");
    assert_eq!(payload.conversation.resource.native_resource_id, "thread-01");
}

#[test]
fn gateway_conversation_snapshots_carry_the_query_boundary_cursor() {
    let get = decode_response(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-get-response.json"
    ))
    .unwrap();
    let ProtocolResponse::ConversationGet {
        response: ResponsePayload::Ok { result },
        ..
    } = get
    else {
        panic!("expected successful conversation.get response");
    };
    assert_eq!(
        result.snapshot_cursor,
        "opaque-gateway-cursor-U05BUFNIT1Q"
    );

    let list = decode_response(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-list-response.json"
    ))
    .unwrap();
    let ProtocolResponse::ConversationList {
        response: ResponsePayload::Ok { result },
        ..
    } = list
    else {
        panic!("expected successful conversation.list response");
    };
    assert_eq!(
        result.snapshot_cursor,
        "opaque-gateway-cursor-U05BUFNIT1Q"
    );
}
