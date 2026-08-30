use super::protocol::{
    permission_from_sandbox, thread_list_params, thread_start_params, turn_start_params,
    turn_steer_params, CodexAppServerError, CodexApprovalKind, CodexApprovalRequest,
    CodexConversationSnapshot, CodexIncoming, CodexNotification, CodexThreadListRequest,
    CodexPermissionLevel, CodexThreadPage, CodexThreadStartRequest, CodexTurn,
    CodexTurnStartRequest, CodexTurnStatus, CodexTurnSteerRequest, JsonRpcId,
    ThreadListResponse, ThreadResponse, TurnResponse, TurnSteerResponse,
};
use codepet_provider_sdk::ApprovalDecision;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub trait JsonRpcReader: Send + 'static {
    fn read_message(&mut self) -> Result<Option<Value>, CodexAppServerError>;
}

pub trait JsonRpcWriter: Send + 'static {
    fn write_message(&mut self, message: &Value) -> Result<(), CodexAppServerError>;
}

pub trait SessionControl: Send + 'static {
    fn shutdown(&mut self) -> Result<(), CodexAppServerError>;
}

struct JsonLineReader<R> {
    reader: BufReader<R>,
}

impl<R: Read> JsonLineReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
        }
    }
}

impl<R: Read + Send + 'static> JsonRpcReader for JsonLineReader<R> {
    fn read_message(&mut self) -> Result<Option<Value>, CodexAppServerError> {
        let mut line = String::new();
        let bytes = self
            .reader
            .read_line(&mut line)
            .map_err(|error| CodexAppServerError::Io(error.to_string()))?;
        if bytes == 0 {
            return Ok(None);
        }
        serde_json::from_str(line.trim())
            .map(Some)
            .map_err(|error| CodexAppServerError::Protocol(format!("invalid JSON: {error}")))
    }
}

struct JsonLineWriter<W> {
    writer: W,
}

impl<W> JsonLineWriter<W> {
    fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write + Send + 'static> JsonRpcWriter for JsonLineWriter<W> {
    fn write_message(&mut self, message: &Value) -> Result<(), CodexAppServerError> {
        serde_json::to_writer(&mut self.writer, message)
            .map_err(|error| CodexAppServerError::Protocol(error.to_string()))?;
        self.writer
            .write_all(b"\n")
            .map_err(|error| CodexAppServerError::Io(error.to_string()))?;
        self.writer
            .flush()
            .map_err(|error| CodexAppServerError::Io(error.to_string()))
    }
}

struct ChildControl {
    child: Child,
}

impl SessionControl for ChildControl {
    fn shutdown(&mut self) -> Result<(), CodexAppServerError> {
        match self.child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(error) => return Err(CodexAppServerError::Io(error.to_string())),
        }
        self.child
            .kill()
            .map_err(|error| CodexAppServerError::Io(error.to_string()))?;
        self.child
            .wait()
            .map_err(|error| CodexAppServerError::Io(error.to_string()))?;
        Ok(())
    }
}

struct SessionInner {
    writer: Mutex<Option<Box<dyn JsonRpcWriter>>>,
    pending: Mutex<HashMap<JsonRpcId, Sender<Result<Value, CodexAppServerError>>>>,
    subscribers: Mutex<Vec<Sender<Result<CodexIncoming, CodexAppServerError>>>>,
    loaded_threads: Mutex<HashMap<String, ThreadLoadState>>,
    loaded_threads_changed: Condvar,
    next_load_evidence: AtomicU64,
    next_id: AtomicI64,
    running: AtomicBool,
    control: Mutex<Option<Box<dyn SessionControl>>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThreadLoadState {
    Resuming,
    Loaded(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexRequestOutcome<T> {
    Success(T),
    NotSent(CodexAppServerError),
    ExplicitRpcReject(CodexAppServerError),
    SentOutcomeUnknown(CodexAppServerError),
}

impl<T> CodexRequestOutcome<T> {
    fn and_then<U>(self, operation: impl FnOnce(T) -> CodexRequestOutcome<U>) -> CodexRequestOutcome<U> {
        match self {
            Self::Success(value) => operation(value),
            Self::NotSent(error) => CodexRequestOutcome::NotSent(error),
            Self::ExplicitRpcReject(error) => CodexRequestOutcome::ExplicitRpcReject(error),
            Self::SentOutcomeUnknown(error) => CodexRequestOutcome::SentOutcomeUnknown(error),
        }
    }

    pub(crate) fn into_result(self) -> Result<T, CodexAppServerError> {
        match self {
            Self::Success(value) => Ok(value),
            Self::NotSent(error)
            | Self::ExplicitRpcReject(error)
            | Self::SentOutcomeUnknown(error) => Err(error),
        }
    }
}

impl SessionInner {
    fn broadcast(&self, message: Result<CodexIncoming, CodexAppServerError>) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.send(message.clone()).is_ok());
        }
    }

    fn fail(&self, error: CodexAppServerError) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(error.clone()));
            }
        }
        self.broadcast(Err(error));
    }

    fn write(&self, message: Value) -> Result<(), CodexAppServerError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| CodexAppServerError::Protocol("writer lock is poisoned".to_string()))?;
        let writer = writer
            .as_mut()
            .ok_or(CodexAppServerError::Shutdown)?;
        writer.write_message(&message)
    }

    fn write_request(&self, message: Value) -> CodexRequestOutcome<()> {
        let mut writer = match self.writer.lock() {
            Ok(writer) => writer,
            Err(_) => {
                return CodexRequestOutcome::NotSent(CodexAppServerError::Protocol(
                    "writer lock is poisoned".to_string(),
                ))
            }
        };
        let Some(writer) = writer.as_mut() else {
            return CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown);
        };
        match writer.write_message(&message) {
            Ok(()) => CodexRequestOutcome::Success(()),
            Err(error) => CodexRequestOutcome::SentOutcomeUnknown(error),
        }
    }

    fn mark_thread_loaded(&self, thread_id: &str) -> u64 {
        let evidence = self.next_load_evidence.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut loaded_threads) = self.loaded_threads.lock() {
            loaded_threads.insert(thread_id.to_string(), ThreadLoadState::Loaded(evidence));
            self.loaded_threads_changed.notify_all();
        }
        evidence
    }

}

