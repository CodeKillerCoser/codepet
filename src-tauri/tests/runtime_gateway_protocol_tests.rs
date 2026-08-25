use code_pet_lib::runtime_gateway::generated::{
    dispatch, ConversationCreateRequest, HandshakeRequest, HandshakeResponse, ProtocolEvent,
    ProtocolFuture, ProtocolRequest, ProtocolResponse, ProtocolServer, ResponsePayload,
};

fn assert_json_round_trip<T>(fixture: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let decoded: T = serde_json::from_value(expected.clone()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(encoded, expected);
}

#[test]
fn shared_wire_fixtures_round_trip_through_generated_serde_types() {
    assert_json_round_trip::<ProtocolRequest>(include_str!(
        "../../protocol/fixtures/handshake-request.json"
    ));
    assert_json_round_trip::<ProtocolResponse>(include_str!(
        "../../protocol/fixtures/handshake-response.json"
    ));
    assert_json_round_trip::<ProtocolResponse>(include_str!(
        "../../protocol/fixtures/turn-send-error.json"
    ));
    assert_json_round_trip::<ProtocolRequest>(include_str!(
        "../../protocol/fixtures/conversation-create-request.json"
    ));
    assert_json_round_trip::<ProtocolEvent>(include_str!(
        "../../protocol/fixtures/conversation-upserted-event.json"
    ));
    assert_json_round_trip::<ProtocolEvent>(include_str!(
        "../../protocol/fixtures/turn-output-delta-event.json"
    ));
    assert_json_round_trip::<ProtocolEvent>(include_str!(
        "../../protocol/fixtures/approval-requested-event.json"
    ));
}

#[test]
fn workspace_root_distinguishes_project_conversations_from_plain_chat() {
    let project_request: ProtocolRequest = serde_json::from_str(include_str!(
        "../../protocol/fixtures/conversation-create-request.json"
    ))
    .unwrap();
    let workspace_root = match project_request {
        ProtocolRequest::ConversationCreate { params, .. } => params.workspace_root,
        _ => panic!("expected conversation.create fixture"),
    };
    assert_eq!(workspace_root.as_deref(), Some("/workspace/code-pet"));

    let plain_chat: ConversationCreateRequest = serde_json::from_str(
        r#"{"providerId":"codex-local","permissionLevel":"read-only"}"#,
    )
    .unwrap();
    assert!(plain_chat.workspace_root.is_none());
}

struct HandshakeServer {
    response: HandshakeResponse,
}

impl ProtocolServer for HandshakeServer {
    fn protocol_handshake<'a>(
        &'a self,
        _request: HandshakeRequest,
    ) -> ProtocolFuture<'a, HandshakeResponse> {
        let response = self.response.clone();
        Box::pin(async move { Ok(response) })
    }
}

#[tokio::test]
async fn generated_dispatcher_routes_to_the_typed_server_method() {
    let request: ProtocolRequest = serde_json::from_str(include_str!(
        "../../protocol/fixtures/handshake-request.json"
    ))
    .unwrap();
    let expected: ProtocolResponse = serde_json::from_str(include_str!(
        "../../protocol/fixtures/handshake-response.json"
    ))
    .unwrap();
    let response = match expected.clone() {
        ProtocolResponse::ProtocolHandshake {
            response: ResponsePayload::Ok { result },
            ..
        } => result,
        _ => panic!("expected successful handshake fixture"),
    };

    let actual = dispatch(&HandshakeServer { response }, request).await;

    assert_eq!(actual, expected);
}
