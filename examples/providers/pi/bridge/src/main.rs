//! Validation bridge, deliberately outside the public SDK until the pi experiment is assessed.
//! Host-facing framing/heartbeat/control lanes stay in the distributed Rust SDK.
use codepet_provider_sdk::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tokio::sync::{oneshot, watch};

const MAX_RECORD: u64 = 16 * 1024 * 1024;
type Reply = Result<Value, ProtocolError>;
struct State {
    pending: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
    events: OnceLock<Arc<dyn ProviderEventSink>>,
}
struct Bridge {
    state: Arc<State>,
    output: std::sync::mpsc::SyncSender<Vec<u8>>,
    sequence: AtomicU64,
}
fn error(message: impl ToString) -> ProtocolError {
    ProtocolError {
        code: "provider_worker_failed".into(),
        message: message.to_string(),
        retryable: false,
        details: None,
    }
}
struct Pending<'a>(&'a State, String);
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.0.pending.lock().unwrap().remove(&self.1);
    }
}
impl Bridge {
    async fn call<Q: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        request: Q,
    ) -> Result<R, ProtocolError> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed).to_string();
        let mut record =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":request}))
                .map_err(error)?;
        if record.len() as u64 > MAX_RECORD {
            return Err(error("Worker request exceeds record limit"));
        }
        record.push(b'\n');
        let (tx, rx) = oneshot::channel();
        self.state.pending.lock().unwrap().insert(id.clone(), tx);
        let _pending = Pending(&self.state, id);
        self.output.try_send(record).map_err(error)?;
        let value = tokio::time::timeout(Duration::from_secs(55), rx)
            .await
            .map_err(error)?
            .map_err(error)??;
        // Keep the typed SDK boundary: invalid worker results never reach Host as valid responses.
        serde_json::from_value(value).map_err(error)
    }
}
include!(concat!(env!("OUT_DIR"), "/forward.rs"));

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let program = arguments
        .next()
        .ok_or("Usage: bridge <worker executable> [arguments...]")?;
    let package = std::env::current_exe()?
        .parent()
        .ok_or("Missing package directory")?
        .to_owned();
    let mut child = process::Command::new(program)
        .args(arguments)
        .current_dir(package)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let state = Arc::new(State {
        pending: Mutex::new(HashMap::new()),
        events: OnceLock::new(),
    });
    let (output, outgoing) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
    let (ended, mut exited) = watch::channel(false);
    let writer_ended = ended.clone();
    std::thread::spawn(move || {
        for record in outgoing {
            if input
                .write_all(&record)
                .and_then(|_| input.flush())
                .is_err()
            {
                break;
            }
        }
        writer_ended.send_replace(true);
    });
    let reader_state = state.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let result = (|| -> Result<(), ProtocolError> {
            loop {
                let mut record = Vec::new();
                let count = reader
                    .by_ref()
                    .take(MAX_RECORD + 1)
                    .read_until(b'\n', &mut record)
                    .map_err(error)?;
                if count == 0 {
                    return Err(error("Worker stdout closed"));
                }
                if count as u64 > MAX_RECORD || record.last() != Some(&b'\n') {
                    return Err(error("Invalid or oversized worker record"));
                }
                let value: Value = serde_json::from_slice(&record).map_err(error)?;
                if value["jsonrpc"] != "2.0" {
                    return Err(error("Invalid worker JSON-RPC version"));
                }
                if value.get("method").is_some() {
                    let event = serde_json::from_value(value).map_err(error)?;
                    reader_state
                        .events
                        .get()
                        .ok_or_else(|| error("Event before runtime ready"))?
                        .publish(event)?;
                } else {
                    let id = value["id"]
                        .as_str()
                        .ok_or_else(|| error("Missing worker response ID"))?;
                    let reply = if let Some(failure) = value.get("error") {
                        Err(failure
                            .get("data")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_else(|| {
                                error(
                                    failure["message"]
                                        .as_str()
                                        .unwrap_or("Worker request failed"),
                                )
                            }))
                    } else {
                        Ok(value
                            .get("result")
                            .cloned()
                            .ok_or_else(|| error("Missing worker result"))?)
                    };
                    if let Some(sender) = reader_state.pending.lock().unwrap().remove(id) {
                        let _ = sender.send(reply);
                    }
                }
            }
        })();
        if let Err(failure) = result {
            eprintln!("{}", failure.message);
        }
        reader_state.pending.lock().unwrap().clear();
        ended.send_replace(true);
    });
    let runtime = serve_stdio(StdioServerOptions::default(), move |events| {
        let _ = state.events.set(events);
        Bridge {
            state,
            output,
            sequence: AtomicU64::new(1),
        }
    });
    tokio::pin!(runtime);
    let result = tokio::select! {
        result = &mut runtime => result.map_err(|e| e.to_string()),
        _ = exited.changed() => Err("Provider worker exited".to_string()),
    };
    // The SDK process owner kills the worker's descendants as well.
    child.kill()?;
    child.wait()?;
    result.map_err(Into::into)
}
