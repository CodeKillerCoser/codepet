use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

struct Options {
    session_id: String,
    resumed: bool,
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("2.1.251 (Claude Code fixture)");
        return;
    }

    let options = parse_options();
    let (input, mut input_reader) = read_input();
    let command_uuid = input
        .get("uuid")
        .and_then(Value::as_str)
        .expect("fixture input uuid");
    let message = input
        .pointer("/message/content")
        .and_then(Value::as_str)
        .expect("fixture input message");
    assert_eq!(input["type"], "user");
    assert_eq!(input["message"]["role"], "user");
    assert!(input["parent_tool_use_id"].is_null());

    let inherited_project_mcp = project_mcp_is_visible();
    let mcp_servers = if inherited_project_mcp {
        json!([{ "name": "fixture-observed", "status": "connected" }])
    } else {
        json!([])
    };
    let mut writer = BufWriter::new(std::io::stdout());
    write_json(
        &mut writer,
        json!({
            "type": "command_lifecycle",
            "command_uuid": command_uuid,
            "state": "queued",
            "uuid": "33333333-3333-4333-8333-333333333333",
            "session_id": options.session_id
        }),
    );
    write_json(
        &mut writer,
        json!({
            "type": "system",
            "subtype": "init",
            "cwd": std::env::current_dir().unwrap(),
            "session_id": options.session_id,
            "model": "claude-sonnet-5",
            "permissionMode": "default",
            "tools": ["Task", "Bash", "Edit", "Read", "Write"],
            "mcp_servers": mcp_servers,
            "capabilities": ["interrupt_receipt_v1", "msg_lifecycle_v1"],
            "uuid": "44444444-4444-4444-8444-444444444444"
        }),
    );
    write_json(
        &mut writer,
        json!({
            "type": "system",
            "subtype": "status",
            "status": "requesting",
            "session_id": options.session_id,
            "uuid": "55555555-5555-4555-8555-555555555555"
        }),
    );

    #[cfg(unix)]
    if message == "ignore sigint" {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
        write_process_probe(std::env::current_dir().unwrap().as_path());
        thread::sleep(Duration::from_secs(60));
        return;
    }

    #[cfg(unix)]
    if message == "stdout close then sleep" {
        write_process_probe(std::env::current_dir().unwrap().as_path());
        writer.flush().unwrap();
        drop(writer);
        unsafe {
            libc::close(libc::STDOUT_FILENO);
        }
        thread::sleep(Duration::from_secs(60));
        return;
    }

    if message == "wait for interrupt" {
        thread::sleep(Duration::from_secs(60));
        return;
    }

    if message == "fail" {
        write_json(
            &mut writer,
            json!({
                "type": "assistant",
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Not logged in" }]
                },
                "parent_tool_use_id": null,
                "session_id": options.session_id,
                "error": "authentication_failed",
                "is_api_error_message": true,
                "uuid": "66666666-6666-4666-8666-666666666666"
            }),
        );
        write_result(&mut writer, &options.session_id, "Not logged in", true);
        std::process::exit(1);
    }

    if message == "two mib result" {
        write_result(
            &mut writer,
            &options.session_id,
            &"x".repeat(2 * 1024 * 1024),
            false,
        );
        return;
    }

    #[cfg(unix)]
    if message == "oversized no newline" {
        write_process_probe(std::env::current_dir().unwrap().as_path());
        writer
            .write_all(&vec![b'x'; 4 * 1024 * 1024 + 1])
            .unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_secs(60));
        return;
    }

    if message == "result then sleep" {
        write_result(&mut writer, &options.session_id, "fixture delayed exit", false);
        #[cfg(unix)]
        write_process_probe(std::env::current_dir().unwrap().as_path());
        thread::sleep(Duration::from_secs(60));
        return;
    }

    if message == "needs approval" {
        write_json(
            &mut writer,
            json!({
                "type": "control_request",
                "request_id": "permission-request-1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Bash",
                    "input": { "command": "touch approved.txt" },
                    "tool_use_id": "tool-use-1",
                    "title": "Run a shell command",
                    "description": "touch approved.txt"
                }
            }),
        );
        let mut response_line = String::new();
        assert!(input_reader.read_line(&mut response_line).unwrap() > 0);
        let response: Value = serde_json::from_str(response_line.trim()).unwrap();
        assert_eq!(response["type"], "control_response");
        assert_eq!(response["response"]["subtype"], "success");
        assert_eq!(response["response"]["request_id"], "permission-request-1");
        let behavior = response["response"]["response"]["behavior"]
            .as_str()
            .unwrap();
        if behavior == "allow" {
            assert_eq!(
                response["response"]["response"]["updatedInput"]["command"],
                "touch approved.txt"
            );
        } else {
            assert_eq!(behavior, "deny");
        }
        let output = if behavior == "allow" {
            "fixture approved"
        } else {
            "fixture denied"
        };
        write_result(&mut writer, &options.session_id, output, false);
        return;
    }

    let output = if message == "inherit project config" {
        assert!(inherited_project_mcp, "fixture project MCP config was not visible");
        "fixture inherited project MCP"
    } else if options.resumed {
        "fixture resumed"
    } else {
        "fixture output"
    };
    for (index, delta) in [output.split_at(8).0, output.split_at(8).1]
        .into_iter()
        .enumerate()
    {
        write_json(
            &mut writer,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": index,
                    "delta": { "type": "text_delta", "text": delta }
                },
                "parent_tool_use_id": null,
                "session_id": options.session_id,
                "uuid": format!("77777777-7777-4777-8777-77777777777{index}")
            }),
        );
    }
    write_json(
        &mut writer,
        json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": output }]
            },
            "parent_tool_use_id": null,
            "session_id": options.session_id,
            "uuid": "88888888-8888-4888-8888-888888888888"
        }),
    );
    write_result(&mut writer, &options.session_id, output, false);
}

