use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};

fn main() {
    let mut reader = BufReader::new(std::io::stdin());
    let mut writer = BufWriter::new(std::io::stdout());
    let mut line = String::new();
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
            handle_client_response(&message);
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        match method {
            "initialize" => respond(&mut writer, id, json!({})),
            "thread/list" => respond(
                &mut writer,
                id,
                json!({
                    "data": [thread("thread-listed", "idle", Vec::new())],
                    "nextCursor": null
                }),
            ),
            "thread/read" | "thread/resume" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-listed");
                respond(
                    &mut writer,
                    id,
                    json!({
                        "thread": thread(thread_id, "idle", Vec::new()),
                        "model": "gpt-fixture",
                        "reasoningEffort": "high",
                        "sandbox": { "type": "workspaceWrite" }
                    }),
                );
            }
            "thread/start" => respond(
                &mut writer,
                id,
                json!({
                    "thread": thread("thread-created", "idle", Vec::new()),
                    "model": params.get("model").cloned().unwrap_or(json!("gpt-fixture")),
                    "reasoningEffort": "high",
                    "sandbox": { "type": "workspaceWrite" }
                }),
            ),
            "turn/start" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-created");
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
                request(
                    &mut writer,
                    json!("approval-one"),
                    "item/commandExecution/requestApproval",
                    json!({
                        "threadId": thread_id,
                        "turnId": "turn-started",
                        "itemId": "command-one",
                        "command": "cargo test",
                        "startedAtMs": 1234,
                        "availableDecisions": ["accept", "decline"]
                    }),
                );
            }
            "turn/steer" => respond(
                &mut writer,
                id,
                json!({ "turnId": params["expectedTurnId"] }),
            ),
            "turn/interrupt" => respond(&mut writer, id, json!({})),
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

fn handle_client_response(message: &Value) {
    if message.get("id") != Some(&json!("approval-one")) {
        return;
    }
    if let Some(marker) = std::env::var_os("CODEPET_FIXTURE_APPROVAL_MARKER") {
        let decision = message
            .pointer("/result/decision")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        std::fs::write(marker, decision).unwrap();
    }
}

fn thread(id: &str, status: &str, turns: Vec<Value>) -> Value {
    json!({
        "id": id,
        "name": format!("Fixture {id}"),
        "preview": "fixture conversation",
        "cwd": "/fixture/workspace",
        "createdAt": 10,
        "updatedAt": 20,
        "status": { "type": status },
        "turns": turns
    })
}

fn turn(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "status": status,
        "startedAt": 30,
        "completedAt": null
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
