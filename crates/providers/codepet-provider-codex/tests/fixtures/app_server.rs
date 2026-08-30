use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

struct Options {
    approval_mode: String,
    marker: Option<PathBuf>,
}

fn main() {
    let options = options();
    let mut reader = BufReader::new(std::io::stdin());
    let mut writer = BufWriter::new(std::io::stdout());
    let mut line = String::new();
    let mut turn_status: Option<String> = None;
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let message: Value = match serde_json::from_str(line.trim()) {
            Ok(message) => message,
            Err(error) => {
                eprintln!("fixture received invalid JSON: {error}");
                std::process::exit(2);
            }
        };
        line.clear();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            handle_client_response(&options, &message);
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        match method {
            "initialize" => respond(
                &mut writer,
                id,
                json!({
                    "codexHome": "/fixture/codex-home",
                    "platformFamily": "unix",
                    "platformOs": "macos",
                    "userAgent": "codex-app-server-fixture/1"
                }),
            ),
            "thread/list" => respond(
                &mut writer,
                id,
                json!({
                    "data": [thread("thread-listed", "idle", Vec::new())],
                    "nextCursor": null
                }),
            ),
            "thread/read" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-listed");
                respond(
                    &mut writer,
                    id,
                    json!({
                        "thread": thread(
                            thread_id,
                            if turn_status.as_deref() == Some("inProgress") { "active" } else { "idle" },
                            turn_status
                                .as_deref()
                                .map(|status| vec![turn("turn-started", status)])
                                .unwrap_or_default()
                        )
                    }),
                );
            }
            "thread/resume" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-listed");
                respond(
                    &mut writer,
                    id,
                    configured_thread_result(thread_id, params.get("model").cloned()),
                );
            }
            "thread/start" => respond(
                &mut writer,
                id,
                configured_thread_result("thread-created", params.get("model").cloned()),
            ),
            "turn/start" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-created");
                turn_status = Some("inProgress".to_string());
                let started_turn = turn("turn-started", "inProgress");
                respond(&mut writer, id, json!({ "turn": started_turn }));
                notify(
                    &mut writer,
                    "turn/started",
                    json!({ "threadId": thread_id, "turn": turn("turn-started", "inProgress") }),
                );
                notify(
                    &mut writer,
                    "item/agentMessage/delta",
                    json!({
                        "threadId": thread_id,
                        "turnId": "turn-started",
                        "itemId": "output-one",
                        "delta": "fixture output"
                    }),
                );
                if options.approval_mode != "none" {
                    let mut approval = json!({
                        "threadId": thread_id,
                        "turnId": "turn-started",
                        "itemId": "command-one",
                        "command": "cargo test",
                        "startedAtMs": 1234,
                        "availableDecisions": ["accept", "decline"]
                    });
                    if options.approval_mode == "additional-network" {
                        approval["additionalPermissions"] =
                            json!({ "network": { "enabled": true } });
                    }
                    request(
                        &mut writer,
                        json!("approval-one"),
                        "item/commandExecution/requestApproval",
                        approval,
                    );
                }
            }
            "turn/steer" => respond(
                &mut writer,
                id,
                json!({ "turnId": params["expectedTurnId"] }),
            ),
            "turn/interrupt" => {
                turn_status = Some("interrupted".to_string());
                respond(&mut writer, id, json!({}));
            }
            other => write_json(
                &mut writer,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("fixture method not found: {other}") }
                }),
            ),
        }
    }
}

fn handle_client_response(options: &Options, message: &Value) {
    if message.get("id") != Some(&json!("approval-one")) {
        return;
    }
    if let Some(marker) = options.marker.as_ref() {
        let outcome = message
            .pointer("/result/decision")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                message
                    .pointer("/error/code")
                    .and_then(Value::as_i64)
                    .map(|code| format!("error:{code}"))
            })
            .unwrap_or_else(|| "missing".to_string());
        std::fs::write(marker, outcome).unwrap();
    }
}

fn options() -> Options {
    let mut approval_mode = "normal".to_string();
    let mut marker = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--approval-mode" => approval_mode = args.next().unwrap_or_default(),
            "--marker" => marker = args.next().map(PathBuf::from),
            _ => {}
        }
    }
    Options {
        approval_mode,
        marker,
    }
}

fn configured_thread_result(thread_id: &str, model: Option<Value>) -> Value {
    json!({
        "thread": thread(thread_id, "idle", Vec::new()),
        "model": model.unwrap_or(json!("gpt-fixture")),
        "modelProvider": "openai",
        "cwd": "/fixture/workspace",
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "reasoningEffort": "high",
        "sandbox": { "type": "workspaceWrite" }
    })
}

fn thread(id: &str, status: &str, turns: Vec<Value>) -> Value {
    let status = if status == "active" {
        json!({ "type": status, "activeFlags": [] })
    } else {
        json!({ "type": status })
    };
    json!({
        "id": id,
        "name": format!("Fixture {id}"),
        "preview": "fixture conversation",
        "cwd": "/fixture/workspace",
        "cliVersion": "0.151.0",
        "createdAt": 10,
        "ephemeral": false,
        "modelProvider": "openai",
        "projectId": null,
        "sessionId": format!("session-{id}"),
        "source": "appServer",
        "updatedAt": 20,
        "status": status,
        "turns": turns
    })
}

fn turn(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "status": status,
        "startedAt": 30,
        "completedAt": if status == "inProgress" { Value::Null } else { json!(31) },
        "items": []
    })
}

fn respond(writer: &mut BufWriter<std::io::Stdout>, id: Value, result: Value) {
    write_json(writer, json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

fn notify(writer: &mut BufWriter<std::io::Stdout>, method: &str, params: Value) {
    write_json(writer, json!({ "jsonrpc": "2.0", "method": method, "params": params }));
}

fn request(
    writer: &mut BufWriter<std::io::Stdout>,
    id: Value,
    method: &str,
    params: Value,
) {
    write_json(
        writer,
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    );
}

fn write_json(writer: &mut BufWriter<std::io::Stdout>, value: Value) {
    serde_json::to_writer(&mut *writer, &value).unwrap();
    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
}
