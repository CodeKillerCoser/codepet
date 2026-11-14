use super::*;
use codepet_gateway_sdk as g;
use serde_json::{json, Value};

async fn fixture() -> (
    tempfile::TempDir,
    Arc<PluginManager>,
    Arc<ProviderGatewayService>,
) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("app-state.sqlite");
    let mut descriptor = plugin("dev.codepet.queue", &["queue-instance"]);
    descriptor
        .env
        .insert("CODEPET_FAKE_QUEUE".into(), "1".into());
    descriptor.env.insert(
        "CODEPET_FAKE_TURN_START_MARKER".into(),
        temp.path().join("starts").to_string_lossy().into_owned(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_TURN_STEER_MARKER".into(),
        temp.path().join("steers").to_string_lossy().into_owned(),
    );
    let manager = build_manager_at(
        "device-queue",
        vec![descriptor],
        256,
        PluginProcessOptions::default(),
        Some(&path),
    );
    let gateway = Arc::new(
        ProviderGatewayService::with_remote_identity_and_state_path(
            manager.clone(),
            RemoteHostIdentity {
                device_id: "device-queue".into(),
                descriptor: DeviceDescriptor {
                    device_name: "Queue".into(),
                    operating_system: "test".into(),
                    system_version: "1".into(),
                },
            },
            &path,
        )
        .unwrap(),
    );
    gateway.start_event_forwarding();
    assert!(manager.start_enabled().await[0].1.is_ok());
    (temp, manager, gateway)
}
async fn rpc(
    gateway: &ProviderGatewayService,
    caller: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let response = gateway
        .dispatch_for_caller_scope(
            caller,
            serde_json::from_value(
                json!({"jsonrpc":"2.0","id":"test","method":method,"params":params}),
            )
            .unwrap(),
        )
        .await;
    match response.response {
        g::JsonRpcResponsePayload::Ok { result } => Ok(result),
        g::JsonRpcResponsePayload::Error { error } => {
            Err(error.data.unwrap()["code"].as_str().unwrap().into())
        }
    }
}
fn route(id: &str) -> Value {
    json!({"providerId":"queue-instance","nativeResourceId":id})
}
fn input(id: &str, request: &str) -> Value {
    json!({"conversation":route(id),"clientRequestId":request,"capabilityRevision":"fake-capabilities-v1","input":{"kind":"text","text":request},"selection":{"accessModeId":"workspace-write","reasoningEffortId":"medium","model":{"kind":"flat","modelId":"fake-model"}}})
}
#[tokio::test]
async fn queue_dispatches_fifo_and_durable_retries_do_not_send_again() {
    let (temp, manager, gateway) = fixture().await;
    let caps = rpc(
        &gateway,
        "phone",
        "provider.describe",
        json!({"providerId":"queue-instance"}),
    )
    .await
    .unwrap();
    assert!(caps["capabilities"]["methods"]
        .as_array()
        .unwrap()
        .contains(&json!("conversation.inputQueue.enqueue")));
    let first = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-idle", "first"),
    )
    .await
    .unwrap();
    let second = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-idle", "second"),
    )
    .await
    .unwrap();
    let expected = format!(
        "queue:{}\nqueue:{}\n",
        first["item"]["id"].as_str().unwrap(),
        second["item"]["id"].as_str().unwrap()
    );
    let starts = async {
        loop {
            if std::fs::read_to_string(temp.path().join("starts"))
                .unwrap_or_default()
                .lines()
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), starts)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(temp.path().join("starts")).unwrap(),
        expected
    );
    let duplicate = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-idle", "first"),
    )
    .await
    .unwrap();
    assert_eq!(duplicate["item"]["state"], "accepted");
    assert_eq!(
        std::fs::read_to_string(temp.path().join("starts")).unwrap(),
        expected
    );
    manager.shutdown().await;
}
#[tokio::test]
async fn queue_edit_delete_and_steer_share_one_authority_and_keep_callers_isolated() {
    let (temp, manager, gateway) = fixture().await;
    let first = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-active", "first"),
    )
    .await
    .unwrap();
    let second = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-active", "second"),
    )
    .await
    .unwrap();
    assert_eq!(
        rpc(
            &gateway,
            "phone",
            "turn.send",
            input("queue-active", "direct")
        )
        .await
        .unwrap_err(),
        "input_queue_pending"
    );
    let edit = json!({"conversation":route("queue-active"),"itemId":first["item"]["id"],"clientRequestId":"edit","expectedRevision":second["revision"],"kind":"edit"});
    let edited = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.withdraw",
        edit.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        rpc(&gateway, "phone", "conversation.inputQueue.withdraw", edit)
            .await
            .unwrap(),
        edited
    );
    let snapshot = rpc(
        &gateway,
        "desktop",
        "conversation.inputQueue.get",
        json!({"conversation":route("queue-active")}),
    )
    .await
    .unwrap();
    assert!(snapshot["recoverableEdits"].as_array().unwrap().is_empty());
    let mut steer = json!({"conversation":route("queue-active"),"itemId":second["item"]["id"],"clientRequestId":"steer","expectedRevision":snapshot["revision"],"turn":route("active-turn"),"capabilityRevision":"fake-capabilities-v1"});
    let mut stale = steer.clone();
    stale["turn"] = route("old-turn");
    assert_eq!(
        rpc(&gateway, "phone", "conversation.inputQueue.steer", stale)
            .await
            .unwrap_err(),
        "stale_active_turn"
    );
    let paused = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.get",
        json!({"conversation": route("queue-active")}),
    )
    .await
    .unwrap();
    assert_eq!(paused["paused"], true);
    steer["expectedRevision"] = paused["revision"].clone();
    let accepted = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.steer",
        steer.clone(),
    )
    .await
    .unwrap();
    assert_eq!(accepted["item"]["state"], "accepted");
    rpc(&gateway, "phone", "conversation.inputQueue.steer", steer)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(temp.path().join("steers")).unwrap(),
        format!("queue:{}\n", second["item"]["id"].as_str().unwrap())
    );
    let third = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-active", "third"),
    )
    .await
    .unwrap();
    rpc(&gateway,"phone","conversation.inputQueue.withdraw",json!({"conversation":route("queue-active"),"itemId":third["item"]["id"],"clientRequestId":"delete","expectedRevision":third["revision"],"kind":"delete"})).await.unwrap();
    let snapshot = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.get",
        json!({"conversation":route("queue-active")}),
    )
    .await
    .unwrap();
    assert!(snapshot["items"].as_array().unwrap().is_empty());
    assert_eq!(snapshot["recoverableEdits"][0]["input"]["text"], "first");
    manager.shutdown().await;
}
#[tokio::test]
async fn interrupt_pauses_pending_messages_and_direct_steer_checks_target() {
    let (_temp, manager, gateway) = fixture().await;
    rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-active", "pending"),
    )
    .await
    .unwrap();
    rpc(
        &gateway,
        "phone",
        "turn.interrupt",
        json!({"conversation":route("queue-active"),"turn":route("active-turn")}),
    )
    .await
    .unwrap();
    let queue = rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.get",
        json!({"conversation":route("queue-active")}),
    )
    .await
    .unwrap();
    assert_eq!(queue["paused"], true);
    assert_eq!(queue["items"][0]["state"], "queued");
    let request = json!({"conversation":route("other-idle"),"turn":route("wrong-turn"),"clientRequestId":"direct-steer","capabilityRevision":"fake-capabilities-v1","input":{"kind":"text","text":"steer"}});
    assert_eq!(
        rpc(&gateway, "phone", "turn.steer", request)
            .await
            .unwrap_err(),
        "stale_active_turn"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn stopping_provider_pauses_queue_even_when_no_terminal_turn_event_arrives() {
    let (_temp, manager, gateway) = fixture().await;
    rpc(
        &gateway,
        "phone",
        "conversation.inputQueue.enqueue",
        input("queue-active", "pending"),
    )
    .await
    .unwrap();
    manager
        .stop_instance(&codepet_provider_sdk::ProviderInstanceRoute {
            device_id: "device-queue".into(),
            provider_plugin_id: "dev.codepet.queue".into(),
            provider_instance_id: "queue-instance".into(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let queue = rpc(
                &gateway,
                "phone",
                "conversation.inputQueue.get",
                json!({"conversation":route("queue-active")}),
            )
            .await
            .unwrap();
            if queue["paused"] == true {
                assert_eq!(queue["items"][0]["state"], "queued");
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    manager.shutdown().await;
}

#[tokio::test]
async fn unknown_conversation_state_is_never_used_as_idle_or_execution_authority() {
    let (temp, manager, gateway) = fixture().await;
    let error = rpc(&gateway,"phone","conversation.inputQueue.enqueue",input("future-state","must-not-run")).await.unwrap_err();
    assert_eq!(error,"conversation_state_unknown");
    assert!(!temp.path().join("starts").exists());
    manager.shutdown().await;
}
