use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

struct Options {
    approval_mode: String,
    marker: Option<PathBuf>,
    request_log: Option<PathBuf>,
}

fn main() {
    let options = options();
    record_session_activity(&options, "process/start", "");
    let mut reader = BufReader::new(std::io::stdin());
    let mut writer = BufWriter::new(std::io::stdout());
    let mut line = String::new();
    let mut active_thread_id = None;
    let mut created_thread_started_in_process = false;
    let mut created_thread_read_failures_remaining = 2usize;
    let mut thread_renamed = false;
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
            handle_client_response(
                &options,
                &mut writer,
                active_thread_id.as_deref(),
                &message,
            );
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        record_request(&options, method, &params);
        record_session_activity(
            &options,
            method,
            params.get("threadId").and_then(Value::as_str).unwrap_or(""),
        );
        match method {
            "initialize" => {
                if options.approval_mode == "observer-initialize-no-response" {
                    continue;
                }
                if options.approval_mode == "execution-initialize-no-response"
                    && observer_discovery_completed(&options)
                {
                    continue;
                }
                if options.approval_mode == "execution-initialize-reject"
                    && observer_discovery_completed(&options)
                {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32003,
                                "message": "fixture rejected execution initialize"
                            }
                        }),
                    );
                    continue;
                }
                respond(
                    &mut writer,
                    id,
                    json!({
                        "codexHome": "/fixture/codex-home",
                        "platformFamily": "unix",
                        "platformOs": "macos",
                        "userAgent": "codex-app-server-fixture/1"
                    }),
                );
            }
            "model/list" => {
                if params["includeHidden"] != false || params["limit"] != 100 {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": { "code": -32602, "message": "model/list discovery parameters were not preserved" }
                        }),
                    );
                    continue;
                }
                respond(
                    &mut writer,
                    id,
                    json!({
                        "data": [{
                            "id": "fixture-model-record",
                            "model": "gpt-fixture",
                            "displayName": "GPT Fixture",
                            "description": "Fixture model",
                            "hidden": false,
                            "isDefault": true,
                            "defaultReasoningEffort": "high",
                            "supportedReasoningEfforts": [
                                {
                                    "reasoningEffort": "low",
                                    "description": "Fixture low reasoning"
                                },
                                {
                                    "reasoningEffort": "high",
                                    "description": "Fixture high reasoning"
                                }
                            ]
                        }, {
                            "id": "fixture-model-secondary-record",
                            "model": "gpt-fixture-secondary",
                            "displayName": "GPT Fixture Secondary",
                            "description": "Second fixture model",
                            "hidden": false,
                            "isDefault": false,
                            "defaultReasoningEffort": "medium",
                            "supportedReasoningEfforts": [
                                {
                                    "reasoningEffort": "high",
                                    "description": "Fixture high reasoning"
                                },
                                {
                                    "reasoningEffort": "medium",
                                    "description": "Fixture medium reasoning"
                                }
                            ]
                        }],
                        "nextCursor": null
                    }),
                );
            }
            "project/list" => {
                if options.approval_mode == "project-unsupported" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": { "code": -32601, "message": "method not found" }
                        }),
                    );
                    continue;
                }
                respond(
                    &mut writer,
                    id,
                    json!({
                        "data": [project("project-fixture", "Fixture Project")],
                        "nextCursor": null
                    }),
                );
            }
            "project/read" => {
                respond(
                    &mut writer,
                    id,
                    json!({
                        "project": project(
                            params["projectId"].as_str().unwrap_or("project-fixture"),
                            "Fixture Project"
                        )
                    }),
                );
            }
            "project/create" => {
                respond(
                    &mut writer,
                    id,
                    json!({
                        "project": project("project-created", params["name"].as_str().unwrap_or("Created Project"))
                    }),
                );
            }
            "project/update" => {
                respond(
                    &mut writer,
                    id,
                    json!({
                        "project": project(
                            params["projectId"].as_str().unwrap_or("project-fixture"),
                            params["name"].as_str().unwrap_or("Fixture Project")
                        )
                    }),
                );
            }
            "project/delete" => respond(&mut writer, id, json!({})),
            "thread/list" => {
                if params["sortKey"] != "updated_at"
                    || params["sortDirection"] != "desc"
                    || params["useStateDbOnly"] != true
                {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": { "code": -32602, "message": "thread/list must use state DB updated_at descending" }
                        }),
                    );
                    continue;
                }
                let (mut data, next_cursor) = if params.get("searchTerm")
                    == Some(&json!("gateway protocol"))
                {
                    if params["cursor"] != "search-cursor" || params["limit"] != 7 {
                        write_json(
                            &mut writer,
                            json!({
                                "id": id,
                                "error": { "code": -32602, "message": "search pagination was not preserved" }
                            }),
                        );
                        continue;
                    }
                    (
                        vec![thread("thread-search-result", "idle", Vec::new())],
                        Some("search-next"),
                    )
                } else {
                    (vec![thread("thread-listed", "idle", Vec::new())], None)
                };
                if let Some(project_id) = params.get("projectId") {
                    for thread in &mut data {
                        thread["projectId"] = project_id.clone();
                    }
                }
                respond(
                    &mut writer,
                    id,
                    json!({
                        "data": data,
                        "nextCursor": next_cursor,
                        "backwardsCursor": "search-back"
                    }),
                );
            }
            "thread/read" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-listed");
                if options.approval_mode == "unmaterialized-before-first-message"
                    && !created_thread_started_in_process
                    && thread_id == "thread-created"
                    && created_thread_read_failures_remaining > 0
                {
                    created_thread_read_failures_remaining -= 1;
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32600,
                                "message": format!("thread not loaded: {thread_id}")
                            }
                        }),
                    );
                    continue;
                }
                if options.approval_mode == "observer-create-read-eventually"
                    && !created_thread_started_in_process
                    && thread_id == "thread-created"
                    && created_thread_read_failures_remaining > 0
                {
                    created_thread_read_failures_remaining -= 1;
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32603,
                                "message": "failed to read thread: thread-store internal error: failed to read session metadata /fixture/rollout.jsonl: rollout at /fixture/rollout.jsonl is empty"
                            }
                        }),
                    );
                    continue;
                }
                if options.approval_mode == "create-read-eventually"
                    && created_thread_started_in_process
                    && thread_id == "thread-created"
                    && created_thread_read_failures_remaining > 0
                {
                    created_thread_read_failures_remaining -= 1;
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32603,
                                "message": "failed to read thread: thread-store internal error: failed to read session metadata: rollout is empty"
                            }
                        }),
                    );
                    continue;
                }
                let turn_state = read_turn_status(&options, thread_id);
                let turns = if params["includeTurns"] == false {
                    Vec::new()
                } else {
                    history_turns(thread_id, turn_state.as_deref())
                };
                let mut response_thread = thread(
                    thread_id,
                    match turn_state.as_deref() {
                        Some("inProgress") => "active",
                        Some("waitingApproval") => "waitingApproval",
                        Some("waitingUserInput") => "waitingUserInput",
                        _ => "idle",
                    },
                    turns,
                );
                if thread_renamed && active_thread_id.as_deref() == Some(thread_id) {
                    response_thread["name"] = json!("Renamed by Codex");
                }
                respond(
                    &mut writer,
                    id,
                    json!({
                        "thread": response_thread
                    }),
                );
            }
            "thread/turns/list" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-listed");
                if params["limit"].as_u64().is_none_or(|limit| !(1..=10).contains(&limit))
                    || params["itemsView"] != "full"
                    || params["sortDirection"] != "desc"
                {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32602,
                                "message": "thread/turns/list must request at most ten full turns in descending order"
                            }
                        }),
                    );
                    continue;
                }
                if options.approval_mode == "unmaterialized-before-first-message"
                    && thread_id == "thread-created"
                {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32600,
                                "message": format!(
                                    "thread {thread_id} is not materialized yet; thread/turns/list is unavailable before first user message"
                                )
                            }
                        }),
                    );
                    continue;
                }
                let turn_state = read_turn_status(&options, thread_id);
                let (data, next_cursor) = turn_page(
                    thread_id,
                    params.get("cursor").and_then(Value::as_str),
                    turn_state.as_deref(),
                );
                respond(
                    &mut writer,
                    id,
                    json!({
                        "data": data,
                        "nextCursor": next_cursor,
                        "backwardsCursor": null
                    }),
                );
            }
            "thread/resume" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-listed");
                if options.approval_mode == "resume-no-response"
                    && thread_id.starts_with("thread-resume-pending")
                {
                    continue;
                }
                if thread_id == "thread-active-writer" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32600,
                                "message": format!("thread {thread_id} already has an active writer")
                            }
                        }),
                    );
                    continue;
                }
                if thread_id == "thread-active-writer-wrong-code" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32000,
                                "message": format!("thread {thread_id} already has an active writer")
                            }
                        }),
                    );
                    continue;
                }
                if thread_id == "thread-active-writer-wrong-message" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32600,
                                "message": "thread writer is held by another runtime"
                            }
                        }),
                    );
                    continue;
                }
                if thread_id == "thread-active-writer-with-data" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32600,
                                "message": format!("thread {thread_id} already has an active writer"),
                                "data": { "owner": "fixture-owner" }
                            }
                        }),
                    );
                    continue;
                }
                if thread_id == "thread-active-writer-other-thread" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32600,
                                "message": "thread a-different-thread already has an active writer"
                            }
                        }),
                    );
                    continue;
                }
                respond(
                    &mut writer,
                    id,
                    configured_thread_result(thread_id, params.get("model").cloned()),
                );
            }
            "thread/start" => {
                clear_turn_status(&options, "thread-created");
                created_thread_started_in_process = true;
                let mut result =
                    configured_thread_result("thread-created", params.get("model").cloned());
                if let Some(project_id) = params.get("projectId") {
                    result["thread"]["projectId"] = project_id.clone();
                }
                respond(
                    &mut writer,
                    id,
                    result,
                );
            }
            "turn/start" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-created");
                if options.approval_mode == "turn-reject" {
                    write_json(
                        &mut writer,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32002,
                                "message": "fixture rejected turn/start"
                            }
                        }),
                    );
                    continue;
                }
                if options.approval_mode == "turn-sent-unknown" {
                    std::process::exit(0);
                }
                active_thread_id = Some(thread_id.to_string());
                let initial_state = if options.approval_mode == "waiting-user-input" {
                    "waitingUserInput"
                } else {
                    "inProgress"
                };
                write_turn_status(&options, thread_id, initial_state);
                let mut started_turn = turn("turn-started", "inProgress");
                let started_user_item = started_turn["items"][0].clone();
                started_turn["items"] = json!([]);
                respond(&mut writer, id, json!({ "turn": started_turn.clone() }));
                if options.approval_mode == "normal" {
                    thread_renamed = true;
                    notify(
                        &mut writer,
                        "thread/name/updated",
                        json!({
                            "threadId": thread_id,
                            "threadName": "Renamed by Codex"
                        }),
                    );
                }
                notify(
                    &mut writer,
                    "turn/started",
                    json!({ "threadId": thread_id, "turn": started_turn }),
                );
                notify(
                    &mut writer,
                    "item/started",
                    json!({
                        "threadId": thread_id,
                        "turnId": "turn-started",
                        "item": started_user_item
                    }),
                );
                notify(
                    &mut writer,
                    "item/agentMessage/delta",
                    json!({
                        "threadId": thread_id,
                        "turnId": "turn-started",
                        "itemId": "agent-one",
                        "delta": "fixture output"
                    }),
                );
                if matches!(
                    options.approval_mode.as_str(),
                    "normal" | "additional-network" | "complete-on-approval"
                ) {
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
                    if options.approval_mode != "additional-network" {
                        write_turn_status(&options, thread_id, "waitingApproval");
                    }
                }
                if options.approval_mode == "crash-after-start" {
                    std::process::exit(0);
                }
            }
            "turn/steer" => respond(
                &mut writer,
                id,
                json!({ "turnId": params["expectedTurnId"] }),
            ),
            "turn/interrupt" => {
                let thread_id = params["threadId"].as_str().unwrap_or("thread-created");
                write_turn_status(&options, thread_id, "interrupted");
                respond(&mut writer, id, json!({}));
            }
            other => write_json(
                &mut writer,
                json!({
                    "id": id,
                    "error": { "code": -32601, "message": format!("fixture method not found: {other}") }
                }),
            ),
        }
    }
    record_session_activity(&options, "process/eof", "");
}

