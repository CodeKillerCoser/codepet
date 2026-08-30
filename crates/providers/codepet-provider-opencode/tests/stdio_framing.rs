use serde_json::Value;
use std::io::Write;
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
