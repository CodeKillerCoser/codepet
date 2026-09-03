use super::protocol::{
    permission_from_sandbox, thread_list_params, thread_start_params, turn_start_params,
    turn_steer_params, output_content_id, reasoning_summary_content_id, text_content_id,
    CodexAppServerError, CodexApprovalKind, CodexApprovalRequest, CodexContentKind,
    CodexConversationSnapshot, CodexIncoming, CodexModel, CodexModelListResponse,
    CodexNotification, CodexPermissionLevel, CodexThreadListRequest, CodexThreadPage,
    CodexThreadStartRequest, CodexTurn, CodexTurnPage, CodexTurnStatus,
    CodexTurnItemsView, CodexTurnStartRequest, CodexTurnSteerRequest, CommandApprovalParams, FileApprovalParams,
    InitializeResponse, JsonRpcId, ThreadConfiguredResponse, ThreadListResponse,
    ThreadReadResponse, ThreadTurnsListResponse, TurnResponse, TurnSteerResponse,
};
use crate::workspace_projection::project_workspace_root;
use codepet_provider_sdk::ApprovalDecision;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{self, Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_APP_SERVER_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_APP_SERVER_STDERR_LINE_BYTES: usize = 64 * 1024;
pub(crate) const THREAD_TURNS_PAGE_LIMIT: u32 = 10;
static NEXT_SESSION_GENERATION: AtomicU64 = AtomicU64::new(1);

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
        let Some(line) = read_bounded_line(&mut self.reader, MAX_APP_SERVER_FRAME_BYTES)? else {
            return Ok(None);
        };
        serde_json::from_slice(&line)
            .map(Some)
            .map_err(|error| CodexAppServerError::Protocol(format!("invalid JSON: {error}")))
    }
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    limit: usize,
) -> Result<Option<Vec<u8>>, CodexAppServerError> {
    let mut captured = Vec::new();
    let mut total = 0usize;
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| CodexAppServerError::Io(error.to_string()))?;
        if available.is_empty() {
            if total == 0 {
                return Ok(None);
            }
            break;
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        total = total.checked_add(consumed).ok_or_else(|| {
            CodexAppServerError::Protocol("App Server line length overflow".to_string())
        })?;
        if captured.len() < limit {
            let remaining = limit - captured.len();
            captured.extend_from_slice(&available[..consumed.min(remaining)]);
        }
        let ended = available[consumed - 1] == b'\n';
        reader.consume(consumed);
        if ended {
            break;
        }
    }
    if total > limit {
        return Err(CodexAppServerError::Protocol(format!(
            "App Server physical line exceeds {limit} bytes"
        )));
    }
    Ok(Some(captured))
}

