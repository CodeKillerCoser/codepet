use crate::{
    dispatch, JsonLineCodec, JsonRpcInboundRequest, JsonRpcResponse, JsonRpcResponsePayload,
    ProtocolDispatchLane, ProtocolError, ProtocolEvent, ProtocolMethod, ProtocolRequest, ProtocolServer,
    ProviderShutdownRequest, ProviderWireMessage, RpcError, DEFAULT_MAX_JSON_LINE_BYTES,
};
use std::fmt::{Display, Formatter};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

/// Publishes typed Provider notifications without exposing stdout or JSON-RPC framing.
pub trait ProviderEventSink: Send + Sync + 'static {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError>;
}

impl<F> ProviderEventSink for F
where
    F: Fn(ProtocolEvent) -> Result<(), ProtocolError> + Send + Sync + 'static,
{
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        self(event)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdioServerOptions {
    pub max_frame_bytes: usize,
    pub max_concurrent_requests: usize,
    pub max_pending_requests: usize,
    pub max_concurrent_control_requests: usize,
    pub max_pending_control_requests: usize,
    pub dispatch_drain_timeout: Duration,
}

impl Default for StdioServerOptions {
    fn default() -> Self {
        Self {
            max_frame_bytes: DEFAULT_MAX_JSON_LINE_BYTES,
            max_concurrent_requests: 16,
            max_pending_requests: 32,
            max_concurrent_control_requests: 2,
            max_pending_control_requests: 4,
            dispatch_drain_timeout: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdioServerError {
    message: String,
}

impl StdioServerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for StdioServerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StdioServerError {}

impl From<ProtocolError> for StdioServerError {
    fn from(error: ProtocolError) -> Self {
        Self::new(error.message)
    }
}

pub async fn serve_stdio<P, F>(
    options: StdioServerOptions,
    factory: F,
) -> Result<(), StdioServerError>
where
    P: ProtocolServer + 'static,
    F: FnOnce(Arc<dyn ProviderEventSink>) -> P,
{
    serve_stdio_with_io(
        BufReader::new(std::io::stdin()),
        BufWriter::new(std::io::stdout()),
        options,
        factory,
    )
    .await
}

/// Runs the stdio JSON-lines runtime over caller-provided I/O.
///
/// This is public so Provider authors can build process-level conformance tests without
/// reimplementing the transport. Production plugins should normally call [`serve_stdio`].
pub async fn serve_stdio_with_io<R, W, P, F>(
    reader: R,
    writer: W,
    options: StdioServerOptions,
    factory: F,
) -> Result<(), StdioServerError>
where
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
    P: ProtocolServer + 'static,
    F: FnOnce(Arc<dyn ProviderEventSink>) -> P,
{
    validate_options(options)?;
    let codec = JsonLineCodec::new(options.max_frame_bytes)?;
    let writer = Arc::new(Mutex::new(writer));
    let output_unavailable = Arc::new(AtomicBool::new(false));
    let (terminal_sender, terminal_receiver) = mpsc::channel(1);
    let events: Arc<dyn ProviderEventSink> = Arc::new(StdioEventWriter {
        codec,
        writer: writer.clone(),
        terminal: terminal_sender.clone(),
        terminal_signalled: AtomicBool::new(false),
        output_unavailable: output_unavailable.clone(),
    });
    let provider = Arc::new(factory(events));
    let (normal_sender, normal_receiver) = mpsc::channel(options.max_pending_requests);
    let (control_sender, control_receiver) =
        mpsc::channel(options.max_pending_control_requests);
    let reader_writer = writer.clone();
    std::thread::Builder::new()
        .name("codepet-provider-stdio-reader".to_string())
        .spawn(move || {
            read_host_messages(
                reader,
                codec,
                reader_writer,
                normal_sender,
                control_sender,
                terminal_sender,
                options,
            );
        })
        .map_err(|error| StdioServerError::new(format!("start Provider stdio reader: {error}")))?;

    run_service_loop(
        provider,
        codec,
        writer,
        normal_receiver,
        control_receiver,
        terminal_receiver,
        output_unavailable,
        options,
    )
    .await
}

fn validate_options(options: StdioServerOptions) -> Result<(), StdioServerError> {
    for (name, value) in [
        ("max_frame_bytes", options.max_frame_bytes),
        ("max_concurrent_requests", options.max_concurrent_requests),
        ("max_pending_requests", options.max_pending_requests),
        (
            "max_concurrent_control_requests",
            options.max_concurrent_control_requests,
        ),
        (
            "max_pending_control_requests",
            options.max_pending_control_requests,
        ),
    ] {
        if value == 0 {
            return Err(StdioServerError::new(format!(
                "Provider stdio option {name} must be greater than zero"
            )));
        }
    }
    if options.dispatch_drain_timeout.is_zero() {
        return Err(StdioServerError::new(
            "Provider stdio dispatch_drain_timeout must be greater than zero",
        ));
    }
    Ok(())
}

struct StdioEventWriter<W> {
    codec: JsonLineCodec,
    writer: Arc<Mutex<W>>,
    terminal: mpsc::Sender<ReaderTerminal>,
    terminal_signalled: AtomicBool,
    output_unavailable: Arc<AtomicBool>,
}

impl<W> ProviderEventSink for StdioEventWriter<W>
where
    W: Write + Send + 'static,
{
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        if self.output_unavailable.load(Ordering::SeqCst) {
            // Once stdin has terminated there is no Host request stream left to observe
            // cleanup events. Treat them as delivered so Provider lifecycle code can finish
            // without either touching a blocked stdout or mistaking the transport closure for
            // a child-process cleanup failure. A write failure that starts terminal cleanup is
            // still returned by the publish call that actually observed it.
            return Ok(());
        }
        let result = {
            let mut writer = lock(&self.writer);
            self.codec
                .write_message(&mut *writer, &ProviderWireMessage::Event(event))
                .and_then(|()| {
                    writer.flush().map_err(|error| ProtocolError {
                        code: "json_line_flush_failed".to_string(),
                        message: format!("flush Provider event: {error}"),
                        retryable: true,
                        details: None,
                    })
                })
        };
        if let Err(error) = &result {
            self.output_unavailable.store(true, Ordering::SeqCst);
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

fn read_host_messages<R, W>(
    mut reader: R,
    codec: JsonLineCodec,
    writer: Arc<Mutex<W>>,
    normal_requests: mpsc::Sender<ProtocolRequest>,
    control_requests: mpsc::Sender<ProtocolRequest>,
    terminal: mpsc::Sender<ReaderTerminal>,
    options: StdioServerOptions,
) where
    R: BufRead,
    W: Write,
{
    loop {
        match codec.read_message(&mut reader) {
            Ok(Some(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)))) => {
                let (sender, max_concurrent, max_pending) = if is_control_request(&request) {
                    (
                        &control_requests,
                        options.max_concurrent_control_requests,
                        options.max_pending_control_requests,
                    )
                } else {
                    (
                        &normal_requests,
                        options.max_concurrent_requests,
                        options.max_pending_requests,
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
                                message: error.to_string(),
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
                        message: error.to_string(),
                    });
                    return;
                }
            }
            Ok(Some(
                ProviderWireMessage::Response(_)
                | ProviderWireMessage::Notification(_)
                | ProviderWireMessage::Event(_),
            )) => {
                eprintln!("CodePet Provider ignored a non-request Host message");
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

async fn run_service_loop<P, W>(
    provider: Arc<P>,
    codec: JsonLineCodec,
    writer: Arc<Mutex<W>>,
    mut normal_requests: mpsc::Receiver<ProtocolRequest>,
    mut control_requests: mpsc::Receiver<ProtocolRequest>,
    mut terminal: mpsc::Receiver<ReaderTerminal>,
    output_unavailable: Arc<AtomicBool>,
    options: StdioServerOptions,
) -> Result<(), StdioServerError>
where
    P: ProtocolServer + 'static,
    W: Write + Send + 'static,
{
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
                output_unavailable.store(true, Ordering::SeqCst);
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
            request = control_requests.recv(), if control_open && control_tasks.len() < options.max_concurrent_control_requests => {
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
            request = normal_requests.recv(), if normal_open && normal_tasks.len() < options.max_concurrent_requests => {
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
        options.dispatch_drain_timeout,
    )
    .await;
    if let Some((response, message)) = fatal_response {
        if let Err(error) = cleanup_result {
            eprintln!("CodePet Provider cleanup after terminal I/O failed: {error}");
        }
        if let Some(response) = response {
            write_terminal_message_with_timeout(
                codec,
                writer,
                response,
                options.dispatch_drain_timeout,
            )?;
        }
        return Err(StdioServerError::new(format!(
            "terminal Provider I/O failed: {message}"
        )));
    }
    match service_result {
        Err(error) => {
            if let Err(cleanup_error) = cleanup_result {
                eprintln!("CodePet Provider cleanup also failed: {cleanup_error}");
            }
            Err(error)
        }
        Ok(()) => cleanup_result,
    }
}

fn write_terminal_message_with_timeout<W>(
    codec: JsonLineCodec,
    writer: Arc<Mutex<W>>,
    message: ProviderWireMessage,
    timeout: Duration,
) -> Result<(), StdioServerError>
where
    W: Write + Send + 'static,
{
    let (completed, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("codepet-provider-terminal-writer".to_string())
        .spawn(move || {
            let _ = completed.send(write_message(&codec, &writer, message));
        })
        .map_err(|error| {
            StdioServerError::new(format!("start Provider terminal response writer: {error}"))
        })?;
    receiver.recv_timeout(timeout).map_err(|error| {
        StdioServerError::new(format!(
            "write Provider terminal response within cleanup deadline: {error}"
        ))
    })?
}

fn dispatch_completion(
    completed: Result<Result<bool, StdioServerError>, tokio::task::JoinError>,
) -> Result<bool, StdioServerError> {
    match completed {
        Ok(result) => result,
        Err(error) => Err(StdioServerError::new(format!(
            "Provider dispatch task failed: {error}"
        ))),
    }
}

async fn dispatch_host_request<P, W>(
    provider: Arc<P>,
    codec: JsonLineCodec,
    writer: Arc<Mutex<W>>,
    request: ProtocolRequest,
) -> Result<bool, StdioServerError>
where
    P: ProtocolServer,
    W: Write,
{
    let should_stop = request.method() == ProtocolMethod::ProviderShutdown;
    let response = dispatch(provider.as_ref(), request).await;
    write_response(&codec, &writer, response)?;
    Ok(should_stop)
}

fn write_response<W: Write>(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<W>>,
    response: JsonRpcResponse,
) -> Result<(), StdioServerError> {
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
        Err(error) => Err(error.into()),
    }
}

fn response_too_large(
    jsonrpc: String,
    id: Option<String>,
    max_frame_bytes: usize,
) -> JsonRpcResponse {
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

async fn shutdown_and_drain<P>(
    provider: &P,
    normal_tasks: &mut JoinSet<Result<bool, StdioServerError>>,
    control_tasks: &mut JoinSet<Result<bool, StdioServerError>>,
    timeout: Duration,
) -> Result<(), StdioServerError>
where
    P: ProtocolServer,
{
    let mut first_error = ProtocolServer::provider_shutdown(provider, ProviderShutdownRequest {})
        .await
        .err()
        .map(StdioServerError::from);
    let drained = tokio::time::timeout(timeout, async {
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
        let _ = tokio::time::timeout(timeout, async {
            while !normal_tasks.is_empty() || !control_tasks.is_empty() {
                tokio::select! {
                    _ = normal_tasks.join_next(), if !normal_tasks.is_empty() => {}
                    _ = control_tasks.join_next(), if !control_tasks.is_empty() => {}
                }
            }
        })
        .await;
        if first_error.is_none() {
            first_error = Some(StdioServerError::new(
                "timed out draining Provider dispatch tasks",
            ));
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn is_control_request(request: &ProtocolRequest) -> bool {
    request.method().dispatch_lane() == ProtocolDispatchLane::Control
}

fn write_overload_response<W: Write>(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<W>>,
    request: ProtocolRequest,
    max_concurrent: usize,
    max_pending: usize,
) -> Result<(), StdioServerError> {
    let jsonrpc = request.jsonrpc_version().to_string();
    let id = request.id().clone();
    let error = ProtocolError {
        code: "provider_overloaded".to_string(),
        message: "Provider request queue is full".to_string(),
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

fn write_message<W: Write>(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<W>>,
    message: ProviderWireMessage,
) -> Result<(), StdioServerError> {
    let mut writer = lock(writer);
    codec.write_message(&mut *writer, &message)?;
    writer.flush().map_err(|error| {
        StdioServerError::new(format!("flush Provider JSON-line message: {error}"))
    })
}

fn write_frame<W: Write>(
    writer: &Arc<Mutex<W>>,
    frame: &[u8],
) -> Result<(), StdioServerError> {
    let mut writer = lock(writer);
    writer
        .write_all(frame)
        .map_err(|error| StdioServerError::new(format!("write Provider JSON line: {error}")))?;
    writer
        .flush()
        .map_err(|error| StdioServerError::new(format!("flush Provider JSON line: {error}")))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
