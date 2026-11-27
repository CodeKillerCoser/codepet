use super::*;

#[tokio::test]
async fn gateway_turn_send_validates_controls_and_deduplicates_client_requests() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("turn-starts.txt");
    let mut descriptor = plugin("dev.codepet.turn-send", &["instance-turn-send"]);
    descriptor.env.insert(
        "CODEPET_FAKE_TURN_START_MARKER".to_string(),
        marker.to_string_lossy().to_string(),
    );
    let manager = build_manager("device-turn-send", vec![descriptor]);
    let gateway = ProviderGatewayService::new(manager.clone()).unwrap();
    assert!(manager.start_enabled().await[0].1.is_ok());

    let provider = gateway
        .provider_list(ProviderListRequest {})
        .await
        .unwrap()
        .providers
        .remove(0);
    assert_eq!(provider.runtime.version.as_deref(), Some("1.0.0-fixture"));
    assert_eq!(provider.runtime.executable_path.as_deref(), Some("/fixture/fake-harness"));
    assert_eq!(
        provider.runtime.authentication.as_ref().unwrap().status,
        codepet_gateway_sdk::ProviderAuthenticationStatus::SignedIn
    );
    let usage = provider.runtime.usage.as_ref().unwrap();
    assert_eq!(usage.display_text, "Fixture usage 42%");
    let details = usage.details.as_ref().unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].data.get("usedPercent"), Some(&serde_json::json!(42)));
    assert!(!details[0].data.contains_key("accessToken"));
    assert_eq!(details[0].data["nested"], serde_json::json!({"safe": true}));
    assert_eq!(provider.capabilities.revision, "fake-capabilities-v1");
    let described = gateway
        .provider_describe(ProviderDescribeRequest { provider_id: provider.id.clone() })
        .await
        .unwrap();
    let Some(turn_send) = described.capabilities.turn_send.as_ref() else {
        panic!("expected turn.send controls");
    };
    assert!(matches!(turn_send.model_catalog, Some(ModelCatalog::FlatModelCatalog(_))));

    let conversation = gateway_resource(
        "device-turn-send",
        "dev.codepet.turn-send",
        "instance-turn-send",
        "conversation-turn-send",
    );
    let request = GatewayTurnSendRequest {
        conversation: conversation.clone(),
        client_request_id: "remote-request-1".to_string(),
        capability_revision: "fake-capabilities-v1".to_string(),
        input: GatewayTurnInput {
            kind: GatewayTurnInputKind::Text,
            text: "hello".to_string(),
        },
        selection: GatewayTurnSelection {
            access_mode_id: Some("workspace-write".to_string()),
            reasoning_effort_id: Some("medium".to_string()),
            model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                kind: FlatModelCatalogKind::Flat,
                model_id: "fake-model".to_string(),
            })),
        },
    };
    let accepted = gateway
        .turn_send_for_caller_scope("remote-client-a", request.clone())
        .await
        .unwrap();
    assert!(accepted.accepted);
    let user_item = accepted.user_item.as_ref().unwrap();
    let codepet_gateway_sdk::ConversationItem::MessageConversationItem(user_item) = user_item else {
        panic!("expected canonical user message");
    };
    assert_eq!(user_item.turn, accepted.turn.resource);
    assert_eq!(user_item.conversation, conversation);
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", request.clone())
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-b", request.clone())
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);

    let mut conflict = request.clone();
    conflict.input.text = "different".to_string();
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", conflict)
            .await
            .unwrap_err()
            .code,
        "client_request_conflict"
    );

    let mut stale = request.clone();
    stale.client_request_id = "remote-request-stale".to_string();
    stale.capability_revision = "stale".to_string();
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", stale)
            .await
            .unwrap_err()
            .code,
        "stale_capability_revision"
    );

    let mut unknown = request.clone();
    unknown.client_request_id = "remote-request-unknown".to_string();
    unknown.selection.model = Some(ModelSelection::FlatModelSelection(FlatModelSelection {
        kind: FlatModelCatalogKind::Flat,
        model_id: "missing-model".to_string(),
    }));
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", unknown)
            .await
            .unwrap_err()
            .code,
        "unknown_turn_selection"
    );

    let mut wrong_shape = request;
    wrong_shape.client_request_id = "remote-request-shape".to_string();
    wrong_shape.selection.model = Some(ModelSelection::GroupedModelSelection(
        GroupedModelSelection {
            kind: GroupedModelCatalogKind::Grouped,
            provider_id: "openai".to_string(),
            model_id: "fake-model".to_string(),
        },
    ));
    assert_eq!(
        gateway
            .turn_send_for_caller_scope("remote-client-a", wrong_shape)
            .await
            .unwrap_err()
            .code,
        "turn_model_shape_mismatch"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn resource_identity_and_unknown_provider_fail_closed() {
    let manager = build_manager(
        "device-identity",
        vec![plugin("dev.codepet.identity", &["instance-identity"])],
    );
    let gateway = Arc::new(ProviderGatewayService::new(manager.clone()).unwrap());
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());

    for native_id in [
        "response-wrong-native",
        "response-empty-native",
        "response-wrong-route",
    ] {
        let error = manager
            .conversation_get(ConversationGetRequest {
                conversation: resource(
                    "device-identity",
                    "dev.codepet.identity",
                    "instance-identity",
                    native_id,
                ),
                cursor: None,
                limit: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error.code.as_str(),
            "protocol_codec_error"
                | "provider_resource_identity_mismatch"
                | "invalid_provider_resource"
                | "provider_resource_route_mismatch"
        ));
    }
    let wrong_history_conversation = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-item-conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_history_conversation.code,
        "provider_resource_identity_mismatch"
    );
    let empty_native = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(empty_native.code, "invalid_provider_resource");
    let empty_device = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "",
                "dev.codepet.identity",
                "instance-identity",
                "conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(empty_device.code, "invalid_provider_route");
    let wrong_plugin = manager
        .conversation_get(ConversationGetRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.other",
                "instance-identity",
                "conversation",
            ),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_plugin.code, "provider_instance_plugin_mismatch");

    let wrong_conversation = manager
        .turn_start(TurnStartRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-conversation",
            ),
            client_request_id: "message-1".to_string(),
            capability_revision: "fake-capabilities-v1".to_string(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "hello".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_conversation.code, "provider_resource_identity_mismatch");

    let wrong_user_item_conversation = manager
        .turn_start(TurnStartRequest {
            conversation: resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-user-item-conversation",
            ),
            client_request_id: "message-wrong-user-item".to_string(),
            capability_revision: "fake-capabilities-v1".to_string(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "hello".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_user_item_conversation.code,
        "provider_resource_identity_mismatch"
    );

    let wrong_gateway_conversation = gateway
        .turn_send(GatewayTurnSendRequest {
            conversation: gateway_resource(
                "device-identity",
                "dev.codepet.identity",
                "instance-identity",
                "response-wrong-conversation",
            ),
            client_request_id: "message-gateway".to_string(),
            capability_revision: "fake-capabilities-v1".to_string(),
            input: GatewayTurnInput {
                kind: GatewayTurnInputKind::Text,
                text: "continue".to_string(),
            },
            selection: GatewayTurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(
        wrong_gateway_conversation.code,
        "provider_resource_identity_mismatch"
    );

    let unknown_provider = gateway
        .conversation_list(GatewayConversationListRequest {
            provider_id: "missing-provider".to_string(),
            cursor: Some("provider-cursor".to_string()),
            limit: Some(10),
            project_filter: all_project_filter(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        unknown_provider.code,
        "unknown_provider"
    );
    manager.shutdown().await;
}