fn read_turn_status(options: &Options, thread_id: &str) -> Option<String> {
    std::fs::read_to_string(turn_state_path(options, thread_id)?).ok()
}

fn write_turn_status(options: &Options, thread_id: &str, status: &str) {
    let Some(path) = turn_state_path(options, thread_id) else {
        return;
    };
    std::fs::write(path, status).unwrap();
}

fn clear_turn_status(options: &Options, thread_id: &str) {
    let Some(path) = turn_state_path(options, thread_id) else {
        return;
    };
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("failed to clear fixture turn state: {error}"),
    }
}

fn turn_state_path(options: &Options, thread_id: &str) -> Option<PathBuf> {
    let marker = options.marker.as_ref()?;
    let safe_thread_id = thread_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    Some(marker.with_file_name(format!("{safe_thread_id}.turn-state")))
}

fn record_request(options: &Options, method: &str, params: &Value) {
    let Some(path) = options.request_log.as_ref() else {
        return;
    };
    let thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    if thread_id.is_empty() {
        writeln!(log, "{method}").unwrap();
    } else {
        writeln!(log, "{method}\t{thread_id}").unwrap();
    }
}

fn record_session_activity(options: &Options, method: &str, thread_id: &str) {
    let Some(marker) = options.marker.as_ref() else {
        return;
    };
    let path = session_log_path(marker, std::process::id());
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(log, "{}\t{method}\t{thread_id}", std::process::id()).unwrap();
}