fn parse_options() -> Options {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.iter().any(|arg| arg == "--print"));
    assert!(args.iter().any(|arg| arg == "--verbose"));
    assert!(args.iter().any(|arg| arg == "--include-partial-messages"));
    assert!(!args.iter().any(|arg| arg == "--include-hook-events"));
    assert_eq!(value_after(&args, "--permission-prompt-tool"), "stdio");
    for inherited_flag in [
        "--safe-mode",
        "--setting-sources",
        "--strict-mcp-config",
        "--mcp-config",
        "--settings",
        "--restricted",
        "--tools",
    ] {
        assert!(!args.iter().any(|arg| arg == inherited_flag));
    }
    assert_eq!(value_after(&args, "--permission-mode"), "manual");
    assert_eq!(value_after(&args, "--input-format"), "stream-json");
    assert_eq!(value_after(&args, "--output-format"), "stream-json");
    assert_eq!(value_after(&args, "--model"), "sonnet");
    assert_eq!(value_after(&args, "--effort"), "high");

    let session = args
        .iter()
        .position(|arg| arg == "--resume")
        .map(|index| (args[index + 1].clone(), true))
        .or_else(|| {
            args.iter()
                .position(|arg| arg == "--session-id")
                .map(|index| (args[index + 1].clone(), false))
        })
        .expect("fixture session flag");
    assert!(uuid::Uuid::parse_str(&session.0).is_ok());
    if session.1 {
        assert!(!args.iter().any(|arg| arg == "--session-id"));
        assert!(!args.iter().any(|arg| arg == "--name"));
    } else {
        assert!(!args.iter().any(|arg| arg == "--resume"));
        assert_eq!(value_after(&args, "--name"), "Fixture conversation");
    }
    assert_eq!(value_after(&args, "--model"), "sonnet");
    assert_eq!(value_after(&args, "--effort"), "high");
    Options {
        session_id: session.0,
        resumed: session.1,
    }
}

fn project_mcp_is_visible() -> bool {
    let path = std::env::current_dir().unwrap().join(".mcp.json");
    fs::read(path)
        .ok()
        .and_then(|contents| serde_json::from_slice::<Value>(&contents).ok())
        .and_then(|config| config.pointer("/mcpServers/fixture-observed").cloned())
        .is_some()
}

fn value_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let index = args.iter().position(|arg| arg == flag).unwrap();
    args.get(index + 1).map(String::as_str).unwrap()
}

fn read_input() -> (Value, BufReader<std::io::Stdin>) {
    let mut reader = BufReader::new(std::io::stdin());
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    let input = serde_json::from_str(line.trim()).unwrap();
    (input, reader)
}

fn write_result(
    writer: &mut BufWriter<std::io::Stdout>,
    session_id: &str,
    result: &str,
    is_error: bool,
) {
    write_json(
        writer,
        json!({
            "type": "result",
            "subtype": "success",
            "is_error": is_error,
            "session_id": session_id,
            "result": result,
            "stop_reason": "end_turn",
            "terminal_reason": if is_error { Some("api_error") } else { None },
            "usage": { "input_tokens": 10, "output_tokens": 2 },
            "total_cost_usd": if is_error { 0.0 } else { 0.001 }
        }),
    );
}

fn write_json(writer: &mut BufWriter<std::io::Stdout>, value: Value) {
    serde_json::to_writer(&mut *writer, &value).unwrap();
    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
}

#[cfg(unix)]
fn write_process_probe(workspace: &Path) {
    fs::write(workspace.join("fixture-root.pid"), std::process::id().to_string()).unwrap();
    let child = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    fs::write(workspace.join("fixture-child.pid"), child.id().to_string()).unwrap();
    drop(child);
}
