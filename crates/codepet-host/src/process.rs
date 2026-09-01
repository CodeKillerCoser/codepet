use crate::catalog::PluginDescriptor;
use crate::{HostError, HostResult};
use codepet_provider_sdk::{
    JsonLineCodec, JsonRpcInboundError, JsonRpcInboundRequest, JsonRpcResponsePayload,
    ProtocolClient, ProtocolError, ProtocolInboundFuture, ProtocolMethod, ProtocolRequest,
    ProtocolTransport, ProtocolTransportFuture, ProviderShutdownRequest,
    ProviderShutdownResponse, ProviderWireMessage, RequestId, RpcError,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Instant;

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

enum WriterCommand {
    Frame(Vec<u8>),
    Close,
}

enum ProcessCommand {
    Kill { reason: String },
}

struct RpcShared {
    codec: JsonLineCodec,
    writer: StdMutex<Option<mpsc::Sender<WriterCommand>>>,
    pending: StdMutex<HashMap<RequestId, oneshot::Sender<Result<Value, ProtocolError>>>>,
    inbound: StdMutex<Option<mpsc::Sender<ProviderWireMessage>>>,
    next_request_id: AtomicU64,
    request_timeout: Duration,
    shutting_down: AtomicBool,
    close_error: StdMutex<Option<ProtocolError>>,
}

impl RpcShared {
    fn begin_shutdown(&self, error: ProtocolError) -> bool {
        let started = self
            .shutting_down
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if started {
            if let Ok(mut close_error) = self.close_error.lock() {
                *close_error = Some(error.clone());
            }
            self.close_inbound();
            self.fail_pending(error);
        }
        started
    }

    fn terminate(&self, error: ProtocolError) {
        self.begin_shutdown(error.clone());
        self.close_inbound();
        self.fail_pending(error);
        if let Ok(mut writer) = self.writer.lock() {
            if let Some(writer) = writer.take() {
                let _ = writer.try_send(WriterCommand::Close);
            }
        }
    }

    fn close_inbound(&self) {
        if let Ok(mut inbound) = self.inbound.lock() {
            inbound.take();
        }
    }

    fn fail_pending(&self, error: ProtocolError) {
        if let Ok(mut pending) = self.pending.lock() {
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(error.clone()));
            }
        }
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    fn closed_error(&self) -> ProtocolError {
        self.close_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .unwrap_or_else(|| {
                protocol_error(
                    "provider_process_closed",
                    "Provider process transport is closed",
                    true,
                )
            })
    }

    fn writer(&self) -> Result<mpsc::Sender<WriterCommand>, ProtocolError> {
        self.writer
            .lock()
            .ok()
            .and_then(|writer| writer.clone())
            .ok_or_else(|| self.closed_error())
    }

    fn insert_pending(
        &self,
        request_id: RequestId,
        sender: oneshot::Sender<Result<Value, ProtocolError>>,
    ) -> Result<(), ProtocolError> {
        self.pending
            .lock()
            .map_err(|_| {
                protocol_error(
                    "provider_pending_unavailable",
                    "Provider pending request state is unavailable",
                    true,
                )
            })?
            .insert(request_id, sender);
        Ok(())
    }

    fn remove_pending(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }

    fn take_pending(
        &self,
        request_id: &str,
    ) -> Option<oneshot::Sender<Result<Value, ProtocolError>>> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(request_id))
    }

    fn send_inbound(&self, message: ProviderWireMessage) -> Result<(), ProtocolError> {
        let sender = self
            .inbound
            .lock()
            .ok()
            .and_then(|sender| sender.clone())
            .ok_or_else(|| self.closed_error())?;
        sender.try_send(message).map_err(|error| {
            protocol_error(
                "provider_inbound_backpressure",
                format!("Provider inbound event queue is unavailable: {error}"),
                true,
            )
        })
    }

    async fn close_writer(&self, deadline: Instant) -> Result<(), ProtocolError> {
        let writer = self
            .writer
            .lock()
            .ok()
            .and_then(|mut writer| writer.take());
        let Some(writer) = writer else {
            return Ok(());
        };
        match tokio::time::timeout_at(deadline, writer.send(WriterCommand::Close)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(self.closed_error()),
            Err(_) => Err(protocol_error(
                "provider_writer_close_timeout",
                "timed out closing Provider stdin",
                true,
            )),
        }
    }
}

struct PendingRequest {
    shared: Weak<RpcShared>,
    request_id: RequestId,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            shared.remove_pending(&self.request_id);
        }
    }
}

