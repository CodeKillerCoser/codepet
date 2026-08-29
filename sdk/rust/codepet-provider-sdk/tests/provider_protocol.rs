use codepet_provider_sdk::{
    decode_event, decode_request, dispatch, JsonRpcResponsePayload, ProtocolEvent, ProtocolFuture,
    ProtocolRequest, ProtocolServer, ProviderInitializeRequest, ProviderInitializeResponse,
    ProviderPluginDescriptor, VersionRange,
};

struct InitializeServer;

impl ProtocolServer for InitializeServer {
    fn provider_initialize<'a>(
        &'a self,
        _request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async {
            Ok(ProviderInitializeResponse {
                selected_version: 1,
                plugin: ProviderPluginDescriptor {
                    plugin_id: "dev.codepet.codex".to_string(),
                    display_name: "Codex".to_string(),
                    version: "1.0.0".to_string(),
                    supported_versions: VersionRange {
                        min_version: 1,
                        max_version: 1,
                    },
                    instance_kinds: vec!["codex".to_string()],
                },
            })
        })
    }
}

#[tokio::test]
async fn json_rpc_dispatcher_routes_initialize_to_the_async_server_trait() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/initialize-request.json"
    ))
    .unwrap();
    let response = dispatch(&InitializeServer, request).await;

    assert_eq!(response.jsonrpc, "2.0");
    assert_eq!(response.id, "provider-init-1");
    let JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected successful result");
    };
    assert_eq!(result["selectedVersion"], 1);
}

#[test]
fn provider_event_preserves_device_instance_and_native_resource_route() {
    let event = decode_event(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/conversation-upserted-event.json"
    ))
    .unwrap();
    let ProtocolEvent::EventConversationUpserted { params, .. } = event else {
        panic!("expected conversation event");
    };
    assert_eq!(params.conversation.resource.device_id, "device-macbook-1");
    assert_eq!(params.conversation.resource.provider_instance_id, "codex-work");
    assert_eq!(params.conversation.resource.native_resource_id, "thread-01");
}

#[test]
fn provider_initialize_fixture_uses_json_rpc_stdio_envelope() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/initialize-request.json"
    ))
    .unwrap();
    assert!(matches!(request, ProtocolRequest::ProviderInitialize { .. }));
}
