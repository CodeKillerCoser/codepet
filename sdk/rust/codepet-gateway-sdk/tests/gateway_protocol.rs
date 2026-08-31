use codepet_gateway_sdk::{
    decode_event, decode_request, decode_response, CurrentCredentialDeleteResponse,
    ConversationContentKind, ConversationItemKind, ConversationItemRole, PairingExchangeRequest,
    PairingExchangeResponse, PairingQrPayload, ProtocolEvent, ProtocolRequest, ProtocolResponse,
    ResponsePayload, GatewayCapability, ModelCatalog, ModelSelection, ProtocolMethod,
};

#[test]
fn gateway_handshake_exchanges_device_descriptors() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/handshake-request.json"
    ))
    .unwrap();
    let ProtocolRequest::ProtocolHandshake { params, .. } = request else {
        panic!("expected protocol.handshake request");
    };
    assert_eq!(params.device.device_name, "Alice's Pixel");
    assert_eq!(params.device.operating_system, "Android");
    assert_eq!(params.device.system_version, "16");

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
    assert_eq!(result.device.descriptor.device_name, "MacBook");
    assert_eq!(result.device.descriptor.operating_system, "macOS");
    assert_eq!(result.device.descriptor.system_version, "15.6");
    assert_eq!(result.device.identity_fingerprint.len(), 64);
    assert_eq!(result.devices[0].device_id, result.device.device_id);
    assert_eq!(result.providers[0].harness.id, "codex");
    assert_eq!(result.providers[0].harness.version.as_deref(), Some("0.151.0"));
    assert_eq!(result.providers[0].capabilities.revision, "codex-session-42");
    let Some(turn_send) = result.providers[0].capabilities.turn_send.as_ref() else {
        panic!("expected advertised turn.send controls");
    };
    let Some(ModelCatalog::FlatModelCatalog(catalog)) = turn_send.model_catalog.as_ref() else {
        panic!("expected flat model catalog");
    };
    assert_eq!(catalog.default_selection.as_ref().unwrap().model_id, "gpt-5");
}

#[test]
fn gateway_turn_send_preserves_revision_selection_and_canonical_user_item() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/turn-send-request.json"
    ))
    .unwrap();
    let ProtocolRequest::TurnSend { params, .. } = request else {
        panic!("expected turn.send request");
    };
    assert_eq!(params.client_request_id, "remote-turn-01");
    assert_eq!(params.capability_revision, "codex-session-42");
    let Some(ModelSelection::FlatModelSelection(model)) = params.selection.model else {
        panic!("expected flat model selection");
    };
    assert_eq!(model.model_id, "gpt-5");

    let response = decode_response(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/turn-send-response.json"
    ))
    .unwrap();
    let ProtocolResponse::TurnSend {
        response: ResponsePayload::Ok { result },
        ..
    } = response
    else {
        panic!("expected successful turn.send response");
    };
    assert!(result.accepted);
    assert_eq!(result.user_item.role, Some(ConversationItemRole::User));
    assert_eq!(result.user_item.turn, result.turn.resource);
    assert_eq!(result.user_item.conversation, result.turn.conversation);
    let Some(ModelSelection::FlatModelSelection(model)) = result.effective_selection.model else {
        panic!("expected effective flat model selection");
    };
    assert_eq!(model.model_id, "gpt-5");
}

#[test]
fn model_catalog_fixtures_cover_both_discriminated_shapes() {
    let flat: ModelCatalog = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/model-catalog-flat.json"
    ))
    .unwrap();
    let ModelCatalog::FlatModelCatalog(flat) = flat else {
        panic!("expected flat catalog");
    };
    assert_eq!(flat.models[0].id, "gpt-5");

    let grouped: ModelCatalog = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/model-catalog-grouped.json"
    ))
    .unwrap();
    let ModelCatalog::GroupedModelCatalog(grouped) = grouped else {
        panic!("expected grouped catalog");
    };
    assert_eq!(grouped.providers[1].id, "anthropic");
    assert_eq!(
        grouped.default_selection.as_ref().unwrap().provider_id,
        "openai"
    );

    assert!(serde_json::from_value::<ModelSelection>(serde_json::json!({
        "modelId": "gpt-5"
    }))
    .is_err());
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
    assert_eq!(request.device.device_name, "Alice's Pixel");
    assert_eq!(request.device.operating_system, "Android");
    assert_eq!(request.device.system_version, "16");

    let response: PairingExchangeResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/pairing-exchange-response.json"
    ))
    .unwrap();
    assert_eq!(response.device.device_id, qr.host_device_id);
    assert_eq!(response.device.descriptor.device_name, qr.display_name);
    assert_eq!(response.device.descriptor.operating_system, "macOS");
    assert_eq!(response.device.descriptor.system_version, "15.6");
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
fn gateway_conversation_search_has_a_typed_capability() {
    assert_eq!(
        ProtocolMethod::ConversationSearch.capability(),
        Some(GatewayCapability::ConversationSearch)
    );
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
    assert_eq!(result.items.len(), 5);
    assert!(result.items.iter().all(|item| {
        item.conversation == result.conversation.resource
    }));
    assert_eq!(result.items[0].kind, ConversationItemKind::Message);
    assert_eq!(result.items[0].role, Some(ConversationItemRole::User));
    assert_eq!(
        result.items[0].contents[0].content_id,
        "message-user-01:input:0"
    );
    assert_eq!(
        result.items[1].contents[0].kind,
        ConversationContentKind::ReasoningSummary
    );
    assert_eq!(
        result.items[3]
            .related_item
            .as_ref()
            .unwrap()
            .native_resource_id,
        "command-01"
    );
    assert!(result.items[3].approval.is_some());

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
