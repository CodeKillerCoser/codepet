use codepet_provider_codex::CodexProvider;
use codepet_provider_sdk::{
    dispatch, JsonLineCodec, JsonRpcInboundRequest, JsonRpcResponse, JsonRpcResponsePayload,
    ProtocolError, ProtocolEvent, ProtocolRequest, ProtocolServer, ProviderShutdownRequest,
    ProviderWireMessage, RpcError, MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES,
};
use std::io::{BufReader, BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;
const MAX_PENDING_HOST_MESSAGES: usize = 32;
const MAX_CONCURRENT_CONTROL_REQUESTS: usize = 2;
const MAX_PENDING_CONTROL_REQUESTS: usize = 4;
const DISPATCH_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

struct StdioEventSink {
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    terminal: mpsc::Sender<ReaderTerminal>,
    terminal_signalled: AtomicBool,
}

impl codepet_provider_codex::ProviderEventSink for StdioEventSink {
    fn publish(&self, event: ProtocolEvent) -> Result<(), codepet_provider_sdk::ProtocolError> {
        let result = {
            let mut writer = lock(&self.writer);
            self.codec
                .write_message(&mut *writer, &ProviderWireMessage::Event(event))
                .and_then(|()| {
                    writer.flush().map_err(|error| codepet_provider_sdk::ProtocolError {
                        code: "json_line_flush_failed".to_string(),
                        message: format!("flush Provider event: {error}"),
                        retryable: true,
                        details: None,
                    })
                })
        };
        if let Err(error) = &result {
            if !self.terminal_signalled.swap(true, Ordering::SeqCst) {
                let _ = self.terminal.try_send(ReaderTerminal::Fatal {
                    response: None,
                    message: error.message.clone(),
                });
            }
        }
        result
    }
}

enum ReaderTerminal {
    Eof,
    Fatal {
        response: Option<ProviderWireMessage>,
        message: String,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Codex Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let codec = JsonLineCodec::new(MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES)
        .map_err(|error| error.message)?;
    let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    let (terminal_sender, terminal_receiver) = mpsc::channel(1);
    let events = Arc::new(StdioEventSink {
        codec,
        writer: writer.clone(),
        terminal: terminal_sender.clone(),
        terminal_signalled: AtomicBool::new(false),
    });
    let provider = Arc::new(CodexProvider::new(events));
    let (normal_sender, normal_receiver) = mpsc::channel(MAX_PENDING_HOST_MESSAGES);
    let (control_sender, control_receiver) = mpsc::channel(MAX_PENDING_CONTROL_REQUESTS);
    let reader_writer = writer.clone();
    std::thread::spawn(move || {
        read_host_messages(
            codec,
            reader_writer,
            normal_sender,
            control_sender,
            terminal_sender,
        );
    });

    run_service_loop(
        provider,
        codec,
        writer,
        normal_receiver,
        control_receiver,
        terminal_receiver,
    )
    .await
}

fn read_host_messages(
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    normal_requests: mpsc::Sender<ProtocolRequest>,
    control_requests: mpsc::Sender<ProtocolRequest>,
    terminal: mpsc::Sender<ReaderTerminal>,
) {
    let mut reader = BufReader::new(std::io::stdin());
    loop {
        match codec.read_message(&mut reader) {
            Ok(Some(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)))) => {
                let (sender, max_concurrent, max_pending) = if is_control_request(&request) {
                    (
                        &control_requests,
                        MAX_CONCURRENT_CONTROL_REQUESTS,
                        MAX_PENDING_CONTROL_REQUESTS,
                    )
                } else {
                    (
                        &normal_requests,
                        MAX_CONCURRENT_HOST_REQUESTS,
                        MAX_PENDING_HOST_MESSAGES,
                    )
                };
                match sender.try_send(request) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(request)) => {
                        if let Err(error) = write_overload_response(
                            &codec,
                            &writer,
                            request,
                            max_concurrent,
                            max_pending,
                        ) {
                            let _ = terminal.try_send(ReaderTerminal::Fatal {
                                response: None,
                                message: error,
                            });
                            return;
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
            Ok(Some(ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(rejection)))) => {
                if let Err(error) = write_message(
                    &codec,
                    &writer,
                    ProviderWireMessage::Response(rejection.into_response()),
                ) {
                    let _ = terminal.try_send(ReaderTerminal::Fatal {
                        response: None,
                        message: error,
                    });
                    return;
                }
            }
            Ok(Some(
                ProviderWireMessage::Response(_)
                | ProviderWireMessage::Notification(_)
                | ProviderWireMessage::Event(_),
            )) => {
                eprintln!("Codex Provider ignored a non-request Host message");
            }
            Ok(None) => {
                let _ = terminal.try_send(ReaderTerminal::Eof);
                return;
            }
            Err(error) => {
                let message = error.error.message.clone();
                let _ = terminal.try_send(ReaderTerminal::Fatal {
                    response: Some(ProviderWireMessage::Response(error.into_response())),
                    message,
                });
                return;
            }
        }
    }
}

async fn run_service_loop(
    provider: Arc<CodexProvider>,
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    mut normal_requests: mpsc::Receiver<ProtocolRequest>,
    mut control_requests: mpsc::Receiver<ProtocolRequest>,
    mut terminal: mpsc::Receiver<ReaderTerminal>,
) -> Result<(), String> {
    let mut normal_tasks = JoinSet::new();
    let mut control_tasks = JoinSet::new();
    let mut service_result = Ok(());
    let mut fatal_response = None;
    let mut normal_open = true;
    let mut control_open = true;

    loop {
        tokio::select! {
            biased;
            terminal_message = terminal.recv() => {
                match terminal_message.unwrap_or(ReaderTerminal::Eof) {
                    ReaderTerminal::Eof => {}
                    ReaderTerminal::Fatal { response, message } => {
                        fatal_response = Some((response, message));
                    }
                }
                break;
            }
            completed = control_tasks.join_next(), if !control_tasks.is_empty() => {
                match dispatch_completion(completed.expect("control dispatch set is not empty")) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => {
                        service_result = Err(error);
                        break;
                    }
                }
            }
            request = control_requests.recv(), if control_open && control_tasks.len() < MAX_CONCURRENT_CONTROL_REQUESTS => {
                let Some(request) = request else {
                    control_open = false;
                    continue;
                };
                let task_provider = provider.clone();
                let task_writer = writer.clone();
                control_tasks.spawn(async move {
                    dispatch_host_request(task_provider, codec, task_writer, request).await
                });
            }
            completed = normal_tasks.join_next(), if !normal_tasks.is_empty() => {
                match dispatch_completion(completed.expect("normal dispatch set is not empty")) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => {
                        service_result = Err(error);
                        break;
                    }
                }
            }
            request = normal_requests.recv(), if normal_open && normal_tasks.len() < MAX_CONCURRENT_HOST_REQUESTS => {
                let Some(request) = request else {
                    normal_open = false;
                    continue;
                };
                let task_provider = provider.clone();
                let task_writer = writer.clone();
                normal_tasks.spawn(async move {
                    dispatch_host_request(task_provider, codec, task_writer, request).await
                });
            }
        }
    }

    let cleanup_result = shutdown_and_drain(
        provider.as_ref(),
        &mut normal_tasks,
        &mut control_tasks,
    )
    .await;
    if let Some((response, message)) = fatal_response {
        if let Err(error) = cleanup_result {
            eprintln!("Codex Provider cleanup after terminal I/O failed: {error}");
        }
        if let Some(response) = response {
            write_message(&codec, &writer, response)?;
        }
        return Err(format!("terminal Provider I/O failed: {message}"));
    }
    match service_result {
        Err(error) => {
            if let Err(cleanup_error) = cleanup_result {
                eprintln!("Codex Provider cleanup also failed: {cleanup_error}");
            }
            Err(error)
        }
        Ok(()) => cleanup_result,
    }
}

