use super::*;

#[test]
fn session_keeps_reading_notifications_between_requests() {
    let (session, _, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
                "method": "turn/started",
                "params": {
                "threadId": "thread-one",
                "turn": turn_fixture("turn-one", "inProgress")
            }
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::Notification(CodexNotification::TurnStarted { thread_id, .. })
            if thread_id == "thread-one"
    ));
    session.shutdown().unwrap();
}

#[test]
fn session_parses_thread_name_updated_notifications() {
    let (session, _, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
            "method": "thread/name/updated",
            "params": {
                "threadId": "thread-one",
                "threadName": "A useful title"
            }
        }))
        .unwrap();

    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::Notification(CodexNotification::ThreadNameUpdated {
            thread_id,
            thread_name: Some(thread_name),
        }) if thread_id == "thread-one" && thread_name == "A useful title"
    ));
    session.shutdown().unwrap();
}

#[test]
fn session_parses_thread_status_changed_notifications() {
    let (session, _, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
            "method": "thread/status/changed",
            "params": {
                "threadId": "thread-one",
                "status": {
                    "type": "active",
                    "activeFlags": ["waitingOnApproval"]
                }
            }
        }))
        .unwrap();

    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::Notification(CodexNotification::ThreadStatusChanged {
            thread_id,
            status: CodexThreadStatus::Active { active_flags },
        }) if thread_id == "thread-one"
            && active_flags == vec![CodexThreadActiveFlag::WaitingOnApproval]
    ));
    session.shutdown().unwrap();
}

#[test]
fn official_notification_metadata_and_missing_params_are_accepted() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
            "method": "remoteControl/status/changed",
            "emittedAtMs": 1234
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::Notification(CodexNotification::Unknown { method })
            if method == "remoteControl/status/changed"
    ));

    peer_sender
        .send(json!({
            "id": "future-request",
            "method": "future/request",
            "trace": { "traceparent": null, "tracestate": null }
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::UnsupportedServerRequest { request_id, method }
            if request_id == JsonRpcId::String("future-request".to_string())
                && method == "future/request"
    ));
    let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["id"], "future-request");
    assert_eq!(response["error"]["code"], -32601);
    assert!(response.get("jsonrpc").is_none());
    assert!(session.is_running());
    session.shutdown().unwrap();
}

#[test]
fn empty_streaming_agent_message_text_does_not_fault_the_reader() {
    let (session, _, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "item": {
                    "type": "agentMessage",
                    "id": "agent-one",
                    "text": ""
                }
            }
        }))
        .unwrap();

    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::Notification(CodexNotification::ItemUpserted {
            item: CodexThreadItem::AgentMessage { id, text },
            ..
        }) if id == "agent-one" && text.is_empty()
    ));
    assert!(session.is_running());
    session.shutdown().unwrap();
}

#[test]
fn output_delta_uses_channel_specific_content_ids_and_ignores_raw_reasoning() {
    let summary = parse_notification(
        "item/reasoning/summaryTextDelta",
        json!({
            "threadId": "thread-one",
            "turnId": "turn-one",
            "itemId": "reasoning-one",
            "summaryIndex": 2,
            "delta": "summary"
        }),
        "generation-one",
    )
    .unwrap();
    assert!(matches!(
        summary,
        CodexNotification::OutputDelta {
            item_id,
            content_id,
            kind: CodexContentKind::ReasoningSummary,
            ..
        } if item_id == "reasoning-one" && content_id == "reasoning-one:summary:2"
    ));

    let raw = parse_notification(
        "item/reasoning/textDelta",
        json!({
            "threadId": "thread-one",
            "turnId": "turn-one",
            "itemId": "reasoning-one",
            "contentIndex": 0,
            "delta": "private raw reasoning"
        }),
        "generation-one",
    )
    .unwrap();
    assert!(matches!(
        raw,
        CodexNotification::Unknown { method }
            if method == "item/reasoning/textDelta"
    ));
}