impl Drop for SessionInner {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(writer) = self.writer.get_mut() {
            writer.take();
        }
        if let Ok(control) = self.control.get_mut() {
            if let Some(control) = control.as_mut() {
                let _ = control.shutdown();
            }
        }
    }
}

#[derive(Clone)]
pub struct CodexAppServerSession {
    inner: Arc<SessionInner>,
}

impl CodexAppServerSession {
    pub fn spawn(
        executable: &Path,
        args: &[String],
    ) -> Result<Self, CodexAppServerError> {
        let mut child = codex_app_server_command(executable, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| CodexAppServerError::Spawn(error.to_string()))?;
        let stdin: ChildStdin = child
            .stdin
            .take()
            .ok_or_else(|| CodexAppServerError::Spawn("stdin is unavailable".to_string()))?;
        let stdout: ChildStdout = child
            .stdout
            .take()
            .ok_or_else(|| CodexAppServerError::Spawn("stdout is unavailable".to_string()))?;
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("Codex App Server: {line}");
                }
            });
        }
        let session = Self::from_parts(
            Box::new(JsonLineReader::new(stdout)),
            Box::new(JsonLineWriter::new(stdin)),
            Some(Box::new(ChildControl { child })),
        );
        if let Err(error) = session.initialize() {
            let _ = session.shutdown();
            return Err(error);
        }
        Ok(session)
    }

    #[cfg(test)]
    pub fn connect(
        reader: Box<dyn JsonRpcReader>,
        writer: Box<dyn JsonRpcWriter>,
    ) -> Result<Self, CodexAppServerError> {
        Self::connect_with_initialize_timeout(reader, writer, INITIALIZE_TIMEOUT)
    }

    #[cfg(test)]
    fn connect_with_initialize_timeout(
        reader: Box<dyn JsonRpcReader>,
        writer: Box<dyn JsonRpcWriter>,
        initialize_timeout: Duration,
    ) -> Result<Self, CodexAppServerError> {
        let session = Self::from_parts(reader, writer, None);
        if let Err(error) = session.initialize_with_timeout(initialize_timeout) {
            let _ = session.shutdown();
            return Err(error);
        }
        Ok(session)
    }

    fn from_parts(
        mut reader: Box<dyn JsonRpcReader>,
        writer: Box<dyn JsonRpcWriter>,
        control: Option<Box<dyn SessionControl>>,
    ) -> Self {
        let inner = Arc::new(SessionInner {
            writer: Mutex::new(Some(writer)),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            loaded_threads: Mutex::new(HashMap::new()),
            loaded_threads_changed: Condvar::new(),
            next_load_evidence: AtomicU64::new(1),
            next_id: AtomicI64::new(1),
            running: AtomicBool::new(true),
            control: Mutex::new(control),
            reader_thread: Mutex::new(None),
        });
        let weak_inner = Arc::downgrade(&inner);
        let handle = thread::spawn(move || loop {
            let message = match reader.read_message() {
                Ok(Some(message)) => message,
                Ok(None) => {
                    if let Some(inner) = weak_inner.upgrade() {
                        inner.fail(CodexAppServerError::ProcessExited);
                    }
                    break;
                }
                Err(error) => {
                    if let Some(inner) = weak_inner.upgrade() {
                        inner.fail(error);
                    }
                    break;
                }
            };
            let Some(inner) = weak_inner.upgrade() else {
                break;
            };
            if let Err(error) = handle_message(&inner, message) {
                inner.fail(error);
                break;
            }
        });
        if let Ok(mut reader_thread) = inner.reader_thread.lock() {
            *reader_thread = Some(handle);
        }
        Self { inner }
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    pub fn subscribe(&self) -> Receiver<Result<CodexIncoming, CodexAppServerError>> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.inner.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    pub fn shutdown(&self) -> Result<(), CodexAppServerError> {
        if !self.inner.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        if let Ok(mut writer) = self.inner.writer.lock() {
            writer.take();
        }
        let control_result = if let Ok(mut control) = self.inner.control.lock() {
            if let Some(control) = control.as_mut() {
                control.shutdown()
            } else {
                Ok(())
            }
        } else {
            Err(CodexAppServerError::Protocol(
                "process control lock is poisoned".to_string(),
            ))
        };
        if let Ok(mut pending) = self.inner.pending.lock() {
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(CodexAppServerError::Shutdown));
            }
        }
        control_result
    }

    pub fn thread_list(
        &self,
        request: CodexThreadListRequest,
    ) -> Result<CodexThreadPage, CodexAppServerError> {
        let workspace_root = request.workspace_root.clone();
        let response: ThreadListResponse =
            self.request("thread/list", thread_list_params(&request))?;
        Ok(CodexThreadPage {
            data: response
                .data
                .into_iter()
                .map(|thread| CodexConversationSnapshot {
                    thread,
                    workspace_root: workspace_root.clone(),
                    permission_level: None,
                    model: None,
                    reasoning_effort: None,
                })
                .collect(),
            next_cursor: response.next_cursor,
        })
    }

    pub fn thread_read(
        &self,
        thread_id: &str,
    ) -> Result<CodexConversationSnapshot, CodexAppServerError> {
        let response: ThreadResponse = self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": true }),
        )?;
        Ok(snapshot_from_response(response, None, None))
    }

    fn request_thread_resume_outcome(
        &self,
        thread_id: &str,
    ) -> CodexRequestOutcome<CodexConversationSnapshot> {
        self.request_outcome("thread/resume", json!({ "threadId": thread_id }))
            .and_then(|response: ThreadResponse| {
                let snapshot = snapshot_from_response(response, None, None);
                if snapshot.thread.id != thread_id {
                    return CodexRequestOutcome::SentOutcomeUnknown(
                        CodexAppServerError::Protocol(format!(
                            "thread/resume returned thread {} for requested thread {thread_id}",
                            snapshot.thread.id
                        )),
                    );
                }
                CodexRequestOutcome::Success(snapshot)
            })
    }

    pub fn ensure_thread_loaded(&self, thread_id: &str) -> Result<u64, CodexAppServerError> {
        self.ensure_thread_loaded_outcome(thread_id).into_result()
    }

    pub(crate) fn ensure_thread_loaded_outcome(
        &self,
        thread_id: &str,
    ) -> CodexRequestOutcome<u64> {
        loop {
            let mut loaded_threads = match self.inner.loaded_threads.lock() {
                Ok(loaded_threads) => loaded_threads,
                Err(_) => {
                    return CodexRequestOutcome::NotSent(CodexAppServerError::Protocol(
                        "loaded thread state lock is poisoned".to_string(),
                    ))
                }
            };
            match loaded_threads.get(thread_id).copied() {
                Some(ThreadLoadState::Loaded(evidence)) => {
                    return CodexRequestOutcome::Success(evidence)
                }
                Some(ThreadLoadState::Resuming) => {
                    loaded_threads = match self
                        .inner
                        .loaded_threads_changed
                        .wait(loaded_threads)
                    {
                        Ok(loaded_threads) => loaded_threads,
                        Err(_) => {
                            return CodexRequestOutcome::NotSent(
                                CodexAppServerError::Protocol(
                                    "loaded thread state lock is poisoned".to_string(),
                                ),
                            )
                        }
                    };
                    drop(loaded_threads);
                }
                None => {
                    loaded_threads.insert(thread_id.to_string(), ThreadLoadState::Resuming);
                    break;
                }
            }
        }
        let result = self.request_thread_resume_outcome(thread_id);
        let mut loaded_threads = match self.inner.loaded_threads.lock() {
            Ok(loaded_threads) => loaded_threads,
            Err(_) => {
                return CodexRequestOutcome::SentOutcomeUnknown(
                    CodexAppServerError::Protocol(
                        "loaded thread state lock is poisoned".to_string(),
                    ),
                )
            }
        };
        let settled = match result {
            CodexRequestOutcome::Success(_) => {
                let evidence = match loaded_threads.get(thread_id).copied() {
                    Some(ThreadLoadState::Loaded(evidence)) => evidence,
                    _ => {
                        let evidence = self
                            .inner
                            .next_load_evidence
                            .fetch_add(1, Ordering::SeqCst);
                        loaded_threads
                            .insert(thread_id.to_string(), ThreadLoadState::Loaded(evidence));
                        evidence
                    }
                };
                CodexRequestOutcome::Success(evidence)
            }
            CodexRequestOutcome::NotSent(error) => {
                if loaded_threads.get(thread_id) == Some(&ThreadLoadState::Resuming) {
                    loaded_threads.remove(thread_id);
                }
                CodexRequestOutcome::NotSent(error)
            }
            CodexRequestOutcome::ExplicitRpcReject(error) => {
                if loaded_threads.get(thread_id) == Some(&ThreadLoadState::Resuming) {
                    loaded_threads.remove(thread_id);
                }
                CodexRequestOutcome::ExplicitRpcReject(error)
            }
            CodexRequestOutcome::SentOutcomeUnknown(error) => {
                if loaded_threads.get(thread_id) == Some(&ThreadLoadState::Resuming) {
                    loaded_threads.remove(thread_id);
                }
                CodexRequestOutcome::SentOutcomeUnknown(error)
            }
        };
        self.inner.loaded_threads_changed.notify_all();
        settled
    }

    pub fn thread_start(
        &self,
        request: CodexThreadStartRequest,
    ) -> Result<CodexConversationSnapshot, CodexAppServerError> {
        self.thread_start_outcome(request).into_result()
    }

    pub(crate) fn thread_start_outcome(
        &self,
        request: CodexThreadStartRequest,
    ) -> CodexRequestOutcome<CodexConversationSnapshot> {
        self.request_outcome("thread/start", thread_start_params(&request))
            .and_then(|response: ThreadResponse| {
                let snapshot = snapshot_from_response(
                    response,
                    request.workspace_root,
                    Some(request.permission_level),
                );
                self.inner.mark_thread_loaded(&snapshot.thread.id);
                CodexRequestOutcome::Success(snapshot)
            })
    }

    pub fn turn_start(
        &self,
        request: CodexTurnStartRequest,
    ) -> Result<CodexTurn, CodexAppServerError> {
        self.ensure_thread_loaded(&request.thread_id)?;
        let response: TurnResponse =
            self.request("turn/start", turn_start_params(&request))?;
        Ok(response.turn)
    }

    pub fn turn_steer(
        &self,
        request: CodexTurnSteerRequest,
    ) -> Result<CodexTurn, CodexAppServerError> {
        self.ensure_thread_loaded(&request.thread_id)?;
        let response: TurnSteerResponse =
            self.request("turn/steer", turn_steer_params(&request))?;
        Ok(CodexTurn {
            id: response.turn_id,
            status: CodexTurnStatus::InProgress,
            started_at: None,
            completed_at: None,
        })
    }

    pub fn turn_interrupt(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<CodexTurn, CodexAppServerError> {
        self.ensure_thread_loaded(thread_id)?;
        let _: Value = self.request(
            "turn/interrupt",
            json!({ "threadId": thread_id, "turnId": turn_id }),
        )?;
        Ok(CodexTurn {
            id: turn_id.to_string(),
            status: CodexTurnStatus::Interrupted,
            started_at: None,
            completed_at: None,
        })
    }

    pub fn respond_to_approval(
        &self,
        approval: &CodexApprovalRequest,
        decision: ApprovalDecision,
    ) -> Result<(), CodexAppServerError> {
        self.ensure_thread_loaded(&approval.thread_id)?;
        let native_decision = match approval.kind {
            CodexApprovalKind::CommandExecution | CodexApprovalKind::FileChange => match decision {
                ApprovalDecision::Approve => "accept",
                ApprovalDecision::Deny => "decline",
            },
        };
        if !approval.available_decisions.is_empty()
            && !approval
                .available_decisions
                .iter()
                .any(|available| available == native_decision)
        {
            return Err(CodexAppServerError::Protocol(format!(
                "Codex approval does not offer decision {native_decision}"
            )));
        }
        self.respond(&approval.request_id, json!({ "decision": native_decision }))
    }

    fn initialize(&self) -> Result<(), CodexAppServerError> {
        self.initialize_with_timeout(INITIALIZE_TIMEOUT)
    }

    fn initialize_with_timeout(&self, timeout: Duration) -> Result<(), CodexAppServerError> {
        let result = self.request_value_with_timeout(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "code-pet",
                    "title": "Code Pet",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false,
                    "optOutNotificationMethods": [],
                },
            }),
            timeout,
        )?;
        let _: Value = serde_json::from_value(result).map_err(|error| {
            CodexAppServerError::Protocol(format!("invalid initialize response: {error}"))
        })?;
        self.notify("initialized", json!({}))
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, CodexAppServerError> {
        self.request_outcome(method, params).into_result()
    }

    fn request_outcome<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> CodexRequestOutcome<T> {
        self.request_value_outcome(method, params)
            .and_then(|result| match serde_json::from_value(result) {
                Ok(result) => CodexRequestOutcome::Success(result),
                Err(error) => CodexRequestOutcome::SentOutcomeUnknown(
                    CodexAppServerError::Protocol(format!(
                        "invalid {method} response: {error}"
                    )),
                ),
            })
    }

    fn request_value_outcome(
        &self,
        method: &str,
        params: Value,
    ) -> CodexRequestOutcome<Value> {
        self.request_value_with_timeout_outcome(method, params, REQUEST_TIMEOUT)
    }

    fn request_value_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CodexAppServerError> {
        self.request_value_with_timeout_outcome(method, params, timeout)
            .into_result()
    }

    fn request_value_with_timeout_outcome(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> CodexRequestOutcome<Value> {
        if !self.is_running() {
            return CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown);
        }
        let id = JsonRpcId::Number(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let (sender, receiver) = mpsc::channel();
        let mut pending = match self.inner.pending.lock() {
            Ok(pending) => pending,
            Err(_) => {
                return CodexRequestOutcome::NotSent(CodexAppServerError::Protocol(
                    "pending map lock is poisoned".to_string(),
                ))
            }
        };
        pending.insert(id.clone(), sender);
        drop(pending);
        let write_outcome = self.inner.write_request(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        match write_outcome {
            CodexRequestOutcome::Success(()) => {}
            CodexRequestOutcome::NotSent(error) => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.remove(&id);
                }
                return CodexRequestOutcome::NotSent(error);
            }
            CodexRequestOutcome::ExplicitRpcReject(error) => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.remove(&id);
                }
                return CodexRequestOutcome::ExplicitRpcReject(error);
            }
            CodexRequestOutcome::SentOutcomeUnknown(error) => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.remove(&id);
                }
                return CodexRequestOutcome::SentOutcomeUnknown(error);
            }
        }
        match receiver.recv_timeout(timeout) {
            Ok(Ok(result)) => CodexRequestOutcome::Success(result),
            Ok(Err(error @ CodexAppServerError::Rpc { .. })) => {
                CodexRequestOutcome::ExplicitRpcReject(error)
            }
            Ok(Err(error)) => CodexRequestOutcome::SentOutcomeUnknown(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.remove(&id);
                }
                let error = CodexAppServerError::Timeout(format!(
                    "{method} did not respond within {} ms",
                    timeout.as_millis()
                ));
                self.inner.fail(error.clone());
                CodexRequestOutcome::SentOutcomeUnknown(error)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => CodexRequestOutcome::SentOutcomeUnknown(
                CodexAppServerError::ProcessExited,
            ),
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), CodexAppServerError> {
        self.write(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn respond(&self, id: &JsonRpcId, result: Value) -> Result<(), CodexAppServerError> {
        self.write(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
    }

    fn write(&self, message: Value) -> Result<(), CodexAppServerError> {
        self.inner.write(message)
    }
}

fn codex_app_server_command(binary: &Path, args: &[String]) -> Command {
    let mut command = Command::new(binary);
    command.args(args);
    command
}

fn snapshot_from_response(
    response: ThreadResponse,
    workspace_root: Option<String>,
    permission_level: Option<CodexPermissionLevel>,
) -> CodexConversationSnapshot {
    CodexConversationSnapshot {
        thread: response.thread,
        workspace_root,
        permission_level: permission_level.or_else(|| permission_from_sandbox(response.sandbox.as_ref())),
        model: response.model,
        reasoning_effort: response.reasoning_effort,
    }
}

fn handle_message(inner: &SessionInner, message: Value) -> Result<(), CodexAppServerError> {
    if let Some(version) = message.get("jsonrpc").and_then(Value::as_str) {
        if version != "2.0" {
            return Err(CodexAppServerError::Protocol(format!(
                "unsupported JSON-RPC version {version}"
            )));
        }
    }
    if message.get("method").is_some() {
        let incoming = parse_incoming(message)?;
        if let CodexIncoming::UnsupportedServerRequest {
            request_id,
            method,
        } = &incoming
        {
            let response = json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {
                    "code": -32601,
                    "message": format!(
                        "Code Pet does not support Codex server request {method}"
                    ),
                    "data": {
                        "method": method,
                        "reason": "unsupported_client_capability"
                    }
                }
            });
            inner.write(response)?;
        }
        if let Some(thread_id) = incoming_thread_id(&incoming) {
            inner.mark_thread_loaded(thread_id);
        }
        inner.broadcast(Ok(incoming));
        return Ok(());
    }
    let id: JsonRpcId = serde_json::from_value(
        message
            .get("id")
            .cloned()
            .ok_or_else(|| CodexAppServerError::Protocol("response is missing id".to_string()))?,
    )
    .map_err(|error| CodexAppServerError::Protocol(format!("invalid response id: {error}")))?;
    let sender = inner
        .pending
        .lock()
        .map_err(|_| CodexAppServerError::Protocol("pending map lock is poisoned".to_string()))?
        .remove(&id)
        .ok_or_else(|| {
            CodexAppServerError::Protocol(format!("response has no matching request id {id}"))
        })?;
    let response = if let Some(error) = message.get("error") {
        Err(CodexAppServerError::Rpc {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown app-server error")
                .to_string(),
        })
    } else {
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    };
    let _ = sender.send(response);
    Ok(())
}