fn observer_discovery_completed(options: &Options) -> bool {
    let Some(marker) = options.marker.as_ref() else {
        return false;
    };
    session_log_paths(marker).into_iter().any(|path| {
        std::fs::read_to_string(path)
            .is_ok_and(|contents| contents.lines().any(|line| line.contains("\tmodel/list\t")))
    })
}

fn session_log_path(marker: &std::path::Path, process_id: u32) -> PathBuf {
    marker.with_extension(format!("sessions.{process_id}"))
}

fn session_log_paths(marker: &std::path::Path) -> Vec<PathBuf> {
    let Some(parent) = marker.parent() else {
        return Vec::new();
    };
    let Some(stem) = marker.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.sessions.");
    std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
                .then(|| entry.path())
        })
        .collect()
}

fn handle_client_response(
    options: &Options,
    writer: &mut BufWriter<std::io::Stdout>,
    active_thread_id: Option<&str>,
    message: &Value,
) {
    if message.get("id") != Some(&json!("approval-one")) {
        return;
    }
    record_session_activity(
        options,
        "approval/response",
        active_thread_id.unwrap_or(""),
    );
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
    if let Some(thread_id) = active_thread_id {
        if options.approval_mode == "complete-on-approval" {
            write_turn_status(options, thread_id, "completed");
            notify(
                writer,
                "turn/completed",
                json!({
                    "threadId": thread_id,
                    "turn": turn("turn-started", "completed")
                }),
            );
        } else {
            write_turn_status(options, thread_id, "inProgress");
        }
    }
}

