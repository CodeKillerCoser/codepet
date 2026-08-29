use crate::catalog::PluginDescriptor;
use crate::{HostError, HostResult};
use codepet_provider_sdk::{
    ApprovalResolveRequest, ConversationCreateRequest, ConversationGetRequest,
    ConversationListRequest, InstanceCapabilitiesRequest, InstanceCreateRequest,
    InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest, JsonLineCodec,
    JsonRpcInboundError, JsonRpcInboundRequest, JsonRpcResponsePayload, ProtocolClient,
    ProtocolError, ProtocolInboundFuture, ProtocolMethod, ProtocolRequest, ProtocolTransport,
    ProtocolTransportFuture, ProviderDescribeRequest,
    ProviderShutdownRequest, ProviderWireMessage, RequestId, RpcError,
    TurnInterruptRequest, TurnStartRequest, TurnSteerRequest,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex};

#[derive(Clone, Debug)]
pub struct PluginProcessOptions {
    pub max_frame_bytes: usize,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub outbound_capacity: usize,
    pub inbound_capacity: usize,
    pub stderr_line_bytes: usize,
    pub stderr_history_lines: usize,
}

impl Default for PluginProcessOptions {
    fn default() -> Self {
        Self {
            max_frame_bytes: codepet_provider_sdk::DEFAULT_MAX_JSON_LINE_BYTES,
            request_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(3),
            outbound_capacity: 64,
            inbound_capacity: 256,
            stderr_line_bytes: 16 * 1024,
            stderr_history_lines: 128,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginProcessExit {
    pub success: bool,
    pub code: Option<i32>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StderrDiagnostic {
    pub timestamp_ms: u64,
    pub line: String,
    pub truncated: bool,
}

#[derive(Clone)]
pub struct PluginProcessDiagnostics {
    inner: Arc<StdMutex<VecDeque<StderrDiagnostic>>>,
}

impl PluginProcessDiagnostics {
    pub fn snapshot(&self) -> Vec<StderrDiagnostic> {
        self.inner
            .lock()
            .map(|diagnostics| diagnostics.iter().cloned().collect())
            .unwrap_or_default()
    }
}

enum WriterCommand {
    Frame(Vec<u8>),
    Close,
}

enum ProcessCommand {
    Kill { reason: String },
}

struct RpcShared {
    codec: JsonLineCodec,
    writer: mpsc::Sender<WriterCommand>,
    pending: Mutex<HashMap<RequestId, oneshot::Sender<Result<Value, ProtocolError>>>>,
    inbound: broadcast::Sender<ProviderWireMessage>,
    next_request_id: AtomicU64,
    request_timeout: Duration,
    closed: AtomicBool,
    close_error: StdMutex<Option<ProtocolError>>,
}

impl RpcShared {
    async fn close(&self, error: ProtocolError) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            if let Ok(mut close_error) = self.close_error.lock() {
                *close_error = Some(error.clone());
            }
        }
        let mut pending = self.pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(error.clone()));
        }
    }

    fn closed_error(&self) -> ProtocolError {
        self.close_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .unwrap_or_else(|| protocol_error(
                "provider_process_closed",
                "Provider process transport is closed",
                true,
            ))
    }
}

pub struct ProviderRpcClient {
    shared: Arc<RpcShared>,
    inbound: Mutex<broadcast::Receiver<ProviderWireMessage>>,
}

impl ProviderRpcClient {
    fn new(shared: Arc<RpcShared>) -> Self {
        Self {
            inbound: Mutex::new(shared.inbound.subscribe()),
            shared,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProviderWireMessage> {
        self.shared.inbound.subscribe()
    }

    async fn request_value(
        &self,
        method: ProtocolMethod,
        params: Value,
    ) -> Result<Value, ProtocolError> {
        if self.shared.closed.load(Ordering::SeqCst) {
            return Err(self.shared.closed_error());
        }
        let sequence = self.shared.next_request_id.fetch_add(1, Ordering::SeqCst);
        let request_id = format!("host-{sequence}");
        let request = typed_request(method, request_id.clone(), params)?;
        let message = ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request));
        let frame = self.shared.codec.encode_message(&message)?;
        let (response_sender, response_receiver) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .await
            .insert(request_id.clone(), response_sender);

        let send = self.shared.writer.send(WriterCommand::Frame(frame));
        match tokio::time::timeout(self.shared.request_timeout, send).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                self.shared.pending.lock().await.remove(&request_id);
                return Err(self.shared.closed_error());
            }
            Err(_) => {
                self.shared.pending.lock().await.remove(&request_id);
                return Err(request_timeout_error(method, &request_id, "write queue"));
            }
        }

        match tokio::time::timeout(self.shared.request_timeout, response_receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(self.shared.closed_error()),
            Err(_) => {
                self.shared.pending.lock().await.remove(&request_id);
                Err(request_timeout_error(method, &request_id, "response"))
            }
        }
    }

    async fn close_writer(&self) -> Result<(), ProtocolError> {
        match tokio::time::timeout(
            self.shared.request_timeout,
            self.shared.writer.send(WriterCommand::Close),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(self.shared.closed_error()),
            Err(_) => Err(protocol_error(
                "provider_writer_close_timeout",
                "timed out closing Provider stdin",
                true,
            )),
        }
    }
}