pub struct ProviderRpcClient {
    shared: Arc<RpcShared>,
    inbound: Mutex<Option<mpsc::Receiver<ProviderWireMessage>>>,
}

impl ProviderRpcClient {
    fn new(shared: Arc<RpcShared>, inbound: mpsc::Receiver<ProviderWireMessage>) -> Self {
        Self {
            shared,
            inbound: Mutex::new(Some(inbound)),
        }
    }

    async fn take_inbound(
        &self,
    ) -> Result<mpsc::Receiver<ProviderWireMessage>, ProtocolError> {
        self.inbound.lock().await.take().ok_or_else(|| {
            protocol_error(
                "provider_inbound_already_taken",
                "Provider inbound message stream already has a consumer",
                false,
            )
        })
    }

    async fn request_value(
        &self,
        method: ProtocolMethod,
        params: Value,
    ) -> Result<Value, ProtocolError> {
        self.request_value_until(
            method,
            params,
            Instant::now() + self.shared.request_timeout,
            false,
        )
        .await
    }

    async fn request_value_until(
        &self,
        method: ProtocolMethod,
        params: Value,
        deadline: Instant,
        during_shutdown: bool,
    ) -> Result<Value, ProtocolError> {
        if self.shared.is_shutting_down() && !during_shutdown {
            return Err(self.shared.closed_error());
        }
        let writer = self.shared.writer()?;
        let sequence = self.shared.next_request_id.fetch_add(1, Ordering::SeqCst);
        let request_id = format!("host-{sequence}");
        let request = ProtocolRequest::from_method_params(method, request_id.clone(), params)?;
        let message = ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request));
        let frame = self.shared.codec.encode_message(&message)?;
        let (response_sender, response_receiver) = oneshot::channel();
        self.shared
            .insert_pending(request_id.clone(), response_sender)?;
        let _pending = PendingRequest {
            shared: Arc::downgrade(&self.shared),
            request_id: request_id.clone(),
        };

        match tokio::time::timeout_at(deadline, writer.send(WriterCommand::Frame(frame))).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(self.shared.closed_error()),
            Err(_) => return Err(request_timeout_error(method, &request_id, "write queue")),
        }

        match tokio::time::timeout_at(deadline, response_receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(self.shared.closed_error()),
            Err(_) => Err(request_timeout_error(method, &request_id, "response")),
        }
    }

    async fn provider_shutdown_until(
        &self,
        deadline: Instant,
    ) -> Result<ProviderShutdownResponse, ProtocolError> {
        let params = serde_json::to_value(ProviderShutdownRequest {}).map_err(|error| {
            protocol_error(
                "provider_request_encode_failed",
                format!("encode provider.shutdown request params: {error}"),
                false,
            )
        })?;
        let response = self
            .request_value_until(
                ProtocolMethod::ProviderShutdown,
                params,
                deadline,
                true,
            )
            .await?;
        serde_json::from_value(response).map_err(|error| {
            protocol_error(
                "provider_response_decode_failed",
                format!("decode provider.shutdown response: {error}"),
                false,
            )
        })
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
            receiver
                .as_mut()
                .ok_or_else(|| {
                    inbound_transport_error("Provider inbound message stream already has a consumer")
                })?
                .recv()
                .await
                .ok_or_else(|| inbound_transport_error(self.shared.closed_error().message))
        })
    }
}

pub struct PluginProcess {
    client: ProtocolClient<ProviderRpcClient>,
    shared: Arc<RpcShared>,
    control: mpsc::Sender<ProcessCommand>,
    exit: watch::Receiver<Option<PluginProcessExit>>,
    diagnostics: Arc<StdMutex<VecDeque<StderrDiagnostic>>>,
    shutdown_gate: Mutex<()>,
    options: PluginProcessOptions,
}

