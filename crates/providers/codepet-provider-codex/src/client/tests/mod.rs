mod incoming;
mod operations;
mod session;
mod titles;
use super::*;
use crate::protocol::{
    CodexPermissionLevel, CodexThreadActiveFlag, CodexThreadItem, CodexThreadStatus,
    CodexTurnStatus,
};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Barrier;
use std::time::Duration;

struct MockReader {
    receiver: Receiver<Value>,
}

impl JsonRpcReader for MockReader {
    fn read_message(&mut self) -> Result<Option<Value>, CodexAppServerError> {
        match self.receiver.recv() {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        }
    }
}

struct MockWriter {
    sender: Sender<Value>,
}

struct MockByteReader {
    receiver: Receiver<Vec<u8>>,
    current: std::io::Cursor<Vec<u8>>,
}

impl Read for MockByteReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let count = self.current.read(output)?;
            if count > 0 {
                return Ok(count);
            }
            match self.receiver.recv() {
                Ok(bytes) => self.current = std::io::Cursor::new(bytes),
                Err(_) => return Ok(0),
            }
        }
    }
}

impl JsonRpcWriter for MockWriter {
    fn write_message(&mut self, message: &Value) -> Result<(), CodexAppServerError> {
        self.sender
            .send(message.clone())
            .map_err(|error| CodexAppServerError::Io(error.to_string()))
    }
}

struct RecordingControl {
    terminated: Arc<AtomicBool>,
}

impl SessionControl for RecordingControl {
    fn shutdown(&mut self) -> Result<(), CodexAppServerError> {
        self.terminated.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn mock_session() -> (CodexAppServerSession, Receiver<Value>, Sender<Value>) {
    let (to_peer_sender, peer_receiver) = mpsc::channel::<Value>();
    let (peer_sender, client_receiver) = mpsc::channel::<Value>();
    let peer = thread::spawn(move || {
        let initialize = peer_receiver.recv().unwrap();
        assert_eq!(initialize["method"], "initialize");
        assert!(initialize.get("jsonrpc").is_none());
        peer_sender
            .send(json!({
                "id": initialize["id"],
                "result": {
                    "codexHome": "/tmp/codex-home",
                    "platformFamily": "unix",
                    "platformOs": "macos",
                    "userAgent": "codex-cli/fixture"
                }
            }))
            .unwrap();
        let initialized = peer_receiver.recv().unwrap();
        assert_eq!(initialized["method"], "initialized");
        assert!(initialized.get("jsonrpc").is_none());
        (peer_receiver, peer_sender)
    });
    let session = CodexAppServerSession::connect(
        Box::new(MockReader {
            receiver: client_receiver,
        }),
        Box::new(MockWriter {
            sender: to_peer_sender,
        }),
    )
    .unwrap();
    let (peer_receiver, client_sender) = peer.join().unwrap();
    (session, peer_receiver, client_sender)
}

fn thread_fixture(id: &str, cwd: &str, status: &str, turns: Vec<Value>) -> Value {
    let status = if status == "active" {
        json!({ "type": status, "activeFlags": [] })
    } else {
        json!({ "type": status })
    };
    json!({
        "id": id,
        "name": null,
        "preview": "fixture",
        "cwd": cwd,
        "cliVersion": "0.151.0",
        "createdAt": 100,
        "ephemeral": false,
        "modelProvider": "openai",
        "projectId": null,
        "sessionId": format!("session-{id}"),
        "source": "appServer",
        "updatedAt": 101,
        "status": status,
        "turns": turns,
        "ignoredFutureField": { "mustNotLeak": true }
    })
}

fn turn_fixture(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "status": status,
        "startedAt": 100,
        "completedAt": null,
        "items": []
    })
}