#[test]
fn invalid_notification_metadata_does_not_stop_other_notifications() {
    let (session, _peer_receiver, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({ "jsonrpc": "1.0", "method": "remoteControl/status/changed" }))
        .unwrap();
    peer_sender
        .send(json!({ "method": "thread/name/updated", "params": {
            "threadId": "thread-other", "threadName": "Still receiving",
        } }))
        .unwrap();
    assert!(
        matches!(notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(),
            CodexIncoming::Notification(CodexNotification::ThreadNameUpdated { thread_id, .. }) if thread_id == "thread-other")
    );
    assert!(session.is_running());
    session.shutdown().unwrap();
}

#[test]
fn command_approval_kind_null_is_unsupported_but_missing_kind_defaults_to_command() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
            "id": "kind-null",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "item-null",
                "startedAtMs": 123,
                "kind": null
            }
        }))
        .unwrap();
    assert!(matches!(
        notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(),
        CodexIncoming::UnsupportedServerRequest {
            request_id: JsonRpcId::String(request_id),
            method
        } if request_id == "kind-null"
            && method == "item/commandExecution/requestApproval"
    ));
    let rejected = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(rejected["id"], "kind-null");
    assert_eq!(rejected["error"]["code"], -32601);

    peer_sender
        .send(json!({
            "id": "kind-missing",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "item-command",
                "startedAtMs": 124
            }
        }))
        .unwrap();
    let accepted = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        accepted,
        CodexIncoming::ApprovalRequested(CodexApprovalRequest {
            kind: CodexApprovalKind::CommandExecution,
            ..
        })
    ));
    session.shutdown().unwrap();
}

#[test]
fn approval_and_unsupported_server_requests_receive_terminal_responses() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let notifications = session.subscribe().unwrap();
    peer_sender
        .send(json!({
            "id": "approval-one",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "item-one",
                "startedAtMs": 123,
                "command": "cargo test",
                "availableDecisions": ["accept", "acceptForSession", "decline", "cancel"],
                "futureField": { "mustNotLeak": true }
            }
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::UnsupportedServerRequest {
            request_id: JsonRpcId::String(request_id),
            method
        } if request_id == "approval-one"
            && method == "item/commandExecution/requestApproval"
    ));
    let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["id"], "approval-one");
    assert_eq!(response["error"]["code"], -32601);

    peer_sender
        .send(json!({
            "id": "approval-safe",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "item-one",
                "startedAtMs": 123,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    let CodexIncoming::ApprovalRequested(approval) = incoming else {
        panic!("expected safe binary approval request");
    };
    assert_eq!(approval.kind, CodexApprovalKind::CommandExecution);
    assert_eq!(approval.description.as_deref(), Some("cargo test"));
    assert_eq!(approval.session_generation, session.generation());
    session
        .respond_to_approval(&approval, ApprovalDecision::Approve)
        .unwrap();
    let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["id"], "approval-safe");
    assert_eq!(response["result"]["decision"], "accept");
    assert!(response.get("method").is_none());

    peer_sender
        .send(json!({
            "id": 2,
            "method": "item/permissions/requestApproval",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "item-two",
                "environmentId": null,
                "startedAtMs": 124,
                "cwd": "/tmp/project",
                "reason": "Allow network access",
                "permissions": {
                    "network": { "enabled": true },
                    "fileSystem": null
                }
            }
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::UnsupportedServerRequest { request_id: JsonRpcId::Number(2), method }
            if method == "item/permissions/requestApproval"
    ));
    let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["id"], 2);
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(
        response["error"]["data"]["reason"],
        "unsupported_client_capability"
    );
    assert!(response.get("result").is_none());

    peer_sender
        .send(json!({
            "id": "unknown-one",
            "method": "item/future/request",
            "params": {}
        }))
        .unwrap();
    let incoming = notifications
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert!(matches!(
        incoming,
        CodexIncoming::UnsupportedServerRequest {
            request_id: JsonRpcId::String(request_id),
            method
        } if request_id == "unknown-one" && method == "item/future/request"
    ));
    let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["id"], "unknown-one");
    assert_eq!(response["error"]["code"], -32601);
    assert!(session.is_running());
    session.shutdown().unwrap();
}
