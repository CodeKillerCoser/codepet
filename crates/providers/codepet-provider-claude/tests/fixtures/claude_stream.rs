use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::thread;
use std::time::Duration;

struct Options {
    session_id: String,
    resumed: bool,
    permission_mode: String,
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("2.1.251 (Claude Code fixture)");
        return;
    }

    let options = parse_options();
    let input = read_input();
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
            "permissionMode": options.permission_mode,
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
        write_json(
            &mut writer,
            json!({
                "type": "result",
                "subtype": "success",
                "is_error": true,
                "session_id": options.session_id,
                "result": "Not logged in",
                "stop_reason": "stop_sequence",
                "terminal_reason": "api_error",
                "usage": { "input_tokens": 0, "output_tokens": 0 },
                "total_cost_usd": 0.0
            }),
        );
        std::process::exit(1);
    }

    let output = if options.resumed {
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
    write_json(
        &mut writer,
        json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "session_id": options.session_id,
            "result": output,
            "stop_reason": "end_turn",
            "terminal_reason": null,
            "usage": { "input_tokens": 10, "output_tokens": 2 },
            "total_cost_usd": 0.001
        }),
    );
}

fn parse_options() -> Options {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.iter().any(|arg| arg == "--print"));
    assert!(args.iter().any(|arg| arg == "--verbose"));
    assert!(args.iter().any(|arg| arg == "--include-partial-messages"));
    assert!(!args.iter().any(|arg| arg == "--include-hook-events"));
    assert!(!args.iter().any(|arg| arg == "--permission-prompt-tool"));
    assert_eq!(value_after(&args, "--input-format"), "stream-json");
    assert_eq!(value_after(&args, "--output-format"), "stream-json");
    assert_eq!(value_after(&args, "--settings"), r#"{"disableAllHooks":true}"#);

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
    assert_eq!(value_after(&args, "--permission-mode"), "acceptEdits");
    assert_eq!(value_after(&args, "--model"), "sonnet");
    assert_eq!(value_after(&args, "--effort"), "high");
    Options {
        session_id: session.0,
        resumed: session.1,
        permission_mode: value_after(&args, "--permission-mode").to_string(),
    }
}

fn value_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let index = args.iter().position(|arg| arg == flag).unwrap();
    args.get(index + 1).map(String::as_str).unwrap()
}

fn read_input() -> Value {
    let mut reader = BufReader::new(std::io::stdin());
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    let input = serde_json::from_str(line.trim()).unwrap();
    line.clear();
    assert_eq!(reader.read_line(&mut line).unwrap(), 0);
    input
}

fn write_json(writer: &mut BufWriter<std::io::Stdout>, value: Value) {
    serde_json::to_writer(&mut *writer, &value).unwrap();
    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
}