impl ProtocolTransport for ProviderRpcClient {
    fn request<'a>(
        &'a self,
        method: ProtocolMethod,
        params: Value,
    ) -> ProtocolTransportFuture<'a> {
        Box::pin(async move { self.request_value(method, params).await })
    }

    fn next_message<'a>(&'a self) -> ProtocolInboundFuture<'a> {
        Box::pin(async move {
            let mut receiver = self.inbound.lock().await;
            match receiver.recv().await {
                Ok(message) => Ok(message),
                Err(broadcast::error::RecvError::Lagged(skipped)) => Err(inbound_transport_error(
                    format!("Provider inbound consumer skipped {skipped} messages"),
                )),
                Err(broadcast::error::RecvError::Closed) => {
                    Err(inbound_transport_error(self.shared.closed_error().message))
                }
            }
        })
    }
}

pub struct PluginProcess {
    plugin_id: String,
    client: ProtocolClient<ProviderRpcClient>,
    control: mpsc::Sender<ProcessCommand>,
    exit: watch::Receiver<Option<PluginProcessExit>>,
    diagnostics: PluginProcessDiagnostics,
    options: PluginProcessOptions,
}

impl PluginProcess {
    pub async fn spawn(
        descriptor: &PluginDescriptor,
        options: PluginProcessOptions,
    ) -> HostResult<Self> {
        validate_executable(&descriptor.executable)?;
        let codec = JsonLineCodec::new(options.max_frame_bytes).map_err(HostError::from)?;
        let mut command = Command::new(&descriptor.executable);
        command
            .args(&descriptor.args)
            .envs(&descriptor.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            HostError::new(
                "provider_spawn_failed",
                format!(
                    "spawn Provider plugin {} from {}: {error}",
                    descriptor.plugin_id,
                    descriptor.executable.display()
                ),
            )
            .retryable(true)
            .with_detail("pluginId", descriptor.plugin_id.clone())
            .with_detail("executable", descriptor.executable.display().to_string())
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            HostError::new(
                "provider_spawn_failed",
                "Provider process did not expose piped stdin",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            HostError::new(
                "provider_spawn_failed",
                "Provider process did not expose piped stdout",
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            HostError::new(
                "provider_spawn_failed",
                "Provider process did not expose piped stderr",
            )
        })?;

        let (writer_sender, writer_receiver) = mpsc::channel(options.outbound_capacity.max(1));
        let (inbound_sender, _) = broadcast::channel(options.inbound_capacity.max(1));
        let (control_sender, control_receiver) = mpsc::channel(4);
        let (exit_sender, exit_receiver) = watch::channel(None);
        let diagnostics = PluginProcessDiagnostics {
            inner: Arc::new(StdMutex::new(VecDeque::with_capacity(
                options.stderr_history_lines.max(1),
            ))),
        };
        let shared = Arc::new(RpcShared {
            codec,
            writer: writer_sender,
            pending: Mutex::new(HashMap::new()),
            inbound: inbound_sender,
            next_request_id: AtomicU64::new(1),
            request_timeout: options.request_timeout,
            closed: AtomicBool::new(false),
            close_error: StdMutex::new(None),
        });

        tokio::spawn(writer_loop(
            stdin,
            writer_receiver,
            shared.clone(),
            control_sender.clone(),
        ));
        tokio::spawn(reader_loop(
            stdout,
            shared.clone(),
            control_sender.clone(),
        ));
        tokio::spawn(stderr_loop(
            stderr,
            diagnostics.inner.clone(),
            options.stderr_line_bytes.max(1),
            options.stderr_history_lines.max(1),
        ));
        tokio::spawn(process_monitor(
            child,
            control_receiver,
            exit_sender,
            shared.clone(),
        ));

        Ok(Self {
            plugin_id: descriptor.plugin_id.clone(),
            client: ProtocolClient::new(ProviderRpcClient::new(shared)),
            control: control_sender,
            exit: exit_receiver,
            diagnostics,
            options,
        })
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn client(&self) -> &ProtocolClient<ProviderRpcClient> {
        &self.client
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProviderWireMessage> {
        self.client.transport().subscribe()
    }

    pub fn exit_receiver(&self) -> watch::Receiver<Option<PluginProcessExit>> {
        self.exit.clone()
    }

    pub fn exit_status(&self) -> Option<PluginProcessExit> {
        self.exit.borrow().clone()
    }

    pub fn stderr_diagnostics(&self) -> Vec<StderrDiagnostic> {
        self.diagnostics.snapshot()
    }

    pub fn diagnostics(&self) -> PluginProcessDiagnostics {
        self.diagnostics.clone()
    }

    pub async fn shutdown(&self) -> HostResult<PluginProcessExit> {
        if let Some(exit) = self.exit_status() {
            return Ok(exit);
        }
        let shutdown_result = match self
            .client
            .provider_shutdown(ProviderShutdownRequest {})
            .await
        {
            Ok(response) if response.accepted => Ok(()),
            Ok(_) => Err(HostError::new(
                "provider_shutdown_declined",
                "Provider declined the shutdown request",
            )),
            Err(error) => Err(HostError::from(error)),
        };
        let _ = self.client.transport().close_writer().await;
        if let Some(exit) = wait_for_exit(&self.exit, self.options.shutdown_timeout).await {
            shutdown_result?;
            return Ok(exit);
        }
        let exit = self.kill("Provider did not exit after shutdown response").await?;
        shutdown_result?;
        Ok(exit)
    }

    pub async fn kill(&self, reason: impl Into<String>) -> HostResult<PluginProcessExit> {
        let reason = reason.into();
        if let Some(exit) = self.exit_status() {
            return Ok(exit);
        }
        if self
            .control
            .send(ProcessCommand::Kill { reason })
            .await
            .is_err()
        {
            if let Some(exit) = wait_for_exit(&self.exit, self.options.shutdown_timeout).await {
                return Ok(exit);
            }
            return Err(HostError::new(
                "provider_process_control_closed",
                "Provider process control channel is closed",
            ));
        }
        wait_for_exit(&self.exit, self.options.shutdown_timeout)
            .await
            .ok_or_else(|| {
                HostError::new(
                    "provider_kill_timeout",
                    "Provider process did not exit after kill",
                )
                .retryable(true)
            })
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        let _ = self.control.try_send(ProcessCommand::Kill {
            reason: "Provider process handle dropped".to_string(),
        });
    }
}

async fn writer_loop(
    mut stdin: ChildStdin,
    mut receiver: mpsc::Receiver<WriterCommand>,
    shared: Arc<RpcShared>,
    control: mpsc::Sender<ProcessCommand>,
) {
    while let Some(command) = receiver.recv().await {
        let result = match command {
            WriterCommand::Frame(frame) => async {
                stdin.write_all(&frame).await?;
                stdin.flush().await
            }
            .await,
            WriterCommand::Close => {
                let result = stdin.shutdown().await;
                if result.is_err() {
                    let _ = control
                        .send(ProcessCommand::Kill {
                            reason: "failed to close Provider stdin".to_string(),
                        })
                        .await;
                }
                return;
            }
        };
        if let Err(error) = result {
            let protocol_error = protocol_error(
                "provider_stdin_write_failed",
                format!("write Provider stdin: {error}"),
                true,
            );
            shared.close(protocol_error.clone()).await;
            let _ = control
                .send(ProcessCommand::Kill {
                    reason: protocol_error.message,
                })
                .await;
            return;
        }
    }
}

async fn reader_loop(
    stdout: ChildStdout,
    shared: Arc<RpcShared>,
    control: mpsc::Sender<ProcessCommand>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let frame = match read_bounded_frame(
            &mut reader,
            shared.codec.max_frame_bytes(),
            "Provider stdout",
        )
        .await
        {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                let error = protocol_error(
                    "provider_stdout_eof",
                    "Provider stdout reached EOF",
                    true,
                );
                shared.close(error.clone()).await;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let _ = control
                        .send(ProcessCommand::Kill {
                            reason: error.message,
                        })
                        .await;
                });
                return;
            }
            Err(error) => {
                shared.close(error.clone()).await;
                let _ = control
                    .send(ProcessCommand::Kill {
                        reason: error.message,
                    })
                    .await;
                return;
            }
        };
        let message = match shared.codec.decode_line(&frame) {
            Ok(message) => message,
            Err(error) => {
                let error = inbound_error_to_protocol(error);
                shared.close(error.clone()).await;
                let _ = control
                    .send(ProcessCommand::Kill {
                        reason: error.message,
                    })
                    .await;
                return;
            }
        };
        match message {
            ProviderWireMessage::Response(response) => {
                let Some(id) = response.id else {
                    let error = protocol_error(
                        "provider_response_missing_id",
                        "Provider response did not contain a request id",
                        false,
                    );
                    shared.close(error.clone()).await;
                    let _ = control
                        .send(ProcessCommand::Kill {
                            reason: error.message,
                        })
                        .await;
                    return;
                };
                let sender = shared.pending.lock().await.remove(&id);
                if let Some(sender) = sender {
                    let result = match response.response {
                        JsonRpcResponsePayload::Ok { result } => Ok(result),
                        JsonRpcResponsePayload::Error { error } => Err(rpc_error_to_protocol(error)),
                    };
                    let _ = sender.send(result);
                }
            }
            ProviderWireMessage::Event(event) => {
                let _ = shared.inbound.send(ProviderWireMessage::Event(event));
            }
            ProviderWireMessage::Notification(notification) => {
                let _ = shared
                    .inbound
                    .send(ProviderWireMessage::Notification(notification));
            }
            ProviderWireMessage::Request(_) => {
                let error = protocol_error(
                    "unexpected_provider_request",
                    "Provider sent a host-to-plugin request on stdout",
                    false,
                );
                shared.close(error.clone()).await;
                let _ = control
                    .send(ProcessCommand::Kill {
                        reason: error.message,
                    })
                    .await;
                return;
            }
        }
    }
}