fn dispatch_completion(
    completed: Result<Result<bool, String>, tokio::task::JoinError>,
) -> Result<bool, String> {
    match completed {
        Ok(result) => result,
        Err(error) => Err(format!("Provider dispatch task failed: {error}")),
    }
}

async fn dispatch_host_request(
    provider: Arc<CodexProvider>,
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    request: ProtocolRequest,
) -> Result<bool, String> {
    let response = dispatch(provider.as_ref(), request).await;
    write_response(&codec, &writer, response)?;
    Ok(provider.is_shutdown())
}

fn write_response(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<BufWriter<std::io::Stdout>>>,
    response: JsonRpcResponse,
) -> Result<(), String> {
    let jsonrpc = response.jsonrpc.clone();
    let id = response.id.clone();
    let message = ProviderWireMessage::Response(response);
    match codec.encode_message(&message) {
        Ok(frame) => write_frame(writer, &frame),
        Err(error) if error.code == "json_line_frame_too_large" => {
            drop(message);
            write_message(
                codec,
                writer,
                ProviderWireMessage::Response(response_too_large(
                    jsonrpc,
                    id,
                    codec.max_frame_bytes(),
                )),
            )
        }
        Err(error) => Err(error.message),
    }
}

fn response_too_large(jsonrpc: String, id: Option<String>, max_frame_bytes: usize) -> JsonRpcResponse {
    let error = ProtocolError {
        code: "provider_response_too_large".to_string(),
        message: format!(
            "Provider response exceeds the {max_frame_bytes}-byte JSON-line limit"
        ),
        retryable: false,
        details: Some(
            [("maxFrameBytes".to_string(), serde_json::json!(max_frame_bytes))]
                .into_iter()
                .collect(),
        ),
    };
    let message = error.message.clone();
    let data = serde_json::to_value(error)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|entries| entries.into_iter().collect());
    JsonRpcResponse {
        jsonrpc,
        id,
        response: JsonRpcResponsePayload::Error {
            error: RpcError {
                code: -32000,
                message,
                data,
            },
        },
    }
}

