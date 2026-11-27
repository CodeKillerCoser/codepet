use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use codepet_gateway_sdk as gateway;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

#[path = "fixtures/terminal_host.rs"]
mod terminal_host;

async fn call(
    service: &codepet_host::ProviderGatewayService,
    owner: &str,
    method: &str,
    params: Value,
) -> gateway::JsonRpcResponse {
    let request = serde_json::from_value(json!({"jsonrpc":"2.0","id":"test",
        "method":method,"params":params}))
    .unwrap();
    service.dispatch_for_caller_scope(owner, request).await
}
fn result(response: gateway::JsonRpcResponse) -> Value {
    match response.response {
        gateway::JsonRpcResponsePayload::Ok { result } => result,
        error => panic!("{error:?}"),
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flooding_pty_does_not_block_heartbeat_or_another_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let service = terminal_host::start(directory.path());
    let noisy = result(
        call(
            &service,
            "phone",
            "terminal.open",
            json!({"sessionKey":"noisy","columns":80,"rows":24}),
        )
        .await,
    );
    let healthy = result(
        call(
            &service,
            "phone",
            "terminal.open",
            json!({"sessionKey":"healthy","columns":80,"rows":24}),
        )
        .await,
    );
    let noisy_id = noisy["session"]["terminalId"].as_str().unwrap();
    let healthy_id = healthy["session"]["terminalId"].as_str().unwrap();
    result(
        call(
            &service,
            "phone",
            "terminal.write",
            json!({
        "terminalId":noisy_id,"sequence":1,"data":BASE64.encode(b"yes codepet-terminal-output\r")}),
        )
        .await,
    );
    tokio::time::sleep(Duration::from_millis(80)).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        let (ping, inventory, healthy_write) = tokio::join!(
            call(&service, "phone", "protocol.ping", json!({"sequence":91})),
            call(&service, "phone", "provider.list", json!({})),
            call(
                &service,
                "phone",
                "terminal.write",
                json!({"terminalId":healthy_id,"sequence":1,
                "data":BASE64.encode(b"printf '\\033[32mHEALTHY-PTY\\033[0m\\n'\r")})
            ),
        );
        assert_eq!(result(ping)["sequence"], 91);
        result(inventory);
        result(healthy_write);
        let mut offset = 0;
        let mut all = Vec::new();
        loop {
            let response = result(
                call(
                    &service,
                    "phone",
                    "terminal.read",
                    json!({"terminalId":healthy_id,"offset":offset}),
                )
                .await,
            );
            offset = response["nextOffset"].as_u64().unwrap();
            let output = BASE64.decode(response["data"].as_str().unwrap()).unwrap();
            all.extend(output);
            if String::from_utf8_lossy(&all).contains("\x1b[32mHEALTHY-PTY\x1b[0m") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    for id in [noisy_id, healthy_id] {
        result(
            call(
                &service,
                "phone",
                "terminal.close",
                json!({"terminalId":id}),
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_rpc_is_scoped_and_does_not_require_a_healthy_provider() {
    let directory = tempfile::tempdir().unwrap();
    let service = terminal_host::start(directory.path());
    let description = result(call(&service, "phone-a", "protocol.describe", json!({})).await);
    assert!(description["features"]
        .as_array()
        .unwrap()
        .contains(&json!("terminal-pty-v1")));
    let opened = result(
        call(
            &service,
            "phone-a",
            "terminal.open",
            json!({"sessionKey":"chat-a","columns":80,"rows":24}),
        )
        .await,
    );
    let id = opened["session"]["terminalId"].as_str().unwrap();
    let denied = call(
        &service,
        "phone-b",
        "terminal.read",
        json!({"terminalId":id,"offset":0}),
    )
    .await;
    assert!(matches!(
        denied.response,
        gateway::JsonRpcResponsePayload::Error { .. }
    ));
    result(
        call(
            &service,
            "phone-b",
            "terminal.close",
            json!({"terminalId":id}),
        )
        .await,
    );
    result(
        call(
            &service,
            "phone-a",
            "terminal.read",
            json!({"terminalId":id,"offset":0}),
        )
        .await,
    );
    // A running shell is not awaited by the control plane or Provider inventory.
    let (ping, providers) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            call(&service, "phone-a", "protocol.ping", json!({"sequence":1})),
            call(&service, "phone-a", "provider.list", json!({}))
        )
    })
    .await
    .unwrap();
    assert_eq!(result(ping)["sequence"], 1);
    assert_eq!(result(providers)["providers"], json!([]));
    result(
        call(
            &service,
            "phone-a",
            "terminal.close",
            json!({"terminalId":id}),
        )
        .await,
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_terminal_rpc_preserves_escape_bytes_and_resize_while_shell_is_busy() {
    let directory = tempfile::tempdir().unwrap();
    let service = terminal_host::start(directory.path());
    let opened = result(
        call(
            &service,
            "phone",
            "terminal.open",
            json!({"sessionKey":"chat","columns":80,"rows":24}),
        )
        .await,
    );
    let id = opened["session"]["terminalId"].as_str().unwrap();
    result(
        call(
            &service,
            "phone",
            "terminal.resize",
            json!({"terminalId":id,"columns":91,"rows":33}),
        )
        .await,
    );
    let input =
        BASE64.encode("printf '\\033[32m终端同步\\033[0m\\n'; stty size; sleep 0.2; exit\r");
    result(
        call(
            &service,
            "phone",
            "terminal.write",
            json!({"terminalId":id,"sequence":1,"data":input}),
        )
        .await,
    );
    let healthy = Arc::clone(&service);
    let heartbeat = tokio::spawn(async move {
        for sequence in 1..10 {
            let pong = result(
                call(
                    &healthy,
                    "phone",
                    "protocol.ping",
                    json!({"sequence":sequence}),
                )
                .await,
            );
            assert_eq!(pong["sequence"], sequence);
        }
    });
    let mut offset = 0;
    let mut all = Vec::new();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let output = result(
                call(
                    &service,
                    "phone",
                    "terminal.read",
                    json!({"terminalId":id,"offset":offset}),
                )
                .await,
            );
            let bytes = BASE64.decode(output["data"].as_str().unwrap()).unwrap();
            assert!(bytes.len() <= 32 * 1024);
            all.extend(bytes);
            offset = output["nextOffset"].as_u64().unwrap();
            if output["exited"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    heartbeat.await.unwrap();
    let output = String::from_utf8_lossy(&all);
    assert!(output.contains("\x1b[32m终端同步\x1b[0m"), "{output:?}");
    assert!(output.contains("33 91"), "{output:?}");
    result(
        call(
            &service,
            "phone",
            "terminal.close",
            json!({"terminalId":id}),
        )
        .await,
    );
}
