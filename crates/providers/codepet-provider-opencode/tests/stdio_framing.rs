use codepet_provider_sdk::{ProviderFrameCodec, ProviderWireMessage};
use serde_json::Value;
use std::io::{BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn standalone_binary_uses_provider_binary_frame_v1() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codepet-provider-opencode"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let frames = [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "provider.initialize",
            "params": {
                "hostClientId": "framing-host",
                "hostDeviceId": "framing-device",
                "hostVersion": "0.1.0",
                "supportedVersions": {"minVersion": 1, "maxVersion": 1}
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "2",
            "method": "provider.describe",
            "params": {}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "3",
            "method": "provider.shutdown",
            "params": {}
        }),
    ];
    let mut responses = Vec::new();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for frame in frames {
        let expected_id = frame["id"].as_str().unwrap();
        let stdin = child.stdin.as_mut().unwrap();
        write_frame(stdin, &frame);
        responses.push(read_response(&mut stdout, expected_id));
    }
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success());
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], "1");
    assert_eq!(responses[0]["result"]["selectedVersion"], 1);
    assert_eq!(responses[1]["result"]["plugin"]["pluginId"], "dev.codepet.opencode");
    assert_eq!(responses[2]["result"]["accepted"], true);
}

#[test]
fn turn_generation_is_unique_across_provider_processes() {
    let first = start_turn_resource_from_fresh_provider();
    let second = start_turn_resource_from_fresh_provider();
    assert_ne!(first, second);
}

fn start_turn_resource_from_fresh_provider() -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codepet-provider-opencode"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let route = serde_json::json!({
        "deviceId": "generation-device",
        "providerPluginId": "dev.codepet.opencode",
        "providerInstanceId": "generation-opencode"
    });
    let mut resource = None;
    for frame in [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "initialize",
            "method": "provider.initialize",
            "params": {
                "hostClientId": "generation-host",
                "hostDeviceId": "generation-device",
                "hostVersion": "0.1.0",
                "supportedVersions": {"minVersion": 1, "maxVersion": 1}
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "create",
            "method": "instance.create",
            "params": {
                "route": route,
                "instanceKind": "opencode",
                "displayName": "Generation OpenCode",
                "settings": {
                    "serverExecutable": env!("CARGO_BIN_EXE_opencode-server-fixture"),
                    "serverVersion": "1.18.25",
                    "serverArgs": ["serve"]
                }
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "start",
            "method": "instance.start",
            "params": {"route": route}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "turn",
            "method": "turn.start",
            "params": {
                "conversation": {
                    "deviceId": "generation-device",
                    "providerPluginId": "dev.codepet.opencode",
                    "providerInstanceId": "generation-opencode",
                    "nativeResourceId": "ses_fixture"
                },
                "clientRequestId": "same-client-message",
                "capabilityRevision": "opencode-server-1.18.25-controls-v1",
                "input": { "kind": "text", "text": "needs approval" },
                "selection": {}
            }
        }),
    ] {
        let expected_id = frame["id"].as_str().unwrap().to_string();
        write_frame(&mut stdin, &frame);
        let response = read_response(&mut stdout, &expected_id);
        assert!(response.get("error").is_none(), "{response}");
        if expected_id == "turn" {
            resource = Some(
                response["result"]["turn"]["resource"]["nativeResourceId"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
    }
    let shutdown = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "shutdown",
        "method": "provider.shutdown",
        "params": {}
    });
    write_frame(&mut stdin, &shutdown);
    let response = read_response(&mut stdout, "shutdown");
    assert_eq!(response["result"]["accepted"], true);
    drop(stdin);
    drop(stdout);
    assert!(child.wait().unwrap().success());
    resource.unwrap()
}

fn write_frame(stdin: &mut std::process::ChildStdin, frame: &Value) {
    let payload = serde_json::to_vec(frame).unwrap();
    let message = codepet_provider_sdk::decode_wire_message(&payload).unwrap();
    ProviderFrameCodec::default()
        .write_message(&mut *stdin, &message)
        .unwrap();
    stdin.flush().unwrap();
}

fn read_response(stdout: &mut BufReader<std::process::ChildStdout>, id: &str) -> Value {
    loop {
        let message = ProviderFrameCodec::default()
            .read_message(stdout)
            .unwrap()
            .expect("Provider response or event frame");
        let response = provider_wire_value(message);
        if response["id"] == id {
            return response;
        }
    }
}

#[cfg(unix)]
#[test]
fn malformed_host_frame_still_cleans_up_the_owned_server() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("opencode.pid");
    let mut child = Command::new(env!("CARGO_BIN_EXE_codepet-provider-opencode"))
        .env("OPENCODE_FIXTURE_PID_FILE", &pid_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let route = serde_json::json!({
        "deviceId": "framing-device",
        "providerPluginId": "dev.codepet.opencode",
        "providerInstanceId": "framing-opencode"
    });
    for frame in [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "provider.initialize",
            "params": {
                "hostClientId": "framing-host",
                "hostDeviceId": "framing-device",
                "hostVersion": "0.1.0",
                "supportedVersions": {"minVersion": 1, "maxVersion": 1}
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "2",
            "method": "instance.create",
            "params": {
                "route": route,
                "instanceKind": "opencode",
                "displayName": "Framing OpenCode",
                "settings": {
                    "serverExecutable": env!("CARGO_BIN_EXE_opencode-server-fixture"),
                    "serverVersion": "1.18.25",
                    "serverArgs": ["serve"]
                }
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "3",
            "method": "instance.start",
            "params": {"route": route}
        }),
    ] {
        let expected_id = frame["id"].clone();
        write_frame(&mut stdin, &frame);
        let response = loop {
            let response = provider_wire_value(
                ProviderFrameCodec::default()
                    .read_message(&mut stdout)
                    .unwrap()
                    .expect("Provider response or event frame"),
            );
            if response["id"] == expected_id {
                break response;
            }
        };
        assert!(response.get("error").is_none(), "{response}");
    }
    assert!(pid_file.exists());
    let server_pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();

    write_raw_provider_frame(&mut stdin, b"{\"jsonrpc\":");
    stdin.flush().unwrap();
    drop(stdin);
    drop(stdout);
    let status = child.wait().unwrap();
    assert!(!status.success());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while unsafe { libc::kill(server_pid, 0) } == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(unsafe { libc::kill(server_pid, 0) }, 0);
}

fn provider_wire_value(message: ProviderWireMessage) -> Value {
    match message {
        ProviderWireMessage::Response(value) => serde_json::to_value(value).unwrap(),
        ProviderWireMessage::Notification(value) => serde_json::to_value(value).unwrap(),
        ProviderWireMessage::Event(value) => serde_json::to_value(value).unwrap(),
        ProviderWireMessage::Request(_) => panic!("Provider stdout emitted a request"),
    }
}

fn write_raw_provider_frame(writer: &mut impl Write, payload: &[u8]) {
    writer
        .write_all(&codepet_provider_sdk::PROVIDER_FRAME_MAGIC)
        .unwrap();
    writer
        .write_all(&[
            codepet_provider_sdk::PROVIDER_FRAME_VERSION,
            codepet_provider_sdk::ProviderFrameEncoding::RawJson as u8,
        ])
        .unwrap();
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .unwrap();
    writer.write_all(payload).unwrap();
    writer.flush().unwrap();
}