async fn stderr_loop(
    stderr: ChildStderr,
    diagnostics: Arc<StdMutex<VecDeque<StderrDiagnostic>>>,
    line_limit: usize,
    history_limit: usize,
) {
    let mut reader = BufReader::new(stderr);
    while let Ok(Some((line, truncated))) = read_diagnostic_line(&mut reader, line_limit).await {
        if let Ok(mut diagnostics) = diagnostics.lock() {
            diagnostics.push_back(StderrDiagnostic {
                timestamp_ms: now_ms(),
                line,
                truncated,
            });
            while diagnostics.len() > history_limit {
                diagnostics.pop_front();
            }
        }
    }
}

async fn process_monitor(
    mut child: tokio::process::Child,
    mut control: mpsc::Receiver<ProcessCommand>,
    exit_sender: watch::Sender<Option<PluginProcessExit>>,
    shared: Arc<RpcShared>,
) {
    let exit = tokio::select! {
        status = child.wait() => match status {
            Ok(status) => PluginProcessExit {
                success: status.success(),
                code: status.code(),
                reason: if status.success() {
                    None
                } else {
                    Some(format!("Provider process exited with status: {status}"))
                },
            },
            Err(error) => PluginProcessExit {
                success: false,
                code: None,
                reason: Some(format!("wait for Provider process: {error}")),
            },
        },
        command = control.recv() => match command {
            Some(ProcessCommand::Kill { reason }) => {
                let kill_error = child.kill().await.err();
                let status = child.wait().await.ok();
                PluginProcessExit {
                    success: false,
                    code: status.and_then(|status| status.code()),
                    reason: Some(match kill_error {
                        Some(error) => format!("{reason}; kill failed: {error}"),
                        None => reason,
                    }),
                }
            }
            None => {
                let _ = child.kill().await;
                let status = child.wait().await.ok();
                PluginProcessExit {
                    success: false,
                    code: status.and_then(|status| status.code()),
                    reason: Some("Provider process control channel closed".to_string()),
                }
            }
        },
    };
    shared
        .close(protocol_error(
            "provider_process_exited",
            exit.reason
                .clone()
                .unwrap_or_else(|| "Provider process exited".to_string()),
            !exit.success,
        ))
        .await;
    let _ = exit_sender.send(Some(exit));
}

