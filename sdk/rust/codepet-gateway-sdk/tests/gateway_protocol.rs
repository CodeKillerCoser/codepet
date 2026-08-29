use codepet_gateway_sdk::{decode_event, decode_request, ProtocolEvent, ProtocolRequest};

#[test]
fn gateway_request_routes_by_device_instance_and_native_resource() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/gateway/v1/fixtures/conversation-get-request.json"
    ))
    .unwrap();
    let ProtocolRequest::ConversationGet { params, .. } = request else {
        panic!("expected conversation.get");
    };
    assert_eq!(params.conversation.device_id, "device-macbook-1");
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
    assert_eq!(payload.conversation.resource.provider_instance_id, "codex-work");
}