fn incoming_thread_id(incoming: &CodexIncoming) -> Option<&str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::ThreadStarted { thread }) => {
            Some(&thread.id)
        }
        CodexIncoming::Notification(
            CodexNotification::TurnStarted { thread_id, .. }
            | CodexNotification::TurnCompleted { thread_id, .. }
            | CodexNotification::OutputDelta { thread_id, .. }
            | CodexNotification::ServerRequestResolved { thread_id, .. },
        ) => Some(thread_id),
        CodexIncoming::ApprovalRequested(approval) => Some(&approval.thread_id),
        CodexIncoming::Notification(CodexNotification::Unknown { .. })
        | CodexIncoming::UnsupportedServerRequest { .. } => None,
    }
}

fn parse_incoming(message: Value) -> Result<CodexIncoming, CodexAppServerError> {
    let method = required_string(&message, "method")?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    if let Some(id_value) = message.get("id") {
        let request_id: JsonRpcId = serde_json::from_value(id_value.clone()).map_err(|error| {
            CodexAppServerError::Protocol(format!("invalid server request id: {error}"))
        })?;
        return parse_server_request(request_id, &method, &params);
    }
    parse_notification(&method, params).map(CodexIncoming::Notification)
}

fn parse_server_request(
    request_id: JsonRpcId,
    method: &str,
    params: &Value,
) -> Result<CodexIncoming, CodexAppServerError> {
    let kind = match method {
        "item/commandExecution/requestApproval" => CodexApprovalKind::CommandExecution,
        "item/fileChange/requestApproval" => CodexApprovalKind::FileChange,
        _ => {
            return Ok(CodexIncoming::UnsupportedServerRequest {
                request_id,
                method: method.to_string(),
            })
        }
    };
    let thread_id = required_string(params, "threadId")?;
    let turn_id = required_string(params, "turnId")?;
    let item_id = required_string(params, "itemId")?;
    let requested_at_ms = params
        .get("startedAtMs")
        .and_then(Value::as_i64)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    let available_decisions: Vec<String> = params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !available_decisions.is_empty()
        && !available_decisions
            .iter()
            .any(|decision| decision == "accept" || decision == "decline")
    {
        return Ok(CodexIncoming::UnsupportedServerRequest {
            request_id,
            method: method.to_string(),
        });
    }
    let (title, description) = match kind {
        CodexApprovalKind::CommandExecution => (
            "Run command".to_string(),
            optional_string(params, "command").or_else(|| optional_string(params, "reason")),
        ),
        CodexApprovalKind::FileChange => (
            "Apply file changes".to_string(),
            optional_string(params, "reason"),
        ),
    };
    Ok(CodexIncoming::ApprovalRequested(CodexApprovalRequest {
        request_id,
        kind,
        thread_id,
        turn_id,
        item_id,
        title,
        description,
        requested_at_ms,
        available_decisions,
    }))
}