async fn wait_for_exit(
    receiver: &watch::Receiver<Option<PluginProcessExit>>,
    timeout: Duration,
) -> Option<PluginProcessExit> {
    if let Some(exit) = receiver.borrow().clone() {
        return Some(exit);
    }
    let mut receiver = receiver.clone();
    let wait = async move {
        loop {
            if receiver.changed().await.is_err() {
                return None;
            }
            if let Some(exit) = receiver.borrow().clone() {
                return Some(exit);
            }
        }
    };
    tokio::time::timeout(timeout, wait).await.ok().flatten()
}

async fn read_bounded_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
    source: &str,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    let mut frame = Vec::with_capacity(limit.min(8192));
    loop {
        let (consumed, complete) = {
            let available = reader.fill_buf().await.map_err(|error| {
                protocol_error(
                    "provider_frame_read_failed",
                    format!("read {source}: {error}"),
                    true,
                )
            })?;
            if available.is_empty() {
                if frame.is_empty() {
                    return Ok(None);
                }
                if frame.ends_with(b"\r") {
                    frame.pop();
                }
                return Ok(Some(frame));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let payload_bytes = newline.unwrap_or(available.len());
            if frame.len().saturating_add(payload_bytes) > limit {
                return Err(protocol_error(
                    "provider_frame_too_large",
                    format!("{source} JSON line exceeds {limit} bytes"),
                    false,
                ));
            }
            frame.extend_from_slice(&available[..payload_bytes]);
            (
                newline.map_or(payload_bytes, |index| index + 1),
                newline.is_some(),
            )
        };
        reader.consume(consumed);
        if complete {
            if frame.ends_with(b"\r") {
                frame.pop();
            }
            return Ok(Some(frame));
        }
    }
}