fn drain_stderr(stderr: impl Read) -> Result<(), CodexAppServerError> {
    let mut reader = BufReader::new(stderr);
    loop {
        match read_bounded_line(&mut reader, MAX_APP_SERVER_STDERR_LINE_BYTES) {
            Ok(Some(line)) => {
                let line = String::from_utf8_lossy(&line);
                eprintln!("Codex App Server: {}", line.trim_end());
            }
            Ok(None) => return Ok(()),
            Err(error) => return Err(error),
        }
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
    generation: String,
    writer: Mutex<Option<Box<dyn JsonRpcWriter>>>,
    pending: Mutex<HashMap<JsonRpcId, Sender<Result<Value, CodexAppServerError>>>>,
    observers: Mutex<SessionObservers>,
    loaded_threads: Mutex<HashMap<String, ThreadLoadState>>,
    thread_configurations: Mutex<HashMap<String, ThreadConfiguration>>,
    loaded_threads_changed: Condvar,
    next_load_evidence: AtomicU64,
    next_id: AtomicI64,
    running: AtomicBool,
    control: Mutex<Option<Box<dyn SessionControl>>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    harness_version: Mutex<Option<String>>,
}

struct SessionObservers {
    terminal_fault: Option<CodexAppServerError>,
    subscribers: Vec<Sender<Result<CodexIncoming, CodexAppServerError>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThreadLoadState {
    Resuming,
    Loaded(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ThreadConfiguration {
    pub(crate) workspace_root: Option<String>,
    pub(crate) permission_level: Option<CodexPermissionLevel>,
    pub(crate) model: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
}

impl ThreadConfiguration {
    fn from_snapshot(snapshot: &CodexConversationSnapshot) -> Self {
        Self {
            workspace_root: snapshot.workspace_root.clone(),
            permission_level: snapshot.permission_level,
            model: snapshot.model.clone(),
            reasoning_effort: snapshot.reasoning_effort.clone(),
        }
    }

    fn apply_to(&self, snapshot: &mut CodexConversationSnapshot) {
        snapshot.workspace_root = self.workspace_root.clone();
        snapshot.permission_level = self.permission_level;
        snapshot.model = self.model.clone();
        snapshot.reasoning_effort = self.reasoning_effort.clone();
    }
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
        let mut observers = self
            .observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.running.load(Ordering::SeqCst) || observers.terminal_fault.is_some() {
            return;
        }
        observers
            .subscribers
            .retain(|subscriber| subscriber.send(message.clone()).is_ok());
    }

    fn fail(&self, error: CodexAppServerError) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        {
            let mut observers = self
                .observers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            observers.terminal_fault = Some(error.clone());
            observers
                .subscribers
                .retain(|subscriber| subscriber.send(Err(error.clone())).is_ok());
        }
        if let Ok(mut pending) = self.pending.lock() {
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(error.clone()));
            }
        }
        if let Ok(mut writer) = self.writer.lock() {
            writer.take();
        }
        let termination = self
            .control
            .lock()
            .map_err(|_| CodexAppServerError::Protocol("process control lock is poisoned".to_string()))
            .and_then(|mut control| match control.as_mut() {
                Some(control) => control.shutdown(),
                None => Ok(()),
            });
        if let Err(termination_error) = termination {
            eprintln!("Codex App Server termination after terminal fault failed: {termination_error}");
        }
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

    fn record_configuration(&self, snapshot: &CodexConversationSnapshot) {
        if let Ok(mut configurations) = self.thread_configurations.lock() {
            configurations.insert(
                snapshot.thread.id.clone(),
                ThreadConfiguration::from_snapshot(snapshot),
            );
        }
    }

    fn apply_configuration(&self, snapshot: &mut CodexConversationSnapshot) {
        if let Ok(configurations) = self.thread_configurations.lock() {
            if let Some(configuration) = configurations.get(&snapshot.thread.id) {
                configuration.apply_to(snapshot);
            }
        }
    }

    fn thread_configuration(&self, thread_id: &str) -> Option<ThreadConfiguration> {
        self.thread_configurations
            .lock()
            .ok()
            .and_then(|configurations| configurations.get(thread_id).cloned())
    }

    fn update_turn_configuration(&self, request: &CodexTurnStartRequest) {
        if let Ok(mut configurations) = self.thread_configurations.lock() {
            if let Some(configuration) = configurations.get_mut(&request.thread_id) {
                if request.permission_level.is_some() {
                    configuration.permission_level = request.permission_level;
                }
                if request.model.is_some() {
                    configuration.model = request.model.clone();
                }
                if request.reasoning_effort.is_some() {
                    configuration.reasoning_effort = request.reasoning_effort.clone();
                }
            }
        }
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
    pub(crate) fn spawn_uninitialized(
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
        let stderr = child.stderr.take();
        let session = Self::from_parts(
            Box::new(JsonLineReader::new(stdout)),
            Box::new(JsonLineWriter::new(stdin)),
            Some(Box::new(ChildControl { child })),
        );
        if let Some(stderr) = stderr {
            session.start_stderr_monitor(stderr);
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
            generation: new_session_generation(),
            writer: Mutex::new(Some(writer)),
            pending: Mutex::new(HashMap::new()),
            observers: Mutex::new(SessionObservers {
                terminal_fault: None,
                subscribers: Vec::new(),
            }),
            loaded_threads: Mutex::new(HashMap::new()),
            thread_configurations: Mutex::new(HashMap::new()),
            loaded_threads_changed: Condvar::new(),
            next_load_evidence: AtomicU64::new(1),
            next_id: AtomicI64::new(1),
            running: AtomicBool::new(true),
            control: Mutex::new(control),
            reader_thread: Mutex::new(None),
            harness_version: Mutex::new(None),
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

    pub fn generation(&self) -> &str {
        &self.inner.generation
    }

    pub(crate) fn thread_configuration(&self, thread_id: &str) -> Option<ThreadConfiguration> {
        self.inner.thread_configuration(thread_id)
    }

    pub fn harness_version(&self) -> Option<String> {
        self.inner
            .harness_version
            .lock()
            .ok()
            .and_then(|version| version.clone())
    }

    pub fn subscribe(
        &self,
    ) -> Result<Receiver<Result<CodexIncoming, CodexAppServerError>>, CodexAppServerError> {
        let (sender, receiver) = mpsc::channel();
        let mut observers = self
            .inner
            .observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(error) = observers.terminal_fault.clone() {
            Err(error)
        } else {
            observers.subscribers.push(sender);
            Ok(receiver)
        }
    }

    fn start_stderr_monitor(&self, stderr: impl Read + Send + 'static) {
        let inner = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            if let Err(error) = drain_stderr(stderr) {
                eprintln!("Codex App Server stderr failed: {error}");
                if let Some(inner) = inner.upgrade() {
                    inner.fail(error);
                }
            }
        });
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
        let response: ThreadListResponse =
            self.request("thread/list", thread_list_params(&request))?;
        Ok(CodexThreadPage {
            data: CodexConversationSnapshot::from_threads(response.data),
            next_cursor: response.next_cursor,
        })
    }

    pub fn thread_read(
        &self,
        thread_id: &str,
    ) -> Result<CodexConversationSnapshot, CodexAppServerError> {
        let response: ThreadReadResponse = self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": true }),
        )?;
        if let Some(turn) = response
            .thread
            .turns
            .iter()
            .find(|turn| turn.items_view != CodexTurnItemsView::Full)
        {
            return Err(CodexAppServerError::Protocol(format!(
                "thread/read returned non-full items for turn {}",
                turn.id
            )));
        }
        let mut snapshot = snapshot_from_thread(response.thread);
        self.inner.apply_configuration(&mut snapshot);
        Ok(snapshot)
    }

    pub fn thread_read_metadata(
        &self,
        thread_id: &str,
    ) -> Result<CodexConversationSnapshot, CodexAppServerError> {
        let mut response: ThreadReadResponse = self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": false }),
        )?;
        response.thread.turns = Vec::new();
        let mut snapshot = snapshot_from_thread(response.thread);
        self.inner.apply_configuration(&mut snapshot);
        Ok(snapshot)
    }

    pub fn thread_turns_list(
        &self,
        thread_id: &str,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<CodexTurnPage, CodexAppServerError> {
        let response: ThreadTurnsListResponse = self.request(
            "thread/turns/list",
            json!({
                "threadId": thread_id,
                "cursor": cursor,
                "limit": limit,
                "sortDirection": "desc",
                "itemsView": "full",
            }),
        )?;
        if let Some(turn) = response
            .data
            .iter()
            .find(|turn| turn.items_view != CodexTurnItemsView::Full)
        {
            return Err(CodexAppServerError::Protocol(format!(
                "thread/turns/list returned non-full items for turn {}",
                turn.id
            )));
        }
        Ok(CodexTurnPage {
            data: response.data,
            next_cursor: response.next_cursor,
        })
    }

    pub fn model_list(&self) -> Result<Vec<CodexModel>, CodexAppServerError> {
        const PAGE_LIMIT: u32 = 100;
        const MAX_PAGES: usize = 100;
        let mut cursor = None;
        let mut models = Vec::new();
        for _ in 0..MAX_PAGES {
            let response: CodexModelListResponse = self.request(
                "model/list",
                json!({
                    "cursor": cursor,
                    "limit": PAGE_LIMIT,
                    "includeHidden": false,
                }),
            )?;
            models.extend(response.data);
            match response.next_cursor {
                Some(next_cursor) if Some(&next_cursor) != cursor.as_ref() => {
                    cursor = Some(next_cursor);
                }
                Some(_) => {
                    return Err(CodexAppServerError::Protocol(
                        "model/list returned a repeated cursor".to_string(),
                    ));
                }
                None => return Ok(models),
            }
        }
        Err(CodexAppServerError::Protocol(
            "model/list exceeded the bounded pagination limit".to_string(),
        ))
    }

    fn request_thread_resume_outcome_with_sender(
        &self,
        thread_id: &str,
        send_request: impl FnOnce(Value) -> CodexRequestOutcome<()>,
    ) -> CodexRequestOutcome<CodexConversationSnapshot> {
        self.request_value_with_timeout_outcome_with_sender(
            "thread/resume",
            json!({ "threadId": thread_id }),
            REQUEST_TIMEOUT,
            send_request,
        )
        .and_then(|response| match serde_json::from_value::<ThreadConfiguredResponse>(response) {
            Ok(response) => CodexRequestOutcome::Success(response),
            Err(error) => CodexRequestOutcome::SentOutcomeUnknown(
                CodexAppServerError::Protocol(format!(
                    "invalid thread/resume response: {error}"
                )),
            ),
        })
        .and_then(|response| {
            let snapshot = snapshot_from_configured_response(response);
            if snapshot.thread.id != thread_id {
                return CodexRequestOutcome::SentOutcomeUnknown(
                    CodexAppServerError::Protocol(format!(
                        "thread/resume returned thread {} for requested thread {thread_id}",
                        snapshot.thread.id
                    )),
                );
            }
            self.inner.record_configuration(&snapshot);
            CodexRequestOutcome::Success(snapshot)
        })
    }

    pub(crate) fn ensure_thread_loaded_outcome(
        &self,
        thread_id: &str,
    ) -> CodexRequestOutcome<u64> {
        self.ensure_thread_loaded_outcome_with_sender(thread_id, |message| {
            self.inner.write_request(message)
        })
    }

    pub(crate) fn ensure_thread_loaded_outcome_with_sender(
        &self,
        thread_id: &str,
        send_request: impl FnOnce(Value) -> CodexRequestOutcome<()>,
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
        let result = self.request_thread_resume_outcome_with_sender(thread_id, send_request);
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

    #[cfg(test)]
    pub fn thread_start(
        &self,
        request: CodexThreadStartRequest,
    ) -> Result<CodexConversationSnapshot, CodexAppServerError> {
        self.thread_start_outcome(request).into_result()
    }

    #[cfg(test)]
    fn thread_start_outcome(
        &self,
        request: CodexThreadStartRequest,
    ) -> CodexRequestOutcome<CodexConversationSnapshot> {
        self.thread_start_outcome_with_sender(request, |message| {
            self.inner.write_request(message)
        })
    }

    pub(crate) fn thread_start_outcome_with_sender(
        &self,
        request: CodexThreadStartRequest,
        send_request: impl FnOnce(Value) -> CodexRequestOutcome<()>,
    ) -> CodexRequestOutcome<CodexConversationSnapshot> {
        self.request_value_with_timeout_outcome_with_sender(
            "thread/start",
            thread_start_params(&request),
            REQUEST_TIMEOUT,
            send_request,
        )
        .and_then(|response| match serde_json::from_value::<ThreadConfiguredResponse>(response) {
            Ok(response) => CodexRequestOutcome::Success(response),
            Err(error) => CodexRequestOutcome::SentOutcomeUnknown(
                CodexAppServerError::Protocol(format!(
                    "invalid thread/start response: {error}"
                )),
            ),
        })
            .and_then(|response: ThreadConfiguredResponse| {
                let snapshot = snapshot_from_configured_response(response);
                self.inner.record_configuration(&snapshot);
                self.inner.mark_thread_loaded(&snapshot.thread.id);
                CodexRequestOutcome::Success(snapshot)
            })
    }

    #[cfg(test)]
    pub fn turn_start(
        &self,
        request: CodexTurnStartRequest,
    ) -> Result<CodexTurn, CodexAppServerError> {
        self.turn_start_outcome(request).into_result()
    }

    pub(crate) fn turn_start_outcome(
        &self,
        request: CodexTurnStartRequest,
    ) -> CodexRequestOutcome<CodexTurn> {
        self.ensure_thread_loaded_outcome(&request.thread_id)
            .and_then(|_| {
                self.request_outcome("turn/start", turn_start_params(&request))
                    .and_then(|response: TurnResponse| {
                        self.inner.update_turn_configuration(&request);
                        CodexRequestOutcome::Success(response.turn)
                    })
            })
    }

    #[cfg(test)]
    pub fn turn_steer(
        &self,
        request: CodexTurnSteerRequest,
    ) -> Result<CodexTurn, CodexAppServerError> {
        self.turn_steer_outcome(request).into_result()
    }

    pub(crate) fn turn_steer_outcome(
        &self,
        request: CodexTurnSteerRequest,
    ) -> CodexRequestOutcome<CodexTurn> {
        self.ensure_thread_loaded_outcome(&request.thread_id)
            .and_then(|_| {
                self.request_outcome("turn/steer", turn_steer_params(&request))
                    .and_then(|response: TurnSteerResponse| {
                        match self.authoritative_turn(&request.thread_id, &response.turn_id) {
                            Ok(turn) => CodexRequestOutcome::Success(turn),
                            Err(error) => CodexRequestOutcome::SentOutcomeUnknown(error),
                        }
                    })
            })
    }

    #[cfg(test)]
    pub fn turn_interrupt(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<CodexTurn, CodexAppServerError> {
        self.turn_interrupt_outcome(thread_id, turn_id).into_result()
    }

    pub(crate) fn turn_interrupt_outcome(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> CodexRequestOutcome<CodexTurn> {
        self.ensure_thread_loaded_outcome(thread_id).and_then(|_| {
            self.request_outcome(
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
            )
            .and_then(|_: Value| match self.authoritative_turn(thread_id, turn_id) {
                Ok(turn) => CodexRequestOutcome::Success(turn),
                Err(error) => CodexRequestOutcome::SentOutcomeUnknown(error),
            })
        })
    }

    #[cfg(test)]
    pub fn respond_to_approval(
        &self,
        approval: &CodexApprovalRequest,
        decision: ApprovalDecision,
    ) -> Result<(), CodexAppServerError> {
        self.respond_to_approval_outcome(approval, decision)
            .into_result()
    }

    pub(crate) fn respond_to_approval_outcome(
        &self,
        approval: &CodexApprovalRequest,
        decision: ApprovalDecision,
    ) -> CodexRequestOutcome<()> {
        self.ensure_thread_loaded_outcome(&approval.thread_id)
            .and_then(|_| {
                let native_decision = match approval.kind {
                    CodexApprovalKind::CommandExecution | CodexApprovalKind::FileChange => {
                        match decision {
                            ApprovalDecision::Approve => "accept",
                            ApprovalDecision::Deny => "decline",
                        }
                    }
                };
                if !approval.available_decisions.is_empty()
                    && !approval
                        .available_decisions
                        .iter()
                        .any(|available| available == native_decision)
                {
                    return CodexRequestOutcome::NotSent(CodexAppServerError::Protocol(
                        format!("Codex approval does not offer decision {native_decision}"),
                    ));
                }
                self.respond_outcome(
                    &approval.request_id,
                    json!({ "decision": native_decision }),
                )
            })
    }

    pub(crate) fn initialize(&self) -> Result<(), CodexAppServerError> {
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
        let initialized: InitializeResponse = serde_json::from_value(result).map_err(|error| {
            CodexAppServerError::Protocol(format!("invalid initialize response: {error}"))
        })?;
        if initialized.codex_home.is_empty()
            || initialized.platform_family.is_empty()
            || initialized.platform_os.is_empty()
            || initialized.user_agent.is_empty()
        {
            return Err(CodexAppServerError::Protocol(
                "initialize response contains an empty required field".to_string(),
            ));
        }
        if let Ok(mut version) = self.inner.harness_version.lock() {
            *version = harness_version_from_user_agent(&initialized.user_agent);
        }
        self.notify("initialized", json!({}))
    }

    fn authoritative_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<CodexTurn, CodexAppServerError> {
        let snapshot = self.thread_read(thread_id)?;
        snapshot
            .thread
            .turns
            .into_iter()
            .find(|turn| turn.id == turn_id)
            .ok_or_else(|| {
                CodexAppServerError::Protocol(format!(
                    "thread/read did not return authoritative turn {turn_id}"
                ))
            })
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
        self.request_value_with_timeout_outcome_with_sender(
            method,
            params,
            timeout,
            |message| self.inner.write_request(message),
        )
    }

    fn request_value_with_timeout_outcome_with_sender(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        send_request: impl FnOnce(Value) -> CodexRequestOutcome<()>,
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
        let write_outcome = send_request(json!({
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

    pub(crate) fn write_prepared_request(&self, message: Value) -> CodexRequestOutcome<()> {
        self.inner.write_request(message)
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), CodexAppServerError> {
        self.write(json!({
            "method": method,
            "params": params,
        }))
    }

    fn respond_outcome(&self, id: &JsonRpcId, result: Value) -> CodexRequestOutcome<()> {
        self.inner.write_request(json!({
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

fn snapshot_from_thread(thread: super::protocol::CodexThread) -> CodexConversationSnapshot {
    CodexConversationSnapshot::from_thread(thread)
}

fn snapshot_from_configured_response(
    response: ThreadConfiguredResponse,
) -> CodexConversationSnapshot {
    let workspace_root = project_workspace_root(Some(&response.cwd));
    CodexConversationSnapshot {
        thread: response.thread,
        workspace_root,
        permission_level: permission_from_sandbox(&response.sandbox),
        model: Some(response.model),
        reasoning_effort: response.reasoning_effort,
    }
}

fn new_session_generation() -> String {
    let counter = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}-{counter:x}", process::id(), nanos)
}

fn harness_version_from_user_agent(user_agent: &str) -> Option<String> {
    user_agent
        .split_whitespace()
        .next()
        .and_then(|product| product.rsplit_once('/').map(|(_, version)| version))
        .filter(|version| !version.trim().is_empty())
        .map(str::to_string)
}

fn handle_message(inner: &SessionInner, message: Value) -> Result<(), CodexAppServerError> {
    let object = message.as_object().ok_or_else(|| {
        CodexAppServerError::Protocol("App Server message must be an object".to_string())
    })?;
    if object
        .get("jsonrpc")
        .is_some_and(|version| version.as_str() != Some("2.0"))
    {
        return Err(CodexAppServerError::Protocol(
            "App Server jsonrpc metadata must be exactly 2.0 when present".to_string(),
        ));
    }
    if let Some(trace) = object.get("trace") {
        if !trace.is_null() {
            let trace = trace.as_object().ok_or_else(|| {
                CodexAppServerError::Protocol(
                    "App Server trace metadata must be an object or null".to_string(),
                )
            })?;
            for field in ["traceparent", "tracestate"] {
                if trace
                    .get(field)
                    .is_some_and(|value| !value.is_null() && !value.is_string())
                {
                    return Err(CodexAppServerError::Protocol(format!(
                        "App Server trace {field} must be a string or null"
                    )));
                }
            }
        }
    }
    if object
        .get("emittedAtMs")
        .is_some_and(|emitted_at| emitted_at.as_i64().is_none())
    {
        return Err(CodexAppServerError::Protocol(
            "App Server emittedAtMs metadata must be an integer".to_string(),
        ));
    }
    if object.contains_key("method") {
        let incoming = parse_incoming(message, &inner.generation)?;
        if let CodexIncoming::UnsupportedServerRequest {
            request_id,
            method,
        } = &incoming
        {
            let response = json!({
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
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    if has_result == has_error {
        return Err(CodexAppServerError::Protocol(
            "App Server response must contain exactly one of result or error".to_string(),
        ));
    }
    let id: JsonRpcId = serde_json::from_value(
        object
            .get("id")
            .cloned()
            .ok_or_else(|| CodexAppServerError::Protocol("response is missing id".to_string()))?,
    )
    .map_err(|error| CodexAppServerError::Protocol(format!("invalid response id: {error}")))?;
    let response = if let Some(error) = object.get("error") {
        let error = error.as_object().ok_or_else(|| {
            CodexAppServerError::Protocol("App Server error must be an object".to_string())
        })?;
        let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
            CodexAppServerError::Protocol("App Server error is missing integer code".to_string())
        })?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .ok_or_else(|| {
                CodexAppServerError::Protocol(
                    "App Server error is missing non-empty message".to_string(),
                )
            })?;
        Err(CodexAppServerError::Rpc {
            code,
            message: message.to_string(),
            data: error
                .get("data")
                .filter(|data| !data.is_null())
                .cloned(),
        })
    } else {
        Ok(object["result"].clone())
    };
    let sender = inner
        .pending
        .lock()
        .map_err(|_| CodexAppServerError::Protocol("pending map lock is poisoned".to_string()))?
        .remove(&id)
        .ok_or_else(|| {
            CodexAppServerError::Protocol(format!("response has no matching request id {id}"))
        })?;
    let _ = sender.send(response);
    Ok(())
}

fn incoming_thread_id(incoming: &CodexIncoming) -> Option<&str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::ThreadStarted { snapshot }) => {
            Some(&snapshot.thread.id)
        }
        CodexIncoming::Notification(
            CodexNotification::ThreadNameUpdated { thread_id, .. }
            | CodexNotification::ThreadStatusChanged { thread_id, .. }
            | CodexNotification::TurnStarted { thread_id, .. }
            | CodexNotification::TurnCompleted { thread_id, .. }
            | CodexNotification::ItemUpserted { thread_id, .. }
            | CodexNotification::OutputDelta { thread_id, .. }
            | CodexNotification::ServerRequestResolved { thread_id, .. },
        ) => Some(thread_id),
        CodexIncoming::ApprovalRequested(approval) => Some(&approval.thread_id),
        CodexIncoming::Notification(CodexNotification::Unknown { .. })
        | CodexIncoming::UnsupportedServerRequest { .. } => None,
    }
}

fn parse_incoming(
    message: Value,
    session_generation: &str,
) -> Result<CodexIncoming, CodexAppServerError> {
    let method = required_string(&message, "method")?;
    let params = message
        .get("params")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(id_value) = message.get("id") {
        let request_id: JsonRpcId = serde_json::from_value(id_value.clone()).map_err(|error| {
            CodexAppServerError::Protocol(format!("invalid server request id: {error}"))
        })?;
        return parse_server_request(request_id, &method, &params, session_generation);
    }
    parse_notification(&method, params, session_generation).map(CodexIncoming::Notification)
}

fn parse_server_request(
    request_id: JsonRpcId,
    method: &str,
    params: &Value,
    session_generation: &str,
) -> Result<CodexIncoming, CodexAppServerError> {
    let unsupported = || CodexIncoming::UnsupportedServerRequest {
        request_id: request_id.clone(),
        method: method.to_string(),
    };
    let (kind, thread_id, turn_id, item_id, requested_at_ms, available_decisions, title, description) =
        match method {
            "item/commandExecution/requestApproval" => {
                if has_any_key(
                    params,
                    &[
                        "additionalPermissions",
                        "networkApprovalContext",
                        "proposedExecpolicyAmendment",
                        "proposedNetworkPolicyAmendments",
                    ],
                ) || params
                    .as_object()
                    .is_some_and(|object| object.get("kind").is_some_and(Value::is_null))
                {
                    return Ok(unsupported());
                }
                let Ok(params) = serde_json::from_value::<CommandApprovalParams>(params.clone()) else {
                    return Ok(unsupported());
                };
                if params.has_unsupported_semantics()
                    || params.thread_id.is_empty()
                    || params.turn_id.is_empty()
                    || params.item_id.is_empty()
                {
                    return Ok(unsupported());
                }
                let Ok(requested_at_ms) = u64::try_from(params.started_at_ms) else {
                    return Ok(unsupported());
                };
                let available_decisions = params.binary_decisions();
                let description = params.command.clone().or(params.reason.clone());
                (
                    CodexApprovalKind::CommandExecution,
                    params.thread_id,
                    params.turn_id,
                    params.item_id,
                    requested_at_ms,
                    available_decisions,
                    "Run command".to_string(),
                    description,
                )
            }
            "item/fileChange/requestApproval" => {
                if has_any_key(params, &["grantRoot"]) {
                    return Ok(unsupported());
                }
                let Ok(params) = serde_json::from_value::<FileApprovalParams>(params.clone()) else {
                    return Ok(unsupported());
                };
                if params.grant_root.is_some()
                    || params.thread_id.is_empty()
                    || params.turn_id.is_empty()
                    || params.item_id.is_empty()
                {
                    return Ok(unsupported());
                }
                let Ok(requested_at_ms) = u64::try_from(params.started_at_ms) else {
                    return Ok(unsupported());
                };
                (
                    CodexApprovalKind::FileChange,
                    params.thread_id,
                    params.turn_id,
                    params.item_id,
                    requested_at_ms,
                    vec!["accept".to_string(), "decline".to_string()],
                    "Apply file changes".to_string(),
                    params.reason,
                )
            }
            _ => return Ok(unsupported()),
        };
    Ok(CodexIncoming::ApprovalRequested(CodexApprovalRequest {
        request_id,
        session_generation: session_generation.to_string(),
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

fn has_any_key(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| keys.iter().any(|key| object.contains_key(*key)))
}

fn parse_notification(
    method: &str,
    params: Value,
    session_generation: &str,
) -> Result<CodexNotification, CodexAppServerError> {
    match method {
        "thread/started" => {
            let thread = deserialize_field(&params, "thread")?;
            Ok(CodexNotification::ThreadStarted {
                snapshot: CodexConversationSnapshot::from_thread(thread),
            })
        }
        "thread/name/updated" => Ok(CodexNotification::ThreadNameUpdated {
            thread_id: required_string(&params, "threadId")?,
            thread_name: params
                .get("threadName")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|error| {
                    CodexAppServerError::Protocol(format!(
                        "invalid notification field threadName: {error}"
                    ))
                })?
                .flatten(),
        }),
        "thread/status/changed" => Ok(CodexNotification::ThreadStatusChanged {
            thread_id: required_string(&params, "threadId")?,
            status: deserialize_field(&params, "status")?,
        }),
        "turn/started" => Ok(CodexNotification::TurnStarted {
            thread_id: required_string(&params, "threadId")?,
            turn: deserialize_field(&params, "turn")?,
        }),
        "turn/completed" => Ok(CodexNotification::TurnCompleted {
            thread_id: required_string(&params, "threadId")?,
            turn: deserialize_field(&params, "turn")?,
        }),
        "item/started" | "item/completed" => {
            let item = deserialize_field(&params, "item")?;
            Ok(CodexNotification::ItemUpserted {
                thread_id: required_string(&params, "threadId")?,
                turn_id: required_string(&params, "turnId")?,
                turn_status: if method == "item/started" {
                    CodexTurnStatus::InProgress
                } else {
                    CodexTurnStatus::Completed
                },
                item,
            })
        }
        "item/agentMessage/delta"
        | "item/plan/delta"
        | "item/commandExecution/outputDelta"
        | "item/fileChange/outputDelta"
        | "item/reasoning/summaryTextDelta" => {
            let item_id = required_string(&params, "itemId")?;
            let (content_id, kind) = match method {
                "item/agentMessage/delta" | "item/plan/delta" => {
                    (text_content_id(&item_id), CodexContentKind::Text)
                }
                "item/reasoning/summaryTextDelta" => {
                    let summary_index = required_usize(&params, "summaryIndex")?;
                    (
                        reasoning_summary_content_id(&item_id, summary_index),
                        CodexContentKind::ReasoningSummary,
                    )
                }
                _ => (output_content_id(&item_id), CodexContentKind::Output),
            };
            Ok(CodexNotification::OutputDelta {
                native_method: method.to_string(),
                thread_id: required_string(&params, "threadId")?,
                turn_id: required_string(&params, "turnId")?,
                item_id,
                content_id,
                kind,
                delta: required_string(&params, "delta")?,
            })
        }
        "serverRequest/resolved" => Ok(CodexNotification::ServerRequestResolved {
            request_id: deserialize_field(&params, "requestId")?,
            thread_id: required_string(&params, "threadId")?,
            session_generation: session_generation.to_string(),
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

fn required_usize(value: &Value, key: &str) -> Result<usize, CodexAppServerError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| CodexAppServerError::Protocol(format!("message is missing {key}")))
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
    use crate::protocol::{
        CodexPermissionLevel, CodexThreadActiveFlag, CodexThreadStatus, CodexTurnStatus,
    };
    use std::fs;
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
    fn empty_initialize_result_is_rejected() {
        let (outgoing_sender, outgoing_receiver) = mpsc::channel::<Value>();
        let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();
        let peer = thread::spawn(move || {
            let initialize = outgoing_receiver.recv().unwrap();
            incoming_sender
                .send(json!({
                    "id": initialize["id"],
                    "result": {}
                }))
                .unwrap();
        });

        let error = CodexAppServerSession::connect(
            Box::new(MockReader {
                receiver: incoming_receiver,
            }),
            Box::new(MockWriter {
                sender: outgoing_sender,
            }),
        )
        .err()
        .expect("an empty initialize result must fail closed");
        peer.join().unwrap();

        assert!(matches!(error, CodexAppServerError::Protocol(_)));
    }

    #[test]
    fn oversized_physical_line_is_drained_before_error() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"12345\n{}\n"));
        let error = read_bounded_line(&mut reader, 4).unwrap_err();
        assert!(matches!(error, CodexAppServerError::Protocol(_)));
        assert_eq!(read_bounded_line(&mut reader, 4).unwrap(), Some(b"{}\n".to_vec()));
    }

    #[test]
    fn oversized_stderr_line_uses_the_session_terminal_path() {
        let (outgoing_sender, _outgoing_receiver) = mpsc::channel::<Value>();
        let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();
        let terminated = Arc::new(AtomicBool::new(false));
        let session = CodexAppServerSession::from_parts(
            Box::new(MockReader {
                receiver: incoming_receiver,
            }),
            Box::new(MockWriter {
                sender: outgoing_sender,
            }),
            Some(Box::new(RecordingControl {
                terminated: terminated.clone(),
            })),
        );
        let notifications = session.subscribe().unwrap();
        let (pending_sender, pending_receiver) = mpsc::channel();
        session
            .inner
            .pending
            .lock()
            .unwrap()
            .insert(JsonRpcId::Number(41), pending_sender);
        let mut stderr = vec![b'x'; MAX_APP_SERVER_STDERR_LINE_BYTES + 1];
        stderr.push(b'\n');

        session.start_stderr_monitor(std::io::Cursor::new(stderr));

        let error = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, CodexAppServerError::Protocol(_)));
        assert_eq!(
            pending_receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(error.clone())
        );
        for _ in 0..100 {
            if terminated.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(terminated.load(Ordering::SeqCst));
        let late_error = match session.subscribe() {
            Err(error) => error,
            Ok(_) => panic!("terminal fault was not retained"),
        };
        assert_eq!(late_error, error);
        assert!(!session.is_running());
        assert!(session.inner.pending.lock().unwrap().is_empty());
        drop(incoming_sender);
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
    fn repeated_thread_reads_do_not_load_or_configure_the_thread() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let operation_session = session.clone();
        let operation = thread::spawn(move || {
            (0..2)
                .map(|_| operation_session.thread_read("thread-writer-held").unwrap())
                .collect::<Vec<_>>()
        });

        for _ in 0..2 {
            let read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(read["method"], "thread/read");
            assert_eq!(read["params"]["threadId"], "thread-writer-held");
            assert_eq!(read["params"]["includeTurns"], true);
            peer_sender
                .send(json!({
                    "id": read["id"],
                    "result": {
                        "thread": thread_fixture(
                            "thread-writer-held",
                            "/tmp/project",
                            "idle",
                            vec![]
                        )
                    }
                }))
                .unwrap();
        }

        let snapshots = operation.join().unwrap();
        assert_eq!(snapshots.len(), 2);
        assert!(snapshots
            .iter()
            .all(|snapshot| snapshot.thread.id == "thread-writer-held"));
        assert!(session.inner.loaded_threads.lock().unwrap().is_empty());
        assert!(session
            .inner
            .thread_configurations
            .lock()
            .unwrap()
            .is_empty());
        assert!(peer_receiver.try_recv().is_err());
        session.shutdown().unwrap();
    }

    #[test]
    fn thread_list_projects_live_and_deleted_managed_worktrees_from_the_wire() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main/project");
        let git_dir = main.join(".git/worktrees/linked");
        let managed_root = temp.path().join(".codex/worktrees");
        let worktree = managed_root.join("linked/project");
        let deleted = managed_root.join("deleted/project");
        fs::create_dir_all(&git_dir).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        let (session, peer_receiver, peer_sender) = mock_session();
        let list_session = session.clone();
        let listed = thread::spawn(move || {
            list_session
                .thread_list(CodexThreadListRequest::default())
                .unwrap()
        });
        let request = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(request["method"], "thread/list");
        peer_sender
            .send(json!({
                "id": request["id"],
                "result": {
                    "data": [
                        thread_fixture(
                            "thread-worktree",
                            worktree.to_str().unwrap(),
                            "idle",
                            vec![]
                        ),
                        thread_fixture(
                            "thread-deleted-worktree",
                            deleted.to_str().unwrap(),
                            "idle",
                            vec![]
                        )
                    ],
                    "nextCursor": null
                }
            }))
            .unwrap();

        let snapshots = listed.join().unwrap().data;

        assert_eq!(snapshots[0].thread.cwd, worktree.to_str().unwrap());
        assert_eq!(snapshots[1].thread.cwd, deleted.to_str().unwrap());
        assert_eq!(
            snapshots[0].workspace_root.as_deref(),
            fs::canonicalize(main).unwrap().to_str()
        );
        assert_eq!(snapshots[0].workspace_root, snapshots[1].workspace_root);
        session.shutdown().unwrap();
    }

    #[test]
    fn session_keeps_reading_notifications_between_requests() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
            peer_sender
                .send(json!({
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
    fn session_parses_thread_name_updated_notifications() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        peer_sender
            .send(json!({
                "method": "thread/name/updated",
                "params": {
                    "threadId": "thread-one",
                    "threadName": "A useful title"
                }
            }))
            .unwrap();

        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::Notification(CodexNotification::ThreadNameUpdated {
                thread_id,
                thread_name: Some(thread_name),
            }) if thread_id == "thread-one" && thread_name == "A useful title"
        ));
        session.shutdown().unwrap();
    }

    #[test]
    fn session_parses_thread_status_changed_notifications() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        peer_sender
            .send(json!({
                "method": "thread/status/changed",
                "params": {
                    "threadId": "thread-one",
                    "status": {
                        "type": "active",
                        "activeFlags": ["waitingOnApproval"]
                    }
                }
            }))
            .unwrap();

        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::Notification(CodexNotification::ThreadStatusChanged {
                thread_id,
                status: CodexThreadStatus::Active { active_flags },
            }) if thread_id == "thread-one"
                && active_flags == vec![CodexThreadActiveFlag::WaitingOnApproval]
        ));
        session.shutdown().unwrap();
    }

    #[test]
    fn peer_exit_is_reported_to_subscribers_and_stops_the_session() {
        let (session, _, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        drop(peer_sender);
        let error = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert_eq!(error, CodexAppServerError::ProcessExited);
        assert!(!session.is_running());
    }

    #[test]
    fn terminal_fault_is_replayed_to_late_subscriber() {
        let (session, _, peer_sender) = mock_session();
        drop(peer_sender);
        for _ in 0..100 {
            if !session.is_running() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!session.is_running());

        let error = match session.subscribe() {
            Err(error) => error,
            Ok(_) => panic!("late subscriber did not receive the terminal fault"),
        };
        assert_eq!(error, CodexAppServerError::ProcessExited);
    }

    #[test]
    fn subscribe_racing_terminal_fault_always_observes_the_fault() {
        let (session, _peer_receiver, peer_sender) = mock_session();
        let subscriber_count = 32;
        let barrier = Arc::new(Barrier::new(subscriber_count + 1));
        let expected = CodexAppServerError::Protocol("forced concurrent fault".to_string());
        let subscribers = (0..subscriber_count)
            .map(|_| {
                let session = session.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    match session.subscribe() {
                        Ok(receiver) => receiver
                            .recv_timeout(Duration::from_secs(1))
                            .unwrap()
                            .unwrap_err(),
                        Err(error) => error,
                    }
                })
            })
            .collect::<Vec<_>>();

        barrier.wait();
        session.inner.fail(expected.clone());

        for subscriber in subscribers {
            assert_eq!(subscriber.join().unwrap(), expected);
        }
        let late_error = match session.subscribe() {
            Err(error) => error,
            Ok(_) => panic!("terminal fault was not retained after the race"),
        };
        assert_eq!(late_error, expected);
        drop(peer_sender);
    }

    #[test]
    fn official_notification_metadata_and_missing_params_are_accepted() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        peer_sender
            .send(json!({
                "method": "remoteControl/status/changed",
                "emittedAtMs": 1234
            }))
            .unwrap();
        let incoming = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::Notification(CodexNotification::Unknown { method })
                if method == "remoteControl/status/changed"
        ));

        peer_sender
            .send(json!({
                "id": "future-request",
                "method": "future/request",
                "trace": { "traceparent": null, "tracestate": null }
            }))
            .unwrap();
        let incoming = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(matches!(
            incoming,
            CodexIncoming::UnsupportedServerRequest { request_id, method }
                if request_id == JsonRpcId::String("future-request".to_string())
                    && method == "future/request"
        ));
        let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(response["id"], "future-request");
        assert_eq!(response["error"]["code"], -32601);
        assert!(response.get("jsonrpc").is_none());
        assert!(session.is_running());
        session.shutdown().unwrap();
    }

    #[test]
    fn output_delta_uses_channel_specific_content_ids_and_ignores_raw_reasoning() {
        let summary = parse_notification(
            "item/reasoning/summaryTextDelta",
            json!({
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "reasoning-one",
                "summaryIndex": 2,
                "delta": "summary"
            }),
            "generation-one",
        )
        .unwrap();
        assert!(matches!(
            summary,
            CodexNotification::OutputDelta {
                item_id,
                content_id,
                kind: CodexContentKind::ReasoningSummary,
                ..
            } if item_id == "reasoning-one" && content_id == "reasoning-one:summary:2"
        ));

        let raw = parse_notification(
            "item/reasoning/textDelta",
            json!({
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "reasoning-one",
                "contentIndex": 0,
                "delta": "private raw reasoning"
            }),
            "generation-one",
        )
        .unwrap();
        assert!(matches!(
            raw,
            CodexNotification::Unknown { method }
                if method == "item/reasoning/textDelta"
        ));
    }

    #[test]
    fn thread_read_rejects_non_full_turn_items() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let request_session = session.clone();
        let request = thread::spawn(move || request_session.thread_read("thread-one"));
        let read = peer_receiver.recv().unwrap();
        let mut turn = turn_fixture("turn-one", "completed");
        turn["itemsView"] = json!("summary");
        peer_sender
            .send(json!({
                "id": read["id"],
                "result": {
                    "thread": thread_fixture(
                        "thread-one",
                        "/tmp/project",
                        "idle",
                        vec![turn]
                    )
                }
            }))
            .unwrap();

        let error = request.join().unwrap().unwrap_err();
        assert!(matches!(
            error,
            CodexAppServerError::Protocol(message)
                if message.contains("non-full items for turn turn-one")
        ));
        session.shutdown().unwrap();
    }

    #[test]
    fn invalid_optional_jsonrpc_metadata_fails_the_session() {
        let (session, _peer_receiver, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        peer_sender
            .send(json!({
                "jsonrpc": "1.0",
                "method": "remoteControl/status/changed"
            }))
            .unwrap();

        let error = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, CodexAppServerError::Protocol(_)));
        assert!(!session.is_running());
    }

    #[test]
    fn command_approval_kind_null_is_unsupported_but_missing_kind_defaults_to_command() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        peer_sender
            .send(json!({
                "id": "kind-null",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "item-null",
                    "startedAtMs": 123,
                    "kind": null
                }
            }))
            .unwrap();
        assert!(matches!(
            notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(),
            CodexIncoming::UnsupportedServerRequest {
                request_id: JsonRpcId::String(request_id),
                method
            } if request_id == "kind-null"
                && method == "item/commandExecution/requestApproval"
        ));
        let rejected = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(rejected["id"], "kind-null");
        assert_eq!(rejected["error"]["code"], -32601);

        peer_sender
            .send(json!({
                "id": "kind-missing",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "item-command",
                    "startedAtMs": 124
                }
            }))
            .unwrap();
        let accepted = notifications
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(matches!(
            accepted,
            CodexIncoming::ApprovalRequested(CodexApprovalRequest {
                kind: CodexApprovalKind::CommandExecution,
                ..
            })
        ));
        session.shutdown().unwrap();
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
        let notifications = session.subscribe().unwrap();
        peer_sender.send(json!({ "id": 999, "result": {} })).unwrap();
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
                    search_term: None,
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
            assert_eq!(normal.workspace_root.as_deref(), Some("/tmp"));
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
                    permission_level: Some(CodexPermissionLevel::FullAccess),
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
        assert_eq!(list["params"]["sortKey"], "updated_at");
        assert_eq!(list["params"]["sortDirection"], "desc");
        assert_eq!(list["params"]["useStateDbOnly"], true);
        assert!(list["params"].get("searchTerm").is_none());
        peer_sender
            .send(json!({
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
                "id": normal["id"],
                "result": {
                    "thread": thread_fixture("thread-normal", "/tmp", "idle", vec![]),
                    "model": "default",
                    "modelProvider": "openai",
                    "cwd": "/tmp",
                    "approvalPolicy": "on-request",
                    "approvalsReviewer": "user",
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
                "id": project["id"],
                "result": {
                    "thread": thread_fixture("thread-project", "/work/project", "idle", vec![]),
                    "model": "gpt-fixture",
                    "modelProvider": "openai",
                    "cwd": "/work/project",
                    "approvalPolicy": "never",
                    "approvalsReviewer": "user",
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
        assert_eq!(start["params"]["clientUserMessageId"], "message-one");
        assert_eq!(start["params"]["sandboxPolicy"]["type"], "dangerFullAccess");
        assert_eq!(start["params"]["approvalPolicy"], "never");
        assert_eq!(start["params"]["model"], "gpt-fixture");
        assert_eq!(start["params"]["effort"], "high");
        peer_sender
            .send(json!({
                "id": start["id"],
                "result": { "turn": turn_fixture("turn-started", "inProgress") }
            }))
            .unwrap();

        let steer = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(steer["method"], "turn/steer");
        assert_eq!(steer["params"]["expectedTurnId"], "turn-started");
        peer_sender
            .send(json!({
                "id": steer["id"],
                "result": { "turnId": "turn-started" }
            }))
            .unwrap();

        let steer_read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(steer_read["method"], "thread/read");
        peer_sender
            .send(json!({
                "id": steer_read["id"],
                "result": {
                    "thread": thread_fixture(
                        "thread-project",
                        "/work/project",
                        "active",
                        vec![turn_fixture("turn-started", "inProgress")]
                    )
                }
            }))
            .unwrap();

        let interrupt = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(interrupt["method"], "turn/interrupt");
        assert_eq!(interrupt["params"]["turnId"], "turn-started");
        peer_sender
            .send(json!({
                "id": interrupt["id"],
                "result": {}
            }))
            .unwrap();

        let interrupt_read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(interrupt_read["method"], "thread/read");
        peer_sender
            .send(json!({
                "id": interrupt_read["id"],
                "result": {
                    "thread": thread_fixture(
                        "thread-project",
                        "/work/project",
                        "idle",
                        vec![turn_fixture("turn-started", "interrupted")]
                    )
                }
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
                            permission_level: None,
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
                    "id": resume["id"],
                    "result": {
                        "thread": thread_fixture(
                            "thread-historical",
                            "/tmp/project",
                            "idle",
                            vec![]
                        ),
                        "model": "default",
                        "modelProvider": "openai",
                        "cwd": "/tmp/project",
                        "approvalPolicy": "on-request",
                        "approvalsReviewer": "user",
                        "reasoningEffort": null,
                        "sandbox": {
                            "type": "workspaceWrite",
                            "writableRoots": [],
                            "networkAccess": false,
                            "excludeTmpdirEnvVar": false,
                            "excludeSlashTmp": false
                        }
                    },
                    "emittedAtMs": 1234
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
                    message,
                    data: None
                }) if message == "no rollout found for thread id thread-missing"
            ));
        }
        assert!(peer_receiver.try_recv().is_err());
        session.shutdown().unwrap();
    }

    #[test]
    fn approval_and_unsupported_server_requests_receive_terminal_responses() {
        let (session, peer_receiver, peer_sender) = mock_session();
        let notifications = session.subscribe().unwrap();
        peer_sender
            .send(json!({
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
        assert!(matches!(
            incoming,
            CodexIncoming::UnsupportedServerRequest {
                request_id: JsonRpcId::String(request_id),
                method
            } if request_id == "approval-one"
                && method == "item/commandExecution/requestApproval"
        ));
        let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(response["id"], "approval-one");
        assert_eq!(response["error"]["code"], -32601);

        peer_sender
            .send(json!({
                "id": "approval-safe",
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "item-one",
                    "startedAtMs": 123,
                    "command": "cargo test",
                    "availableDecisions": ["accept", "decline"]
                }
            }))
            .unwrap();
        let incoming = notifications.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        let CodexIncoming::ApprovalRequested(approval) = incoming else {
            panic!("expected safe binary approval request");
        };
        assert_eq!(approval.kind, CodexApprovalKind::CommandExecution);
        assert_eq!(approval.description.as_deref(), Some("cargo test"));
        assert_eq!(approval.session_generation, session.generation());
        session
            .respond_to_approval(&approval, ApprovalDecision::Approve)
            .unwrap();
        let response = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(response["id"], "approval-safe");
        assert_eq!(response["result"]["decision"], "accept");
        assert!(response.get("method").is_none());

        peer_sender
            .send(json!({
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
}
