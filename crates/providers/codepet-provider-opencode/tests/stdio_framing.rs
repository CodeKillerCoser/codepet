use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn standalone_binary_uses_provider_json_line_framing() {
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
    {
        let stdin = child.stdin.as_mut().unwrap();
        for frame in frames {
            serde_json::to_writer(&mut *stdin, &frame).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], "1");
    assert_eq!(responses[0]["result"]["selectedVersion"], 1);
    assert_eq!(responses[1]["result"]["plugin"]["pluginId"], "dev.codepet.opencode");
    assert_eq!(responses[2]["result"]["accepted"], true);
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
        serde_json::to_writer(&mut stdin, &frame).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let response = loop {
            let mut response = String::new();
            stdout.read_line(&mut response).unwrap();
            let response: Value = serde_json::from_str(&response).unwrap();
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

    stdin.write_all(b"{\"jsonrpc\":\n").unwrap();
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