impl PluginProcess {
    pub fn spawn(
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
        #[cfg(unix)]
        command.process_group(0);
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
        let process_id = child.id().ok_or_else(|| {
            HostError::new(
                "provider_spawn_failed",
                "Provider process did not expose a process id",
            )
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
        let (inbound_sender, inbound_receiver) = mpsc::channel(options.inbound_capacity.max(1));
        let (control_sender, control_receiver) = mpsc::channel(4);
        let (exit_sender, exit_receiver) = watch::channel(None);
        let diagnostics = Arc::new(StdMutex::new(VecDeque::with_capacity(
            options.stderr_history_lines.max(1),
        )));
        let shared = Arc::new(RpcShared {
            codec,
            writer: StdMutex::new(Some(writer_sender)),
            pending: StdMutex::new(HashMap::new()),
            inbound: StdMutex::new(Some(inbound_sender)),
            next_request_id: AtomicU64::new(1),
            request_timeout: options.request_timeout,
            shutting_down: AtomicBool::new(false),
            close_error: StdMutex::new(None),
        });

        let writer = spawn_tracked(writer_loop(
            stdin,
            writer_receiver,
            Arc::downgrade(&shared),
            control_sender.clone(),
        ));
        let reader = spawn_tracked(reader_loop(
            stdout,
            shared.clone(),
            control_sender.clone(),
        ));
        let stderr = spawn_tracked(stderr_loop(
            stderr,
            diagnostics.clone(),
            options.stderr_line_bytes.max(1),
            options.stderr_history_lines.max(1),
        ));
        spawn_tracked(process_monitor(
            child,
            process_id,
            control_receiver,
            exit_sender,
            shared.clone(),
            writer,
            reader,
            stderr,
            options.shutdown_timeout,
        ));

        Ok(Self {
            client: ProtocolClient::new(ProviderRpcClient::new(
                shared.clone(),
                inbound_receiver,
            )),
            shared,
            control: control_sender,
            exit: exit_receiver,
            diagnostics,
            shutdown_gate: Mutex::new(()),
            options,
        })
    }

    pub fn client(&self) -> &ProtocolClient<ProviderRpcClient> {
        &self.client
    }

    pub fn exit_receiver(&self) -> watch::Receiver<Option<PluginProcessExit>> {
        self.exit.clone()
    }

    pub async fn take_inbound(
        &self,
    ) -> HostResult<mpsc::Receiver<ProviderWireMessage>> {
        self.client
            .transport()
            .take_inbound()
            .await
            .map_err(HostError::from)
    }

    fn exit_status(&self) -> Option<PluginProcessExit> {
        self.exit.borrow().clone()
    }

    pub(crate) fn is_available(&self) -> bool {
        !self.shared.is_shutting_down() && self.exit_status().is_none()
    }

    pub fn stderr_diagnostics(&self) -> Vec<StderrDiagnostic> {
        self.diagnostics
            .lock()
            .map(|diagnostics| diagnostics.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn diagnostics_handle(
        &self,
    ) -> Arc<StdMutex<VecDeque<StderrDiagnostic>>> {
        self.diagnostics.clone()
    }

    pub async fn shutdown(&self) -> HostResult<PluginProcessExit> {
        let _shutdown = self.shutdown_gate.lock().await;
        if let Some(exit) = self.exit_status() {
            return Ok(exit);
        }
        let deadline = Instant::now() + self.options.shutdown_timeout;
        let shutdown_error = protocol_error(
            "provider_shutting_down",
            "Provider process is shutting down",
            true,
        );
        let first = self.shared.begin_shutdown(shutdown_error);
        let shutdown_result = if first {
            match self.client.transport().provider_shutdown_until(deadline).await {
                Ok(response) if response.accepted => Ok(()),
                Ok(_) => Err(HostError::new(
                    "provider_shutdown_declined",
                    "Provider declined the shutdown request",
                )),
                Err(error) => Err(HostError::from(error)),
            }
        } else {
            Ok(())
        };
        let _ = self.shared.close_writer(deadline).await;
        if let Some(exit) = wait_for_exit_until(&self.exit, deadline).await {
            shutdown_result?;
            return Ok(exit);
        }
        let exit = self
            .kill_now("Provider did not exit before the configured shutdown timeout")
            .await?;
        shutdown_result?;
        Ok(exit)
    }

    pub(crate) async fn force_kill(
        &self,
        reason: impl Into<String>,
    ) -> HostResult<PluginProcessExit> {
        if let Some(exit) = self.exit_status() {
            return Ok(exit);
        }
        self.kill_now(reason.into()).await
    }

    async fn kill_now(&self, reason: impl Into<String>) -> HostResult<PluginProcessExit> {
        let reason = reason.into();
        self.shared.terminate(protocol_error(
            "provider_process_killed",
            reason.clone(),
            true,
        ));
        let _ = self.control.try_send(ProcessCommand::Kill { reason });
        wait_for_exit_completion(&self.exit).await
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        self.shared.terminate(protocol_error(
            "provider_process_dropped",
            "Provider process handle dropped",
            true,
        ));
        let _ = self.control.try_send(ProcessCommand::Kill {
            reason: "Provider process handle dropped".to_string(),
        });
    }
}

async fn writer_loop(
    mut stdin: ChildStdin,
    mut receiver: mpsc::Receiver<WriterCommand>,
    shared: Weak<RpcShared>,
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
                if let Err(error) = result {
                    if let Some(shared) = shared.upgrade() {
                        let failure = protocol_error(
                            "provider_stdin_close_failed",
                            format!("close Provider stdin: {error}"),
                            true,
                        );
                        shared.terminate(failure.clone());
                        let _ = control
                            .send(ProcessCommand::Kill {
                                reason: failure.message,
                            })
                            .await;
                    }
                }
                return;
            }
        };
        if let Err(error) = result {
            if let Some(shared) = shared.upgrade() {
                let failure = protocol_error(
                    "provider_stdin_write_failed",
                    format!("write Provider stdin: {error}"),
                    true,
                );
                shared.terminate(failure.clone());
                let _ = control
                    .send(ProcessCommand::Kill {
                        reason: failure.message,
                    })
                    .await;
            }
            return;
        }
    }
    let _ = stdin.shutdown().await;
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
            Ok(None) if shared.is_shutting_down() => return,
            Ok(None) => {
                fail_transport(
                    &shared,
                    &control,
                    protocol_error(
                        "provider_stdout_eof",
                        "Provider stdout reached EOF",
                        true,
                    ),
                )
                .await;
                return;
            }
            Err(error) => {
                fail_transport(&shared, &control, error).await;
                return;
            }
        };
        let message = match shared.codec.decode_line(&frame) {
            Ok(message) => message,
            Err(error) => {
                fail_transport(&shared, &control, inbound_error_to_protocol(error)).await;
                return;
            }
        };
        match message {
            ProviderWireMessage::Response(response) => {
                let Some(id) = response.id else {
                    fail_transport(
                        &shared,
                        &control,
                        protocol_error(
                            "provider_response_missing_id",
                            "Provider response did not contain a request id",
                            false,
                        ),
                    )
                    .await;
                    return;
                };
                if let Some(sender) = shared.take_pending(&id) {
                    let result = match response.response {
                        JsonRpcResponsePayload::Ok { result } => Ok(result),
                        JsonRpcResponsePayload::Error { error } => {
                            Err(rpc_error_to_protocol(error))
                        }
                    };
                    let _ = sender.send(result);
                }
            }
            ProviderWireMessage::Event(event) => {
                if shared.is_shutting_down() {
                    continue;
                }
                if let Err(error) = shared.send_inbound(ProviderWireMessage::Event(event)) {
                    fail_transport(&shared, &control, error).await;
                    return;
                }
            }
            ProviderWireMessage::Notification(notification) => {
                if shared.is_shutting_down() {
                    continue;
                }
                if let Err(error) = shared
                    .send_inbound(ProviderWireMessage::Notification(notification))
                {
                    fail_transport(&shared, &control, error).await;
                    return;
                }
            }
            ProviderWireMessage::Request(_) => {
                fail_transport(
                    &shared,
                    &control,
                    protocol_error(
                        "unexpected_provider_request",
                        "Provider sent a host-to-plugin request on stdout",
                        false,
                    ),
                )
                .await;
                return;
            }
        }
    }
}