fn options() -> Options {
    let mut approval_mode = "normal".to_string();
    let mut marker = None;
    let mut request_log = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--approval-mode" => approval_mode = args.next().unwrap_or_default(),
            "--marker" => marker = args.next().map(PathBuf::from),
            "--request-log" => request_log = args.next().map(PathBuf::from),
            _ => {}
        }
    }
    Options {
        approval_mode,
        marker,
        request_log,
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

fn project(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "roots": [{ "path": "/fixture/workspace" }],
        "metadata": { "fixture": "true" },
        "position": 1,
        "createdAt": 1700000000,
        "updatedAt": 1700000001
    })
}

fn thread(id: &str, status: &str, turns: Vec<Value>) -> Value {
    let status = match status {
        "active" => json!({ "type": "active", "activeFlags": [] }),
        "waitingApproval" => {
            json!({ "type": "active", "activeFlags": ["waitingOnApproval"] })
        }
        "waitingUserInput" => {
            json!({ "type": "active", "activeFlags": ["waitingOnUserInput"] })
        }
        status => json!({ "type": status }),
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
    let activity_status = if status == "inProgress" { "inProgress" } else { "completed" };
    json!({
        "id": id,
        "status": status,
        "startedAt": 30,
        "completedAt": if status == "inProgress" { Value::Null } else { json!(31) },
        "itemsView": "full",
        "items": [
            {
                "type": "userMessage",
                "id": "user-one",
                "clientId": "message-one",
                "content": [
                    { "type": "text", "text": "run fixture" },
                    { "type": "image", "imageUrl": "data:image/png;base64,private" }
                ]
            },
            {
                "type": "agentMessage",
                "id": "agent-one",
                "text": "fixture answer"
            },
            {
                "type": "reasoning",
                "id": "reasoning-one",
                "summary": ["Checked the fixture"],
                "content": ["private raw reasoning"]
            },
            {
                "type": "commandExecution",
                "id": "command-one",
                "command": "cargo test",
                "status": activity_status,
                "aggregatedOutput": if status == "inProgress" { Value::Null } else { json!("tests passed") }
            },
            {
                "type": "fileChange",
                "id": "file-one",
                "status": activity_status,
                "changes": [{ "path": "private-a" }, { "path": "private-b" }]
            },
            {
                "type": "mcpToolCall",
                "id": "mcp-one",
                "server": "fixture-server",
                "tool": "lookup",
                "status": activity_status,
                "arguments": { "private": true }
            },
            {
                "type": "dynamicToolCall",
                "id": "dynamic-one",
                "namespace": "fixture",
                "tool": "inspect",
                "status": activity_status,
                "arguments": { "private": true }
            },
            {
                "type": "futureCodexItem",
                "id": "unknown-one",
                "privatePayload": "must not escape"
            }
        ]
    })
}

fn large_turn(id: &str, status: &str) -> Value {
    let mut value = turn(id, status);
    value["items"][1]["text"] = Value::String("x".repeat(1024 * 1024 + 4096));
    value
}

fn history_turns(thread_id: &str, turn_state: Option<&str>) -> Vec<Value> {
    if thread_id == "thread-large" {
        return vec![large_turn("turn-large", "completed")];
    }
    let turn_status = turn_state.map(|state| match state {
        "waitingApproval" | "waitingUserInput" => "inProgress",
        status => status,
    });
    turn_status
        .map(|status| vec![turn("turn-started", status)])
        .unwrap_or_else(|| vec![turn("turn-history", "completed")])
}

fn turn_page(
    thread_id: &str,
    cursor: Option<&str>,
    turn_state: Option<&str>,
) -> (Vec<Value>, Option<&'static str>) {
    match (thread_id, cursor) {
        ("thread-paginated", None) => (
            vec![paged_turn("turn-page-two", "agent-page-two", "page two")],
            Some("page-two"),
        ),
        ("thread-paginated", Some("page-two")) => (
            vec![paged_turn("turn-page-one", "agent-page-one", "page one")],
            None,
        ),
        ("thread-output-too-large", None) => (
            vec![large_agent_turn("turn-large-three", "agent-large-three")],
            Some("large-page-two"),
        ),
        ("thread-output-too-large", Some("large-page-two")) => (
            vec![large_agent_turn("turn-large-two", "agent-large-two")],
            Some("large-page-three"),
        ),
        ("thread-output-too-large", Some("large-page-three")) => (
            vec![large_agent_turn("turn-large-one", "agent-large-one")],
            None,
        ),
        _ => (history_turns(thread_id, turn_state), None),
    }
}

fn paged_turn(turn_id: &str, item_id: &str, text: &str) -> Value {
    json!({
        "id": turn_id,
        "status": "completed",
        "startedAt": 30,
        "completedAt": 31,
        "itemsView": "full",
        "items": [{
            "type": "agentMessage",
            "id": item_id,
            "text": text
        }]
    })
}

fn large_agent_turn(turn_id: &str, item_id: &str) -> Value {
    paged_turn(turn_id, item_id, &"x".repeat(6 * 1024 * 1024))
}

fn respond(writer: &mut BufWriter<std::io::Stdout>, id: Value, result: Value) {
    write_json(writer, json!({ "id": id, "result": result }));
}

fn notify(writer: &mut BufWriter<std::io::Stdout>, method: &str, params: Value) {
    write_json(
        writer,
        json!({ "method": method, "params": params, "emittedAtMs": 1234 }),
    );
}

fn request(
    writer: &mut BufWriter<std::io::Stdout>,
    id: Value,
    method: &str,
    params: Value,
) {
    write_json(
        writer,
        json!({
            "id": id,
            "method": method,
            "params": params,
            "trace": { "traceparent": null, "tracestate": null }
        }),
    );
}

fn write_json(writer: &mut BufWriter<std::io::Stdout>, value: Value) {
    serde_json::to_writer(&mut *writer, &value).unwrap();
    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
}
