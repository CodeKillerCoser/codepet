use super::protocol::{
    output_content_id, permission_from_sandbox, reasoning_summary_content_id, text_content_id,
    thread_list_params, thread_start_params, turn_start_params, turn_steer_params,
    CodexAppServerError, CodexApprovalKind, CodexApprovalRequest, CodexContentKind,
    CodexConversationSnapshot, CodexIncoming, CodexModel, CodexModelListResponse,
    CodexNotification, CodexPermissionLevel, CodexProject, CodexProjectCreateRequest,
    CodexProjectPage, CodexProjectUpdateRequest, CodexThreadListRequest, CodexThreadPage,
    CodexThreadStartRequest, CodexTurn, CodexTurnItemsView, CodexTurnPage, CodexTurnStartRequest,
    CodexTurnStatus, CodexTurnSteerRequest, CommandApprovalParams, FileApprovalParams,
    InitializeResponse, JsonRpcId, ProjectListResponse, ProjectResponse, ThreadConfiguredResponse,
    ThreadListResponse, ThreadReadResponse, ThreadTurnsListResponse, TurnResponse,
    TurnSteerResponse,
};
use codepet_provider_sdk::ApprovalDecision;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

mod compatible;
mod projects;
mod title;
use codepet_provider_sdk::process::Child;
use codepet_provider_sdk::process::Command;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{self, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_APP_SERVER_STDERR_LINE_BYTES: usize = 64 * 1024;
const APP_SERVER_CLIENT_NAME: &str = "code-pet";
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

mod framing;
use framing::{drain_stderr, JsonLineReader};

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
    ephemeral_threads: Mutex<HashSet<String>>,
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
    fn and_then<U>(
        self,
        operation: impl FnOnce(T) -> CodexRequestOutcome<U>,
    ) -> CodexRequestOutcome<U> {
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
    fn reject_message(
        &self,
        request_id: Option<JsonRpcId>,
        method: Option<String>,
        error: CodexAppServerError,
    ) {
        eprintln!("Codex App Server rejected message id={request_id:?} method={method:?}: {error}");
        let Some(id) = request_id else {
            return;
        };
        if method.is_some() {
            // Server request ids are independent of our pending client request ids.
            if let Err(reply_error) = self.write(json!({
                "id": id, "error": { "code": -32600, "message": error.to_string() },
            })) {
                eprintln!("Codex App Server could not reject server request: {reply_error}");
            }
        } else if let Ok(mut pending) = self.pending.lock() {
            if let Some(sender) = pending.remove(&id) {
                let _ = sender.send(Err(error));
            }
        }
    }

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
            .map_err(|_| {
                CodexAppServerError::Protocol("process control lock is poisoned".to_string())
            })
            .and_then(|mut control| match control.as_mut() {
                Some(control) => control.shutdown(),
                None => Ok(()),
            });
        if let Err(termination_error) = termination {
            eprintln!(
                "Codex App Server termination after terminal fault failed: {termination_error}"
            );
        }
    }

    fn write(&self, message: Value) -> Result<(), CodexAppServerError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| CodexAppServerError::Protocol("writer lock is poisoned".to_string()))?;
        let writer = writer.as_mut().ok_or(CodexAppServerError::Shutdown)?;
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
        data_directory: Option<&Path>,
    ) -> Result<Self, CodexAppServerError> {
        let mut command = codex_app_server_command(executable, args);
        if let Some(directory) = data_directory {
            std::fs::create_dir_all(directory)
                .map_err(|error| CodexAppServerError::Spawn(error.to_string()))?;
            command.env("CODEX_HOME", directory);
        }
        let mut child = command
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
            ephemeral_threads: Mutex::new(HashSet::new()),
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
                Err(CodexAppServerError::RejectedMessage {
                    request_id,
                    method,
                    error,
                }) => {
                    let Some(inner) = weak_inner.upgrade() else {
                        break;
                    };
                    inner.reject_message(request_id, method, *error);
                    continue;
                }
                Err(CodexAppServerError::Protocol(message)) => {
                    eprintln!("Codex App Server rejected message: {message}");
                    continue;
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
            let request_id = message
                .get("id")
                .cloned()
                .and_then(|id| serde_json::from_value(id).ok());
            let method = message
                .get("method")
                .map(|method| method.as_str().unwrap_or("").to_owned());
            if let Err(error) = handle_message(&inner, message) {
                inner.reject_message(request_id, method, error);
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

    fn start_stderr_monitor(&self, stderr: impl Read + Send + 'static) -> JoinHandle<()> {
        thread::spawn(move || {
            if let Err(error) = drain_stderr(stderr) {
                eprintln!("Codex App Server stderr failed: {error}");
            }
        })
    }

    pub fn shutdown(&self) -> Result<(), CodexAppServerError> {
        // A failed kill must remain retryable even after RPC admission is closed.
        self.inner.running.store(false, Ordering::SeqCst);
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

    /// Test the native loaded namespace independently of directory listing.
    #[cfg(test)]
    pub fn thread_loaded_list(
        &self,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<String>, Option<String>), CodexAppServerError> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            data: Vec<String>,
            next_cursor: Option<String>,
        }
        let page: Page =
            self.request("thread/loaded/list", json!({"cursor":cursor,"limit":limit}))?;
        Ok((page.data, page.next_cursor))
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

    pub fn account_read(&self) -> Result<Value, CodexAppServerError> {
        self.request("account/read", json!({ "refreshToken": false }))
    }

    pub fn account_usage_read(&self) -> Result<Value, CodexAppServerError> {
        self.request("account/usage/read", Value::Null)
    }

    pub fn account_rate_limits_read(&self) -> Result<Value, CodexAppServerError> {
        self.request("account/rateLimits/read", json!({}))
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

    pub fn thread_turns_list_with_view(
        &self,
        thread_id: &str,
        cursor: Option<String>,
        limit: u32,
        items_view: CodexTurnItemsView,
    ) -> Result<CodexTurnPage, CodexAppServerError> {
        let response: ThreadTurnsListResponse = self.request(
            "thread/turns/list",
            json!({
                "threadId": thread_id,
                "cursor": cursor,
                "limit": limit,
                "sortDirection": "desc",
                "itemsView": items_view,
            }),
        )?;
        if let Some(turn) = response
            .data
            .iter()
            .find(|turn| turn.items_view != items_view)
        {
            return Err(CodexAppServerError::Protocol(format!(
                "thread/turns/list returned {:?} items for {:?} request on turn {}",
                turn.items_view, items_view, turn.id
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
            json!({ "threadId": thread_id, "excludeTurns": true }),
            REQUEST_TIMEOUT,
            send_request,
        )
        .and_then(
            |response| match serde_json::from_value::<ThreadConfiguredResponse>(response) {
                Ok(response) => CodexRequestOutcome::Success(response),
                Err(error) => {
                    CodexRequestOutcome::SentOutcomeUnknown(CodexAppServerError::Protocol(format!(
                        "invalid thread/resume response: {error}"
                    )))
                }
            },
        )
        .and_then(|response| {
            let snapshot = snapshot_from_configured_response(response);
            if snapshot.thread.id != thread_id {
                return CodexRequestOutcome::SentOutcomeUnknown(CodexAppServerError::Protocol(
                    format!(
                        "thread/resume returned thread {} for requested thread {thread_id}",
                        snapshot.thread.id
                    ),
                ));
            }
            self.inner.record_configuration(&snapshot);
            CodexRequestOutcome::Success(snapshot)
        })
    }

    pub(crate) fn ensure_thread_loaded_outcome(&self, thread_id: &str) -> CodexRequestOutcome<u64> {
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
                    loaded_threads = match self.inner.loaded_threads_changed.wait(loaded_threads) {
                        Ok(loaded_threads) => loaded_threads,
                        Err(_) => {
                            return CodexRequestOutcome::NotSent(CodexAppServerError::Protocol(
                                "loaded thread state lock is poisoned".to_string(),
                            ))
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
                return CodexRequestOutcome::SentOutcomeUnknown(CodexAppServerError::Protocol(
                    "loaded thread state lock is poisoned".to_string(),
                ))
            }
        };
        let settled = match result {
            CodexRequestOutcome::Success(_) => {
                let evidence = match loaded_threads.get(thread_id).copied() {
                    Some(ThreadLoadState::Loaded(evidence)) => evidence,
                    _ => {
                        let evidence = self.inner.next_load_evidence.fetch_add(1, Ordering::SeqCst);
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
        self.thread_start_outcome_with_sender(request, |message| self.inner.write_request(message))
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
        .and_then(
            |response| match serde_json::from_value::<ThreadConfiguredResponse>(response) {
                Ok(response) => CodexRequestOutcome::Success(response),
                Err(error) => {
                    CodexRequestOutcome::SentOutcomeUnknown(CodexAppServerError::Protocol(format!(
                        "invalid thread/start response: {error}"
                    )))
                }
            },
        )
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
        self.turn_interrupt_outcome(thread_id, turn_id)
            .into_result()
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
            .and_then(
                |_: Value| match self.authoritative_turn(thread_id, turn_id) {
                    Ok(turn) => CodexRequestOutcome::Success(turn),
                    Err(error) => CodexRequestOutcome::SentOutcomeUnknown(error),
                },
            )
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
                            ApprovalDecision::Deny | ApprovalDecision::Unrecognized(_) => "decline",
                        }
                    }
                };
                if !approval.available_decisions.is_empty()
                    && !approval
                        .available_decisions
                        .iter()
                        .any(|available| available == native_decision)
                {
                    return CodexRequestOutcome::NotSent(CodexAppServerError::Protocol(format!(
                        "Codex approval does not offer decision {native_decision}"
                    )));
                }
                self.respond_outcome(&approval.request_id, json!({ "decision": native_decision }))
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
                    "name": APP_SERVER_CLIENT_NAME,
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
                    CodexAppServerError::Protocol(format!("invalid {method} response: {error}")),
                ),
            })
    }

    fn request_value_outcome(&self, method: &str, params: Value) -> CodexRequestOutcome<Value> {
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
        self.request_value_with_timeout_outcome_with_sender(method, params, timeout, |message| {
            self.inner.write_request(message)
        })
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
                CodexRequestOutcome::SentOutcomeUnknown(error)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                CodexRequestOutcome::SentOutcomeUnknown(CodexAppServerError::ProcessExited)
            }
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
    let mut command = codepet_provider_sdk::local_runtime::command(binary);
    command.env(
        crate::desktop_takeover::HARNESS_MARKER,
        std::process::id().to_string(),
    );
    command.args(args);
    command
}

fn snapshot_from_thread(thread: super::protocol::CodexThread) -> CodexConversationSnapshot {
    CodexConversationSnapshot::from_thread(thread)
}

fn snapshot_from_configured_response(
    response: ThreadConfiguredResponse,
) -> CodexConversationSnapshot {
    let workspace_root = Some(response.cwd.clone());
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
    let mut previous_was_codex = false;
    for product in user_agent.split_whitespace() {
        if let Some((name, version)) = product.rsplit_once('/') {
            // Without an originator override, Codex uses initialize.clientInfo.name
            // as the product name, followed by the native Harness version.
            if (name.eq_ignore_ascii_case(APP_SERVER_CLIENT_NAME)
                || name.to_ascii_lowercase().contains("codex")
                || previous_was_codex)
                && !version.trim().is_empty()
            {
                return Some(version.trim().to_string());
            }
        }
        previous_was_codex = product.eq_ignore_ascii_case("codex");
    }
    None
}

mod incoming;
use incoming::*;

#[cfg(test)]
mod tests;