async fn read_diagnostic_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<(String, bool)>> {
    let mut payload = Vec::with_capacity(limit.min(1024));
    let mut truncated = false;
    loop {
        let (consumed, complete) = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                if payload.is_empty() && !truncated {
                    return Ok(None);
                }
                return Ok(Some((String::from_utf8_lossy(&payload).into_owned(), truncated)));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let payload_bytes = newline.unwrap_or(available.len());
            let remaining = limit.saturating_sub(payload.len());
            let retained = payload_bytes.min(remaining);
            payload.extend_from_slice(&available[..retained]);
            if retained < payload_bytes {
                truncated = true;
            }
            (
                newline.map_or(payload_bytes, |index| index + 1),
                newline.is_some(),
            )
        };
        reader.consume(consumed);
        if complete {
            if payload.ends_with(b"\r") {
                payload.pop();
            }
            return Ok(Some((String::from_utf8_lossy(&payload).into_owned(), truncated)));
        }
    }
}

fn typed_request(
    method: ProtocolMethod,
    id: RequestId,
    params: Value,
) -> Result<ProtocolRequest, ProtocolError> {
    let jsonrpc = "2.0".to_string();
    match method {
        ProtocolMethod::ProviderInitialize => Ok(ProtocolRequest::ProviderInitialize {
            jsonrpc,
            id,
            params: decode_params(method, params)?,
        }),
        ProtocolMethod::ProviderDescribe => Ok(ProtocolRequest::ProviderDescribe {
            jsonrpc,
            id,
            params: decode_params::<ProviderDescribeRequest>(method, params)?,
        }),
        ProtocolMethod::InstanceCreate => Ok(ProtocolRequest::InstanceCreate {
            jsonrpc,
            id,
            params: decode_params::<InstanceCreateRequest>(method, params)?,
        }),
        ProtocolMethod::InstanceStart => Ok(ProtocolRequest::InstanceStart {
            jsonrpc,
            id,
            params: decode_params::<InstanceStartRequest>(method, params)?,
        }),
        ProtocolMethod::InstanceStop => Ok(ProtocolRequest::InstanceStop {
            jsonrpc,
            id,
            params: decode_params::<InstanceStopRequest>(method, params)?,
        }),
        ProtocolMethod::InstanceDestroy => Ok(ProtocolRequest::InstanceDestroy {
            jsonrpc,
            id,
            params: decode_params::<InstanceDestroyRequest>(method, params)?,
        }),
        ProtocolMethod::InstanceCapabilities => Ok(ProtocolRequest::InstanceCapabilities {
            jsonrpc,
            id,
            params: decode_params::<InstanceCapabilitiesRequest>(method, params)?,
        }),
        ProtocolMethod::ConversationList => Ok(ProtocolRequest::ConversationList {
            jsonrpc,
            id,
            params: decode_params::<ConversationListRequest>(method, params)?,
        }),
        ProtocolMethod::ConversationGet => Ok(ProtocolRequest::ConversationGet {
            jsonrpc,
            id,
            params: decode_params::<ConversationGetRequest>(method, params)?,
        }),
        ProtocolMethod::ConversationCreate => Ok(ProtocolRequest::ConversationCreate {
            jsonrpc,
            id,
            params: decode_params::<ConversationCreateRequest>(method, params)?,
        }),
        ProtocolMethod::TurnStart => Ok(ProtocolRequest::TurnStart {
            jsonrpc,
            id,
            params: decode_params::<TurnStartRequest>(method, params)?,
        }),
        ProtocolMethod::TurnSteer => Ok(ProtocolRequest::TurnSteer {
            jsonrpc,
            id,
            params: decode_params::<TurnSteerRequest>(method, params)?,
        }),
        ProtocolMethod::TurnInterrupt => Ok(ProtocolRequest::TurnInterrupt {
            jsonrpc,
            id,
            params: decode_params::<TurnInterruptRequest>(method, params)?,
        }),
        ProtocolMethod::ApprovalResolve => Ok(ProtocolRequest::ApprovalResolve {
            jsonrpc,
            id,
            params: decode_params::<ApprovalResolveRequest>(method, params)?,
        }),
        ProtocolMethod::ProviderShutdown => Ok(ProtocolRequest::ProviderShutdown {
            jsonrpc,
            id,
            params: decode_params::<ProviderShutdownRequest>(method, params)?,
        }),
    }
}