async fn shutdown_and_drain(
    provider: &CodexProvider,
    normal_tasks: &mut JoinSet<Result<bool, String>>,
    control_tasks: &mut JoinSet<Result<bool, String>>,
) -> Result<(), String> {
    let mut first_error = None;
    if !provider.is_shutdown() {
        if let Err(error) = ProtocolServer::provider_shutdown(provider, ProviderShutdownRequest {}).await {
            first_error = Some(error.message);
        }
    }
    let drained = tokio::time::timeout(DISPATCH_DRAIN_TIMEOUT, async {
        while !normal_tasks.is_empty() || !control_tasks.is_empty() {
            tokio::select! {
                completed = normal_tasks.join_next(), if !normal_tasks.is_empty() => {
                    if let Err(error) = dispatch_completion(
                        completed.expect("normal dispatch set is not empty"),
                    ) {
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
                completed = control_tasks.join_next(), if !control_tasks.is_empty() => {
                    if let Err(error) = dispatch_completion(
                        completed.expect("control dispatch set is not empty"),
                    ) {
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
            }
        }
    })
    .await
    .is_ok();
    if !drained {
        normal_tasks.abort_all();
        control_tasks.abort_all();
        let _ = tokio::time::timeout(DISPATCH_DRAIN_TIMEOUT, async {
            while !normal_tasks.is_empty() || !control_tasks.is_empty() {
                tokio::select! {
                    _ = normal_tasks.join_next(), if !normal_tasks.is_empty() => {}
                    _ = control_tasks.join_next(), if !control_tasks.is_empty() => {}
                }
            }
        })
        .await;
        if first_error.is_none() {
            first_error = Some("timed out draining Provider dispatch tasks".to_string());
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn is_control_request(request: &ProtocolRequest) -> bool {
    matches!(
        request,
        ProtocolRequest::InstanceStop { .. }
            | ProtocolRequest::InstanceDestroy { .. }
            | ProtocolRequest::ProviderShutdown { .. }
    )
}

fn write_overload_response(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<BufWriter<std::io::Stdout>>>,
    request: ProtocolRequest,
    max_concurrent: usize,
    max_pending: usize,
) -> Result<(), String> {
    let (jsonrpc, id) = request_envelope(request);
    let error = ProtocolError {
        code: "provider_overloaded".to_string(),
        message: "Codex Provider request queue is full".to_string(),
        retryable: true,
        details: Some(
            [
                (
                    "maxConcurrentRequests".to_string(),
                    serde_json::json!(max_concurrent),
                ),
                (
                    "maxPendingRequests".to_string(),
                    serde_json::json!(max_pending),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    };
    let message = error.message.clone();
    let data = serde_json::to_value(error)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|entries| entries.into_iter().collect());
    write_message(
        codec,
        writer,
        ProviderWireMessage::Response(JsonRpcResponse {
            jsonrpc,
            id: Some(id),
            response: JsonRpcResponsePayload::Error {
                error: RpcError {
                    code: -32000,
                    message,
                    data,
                },
            },
        }),
    )
}

fn request_envelope(request: ProtocolRequest) -> (String, String) {
    match request {
        ProtocolRequest::ProviderInitialize { jsonrpc, id, .. }
        | ProtocolRequest::ProviderDescribe { jsonrpc, id, .. }
        | ProtocolRequest::InstanceCreate { jsonrpc, id, .. }
        | ProtocolRequest::InstanceStart { jsonrpc, id, .. }
        | ProtocolRequest::InstanceStop { jsonrpc, id, .. }
        | ProtocolRequest::InstanceDestroy { jsonrpc, id, .. }
        | ProtocolRequest::InstanceCapabilities { jsonrpc, id, .. }
        | ProtocolRequest::ConversationList { jsonrpc, id, .. }
        | ProtocolRequest::ConversationSearch { jsonrpc, id, .. }
        | ProtocolRequest::ConversationGet { jsonrpc, id, .. }
        | ProtocolRequest::ConversationCreate { jsonrpc, id, .. }
        | ProtocolRequest::TurnStart { jsonrpc, id, .. }
        | ProtocolRequest::TurnSteer { jsonrpc, id, .. }
        | ProtocolRequest::TurnInterrupt { jsonrpc, id, .. }
        | ProtocolRequest::ApprovalResolve { jsonrpc, id, .. }
        | ProtocolRequest::ProviderShutdown { jsonrpc, id, .. } => (jsonrpc, id),
    }
}

fn write_message(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<BufWriter<std::io::Stdout>>>,
    message: ProviderWireMessage,
) -> Result<(), String> {
    let mut writer = lock(writer);
    codec
        .write_message(&mut *writer, &message)
        .map_err(|error| error.message)?;
    writer.flush().map_err(|error| error.to_string())
}

fn write_frame(
    writer: &Arc<Mutex<BufWriter<std::io::Stdout>>>,
    frame: &[u8],
) -> Result<(), String> {
    let mut writer = lock(writer);
    writer.write_all(frame).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