async fn fail_transport(
    shared: &RpcShared,
    control: &mpsc::Sender<ProcessCommand>,
    error: ProtocolError,
) {
    shared.terminate(error.clone());
    let _ = control
        .send(ProcessCommand::Kill {
            reason: error.message,
        })
        .await;
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

#[cfg(unix)]
fn signal_process_group(process_id: u32) -> std::io::Result<()> {
    let process_group = i32::try_from(process_id)
        .map_err(|_| std::io::Error::other("Provider process id exceeds i32"))?;
    let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(unix)]
async fn kill_process_tree(
    child: &mut tokio::process::Child,
    process_id: u32,
) -> std::io::Result<()> {
    match signal_process_group(process_id) {
        Ok(()) => Ok(()),
        Err(group_error) => {
            if child.try_wait()?.is_none() {
                child.kill().await
            } else {
                Err(group_error)
            }
        }
    }
}

#[cfg(windows)]
async fn kill_process_tree(
    child: &mut tokio::process::Child,
    process_id: u32,
) -> std::io::Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &process_id.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    if status.success() || child.try_wait()?.is_some() {
        Ok(())
    } else {
        child.kill().await
    }
}

#[cfg(not(any(unix, windows)))]
async fn kill_process_tree(
    child: &mut tokio::process::Child,
    _process_id: u32,
) -> std::io::Result<()> {
    child.kill().await
}