fn decode_params<T: DeserializeOwned>(
    method: ProtocolMethod,
    params: Value,
) -> Result<T, ProtocolError> {
    serde_json::from_value(params).map_err(|error| {
        protocol_error(
            "provider_request_encode_failed",
            format!("encode {} request params: {error}", method.as_str()),
            false,
        )
    })
}

fn inbound_error_to_protocol(error: JsonRpcInboundError) -> ProtocolError {
    let mut details = error.error.data.unwrap_or_default();
    if let Some(id) = error.id {
        details.insert("requestId".to_string(), Value::String(id));
    }
    details.insert("rpcCode".to_string(), Value::from(error.error.code));
    ProtocolError {
        code: "provider_invalid_frame".to_string(),
        message: error.error.message,
        retryable: false,
        details: Some(details),
    }
}

fn inbound_transport_error(message: impl Into<String>) -> JsonRpcInboundError {
    JsonRpcInboundError {
        id: None,
        error: RpcError {
            code: codepet_provider_sdk::JSON_RPC_INTERNAL_ERROR,
            message: message.into(),
            data: None,
        },
    }
}

fn rpc_error_to_protocol(error: RpcError) -> ProtocolError {
    if let Some(data) = error.data.clone() {
        let value = Value::Object(data.clone().into_iter().collect());
        if let Ok(protocol_error) = serde_json::from_value::<ProtocolError>(value) {
            return protocol_error;
        }
    }
    let mut details = error.data.unwrap_or_default();
    details.insert("rpcCode".to_string(), Value::from(error.code));
    ProtocolError {
        code: "provider_rpc_error".to_string(),
        message: error.message,
        retryable: false,
        details: Some(details),
    }
}

fn request_timeout_error(method: ProtocolMethod, request_id: &str, stage: &str) -> ProtocolError {
    let mut details = BTreeMap::new();
    details.insert("method".to_string(), Value::String(method.as_str().to_string()));
    details.insert("requestId".to_string(), Value::String(request_id.to_string()));
    details.insert("stage".to_string(), Value::String(stage.to_string()));
    ProtocolError {
        code: "provider_request_timeout".to_string(),
        message: format!("Provider {} timed out waiting for {stage}", method.as_str()),
        retryable: true,
        details: Some(details),
    }
}

fn validate_executable(executable: &Path) -> HostResult<()> {
    let metadata = std::fs::metadata(executable).map_err(|error| {
        HostError::new(
            "provider_executable_unavailable",
            format!("inspect Provider executable {}: {error}", executable.display()),
        )
        .with_detail("executable", executable.display().to_string())
    })?;
    if !metadata.is_file() {
        return Err(HostError::new(
            "provider_executable_invalid",
            format!("Provider executable is not a file: {}", executable.display()),
        ));
    }
    Ok(())
}

fn protocol_error(code: &str, message: impl Into<String>, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.to_string(),
        message: message.into(),
        retryable,
        details: None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
