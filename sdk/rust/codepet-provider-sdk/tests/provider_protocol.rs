use codepet_provider_sdk::{
    decode_event, decode_request, decode_response, dispatch, ContentBlock, ConversationContentKind,
    ConversationGetResponse, ConversationItem, ConversationItemStatus, InstanceCreateRequest,
    InstanceCreateResponse, JsonRpcResponsePayload, ProtocolEvent, ProtocolFuture, ProtocolMethod,
    ModelSelection, ProtocolRequest, ProtocolServer, ProviderCapability, ProviderInitializeRequest,
    ProviderInitializeResponse, ProviderPluginDescriptor, TurnStartResponse, VersionRange,
    StdioServerOptions, ToolInput, ToolOutcome, serve_stdio_with_io,
};
use std::io::Cursor;

struct InitializeServer;

fn descriptor() -> ProviderPluginDescriptor {
    ProviderPluginDescriptor {
        plugin_id: "dev.codepet.codex".to_string(),
        display_name: "Codex".to_string(),
        version: "1.0.0".to_string(),
        default_workspace_root: Some("/Users/example/.codex".to_string()),
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

#[tokio::test]
async fn stdio_runtime_rejects_zero_capacity_before_starting_the_provider() {
    let options = StdioServerOptions {
        max_pending_requests: 0,
        ..StdioServerOptions::default()
    };
    let result = serve_stdio_with_io(
        Cursor::new(Vec::<u8>::new()),
        Vec::<u8>::new(),
        options,
        |_| InitializeServer,
    )
    .await;
    assert!(result
        .unwrap_err()
        .message()
        .contains("max_pending_requests"));
}

#[test]
fn provider_event_uses_canonical_agent_resource_identity() {
    let event = decode_event(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/conversation-upserted-event.json"
    ))
    .unwrap();
    let ProtocolEvent::EventConversationUpserted { params, .. } = event else {
        panic!("expected conversation event");
    };
    assert_eq!(params.conversation.resource.provider_id, "codex-work");
    assert_eq!(params.conversation.resource.native_resource_id, "thread-01");
}

#[test]
fn conversation_get_fixture_preserves_ordered_items_and_stable_content_ids() {
    let response = decode_response(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/conversation-get-response.json"
    ))
    .unwrap();
    let JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected conversation.get result");
    };
    let response: ConversationGetResponse = serde_json::from_value(result).unwrap();

    let ConversationItem::MessageConversationItem(message) = &response.items[0] else {
        panic!("expected message item");
    };
    assert_eq!(message.resource.native_resource_id, "user-one");
    assert_eq!(message.conversation, response.conversation.resource);
    let ContentBlock::TextContentBlock(text) = &message.contents[0] else {
        panic!("expected text content");
    };
    assert_eq!(text.content_id, "user-one:input:0");

    let ConversationItem::ReasoningConversationItem(reasoning) = &response.items[1] else {
        panic!("expected reasoning item");
    };
    assert_eq!(reasoning.conversation, response.conversation.resource);
    assert!(matches!(reasoning.contents[0], ContentBlock::ReasoningSummaryContentBlock(_)));

    let ConversationItem::CommandConversationItem(command) = &response.items[2] else {
        panic!("expected command item");
    };
    assert_eq!(command.resource.native_resource_id, "command-one");
    let ToolInput::CommandToolInput(input) = &command.tool.input else {
        panic!("expected command input");
    };
    let input_truncation = input.truncation.as_ref().expect("expected command truncation metadata");
    assert_eq!(input_truncation.original_bytes, 4096);
    assert_eq!(input_truncation.retained_bytes, 10);
    let Some(ToolOutcome::ToolSuccessOutcome(outcome)) = &command.tool.outcome else {
        panic!("expected success outcome");
    };
    assert_eq!(outcome.content.len(), 2);
    let ContentBlock::OutputContentBlock(output) = &outcome.content[0] else {
        panic!("expected output content");
    };
    let truncation = output.truncation.as_ref().expect("expected truncation metadata");
    assert_eq!(truncation.original_bytes, 12000);
    assert_eq!(truncation.retained_bytes, 12);
    assert!(matches!(outcome.content[1], ContentBlock::StructuredJsonContentBlock(_)));

    let ConversationItem::ApprovalConversationItem(approval) = &response.items[3] else {
        panic!("expected approval item");
    };
    assert_eq!(approval.status, ConversationItemStatus::Approved);
    assert_eq!(
        approval.related_item
            .as_ref()
            .unwrap()
            .native_resource_id,
        "command-one"
    );
    assert!(matches!(response.items[4], ConversationItem::UnknownConversationItem(_)));
}

#[test]
fn generated_unions_reject_mixed_authority_fixtures() {
    assert!(serde_json::from_slice::<ToolInput>(include_bytes!(
        "../../../../protocol/agent/v1/fixtures/invalid-tool-input-mixed.json"
    ))
    .is_err());
    assert!(serde_json::from_slice::<ToolOutcome>(include_bytes!(
        "../../../../protocol/agent/v1/fixtures/invalid-tool-outcome-mixed.json"
    ))
    .is_err());
    assert!(serde_json::from_slice::<ConversationItem>(include_bytes!(
        "../../../../protocol/agent/v1/fixtures/invalid-command-item-duplicate-content.json"
    ))
    .is_err());
}

#[test]
fn turn_output_delta_fixture_addresses_both_item_and_content() {
    let event = decode_event(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/turn-output-delta-event.json"
    ))
    .unwrap();
    let ProtocolEvent::EventTurnOutputDelta { params, .. } = event else {
        panic!("expected turn output delta");
    };
    assert_eq!(params.item_id, "agent-one");
    assert_eq!(params.content_id, "agent-one:text");
    assert_eq!(params.kind, ConversationContentKind::Text);
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
    assert_eq!(result["instance"]["harness"]["id"], "codex");
    assert_eq!(result["instance"]["capabilities"]["revision"], "codex-catalog-1");
}

#[test]
fn turn_start_fixture_preserves_selection_and_allows_deferred_user_item() {
    let request = decode_request(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/turn-start-request.json"
    ))
    .unwrap();
    let ProtocolRequest::TurnStart { params, .. } = request else {
        panic!("expected turn.start request");
    };
    assert_eq!(params.client_request_id, "remote-turn-01");
    assert_eq!(params.capability_revision, "codex-session-42");
    let Some(ModelSelection::FlatModelSelection(model)) = params.selection.model else {
        panic!("expected flat model selection");
    };
    assert_eq!(model.model_id, "gpt-5");

    let response = decode_response(include_bytes!(
        "../../../../protocol/provider/v1/fixtures/turn-start-response.json"
    ))
    .unwrap();
    let JsonRpcResponsePayload::Ok { result } = response.response else {
        panic!("expected turn.start result");
    };
    let response: TurnStartResponse = serde_json::from_value(result).unwrap();
    assert!(response.accepted);
    assert!(response.user_item.is_none());
    assert_eq!(serde_json::to_value(&response).unwrap()["userItem"], serde_json::Value::Null);
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
        br#"{"jsonrpc":"2.0","id":"instance-kind-1","method":"instance.create","params":{"route":{"deviceId":"device-1","providerPluginId":"dev.codepet.codex","providerInstanceId":"instance-1"},"instanceKind":"qoder","displayName":"Qoder","settings":{}}}"#,
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
    assert_eq!(
        ProtocolMethod::ConversationSearch.capability(),
        Some(ProviderCapability::ConversationSearch)
    );
    assert_eq!(ProtocolMethod::ProviderInitialize.capability(), None);
}
