use codepet_provider_sdk::{
    decode_event, decode_request, decode_response, dispatch, JsonRpcResponsePayload, ProtocolEvent,
    InstanceCreateRequest, InstanceCreateResponse, ProtocolFuture, ProtocolMethod, ProtocolRequest,
    ProtocolServer, ProviderCapability, ProviderInitializeRequest, ProviderInitializeResponse,
    ProviderPluginDescriptor, VersionRange,
};

struct InitializeServer;

fn descriptor() -> ProviderPluginDescriptor {
    ProviderPluginDescriptor {
        plugin_id: "dev.codepet.codex".to_string(),
        display_name: "Codex".to_string(),
        version: "1.0.0".to_string(),
        supported_versions: VersionRange {
            min_version: 1,
            max_version: 1,
        },
        instance_kinds: vec!["codex".to_string()],
    }
}

impl ProtocolServer for InitializeServer {
    fn provider_initialize<'a>(
        &'a self,
        _request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async {
            Ok(ProviderInitializeResponse {
                selected_version: 1,
                plugin: descriptor(),
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
    assert_eq!(response.id.as_deref(), Some("provider-init-1"));
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

#[test]
fn instance_kind_is_required_across_descriptor_request_and_instance_response() {
    let descriptor = descriptor();
    descriptor.validate_instance_kind("codex").unwrap();
    let error = descriptor.validate_instance_kind("qoder").unwrap_err();
    assert_eq!(error.code, "unsupported_instance_kind");
    let mut empty_descriptor = descriptor.clone();
    empty_descriptor.instance_kinds.clear();
    assert_eq!(
        empty_descriptor.validate_instance_kinds().unwrap_err().code,
        "invalid_provider_descriptor"
    );
    let mut blank_kind_descriptor = descriptor.clone();
    blank_kind_descriptor.instance_kinds = vec![String::new()];
    assert_eq!(
        blank_kind_descriptor
            .validate_instance_kinds()
            .unwrap_err()
            .code,
        "invalid_provider_descriptor"
    );

    let request = decode_request(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/instance-create-request.json"
    ))
    .unwrap();
    let ProtocolRequest::InstanceCreate { params, .. } = request else {
        panic!("expected instance.create request");
    };
    assert_eq!(params.instance_kind, "codex");

    let response = decode_response(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/instance-create-response.json"
    ))
    .unwrap();
    let JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected instance.create result");
    };
    assert_eq!(result["instance"]["instanceKind"], "codex");
}

struct InstanceKindServer {
    descriptor: ProviderPluginDescriptor,
}

impl ProtocolServer for InstanceKindServer {
    fn instance_create<'a>(
        &'a self,
        request: InstanceCreateRequest,
    ) -> ProtocolFuture<'a, InstanceCreateResponse> {
        Box::pin(async move {
            self.descriptor
                .validate_instance_kind(&request.instance_kind)?;
            unreachable!("the test only exercises unsupported selection")
        })
    }
}

#[tokio::test]
async fn instance_create_server_can_fail_closed_on_unsupported_kind() {
    let request = decode_request(
        br#"{"jsonrpc":"2.0","id":"instance-kind-1","method":"instance.create","params":{"route":{"deviceId":"device-1","providerInstanceId":"instance-1"},"instanceKind":"qoder","displayName":"Qoder","settings":{}}}"#,
    )
    .unwrap();
    let response = dispatch(
        &InstanceKindServer {
            descriptor: descriptor(),
        },
        request,
    )
    .await;
    let JsonRpcResponsePayload::Error { error } = response.response else {
        panic!("expected unsupported instance kind error");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!("unsupported_instance_kind"))
    );
}

#[test]
fn generated_method_capability_mapping_is_typed() {
    assert_eq!(
        ProtocolMethod::ConversationList.capability(),
        Some(ProviderCapability::ConversationList)
    );
    assert_eq!(ProtocolMethod::ProviderInitialize.capability(), None);
}