#[cfg(unix)]
fn cleanup_descendants_after_exit(process_id: u32) {
    let _ = signal_process_group(process_id);
}

#[cfg(not(unix))]
fn cleanup_descendants_after_exit(_process_id: u32) {}

async fn process_monitor(
    mut child: tokio::process::Child,
    process_id: u32,
    mut control: mpsc::Receiver<ProcessCommand>,
    exit_sender: watch::Sender<Option<PluginProcessExit>>,
    shared: Arc<RpcShared>,
    writer: JoinHandle<()>,
    reader: JoinHandle<()>,
    stderr: JoinHandle<()>,
    drain_timeout: Duration,
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
                let kill_error = kill_process_tree(&mut child, process_id).await.err();
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
                let _ = kill_process_tree(&mut child, process_id).await;
                let status = child.wait().await.ok();
                PluginProcessExit {
                    success: false,
                    code: status.and_then(|status| status.code()),
                    reason: Some("Provider process control channel closed".to_string()),
                }
            }
        },
    };
    cleanup_descendants_after_exit(process_id);
    shared.terminate(protocol_error(
        "provider_process_exited",
        exit.reason
            .clone()
            .unwrap_or_else(|| "Provider process exited".to_string()),
        true,
    ));
    drain_io_tasks(writer, reader, stderr, drain_timeout).await;
    let _ = exit_sender.send(Some(exit));
}

async fn drain_io_tasks(
    writer: JoinHandle<()>,
    reader: JoinHandle<()>,
    stderr: JoinHandle<()>,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    tokio::join!(
        drain_task(writer, deadline),
        drain_task(reader, deadline),
        drain_task(stderr, deadline),
    );
}

async fn drain_task(mut task: JoinHandle<()>, deadline: Instant) {
    if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
}

async fn wait_for_exit_until(
    receiver: &watch::Receiver<Option<PluginProcessExit>>,
    deadline: Instant,
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
    tokio::time::timeout_at(deadline, wait).await.ok().flatten()
}

async fn wait_for_exit_completion(
    receiver: &watch::Receiver<Option<PluginProcessExit>>,
) -> HostResult<PluginProcessExit> {
    let mut receiver = receiver.clone();
    loop {
        if let Some(exit) = receiver.borrow().clone() {
            return Ok(exit);
        }
        if receiver.changed().await.is_err() {
            return Err(HostError::new(
                "provider_process_monitor_closed",
                "Provider process monitor closed before publishing an exit result",
            ));
        }
    }
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
    details.insert(
        "method".to_string(),
        Value::String(method.as_str().to_string()),
    );
    details.insert(
        "requestId".to_string(),
        Value::String(request_id.to_string()),
    );
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

#[cfg(test)]
static ACTIVE_PROCESS_TASKS: AtomicUsize = AtomicUsize::new(0);

fn spawn_tracked<F>(future: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        #[cfg(test)]
        let _task = ActiveProcessTask::new();
        future.await;
    })
}

#[cfg(test)]
struct ActiveProcessTask;