fn parse_notification(
    method: &str,
    params: Value,
) -> Result<CodexNotification, CodexAppServerError> {
    match method {
        "thread/started" => Ok(CodexNotification::ThreadStarted {
            thread: deserialize_field(&params, "thread")?,
        }),
        "turn/started" => Ok(CodexNotification::TurnStarted {
            thread_id: required_string(&params, "threadId")?,
            turn: deserialize_field(&params, "turn")?,
        }),
        "turn/completed" => Ok(CodexNotification::TurnCompleted {
            thread_id: required_string(&params, "threadId")?,
            turn: deserialize_field(&params, "turn")?,
        }),
        "item/agentMessage/delta"
        | "item/commandExecution/outputDelta"
        | "item/fileChange/outputDelta"
        | "item/reasoning/textDelta"
        | "item/reasoning/summaryTextDelta" => {
            let kind = match method {
                "item/agentMessage/delta" => "assistant-message",
                "item/commandExecution/outputDelta" => "command-output",
                "item/fileChange/outputDelta" => "file-change-output",
                _ => "reasoning",
            };
            Ok(CodexNotification::OutputDelta {
                native_method: method.to_string(),
                thread_id: required_string(&params, "threadId")?,
                turn_id: required_string(&params, "turnId")?,
                item_id: required_string(&params, "itemId")?,
                kind: kind.to_string(),
                delta: required_string(&params, "delta")?,
            })
        }
        "serverRequest/resolved" => Ok(CodexNotification::ServerRequestResolved {
            request_id: deserialize_field(&params, "requestId")?,
            thread_id: required_string(&params, "threadId")?,
        }),
        _ => Ok(CodexNotification::Unknown {
            method: method.to_string(),
        }),
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, CodexAppServerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CodexAppServerError::Protocol(format!("message is missing {key}")))
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn deserialize_field<T: DeserializeOwned>(
    value: &Value,
    key: &str,
) -> Result<T, CodexAppServerError> {
    serde_json::from_value(
        value
            .get(key)
            .cloned()
            .ok_or_else(|| CodexAppServerError::Protocol(format!("message is missing {key}")))?,
    )
    .map_err(|error| CodexAppServerError::Protocol(format!("invalid {key}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, Sender};
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

    impl JsonRpcWriter for MockWriter {
        fn write_message(&mut self, message: &Value) -> Result<(), CodexAppServerError> {
            self.sender
                .send(message.clone())
                .map_err(|error| CodexAppServerError::Io(error.to_string()))
        }
    }

    fn mock_session() -> (CodexAppServerSession, Receiver<Value>, Sender<Value>) {
        let (to_peer_sender, peer_receiver) = mpsc::channel::<Value>();
        let (peer_sender, client_receiver) = mpsc::channel::<Value>();
        let peer = thread::spawn(move || {
            let initialize = peer_receiver.recv().unwrap();
            assert_eq!(initialize["method"], "initialize");
            peer_sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": initialize["id"],
                    "result": { "userAgent": "codex-cli/fixture" }
                }))
                .unwrap();
            let initialized = peer_receiver.recv().unwrap();
            assert_eq!(initialized["method"], "initialized");
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

    #[test]
    fn app_server_command_uses_the_resolved_executable_path() {
        let executable = Path::new("/resolved/runtime/codex");
        let args = vec![
            "app-server".to_string(),
            "--listen".to_string(),
            "stdio://".to_string(),
        ];
        let command = codex_app_server_command(executable, &args);

        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["app-server", "--listen", "stdio://"]
                .iter()
                .map(std::ffi::OsStr::new)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn initialize_timeout_fails_closed_instead_of_blocking_forever() {
        let (outgoing_sender, _outgoing_receiver) = mpsc::channel::<Value>();
        let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();

        let error = CodexAppServerSession::connect_with_initialize_timeout(
            Box::new(MockReader {
                receiver: incoming_receiver,
            }),
            Box::new(MockWriter {
                sender: outgoing_sender,
            }),
            Duration::from_millis(20),
        )
        .err()
        .expect("a silent App Server must time out");
        drop(incoming_sender);

        assert!(matches!(error, CodexAppServerError::Timeout(_)));
    }

    #[test]
    fn persistent_session_initializes_once_and_matches_out_of_order_responses() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let first_session = session.clone();
        let first = thread::spawn(move || {
            first_session.thread_read("thread-one").unwrap().thread.id
        });
        let second_session = session.clone();
        let second = thread::spawn(move || {
            second_session.thread_read("thread-two").unwrap().thread.id
        });
        let request_a = peer_receiver.recv().unwrap();
        let request_b = peer_receiver.recv().unwrap();
        assert_eq!(request_a["method"], "thread/read");
        assert_eq!(request_b["method"], "thread/read");
        for request in [&request_b, &request_a] {
            let thread_id = request["params"]["threadId"].as_str().unwrap();
            peer_sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": {
                        "thread": thread_fixture(thread_id, "/tmp/project", "idle", vec![])
                    }
                }))
                .unwrap();
        }
        let mut ids = vec![first.join().unwrap(), second.join().unwrap()];
        ids.sort();
        assert_eq!(ids, vec!["thread-one", "thread-two"]);
        assert!(session.is_running());
        session.shutdown().unwrap();
        assert!(!session.is_running());
    }

    #[test]
    fn session_keeps_reading_notifications_between_requests() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe();
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "method": "turn/started",
                "params": {
                    "threadId": "thread-one",
                    "turn": turn_fixture("turn-one", "inProgress")
                }
            }))
            .unwrap();
        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::Notification(CodexNotification::TurnStarted { thread_id, .. })
                if thread_id == "thread-one"
        ));
        session.shutdown().unwrap();
    }

    #[test]
    fn peer_exit_is_reported_to_subscribers_and_stops_the_session() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe();
        drop(peer_sender);
        let error = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert_eq!(error, CodexAppServerError::ProcessExited);
        assert!(!session.is_running());
    }

    #[test]
    fn stopped_session_reports_not_sent_without_allocating_or_writing() {
        let (session, peer_receiver, peer_sender) = mock_session();
        session.shutdown().unwrap();
        let next_id = session.inner.next_id.load(Ordering::SeqCst);

        let outcome = session.ensure_thread_loaded_outcome("thread-stopped");

        assert_eq!(
            outcome,
            CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown)
        );
        assert_eq!(session.inner.next_id.load(Ordering::SeqCst), next_id);
        assert!(session.inner.pending.lock().unwrap().is_empty());
        assert!(peer_receiver.try_recv().is_err());
        drop(peer_sender);
    }

    #[test]
    fn unmatched_response_id_is_a_protocol_error() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe();
        peer_sender
            .send(json!({ "jsonrpc": "2.0", "id": 999, "result": {} }))
            .unwrap();
        let error = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, CodexAppServerError::Protocol(_)));
        assert!(!session.is_running());
    }

    #[test]
    fn typed_operations_preserve_normal_and_project_thread_start_parameters() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let operation_session = session.clone();
        let operations = thread::spawn(move || {
            let page = operation_session
                .thread_list(CodexThreadListRequest {
                    cursor: Some("cursor-one".to_string()),
                    limit: Some(20),
                    workspace_root: Some("/work/project".to_string()),
                })
                .unwrap();
            assert_eq!(page.data.len(), 1);
            let read = operation_session.thread_read("thread-read").unwrap();
            assert_eq!(read.thread.id, "thread-read");
            let normal = operation_session
                .thread_start(CodexThreadStartRequest {
                    workspace_root: None,
                    permission_level: CodexPermissionLevel::ReadOnly,
                    model: None,
                    reasoning_effort: None,
                })
                .unwrap();
            assert_eq!(normal.workspace_root, None);
            assert_eq!(
                normal.permission_level,
                Some(CodexPermissionLevel::ReadOnly)
            );
            let project = operation_session
                .thread_start(CodexThreadStartRequest {
                    workspace_root: Some("/work/project".to_string()),
                    permission_level: CodexPermissionLevel::FullAccess,
                    model: Some("gpt-fixture".to_string()),
                    reasoning_effort: Some("high".to_string()),
                })
                .unwrap();
            assert_eq!(project.workspace_root.as_deref(), Some("/work/project"));
            assert_eq!(
                project.permission_level,
                Some(CodexPermissionLevel::FullAccess)
            );
            assert_eq!(project.model.as_deref(), Some("gpt-fixture"));
            assert_eq!(project.reasoning_effort.as_deref(), Some("high"));
            let turn = operation_session
                .turn_start(CodexTurnStartRequest {
                    thread_id: "thread-project".to_string(),
                    message: "hello".to_string(),
                    client_message_id: Some("message-one".to_string()),
                    model: Some("gpt-fixture".to_string()),
                    reasoning_effort: Some("high".to_string()),
                })
                .unwrap();
            assert_eq!(turn.id, "turn-started");
            let steered = operation_session
                .turn_steer(CodexTurnSteerRequest {
                    thread_id: "thread-project".to_string(),
                    expected_turn_id: "turn-started".to_string(),
                    message: "steer".to_string(),
                    client_message_id: Some("message-two".to_string()),
                })
                .unwrap();
            assert_eq!(steered.id, "turn-started");
            let interrupted = operation_session
                .turn_interrupt("thread-project", "turn-started")
                .unwrap();
            assert_eq!(interrupted.status, CodexTurnStatus::Interrupted);
        });

        let list = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(list["method"], "thread/list");
        assert_eq!(list["params"]["cwd"], "/work/project");
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": list["id"],
                "result": {
                    "data": [thread_fixture("thread-listed", "/work/project", "idle", vec![])],
                    "nextCursor": null
                }
            }))
            .unwrap();

        let read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(read["method"], "thread/read");
        assert_eq!(read["params"]["includeTurns"], true);
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": read["id"],
                "result": {
                    "thread": thread_fixture("thread-read", "/tmp", "idle", vec![])
                }
            }))
            .unwrap();

        let normal = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(normal["method"], "thread/start");
        assert!(normal["params"].get("cwd").is_none());
        assert_eq!(normal["params"]["sandbox"], "read-only");
        assert_eq!(normal["params"]["approvalPolicy"], "on-request");
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": normal["id"],
                "result": {
                    "thread": thread_fixture("thread-normal", "/tmp", "idle", vec![]),
                    "model": "default",
                    "reasoningEffort": "medium",
                    "sandbox": {
                        "type": "readOnly",
                        "networkAccess": false
                    }
                }
            }))
            .unwrap();

        let project = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(project["method"], "thread/start");
        assert_eq!(project["params"]["cwd"], "/work/project");
        assert_eq!(project["params"]["sandbox"], "danger-full-access");
        assert_eq!(project["params"]["approvalPolicy"], "never");
        assert_eq!(
            project["params"]["config"]["model_reasoning_effort"],
            "high"
        );
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": project["id"],
                "result": {
                    "thread": thread_fixture("thread-project", "/work/project", "idle", vec![]),
                    "model": "gpt-fixture",
                    "reasoningEffort": "high",
                    "sandbox": {
                        "type": "dangerFullAccess"
                    }
                }
            }))
            .unwrap();

        let start = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(start["method"], "turn/start");
        assert_eq!(start["params"]["input"][0]["text"], "hello");
        assert_eq!(start["params"]["effort"], "high");
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": start["id"],
                "result": { "turn": turn_fixture("turn-started", "inProgress") }
            }))
            .unwrap();

        let steer = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(steer["method"], "turn/steer");
        assert_eq!(steer["params"]["expectedTurnId"], "turn-started");
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": steer["id"],
                "result": { "turnId": "turn-started" }
            }))
            .unwrap();

        let interrupt = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(interrupt["method"], "turn/interrupt");
        assert_eq!(interrupt["params"]["turnId"], "turn-started");
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": interrupt["id"],
                "result": {}
            }))
            .unwrap();

        operations.join().unwrap();
        session.shutdown().unwrap();
    }

    #[test]
    fn historical_threads_resume_once_per_app_server_session() {
        for session_number in 1..=2 {
            let (session, peer_receiver, peer_sender) = mock_session();
            let operation_session = session.clone();
            let operation = thread::spawn(move || {
                for message_number in 1..=2 {
                    operation_session
                        .turn_start(CodexTurnStartRequest {
                            thread_id: "thread-historical".to_string(),
                            message: format!("message {message_number}"),
                            client_message_id: Some(format!(
                                "session-{session_number}-message-{message_number}"
                            )),
                            model: None,
                            reasoning_effort: None,
                        })
                        .unwrap();
                }
            });

            let resume = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(resume["method"], "thread/resume");
            assert_eq!(resume["params"]["threadId"], "thread-historical");
            peer_sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": resume["id"],
                    "result": {
                        "thread": thread_fixture(
                            "thread-historical",
                            "/tmp/project",
                            "idle",
                            vec![]
                        ),
                        "sandbox": {
                            "type": "workspaceWrite",
                            "writableRoots": [],
                            "networkAccess": false,
                            "excludeTmpdirEnvVar": false,
                            "excludeSlashTmp": false
                        }
                    }
                }))
                .unwrap();

            for message_number in 1..=2 {
                let start = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(start["method"], "turn/start");
                assert_eq!(
                    start["params"]["clientUserMessageId"],
                    format!("session-{session_number}-message-{message_number}")
                );
                peer_sender
                    .send(json!({
                        "jsonrpc": "2.0",
                        "id": start["id"],
                        "result": {
                            "turn": turn_fixture(
                                &format!("turn-{session_number}-{message_number}"),
                                "inProgress"
                            )
                        }
                    }))
                    .unwrap();
            }

            operation.join().unwrap();
            assert!(peer_receiver.try_recv().is_err());
            session.shutdown().unwrap();
        }
    }

    #[test]
    fn failed_resume_wakes_waiter_to_retry_without_caching_failure() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let first_session = session.clone();
        let first = thread::spawn(move || {
            first_session.ensure_thread_loaded_outcome("thread-missing")
        });
        let first_resume = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(first_resume["method"], "thread/resume");

        let second_session = session.clone();
        let second = thread::spawn(move || {
            second_session.ensure_thread_loaded_outcome("thread-missing")
        });
        assert!(peer_receiver
            .recv_timeout(Duration::from_millis(20))
            .is_err());
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": first_resume["id"],
                "error": {
                    "code": -32600,
                    "message": "no rollout found for thread id thread-missing"
                }
            }))
            .unwrap();

        let second_resume = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(second_resume["method"], "thread/resume");
        assert_eq!(second_resume["params"]["threadId"], "thread-missing");
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": second_resume["id"],
                "error": {
                    "code": -32600,
                    "message": "no rollout found for thread id thread-missing"
                }
            }))
            .unwrap();

        for outcome in [first.join().unwrap(), second.join().unwrap()] {
            assert!(matches!(
                outcome,
                CodexRequestOutcome::ExplicitRpcReject(CodexAppServerError::Rpc {
                    code: -32600,
                    message
                }) if message == "no rollout found for thread id thread-missing"
            ));
        }
        assert!(peer_receiver.try_recv().is_err());
        session.shutdown().unwrap();
    }

    #[test]
    fn approval_and_unsupported_server_requests_receive_terminal_responses() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let notifications = session.subscribe();
        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": "approval-one",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "item-one",
                    "startedAtMs": 123,
                    "command": "cargo test",
                    "availableDecisions": ["accept", "acceptForSession", "decline", "cancel"],
                    "futureField": { "mustNotLeak": true }
                }
            }))
            .unwrap();
        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        let CodexIncoming::ApprovalRequested(approval) = incoming else {
            panic!("expected approval request");
        };
        assert_eq!(approval.kind, CodexApprovalKind::CommandExecution);
        assert_eq!(approval.description.as_deref(), Some("cargo test"));
        session
            .respond_to_approval(&approval, ApprovalDecision::Approve)
            .unwrap();
        let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(response["id"], "approval-one");
        assert_eq!(response["result"]["decision"], "accept");
        assert!(response.get("method").is_none());

        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "item/permissions/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "item-two",
                    "environmentId": null,
                    "startedAtMs": 124,
                    "cwd": "/tmp/project",
                    "reason": "Allow network access",
                    "permissions": {
                        "network": { "enabled": true },
                        "fileSystem": null
                    }
                }
            }))
            .unwrap();
        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::UnsupportedServerRequest { request_id: JsonRpcId::Number(2), method }
                if method == "item/permissions/requestApproval"
        ));
        let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(response["id"], 2);
        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(
            response["error"]["data"]["reason"],
            "unsupported_client_capability"
        );
        assert!(response.get("result").is_none());

        peer_sender
            .send(json!({
                "jsonrpc": "2.0",
                "id": "unknown-one",
                "method": "item/future/request",
                "params": {}
            }))
            .unwrap();
        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::UnsupportedServerRequest {
                request_id: JsonRpcId::String(request_id),
                method
            } if request_id == "unknown-one" && method == "item/future/request"
        ));
        let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(response["id"], "unknown-one");
        assert_eq!(response["error"]["code"], -32601);
        assert!(session.is_running());
        session.shutdown().unwrap();
    }

    fn thread_fixture(id: &str, cwd: &str, status: &str, turns: Vec<Value>) -> Value {
        json!({
            "id": id,
            "name": null,
            "preview": "fixture",
            "cwd": cwd,
            "createdAt": 100,
            "updatedAt": 101,
            "status": { "type": status },
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
}
