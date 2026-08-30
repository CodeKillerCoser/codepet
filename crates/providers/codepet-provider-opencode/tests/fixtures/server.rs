use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

struct FixtureState {
    sessions: HashMap<String, Value>,
    active: HashSet<String>,
    subscribers: Vec<Sender<String>>,
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let hostname = argument(&args, "--hostname").unwrap_or("127.0.0.1");
    let port = argument(&args, "--port")
        .and_then(|value| value.parse::<u16>().ok())
        .expect("fixture requires --port");
    assert_eq!(args.get(1).map(String::as_str), Some("serve"));
    let fixture_directory = std::env::temp_dir()
        .join("opencode-fixture")
        .to_string_lossy()
        .to_string();
    let initial = session(
        "ses_fixture",
        "Fixture session",
        &fixture_directory,
        1_700_000_000_000,
    );
    let state = Arc::new(Mutex::new(FixtureState {
        sessions: [("ses_fixture".to_string(), initial)].into_iter().collect(),
        active: HashSet::new(),
        subscribers: Vec::new(),
    }));
    let listener = TcpListener::bind((hostname, port)).expect("bind fixture server");
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let state = state.clone();
        thread::spawn(move || handle_connection(stream, state));
    }
}

fn argument<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|value| value == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn handle_connection(stream: TcpStream, state: Arc<Mutex<FixtureState>>) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).ok() == Some(0) {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok() == Some(0) {
            return;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let path = target.split('?').next().unwrap_or(&target);
    if method == "GET" && path == "/api/event" {
        serve_events(reader.into_inner(), state);
        return;
    }
    let response = route(&method, path, &body, &state);
    let mut stream = reader.into_inner();
    let _ = write_response(&mut stream, response.0, response.1.as_bytes());
}

fn route(
    method: &str,
    path: &str,
    body: &[u8],
    state: &Arc<Mutex<FixtureState>>,
) -> (u16, String) {
    if method == "GET" && path == "/global/health" {
        return (200, json!({"healthy": true, "version": "1.18.25"}).to_string());
    }
    if method == "POST" && path == "/global/dispose" {
        return (200, "true".to_string());
    }
    if method == "GET" && path == "/api/session" {
        let sessions = lock(state).sessions.values().cloned().collect::<Vec<_>>();
        return (
            200,
            json!({"data": sessions, "cursor": {"previous": null, "next": null}}).to_string(),
        );
    }
    if method == "GET" && path == "/api/session/active" {
        let active = lock(state)
            .active
            .iter()
            .map(|id| (id.clone(), json!({"type": "running"})))
            .collect::<serde_json::Map<_, _>>();
        return (200, json!({"data": active}).to_string());
    }
    if method == "POST" && path == "/api/session" {
        let request: Value = serde_json::from_slice(body).unwrap();
        let directory = request["location"]["directory"].as_str().unwrap();
        let created = session("ses_created", "New session", directory, 1_700_000_001_000);
        lock(state)
            .sessions
            .insert("ses_created".to_string(), created.clone());
        broadcast(
            state,
            json!({
                "id": "evt_created",
                "type": "session.created",
                "data": {"sessionID": "ses_created", "info": created}
            }),
        );
        return (200, json!({"data": created}).to_string());
    }
    if let Some(session_id) = path.strip_prefix("/api/session/") {
        if !session_id.contains('/') && method == "GET" {
            return lock(state)
                .sessions
                .get(session_id)
                .cloned()
                .map(|session| (200, json!({"data": session}).to_string()))
                .unwrap_or_else(|| (404, json!({"error": "not found"}).to_string()));
        }
    }
    if method == "POST" && path.ends_with("/prompt") {
        let session_id = path
            .trim_start_matches("/api/session/")
            .trim_end_matches("/prompt")
            .trim_end_matches('/');
        let request: Value = serde_json::from_slice(body).unwrap();
        let message_id = request["id"].as_str().unwrap();
        let delivery = request["delivery"].as_str().unwrap();
        let timestamp = 1_700_000_002_000u64;
        if delivery == "queue" {
            lock(state).active.insert(session_id.to_string());
        }
        broadcast(
            state,
            json!({
                "id": format!("evt_admitted_{message_id}"),
                "type": "session.next.prompt.admitted",
                "data": {
                    "timestamp": timestamp,
                    "sessionID": session_id,
                    "messageID": message_id,
                    "prompt": {"text": request["prompt"]["text"]},
                    "delivery": delivery
                }
            }),
        );
        if delivery == "queue" {
            broadcast(
                state,
                json!({
                    "id": "evt_step",
                    "type": "session.next.step.started",
                    "data": {
                        "timestamp": timestamp + 1,
                        "sessionID": session_id,
                        "assistantMessageID": "msg_assistant_fixture",
                        "agent": "build",
                        "model": {"id": "fixture", "providerID": "fixture"}
                    }
                }),
            );
            broadcast(
                state,
                json!({
                    "id": "evt_delta",
                    "type": "session.next.text.delta",
                    "data": {
                        "timestamp": timestamp + 2,
                        "sessionID": session_id,
                        "assistantMessageID": "msg_assistant_fixture",
                        "textID": "txt_fixture",
                        "delta": "fixture output"
                    }
                }),
            );
            if request["prompt"]["text"] == "needs approval" {
                broadcast(
                    state,
                    json!({
                        "id": "evt_permission",
                        "type": "permission.v2.asked",
                        "data": {
                            "id": "per_fixture",
                            "sessionID": session_id,
                            "action": "bash",
                            "resources": ["echo fixture"],
                            "source": {
                                "type": "tool",
                                "messageID": message_id,
                                "callID": "call_fixture"
                            }
                        }
                    }),
                );
            }
        }
        return (
            200,
            json!({
                "data": {
                    "admittedSeq": 1,
                    "id": message_id,
                    "sessionID": session_id,
                    "prompt": request["prompt"],
                    "delivery": delivery,
                    "timeCreated": timestamp
                }
            })
            .to_string(),
        );
    }
    if method == "POST" && path.ends_with("/interrupt") {
        let session_id = path
            .trim_start_matches("/api/session/")
            .trim_end_matches("/interrupt")
            .trim_end_matches('/');
        lock(state).active.remove(session_id);
        return (204, String::new());
    }
    if method == "POST" && path.ends_with("/reply") && path.contains("/permission/") {
        let segments = path.split('/').collect::<Vec<_>>();
        let session_id = segments.get(3).copied().unwrap_or("");
        let request_id = segments.get(5).copied().unwrap_or("");
        let request: Value = serde_json::from_slice(body).unwrap();
        broadcast(
            state,
            json!({
                "id": "evt_permission_replied",
                "type": "permission.v2.replied",
                "data": {
                    "sessionID": session_id,
                    "requestID": request_id,
                    "reply": request["reply"]
                }
            }),
        );
        return (204, String::new());
    }
    (404, json!({"error": "fixture route not found"}).to_string())
}

fn session(id: &str, title: &str, directory: &str, timestamp: u64) -> Value {
    json!({
        "id": id,
        "projectID": "project_fixture",
        "cost": 0,
        "tokens": {
            "input": 0,
            "output": 0,
            "reasoning": 0,
            "cache": {"read": 0, "write": 0}
        },
        "time": {"created": timestamp, "updated": timestamp},
        "title": title,
        "location": {"directory": directory}
    })
}

fn serve_events(mut stream: TcpStream, state: Arc<Mutex<FixtureState>>) {
    let (sender, receiver) = mpsc::channel();
    lock(&state).subscribers.push(sender);
    let headers = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n: connected\n\n";
    if stream.write_all(headers).and_then(|_| stream.flush()).is_err() {
        return;
    }
    while let Ok(event) = receiver.recv() {
        if stream
            .write_all(format!("event: message\ndata: {event}\n\n").as_bytes())
            .and_then(|_| stream.flush())
            .is_err()
        {
            return;
        }
    }
}

fn broadcast(state: &Arc<Mutex<FixtureState>>, event: Value) {
    let event = event.to_string();
    lock(state)
        .subscribers
        .retain(|subscriber| subscriber.send(event.clone()).is_ok());
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