#[cfg(test)]
impl ActiveProcessTask {
    fn new() -> Self {
        ACTIVE_PROCESS_TASKS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for ActiveProcessTask {
    fn drop(&mut self) {
        ACTIVE_PROCESS_TASKS.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{
        wait_for_exit_completion, PluginProcess, PluginProcessOptions,
        ACTIVE_PROCESS_TASKS,
    };
    use crate::PluginDescriptor;
    use codepet_provider_sdk::ProviderDescribeRequest;
    use std::collections::BTreeMap;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    fn open_file_descriptors() -> usize {
        std::fs::read_dir("/dev/fd")
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0)
    }

    #[derive(Clone, Copy)]
    enum TerminalAction {
        RequestFailure,
        Timeout,
        Shutdown,
        Drop,
    }

    struct TerminalCase {
        name: &'static str,
        script: &'static str,
        action: TerminalAction,
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_cases_release_tasks_handles_pending_inbound_and_pipes() {
        let baseline_tasks = ACTIVE_PROCESS_TASKS.load(Ordering::SeqCst);
        let baseline_fds = open_file_descriptors();
        let cases = [
            TerminalCase {
                name: "malformed",
                script: "IFS= read -r request; printf '{malformed-json}\\n'; sleep 1",
                action: TerminalAction::RequestFailure,
            },
            TerminalCase {
                name: "oversized",
                script: "IFS= read -r request; printf '%20000s\\n' x; sleep 1",
                action: TerminalAction::RequestFailure,
            },
            TerminalCase {
                name: "eof",
                script: "IFS= read -r request",
                action: TerminalAction::RequestFailure,
            },
            TerminalCase {
                name: "crash",
                script: "IFS= read -r request; exit 17",
                action: TerminalAction::RequestFailure,
            },
            TerminalCase {
                name: "timeout",
                script: "IFS= read -r request; sleep 1",
                action: TerminalAction::Timeout,
            },
            TerminalCase {
                name: "normal-shutdown",
                script: "IFS= read -r request; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":\"host-1\",\"result\":{\"accepted\":true}}'",
                action: TerminalAction::Shutdown,
            },
            TerminalCase {
                name: "drop",
                script: "IFS= read -r request; sleep 1",
                action: TerminalAction::Drop,
            },
        ];
        for case in cases {
            let descriptor = PluginDescriptor {
                plugin_id: format!("dev.codepet.terminal-{}", case.name),
                display_name: "Failure fixture".to_string(),
                executable: "/bin/sh".into(),
                args: vec![
                    "-c".to_string(),
                    case.script.to_string(),
                ],
                env: BTreeMap::new(),
                enabled: true,
                instances: Vec::new(),
            };
            let process = Arc::new(
                PluginProcess::spawn(
                    &descriptor,
                    PluginProcessOptions {
                        max_frame_bytes: 1024,
                        request_timeout: Duration::from_millis(50),
                        shutdown_timeout: Duration::from_millis(500),
                        ..PluginProcessOptions::default()
                    },
                )
                .unwrap(),
            );
            let process_weak = Arc::downgrade(&process);
            let shared_weak = Arc::downgrade(&process.shared);
            let mut inbound = process.take_inbound().await.unwrap();
            match case.action {
                TerminalAction::RequestFailure => {
                    process
                        .client()
                        .provider_describe(ProviderDescribeRequest {})
                        .await
                        .unwrap_err();
                    wait_for_exit_completion(&process.exit).await.unwrap();
                }
                TerminalAction::Timeout => {
                    let error = process
                        .client()
                        .provider_describe(ProviderDescribeRequest {})
                        .await
                        .unwrap_err();
                    assert_eq!(error.code, "provider_request_timeout");
                    assert!(process.shared.pending.lock().unwrap().is_empty());
                    process.force_kill("terminal timeout test").await.unwrap();
                }
                TerminalAction::Shutdown => {
                    assert!(process.shutdown().await.unwrap().success);
                }
                TerminalAction::Drop => {
                    drop(process);
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while process_weak.upgrade().is_some()
                            || shared_weak.upgrade().is_some()
                            || ACTIVE_PROCESS_TASKS.load(Ordering::SeqCst) != baseline_tasks
                        {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert!(inbound.recv().await.is_none());
                    continue;
                }
            }
            assert!(process.shared.writer.lock().unwrap().is_none());
            assert!(process.shared.pending.lock().unwrap().is_empty());
            assert!(process.shared.inbound.lock().unwrap().is_none());
            assert!(inbound.recv().await.is_none());
            drop(process);
            tokio::time::timeout(Duration::from_secs(2), async {
                while process_weak.upgrade().is_some()
                    || shared_weak.upgrade().is_some()
                    || ACTIVE_PROCESS_TASKS.load(Ordering::SeqCst) != baseline_tasks
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        assert!(open_file_descriptors() <= baseline_fds.saturating_add(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn force_kill_terminates_the_provider_process_group() {
        let directory = tempfile::tempdir().unwrap();
        let descendant_pid = directory.path().join("descendant.pid");
        let mut env = BTreeMap::new();
        env.insert(
            "CODEPET_DESCENDANT_PID".to_string(),
            descendant_pid.display().to_string(),
        );
        let descriptor = PluginDescriptor {
            plugin_id: "dev.codepet.process-tree".to_string(),
            display_name: "Process tree fixture".to_string(),
            executable: "/bin/sh".into(),
            args: vec![
                "-c".to_string(),
                "sleep 30 & echo $! > \"$CODEPET_DESCENDANT_PID\"; wait".to_string(),
            ],
            env,
            enabled: true,
            instances: Vec::new(),
        };
        let process = PluginProcess::spawn(
            &descriptor,
            PluginProcessOptions {
                shutdown_timeout: Duration::from_millis(500),
                ..PluginProcessOptions::default()
            },
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !descendant_pid.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(&descendant_pid)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();

        process.force_kill("process tree test").await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
