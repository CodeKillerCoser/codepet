use super::protocol::{
    broadcast_envelope, message_targets_client, method, request_envelope, required_string,
    source_client_id, version, DesktopIpcError, IpcResponse, CLIENT_STATUS_VERSION,
    INITIALIZE_VERSION, INITIAL_CLIENT_ID, IPC_ROUTER_VERSION, LOCAL_HOST_ID,
    METHOD_CLIENT_STATUS_CHANGED, METHOD_INITIALIZE,
    METHOD_THREAD_FOLLOWER_LOAD_COMPLETE_HISTORY,
    METHOD_THREAD_OWNER_DISCOVERY, METHOD_THREAD_STREAM_FOLLOWING_CHANGED,
    METHOD_THREAD_STREAM_FOLLOWING_STATUS_REQUESTED, METHOD_THREAD_STREAM_STATE_CHANGED,
    THREAD_STREAM_STATE_VERSION,
};
use super::state::{FollowerStore, StateChangeOutcome, ThreadSnapshot};
use super::transport::{read_frame, write_frame};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

const ROUTER_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const COMPLETE_HISTORY_TIMEOUT: Duration = Duration::from_secs(305);
const COMPLETE_HISTORY_REVISION_TIMEOUT: Duration = Duration::from_secs(30);
const PARALLEL_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(360);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

#[cfg(unix)]
type IpcStream = std::os::unix::net::UnixStream;

#[cfg(not(unix))]
struct IpcStream;

#[cfg(not(unix))]
impl std::io::Read for IpcStream {
    fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Codex Desktop IPC requires a Unix socket",
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopConnectionStatus {
    Connecting,
    Ready,
    Unavailable,
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopConnectionSnapshot {
    pub status: DesktopConnectionStatus,
    pub client_id: Option<String>,
    pub generation: u64,
    pub error: Option<DesktopIpcError>,
}

#[derive(Clone, Debug)]
pub enum DesktopClientEvent {
    ConnectionChanged(DesktopConnectionSnapshot),
    ThreadDiscovered {
        conversation_id: String,
    },
    ThreadStateChanged {
        snapshot: ThreadSnapshot,
        bootstrapped: bool,
        generation: u64,
        follower_epoch: u64,
    },
    ThreadBootstrapped {
        snapshot: ThreadSnapshot,
        generation: u64,
        follower_epoch: u64,
    },
    RevisionGap {
        conversation_id: String,
        expected_base_revision: Option<u64>,
        received_base_revision: Option<u64>,
    },
    FollowerStateReset {
        reason: String,
        generation: u64,
        follower_epoch: u64,
    },
    Diagnostic(String),
}

struct ConnectionState {
    status: DesktopConnectionStatus,
    client_id: Option<String>,
    writer: Option<IpcStream>,
    generation: u64,
    error: Option<DesktopIpcError>,
    shutdown: bool,
}

type PendingResult = Result<IpcResponse, DesktopIpcError>;

struct ClientInner {
    connection: Mutex<ConnectionState>,
    connection_wake: Condvar,
    pending: Mutex<HashMap<String, Sender<PendingResult>>>,
    follower: Mutex<FollowerStore>,
    follower_wake: Condvar,
    subscribers: Mutex<Vec<Sender<DesktopClientEvent>>>,
    bootstrap_in_flight: Mutex<HashSet<String>>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct CodexDesktopClient {
    inner: Arc<ClientInner>,
}

impl CodexDesktopClient {
    pub fn spawn() -> Self {
        let state = ConnectionState {
            status: DesktopConnectionStatus::Connecting,
            client_id: None,
            writer: None,
            generation: 0,
            error: None,
            shutdown: false,
        };
        let inner = Arc::new(ClientInner {
            connection: Mutex::new(state),
            connection_wake: Condvar::new(),
            pending: Mutex::new(HashMap::new()),
            follower: Mutex::new(FollowerStore::default()),
            follower_wake: Condvar::new(),
            subscribers: Mutex::new(Vec::new()),
            bootstrap_in_flight: Mutex::new(HashSet::new()),
            supervisor: Mutex::new(None),
        });
        let supervisor_inner = inner.clone();
        let handle = thread::spawn(move || {
            supervisor_loop(supervisor_inner, None, Vec::new())
        });
        *lock(&inner.supervisor) = Some(handle);
        Self { inner }
    }

    pub fn connection_snapshot(&self) -> DesktopConnectionSnapshot {
        connection_snapshot(&lock(&self.inner.connection))
    }

    pub fn subscribe(&self) -> Receiver<DesktopClientEvent> {
        let (sender, receiver) = mpsc::channel();
        let mut subscribers = lock(&self.inner.subscribers);
        subscribers.push(sender.clone());
        let _ = sender.send(DesktopClientEvent::ConnectionChanged(
            self.connection_snapshot(),
        ));
        for conversation_id in self.known_threads() {
            let _ = sender.send(DesktopClientEvent::ThreadDiscovered { conversation_id });
        }
        drop(subscribers);
        receiver
    }

    pub fn known_threads(&self) -> Vec<String> {
        lock(&self.inner.follower).known_threads()
    }

    pub fn is_current_follower_epoch(&self, generation: u64, follower_epoch: u64) -> bool {
        let connection = lock(&self.inner.connection);
        if connection.status != DesktopConnectionStatus::Ready
            || connection.generation != generation
        {
            return false;
        }
        drop(connection);
        lock(&self.inner.follower).epoch() == follower_epoch
    }

    pub fn bootstrap_thread(
        &self,
        conversation_id: &str,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        self.bootstrap_thread_with_policy(conversation_id, false)
    }

    pub fn bootstrap_followed_thread(
        &self,
        conversation_id: &str,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        self.bootstrap_thread_with_policy(conversation_id, true)
    }

    fn bootstrap_thread_with_policy(
        &self,
        conversation_id: &str,
        persist_known_thread: bool,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Err(DesktopIpcError::Protocol(
                "thread id cannot be empty".to_string(),
            ));
        }
        if persist_known_thread {
            lock(&self.inner.follower).remember_thread(conversation_id);
        }
        loop {
            let mut in_flight = lock(&self.inner.bootstrap_in_flight);
            if in_flight.insert(conversation_id.to_string()) {
                drop(in_flight);
                lock(&self.inner.follower).begin_bootstrap(conversation_id);
                break;
            } else {
                drop(in_flight);
                match self.wait_for_bootstrapped(conversation_id, PARALLEL_BOOTSTRAP_TIMEOUT) {
                    Ok(snapshot) => return Ok(snapshot),
                    Err(DesktopIpcError::Protocol(message))
                        if message.starts_with("parallel bootstrap failed") => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let generation = match self.ready_generation() {
            Ok(generation) => generation,
            Err(error) => {
                lock(&self.inner.bootstrap_in_flight).remove(conversation_id);
                if !persist_known_thread {
                    lock(&self.inner.follower)
                        .discard_unremembered_thread(conversation_id);
                }
                self.inner.follower_wake.notify_all();
                return Err(error);
            }
        };
        let result = self.bootstrap_thread_inner(conversation_id, generation);
        lock(&self.inner.bootstrap_in_flight).remove(conversation_id);
        if !persist_known_thread {
            lock(&self.inner.follower).discard_unremembered_thread(conversation_id);
        }
        self.inner.follower_wake.notify_all();
        let should_disconnect = matches!(
            &result,
            Err(DesktopIpcError::Protocol(_) | DesktopIpcError::Timeout(_))
        );
        if should_disconnect {
            if let Err(error) = &result {
                disconnect_if_generation(&self.inner, generation, error.clone());
            }
        }
        result
    }

    fn bootstrap_thread_inner(
        &self,
        conversation_id: &str,
        generation: u64,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        let owner = self.discover_owner(conversation_id, generation)?;
        lock(&self.inner.follower).bind_owner(conversation_id, &owner)?;
        self.send_following_changed(conversation_id, true, Some(&owner), Some(generation))?;
        self.wait_for_revision(
            conversation_id,
            &owner,
            None,
            SNAPSHOT_TIMEOUT,
            false,
        )?;

        let response = self.request(
            IPC_ROUTER_VERSION,
            METHOD_THREAD_FOLLOWER_LOAD_COMPLETE_HISTORY,
            json!({ "conversationId": conversation_id }),
            Some(&owner),
            COMPLETE_HISTORY_TIMEOUT,
            Some(generation),
        )?;
        let history_result = match response
            .success_result(METHOD_THREAD_FOLLOWER_LOAD_COMPLETE_HISTORY)
        {
            Ok(result) => result,
            Err(error) => {
                if matches!(error, DesktopIpcError::Protocol(_)) {
                    disconnect_if_generation(&self.inner, generation, error.clone());
                }
                return Err(error);
            }
        };
        if let Err(error) = response.ensure_handled_by(&owner) {
            disconnect_if_generation(&self.inner, generation, error.clone());
            return Err(error);
        }
        let expected_revision = history_result
            .get("revision")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                DesktopIpcError::Protocol(
                    "complete-history response is missing revision".to_string(),
                )
            })
            .map_err(|error| {
                disconnect_if_generation(&self.inner, generation, error.clone());
                error
            })?;
        self.wait_for_revision(
            conversation_id,
            &owner,
            Some(expected_revision),
            COMPLETE_HISTORY_REVISION_TIMEOUT,
            false,
        )?;
        let mut subscribers = lock(&self.inner.subscribers);
        self.ensure_generation(generation)?;
        let (snapshot, follower_epoch) = {
            let mut follower = lock(&self.inner.follower);
            let snapshot = follower.complete_bootstrap(
                conversation_id,
                &owner,
                expected_revision,
            )?;
            (snapshot, follower.epoch())
        };
        emit_to_subscribers(
            &mut subscribers,
            DesktopClientEvent::ThreadBootstrapped {
                snapshot: snapshot.clone(),
                generation,
                follower_epoch,
            },
        );
        self.inner.follower_wake.notify_all();
        Ok(snapshot)
    }

    fn wait_for_bootstrapped(
        &self,
        conversation_id: &str,
        timeout: Duration,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        let deadline = Instant::now() + timeout;
        let mut store = lock(&self.inner.follower);
        loop {
            if store.is_bootstrapped(conversation_id) {
                return store.snapshot(conversation_id).ok_or_else(|| {
                    DesktopIpcError::Protocol("bootstrapped thread lost its snapshot".to_string())
                });
            }
            if !lock(&self.inner.bootstrap_in_flight).contains(conversation_id) {
                return Err(DesktopIpcError::Protocol(format!(
                    "parallel bootstrap failed for thread {conversation_id}"
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(DesktopIpcError::Timeout(format!(
                    "waiting for thread {conversation_id} bootstrap"
                )));
            }
            let (next, _) = self
                .inner
                .follower_wake
                .wait_timeout(store, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            store = next;
        }
    }

    fn discover_owner(
        &self,
        conversation_id: &str,
        generation: u64,
    ) -> Result<String, DesktopIpcError> {
        let response = self.request(
            IPC_ROUTER_VERSION,
            METHOD_THREAD_OWNER_DISCOVERY,
            json!({
                "hostId": LOCAL_HOST_ID,
                "conversationId": conversation_id,
            }),
            None,
            ROUTER_REQUEST_TIMEOUT,
            Some(generation),
        )?;
        if let Err(error) = response.success_result(METHOD_THREAD_OWNER_DISCOVERY) {
            if matches!(error, DesktopIpcError::Protocol(_)) {
                disconnect_if_generation(&self.inner, generation, error.clone());
            }
            return Err(error);
        }
        response.handled_by_client_id.ok_or_else(|| {
            let error = DesktopIpcError::Protocol(
                "owner discovery response is missing owner client id".to_string(),
            );
            disconnect_if_generation(&self.inner, generation, error.clone());
            error
        })
    }

    fn verify_owner_candidate(
        &self,
        conversation_id: &str,
        candidate_owner: &str,
        generation: u64,
    ) {
        match self.discover_owner(conversation_id, generation) {
            Ok(discovered_owner) if discovered_owner == candidate_owner => {
                disconnect_if_generation(
                    &self.inner,
                    generation,
                    DesktopIpcError::Disconnected(
                        "Desktop thread ownership changed; reconnecting follower".to_string(),
                    ),
                );
            }
            Ok(_) => emit_diagnostic(
                &self.inner,
                format!(
                    "ignored unverified owner-transfer request for thread {conversation_id}"
                ),
            ),
            Err(error) => emit_diagnostic(
                &self.inner,
                format!(
                    "could not verify owner-transfer request for thread {conversation_id}: {error}"
                ),
            ),
        }
    }

    fn ready_generation(&self) -> Result<u64, DesktopIpcError> {
        let connection = lock(&self.inner.connection);
        if connection.shutdown {
            return Err(DesktopIpcError::Shutdown);
        }
        if connection.status != DesktopConnectionStatus::Ready {
            return Err(connection.error.clone().unwrap_or_else(|| {
                DesktopIpcError::Disconnected("router is not connected".to_string())
            }));
        }
        Ok(connection.generation)
    }

    fn ensure_generation(&self, generation: u64) -> Result<(), DesktopIpcError> {
        let connection = lock(&self.inner.connection);
        if connection.status == DesktopConnectionStatus::Ready
            && connection.generation == generation
        {
            return Ok(());
        }
        Err(DesktopIpcError::Disconnected(
            "Desktop IPC connection changed during follower bootstrap".to_string(),
        ))
    }

    fn wait_for_revision(
        &self,
        conversation_id: &str,
        owner_client_id: &str,
        minimum_revision: Option<u64>,
        timeout: Duration,
        require_bootstrapped: bool,
    ) -> Result<ThreadSnapshot, DesktopIpcError> {
        let deadline = Instant::now() + timeout;
        let mut store = lock(&self.inner.follower);
        loop {
            if store.was_invalidated(conversation_id) {
                return Err(DesktopIpcError::Disconnected(format!(
                    "thread {conversation_id} owner state was invalidated"
                )));
            }
            if let Some(snapshot) = store.snapshot(conversation_id) {
                let revision_ready = minimum_revision.map_or(true, |revision| snapshot.revision >= revision);
                let bootstrap_ready = !require_bootstrapped || store.is_bootstrapped(conversation_id);
                if snapshot.owner_client_id == owner_client_id && revision_ready && bootstrap_ready {
                    return Ok(snapshot);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(DesktopIpcError::Timeout(format!(
                    "waiting for thread {conversation_id} snapshot revision {:?}",
                    minimum_revision
                )));
            }
            let (next, _) = self
                .inner
                .follower_wake
                .wait_timeout(store, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            store = next;
        }
    }

    fn request(
        &self,
        version: u64,
        method: &str,
        params: Value,
        target_client_id: Option<&str>,
        timeout: Duration,
        expected_generation: Option<u64>,
    ) -> Result<IpcResponse, DesktopIpcError> {
        let request_id = Uuid::new_v4().to_string();
        let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let (sender, receiver) = mpsc::channel();
        let (generation, write_error) = {
            let mut connection = lock(&self.inner.connection);
            if connection.shutdown {
                return Err(DesktopIpcError::Shutdown);
            }
            if connection.status != DesktopConnectionStatus::Ready {
                return Err(connection.error.clone().unwrap_or_else(|| {
                    DesktopIpcError::Disconnected("router is not connected".to_string())
                }));
            }
            if expected_generation.is_some_and(|expected| expected != connection.generation) {
                return Err(DesktopIpcError::Disconnected(
                    "Desktop IPC connection changed before request dispatch".to_string(),
                ));
            }
            let generation = connection.generation;
            let client_id = connection.client_id.clone().ok_or_else(|| {
                DesktopIpcError::Protocol("connected client is missing identity".to_string())
            })?;
            lock(&self.inner.pending).insert(request_id.clone(), sender);
            let envelope = request_envelope(
                &request_id,
                &client_id,
                version,
                method,
                params,
                target_client_id,
                timeout_ms,
            );
            let result = write_connection(&mut connection, &envelope);
            if let Err(error) = result {
                lock(&self.inner.pending).remove(&request_id);
                (generation, Some(error))
            } else {
                (generation, None)
            }
        };
        if let Some(error) = write_error {
            disconnect_if_generation(&self.inner, generation, error.clone());
            return Err(error);
        }
        match receiver.recv_timeout(timeout + Duration::from_millis(250)) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.inner.pending).remove(&request_id);
                Err(DesktopIpcError::Timeout(method.to_string()))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(DesktopIpcError::Disconnected(
                format!("response channel closed for {method}"),
            )),
        }
    }

    fn send_following_changed(
        &self,
        conversation_id: &str,
        following: bool,
        target_client_id: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<(), DesktopIpcError> {
        let targets = target_client_id.map(|target| vec![target.to_string()]);
        self.broadcast(
            IPC_ROUTER_VERSION,
            METHOD_THREAD_STREAM_FOLLOWING_CHANGED,
            json!({
                "conversationId": conversation_id,
                "hostId": LOCAL_HOST_ID,
                "following": following,
            }),
            targets.as_deref(),
            expected_generation,
        )
    }

    fn broadcast(
        &self,
        version: u64,
        method: &str,
        params: Value,
        target_client_ids: Option<&[String]>,
        expected_generation: Option<u64>,
    ) -> Result<(), DesktopIpcError> {
        let (generation, result) = {
            let mut connection = lock(&self.inner.connection);
            if connection.status != DesktopConnectionStatus::Ready {
                return Err(connection.error.clone().unwrap_or_else(|| {
                    DesktopIpcError::Disconnected("router is not connected".to_string())
                }));
            }
            if expected_generation.is_some_and(|expected| expected != connection.generation) {
                return Err(DesktopIpcError::Disconnected(
                    "Desktop IPC connection changed before broadcast dispatch".to_string(),
                ));
            }
            let generation = connection.generation;
            let client_id = connection.client_id.clone().ok_or_else(|| {
                DesktopIpcError::Protocol("connected client is missing identity".to_string())
            })?;
            let envelope = broadcast_envelope(
                &client_id,
                version,
                method,
                params,
                target_client_ids,
            );
            (generation, write_connection(&mut connection, &envelope))
        };
        if let Err(error) = &result {
            disconnect_if_generation(&self.inner, generation, error.clone());
        }
        result
    }

    pub fn shutdown(&self) -> Result<(), DesktopIpcError> {
        let snapshot = {
            let mut connection = lock(&self.inner.connection);
            if connection.shutdown {
                return Ok(());
            }
            connection.shutdown = true;
            connection.status = DesktopConnectionStatus::Shutdown;
            shutdown_stream(connection.writer.take());
            connection_snapshot(&connection)
        };
        emit(
            &self.inner,
            DesktopClientEvent::ConnectionChanged(snapshot),
        );
        self.inner.connection_wake.notify_all();
        fail_pending(&self.inner, DesktopIpcError::Shutdown);
        self.inner.follower_wake.notify_all();
        if let Some(handle) = lock(&self.inner.supervisor).take() {
            handle
                .join()
                .map_err(|_| DesktopIpcError::Shutdown)?;
        }
        Ok(())
    }
}

fn supervisor_loop(
    inner: Arc<ClientInner>,
    mut reader: Option<IpcStream>,
    initial_messages: Vec<Value>,
) {
    let mut backoff = INITIAL_BACKOFF;
    let mut reader_generation = reader
        .as_ref()
        .and_then(|_| current_ready_generation(&inner));
    for message in initial_messages {
        if let Some(generation) = reader_generation {
            handle_message(&inner, message, generation);
        }
    }
    if connection_failure(&inner).is_some() {
        reader = None;
        reader_generation = None;
        if wait_for_retry(&inner, backoff) {
            return;
        }
        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
    }
    loop {
        if is_shutdown(&inner) {
            return;
        }
        if reader.is_none() {
            match connect_and_initialize() {
                Ok((next_reader, writer, client_id, buffered)) => {
                    let mut subscribers = lock(&inner.subscribers);
                    reset_follower(&inner);
                    inner.follower_wake.notify_all();
                    let (generation, connection_event) = {
                        let mut connection = lock(&inner.connection);
                        if connection.shutdown {
                            shutdown_stream(Some(writer));
                            return;
                        }
                        connection.writer = Some(writer);
                        connection.client_id = Some(client_id);
                        connection.generation = connection.generation.saturating_add(1);
                        connection.status = DesktopConnectionStatus::Ready;
                        connection.error = None;
                        (connection.generation, connection_snapshot(&connection))
                    };
                    emit_to_subscribers(
                        &mut subscribers,
                        DesktopClientEvent::ConnectionChanged(connection_event),
                    );
                    drop(subscribers);
                    reader = Some(next_reader);
                    reader_generation = Some(generation);
                    for message in buffered {
                        handle_message(&inner, message, generation);
                    }
                    if connection_failure(&inner).is_some() {
                        reader = None;
                        reader_generation = None;
                        if wait_for_retry(&inner, backoff) {
                            return;
                        }
                        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                    } else {
                        backoff = INITIAL_BACKOFF;
                    }
                    continue;
                }
                Err(error) => {
                    set_unavailable_status(&inner, error);
                    if wait_for_retry(&inner, backoff) {
                        return;
                    }
                    backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                    continue;
                }
            }
        }

        let read_result = read_frame(reader.as_mut().expect("reader checked above"));
        match read_result {
            Ok(message) => {
                if let Some(generation) = reader_generation {
                    handle_message(&inner, message, generation);
                }
                if connection_failure(&inner).is_some() {
                    reader = None;
                    reader_generation = None;
                    if wait_for_retry(&inner, backoff) {
                        return;
                    }
                    backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                }
            }
            Err(error) => {
                reader = None;
                if let Some(generation) = reader_generation.take() {
                    disconnect_if_generation(&inner, generation, error);
                }
                if wait_for_retry(&inner, backoff) {
                    return;
                }
                backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
            }
        }
    }
}

fn handle_message(inner: &Arc<ClientInner>, message: Value, generation: u64) {
    let client_id = {
        let connection = lock(&inner.connection);
        if connection.status != DesktopConnectionStatus::Ready
            || connection.generation != generation
        {
            return;
        }
        connection
            .client_id
            .clone()
            .unwrap_or_else(|| INITIAL_CLIENT_ID.to_string())
    };
    if !message_targets_client(&message, &client_id) {
        return;
    }
    match message.get("type").and_then(Value::as_str) {
        Some("response") => match IpcResponse::from_value(&message) {
            Ok(response) => {
                if let Some(sender) = lock(&inner.pending).remove(&response.request_id) {
                    let _ = sender.send(Ok(response));
                }
            }
            Err(error) => disconnect_if_generation(inner, generation, error),
        },
        Some("broadcast") => handle_broadcast(inner, &message, generation),
        Some("client-discovery-request") => respond_to_discovery(inner, &message, generation),
        Some("request") => respond_to_unsupported_request(inner, &message, generation),
        Some(other) => emit_diagnostic(inner, format!("ignored unknown IPC message type {other}")),
        None => emit_diagnostic(inner, "ignored IPC message without type".to_string()),
    }
}

fn handle_broadcast(inner: &Arc<ClientInner>, message: &Value, generation: u64) {
    let Some(method_name) = method(message) else {
        emit_diagnostic(inner, "ignored IPC broadcast without method".to_string());
        return;
    };
    let params = message.get("params").unwrap_or(&Value::Null);
    match method_name {
        METHOD_THREAD_STREAM_STATE_CHANGED => {
            if version(message) != Some(THREAD_STREAM_STATE_VERSION) {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(format!(
                        "thread state version mismatch: expected {THREAD_STREAM_STATE_VERSION}, received {:?}",
                        version(message)
                    )),
                );
                return;
            }
            handle_stream_change(inner, message, params, generation)
        }
        METHOD_THREAD_STREAM_FOLLOWING_CHANGED => {
            if version(message) != Some(IPC_ROUTER_VERSION) {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(format!(
                        "following version mismatch: expected {IPC_ROUTER_VERSION}, received {:?}",
                        version(message)
                    )),
                );
                return;
            }
            let Some(host_id) = params.get("hostId").and_then(Value::as_str) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following broadcast is missing hostId".to_string(),
                    ),
                );
                return;
            };
            if host_id != LOCAL_HOST_ID {
                return;
            }
            let Some(following) = params.get("following").and_then(Value::as_bool) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following broadcast is missing following".to_string(),
                    ),
                );
                return;
            };
            if !following {
                return;
            }
            let Some(conversation_id) = params.get("conversationId").and_then(Value::as_str) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following broadcast is missing conversationId".to_string(),
                    ),
                );
                return;
            };
            if source_client_id(message).is_none() {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following broadcast is missing sourceClientId".to_string(),
                    ),
                );
                return;
            }
            let should_bootstrap = {
                let mut follower = lock(&inner.follower);
                follower.remember_thread(conversation_id)
                    || !follower.is_bootstrapped(conversation_id)
            };
            if should_bootstrap {
                emit(
                    inner,
                    DesktopClientEvent::ThreadDiscovered {
                        conversation_id: conversation_id.to_string(),
                    },
                );
            }
        }
        METHOD_THREAD_STREAM_FOLLOWING_STATUS_REQUESTED => {
            if version(message) != Some(IPC_ROUTER_VERSION) {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(format!(
                        "following-status version mismatch: expected {IPC_ROUTER_VERSION}, received {:?}",
                        version(message)
                    )),
                );
                return;
            }
            let Some(host_id) = params.get("hostId").and_then(Value::as_str) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following-status broadcast is missing hostId".to_string(),
                    ),
                );
                return;
            };
            if host_id != LOCAL_HOST_ID {
                return;
            }
            let Some(conversation_id) = params.get("conversationId").and_then(Value::as_str) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following-status broadcast is missing conversationId".to_string(),
                    ),
                );
                return;
            };
            let Some(requester) = source_client_id(message) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "following-status broadcast is missing sourceClientId".to_string(),
                    ),
                );
                return;
            };
            if !lock(&inner.follower).is_bootstrapped(conversation_id) {
                lock(&inner.follower).remember_thread(conversation_id);
                emit(
                    inner,
                    DesktopClientEvent::ThreadDiscovered {
                        conversation_id: conversation_id.to_string(),
                    },
                );
                return;
            }
            if lock(&inner.follower).expected_owner(conversation_id).as_deref()
                != Some(requester)
            {
                let client = CodexDesktopClient { inner: inner.clone() };
                let conversation_id = conversation_id.to_string();
                let requester = requester.to_string();
                thread::spawn(move || {
                    client.verify_owner_candidate(
                        &conversation_id,
                        &requester,
                        generation,
                    );
                });
                return;
            }
            let client = CodexDesktopClient { inner: inner.clone() };
            if let Err(error) = client.send_following_changed(
                conversation_id,
                true,
                Some(requester),
                Some(generation),
            ) {
                emit_diagnostic(inner, error.to_string());
            }
        }
        METHOD_CLIENT_STATUS_CHANGED => {
            if version(message) != Some(CLIENT_STATUS_VERSION) {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(format!(
                        "client-status version mismatch: expected {CLIENT_STATUS_VERSION}, received {:?}",
                        version(message)
                    )),
                );
                return;
            }
            let Some(changed_client_id) = params.get("clientId").and_then(Value::as_str) else {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "client-status broadcast is missing clientId".to_string(),
                    ),
                );
                return;
            };
            if source_client_id(message) != Some(changed_client_id) {
                disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "client-status source identity does not match payload".to_string(),
                    ),
                );
                return;
            }
            match params.get("status").and_then(Value::as_str) {
                Some("disconnected") => {
                    let mut subscribers = lock(&inner.subscribers);
                    let (affected_count, rediscover, follower_epoch) = {
                        let mut follower = lock(&inner.follower);
                        let affected = follower.forget_owner(changed_client_id);
                        if affected.is_empty() {
                            (0, Vec::new(), follower.epoch())
                        } else {
                            follower.reset_for_reconnect();
                            (
                                affected.len(),
                                follower.known_threads(),
                                follower.epoch(),
                            )
                        }
                    };
                    if affected_count > 0 {
                        inner.follower_wake.notify_all();
                        emit_to_subscribers(
                            &mut subscribers,
                            DesktopClientEvent::FollowerStateReset {
                                reason: "Desktop thread owner disconnected".to_string(),
                                generation,
                                follower_epoch,
                            },
                        );
                        drop(subscribers);
                        emit_diagnostic(
                            inner,
                            format!(
                                "Desktop thread owner disconnected; re-discovering {} followed thread(s)",
                                rediscover.len()
                            ),
                        );
                        for conversation_id in rediscover {
                            emit(inner, DesktopClientEvent::ThreadDiscovered { conversation_id });
                        }
                    } else {
                        drop(subscribers);
                    }
                }
                Some("connected") => {
                    let pending = lock(&inner.follower).threads_needing_bootstrap();
                    for conversation_id in pending {
                        emit(inner, DesktopClientEvent::ThreadDiscovered { conversation_id });
                    }
                }
                Some(other) => emit_diagnostic(
                    inner,
                    format!("ignored unknown Desktop client status {other}"),
                ),
                None => disconnect_if_generation(
                    inner,
                    generation,
                    DesktopIpcError::Protocol(
                        "client-status broadcast is missing status".to_string(),
                    ),
                ),
            }
        }
        other => emit_diagnostic(inner, format!("ignored unknown IPC broadcast method {other}")),
    }
}

fn handle_stream_change(
    inner: &Arc<ClientInner>,
    message: &Value,
    params: &Value,
    generation: u64,
) {
    let Some(host_id) = params.get("hostId").and_then(Value::as_str) else {
        disconnect_if_generation(
            inner,
            generation,
            DesktopIpcError::Protocol(
                "thread state broadcast is missing hostId".to_string(),
            ),
        );
        return;
    };
    if host_id != LOCAL_HOST_ID {
        return;
    }
    let Some(conversation_id) = params.get("conversationId").and_then(Value::as_str) else {
        disconnect_if_generation(
            inner,
            generation,
            DesktopIpcError::Protocol(
                "thread state broadcast is missing conversationId".to_string(),
            ),
        );
        return;
    };
    let Some(owner_client_id) = source_client_id(message) else {
        disconnect_if_generation(
            inner,
            generation,
            DesktopIpcError::Protocol(
                "thread state broadcast is missing sourceClientId".to_string(),
            ),
        );
        return;
    };
    let Some(change) = params.get("change") else {
        disconnect_if_generation(
            inner,
            generation,
            DesktopIpcError::Protocol(
                "thread state broadcast is missing change".to_string(),
            ),
        );
        return;
    };
    let (outcome, snapshot, bootstrapped, follower_epoch) = {
        let mut follower = lock(&inner.follower);
        match follower.apply_stream_change(conversation_id, owner_client_id, change) {
            Ok(outcome) => {
                let snapshot = follower.snapshot(conversation_id);
                let bootstrapped = follower.is_bootstrapped(conversation_id);
                let follower_epoch = follower.epoch();
                (outcome, snapshot, bootstrapped, follower_epoch)
            }
            Err(error) => {
                drop(follower);
                disconnect_if_generation(inner, generation, error);
                return;
            }
        }
    };
    inner.follower_wake.notify_all();
    match outcome {
        StateChangeOutcome::Applied => {
            if let Some(snapshot) = snapshot {
                emit(
                    inner,
                    DesktopClientEvent::ThreadStateChanged {
                        snapshot,
                        bootstrapped,
                        generation,
                        follower_epoch,
                    },
                );
            }
        }
        StateChangeOutcome::Gap {
            expected_base_revision,
            received_base_revision,
            ..
        } => {
            emit(
                inner,
                DesktopClientEvent::RevisionGap {
                    conversation_id: conversation_id.to_string(),
                    expected_base_revision,
                    received_base_revision,
                },
            );
            disconnect_if_generation(
                inner,
                generation,
                DesktopIpcError::Disconnected(
                    "Desktop thread revision became discontinuous".to_string(),
                ),
            );
        }
        StateChangeOutcome::AwaitingSnapshot | StateChangeOutcome::Ignored => {}
    }
}

fn respond_to_discovery(inner: &Arc<ClientInner>, message: &Value, generation: u64) {
    let Some(request_id) = message.get("requestId").and_then(Value::as_str) else {
        emit_diagnostic(inner, "client discovery request is missing requestId".to_string());
        return;
    };
    let response = json!({
        "type": "client-discovery-response",
        "requestId": request_id,
        "response": { "canHandle": false },
    });
    let result = write_for_generation(inner, generation, &response);
    if let Err(error) = result {
        disconnect_if_generation(inner, generation, error);
    }
}

fn respond_to_unsupported_request(
    inner: &Arc<ClientInner>,
    message: &Value,
    generation: u64,
) {
    let Ok(request_id) = required_string(message, "requestId") else {
        return;
    };
    let method_name = method(message).unwrap_or("unknown");
    let client_id = {
        let connection = lock(&inner.connection);
        if connection.status != DesktopConnectionStatus::Ready
            || connection.generation != generation
        {
            return;
        }
        connection
            .client_id
            .clone()
            .unwrap_or_else(|| INITIAL_CLIENT_ID.to_string())
    };
    let response = json!({
        "type": "response",
        "requestId": request_id,
        "resultType": "error",
        "method": method_name,
        "handledByClientId": client_id,
        "error": "no-handler-for-request",
    });
    let result = write_for_generation(inner, generation, &response);
    if let Err(error) = result {
        disconnect_if_generation(inner, generation, error);
    }
}

fn disconnect_if_generation(
    inner: &Arc<ClientInner>,
    expected_generation: u64,
    error: DesktopIpcError,
) {
    let mut subscribers = lock(&inner.subscribers);
    let snapshot = {
        let mut connection = lock(&inner.connection);
        if connection.generation != expected_generation {
            return;
        }
        shutdown_stream(connection.writer.take());
        connection.client_id = None;
        if connection.shutdown {
            None
        } else {
            connection.generation = connection.generation.saturating_add(1);
            connection.status = DesktopConnectionStatus::Unavailable;
            connection.error = Some(error.clone());
            Some(connection_snapshot(&connection))
        }
    };
    let Some(snapshot) = snapshot else {
        drop(subscribers);
        fail_pending(inner, DesktopIpcError::Shutdown);
        return;
    };
    reset_follower(inner);
    inner.follower_wake.notify_all();
    fail_pending(inner, DesktopIpcError::Disconnected(error.to_string()));
    emit_to_subscribers(
        &mut subscribers,
        DesktopClientEvent::ConnectionChanged(snapshot),
    );
}

fn reset_follower(inner: &Arc<ClientInner>) -> u64 {
    let mut follower = lock(&inner.follower);
    follower.reset_for_reconnect();
    follower.epoch()
}

fn fail_pending(inner: &Arc<ClientInner>, error: DesktopIpcError) {
    let pending = std::mem::take(&mut *lock(&inner.pending));
    for sender in pending.into_values() {
        let _ = sender.send(Err(error.clone()));
    }
}

fn set_unavailable_status(inner: &Arc<ClientInner>, error: DesktopIpcError) {
    let snapshot = {
        let mut connection = lock(&inner.connection);
        if connection.shutdown
            || (connection.status == DesktopConnectionStatus::Unavailable
                && connection.error.as_ref() == Some(&error))
        {
            return;
        }
        connection.status = DesktopConnectionStatus::Unavailable;
        connection.error = Some(error);
        connection_snapshot(&connection)
    };
    emit(inner, DesktopClientEvent::ConnectionChanged(snapshot));
}

fn connection_snapshot(connection: &ConnectionState) -> DesktopConnectionSnapshot {
    DesktopConnectionSnapshot {
        status: connection.status,
        client_id: connection.client_id.clone(),
        generation: connection.generation,
        error: connection.error.clone(),
    }
}

fn emit(inner: &Arc<ClientInner>, event: DesktopClientEvent) {
    emit_to_subscribers(&mut lock(&inner.subscribers), event);
}

fn emit_to_subscribers(
    subscribers: &mut Vec<Sender<DesktopClientEvent>>,
    event: DesktopClientEvent,
) {
    subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
}

fn emit_diagnostic(inner: &Arc<ClientInner>, message: String) {
    emit(inner, DesktopClientEvent::Diagnostic(message));
}

fn wait_for_retry(inner: &Arc<ClientInner>, duration: Duration) -> bool {
    let connection = lock(&inner.connection);
    let (connection, _) = inner
        .connection_wake
        .wait_timeout_while(connection, duration, |connection| !connection.shutdown)
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    connection.shutdown
}

fn is_shutdown(inner: &Arc<ClientInner>) -> bool {
    lock(&inner.connection).shutdown
}

fn connection_failure(inner: &Arc<ClientInner>) -> Option<DesktopIpcError> {
    let connection = lock(&inner.connection);
    (connection.status == DesktopConnectionStatus::Unavailable)
        .then(|| connection.error.clone())
        .flatten()
}

fn current_ready_generation(inner: &Arc<ClientInner>) -> Option<u64> {
    let connection = lock(&inner.connection);
    (connection.status == DesktopConnectionStatus::Ready).then_some(connection.generation)
}

fn write_for_generation(
    inner: &Arc<ClientInner>,
    generation: u64,
    message: &Value,
) -> Result<(), DesktopIpcError> {
    let mut connection = lock(&inner.connection);
    if connection.status != DesktopConnectionStatus::Ready
        || connection.generation != generation
    {
        return Err(DesktopIpcError::Disconnected(
            "Desktop IPC connection changed before response dispatch".to_string(),
        ));
    }
    write_connection(&mut connection, message)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(unix)]
fn connect_and_initialize() -> Result<(IpcStream, IpcStream, String, Vec<Value>), DesktopIpcError> {
    use super::transport::connect_validated_socket;

    let mut stream = connect_validated_socket()?;
    stream
        .set_read_timeout(Some(ROUTER_REQUEST_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(ROUTER_REQUEST_TIMEOUT)))
        .map_err(|error| DesktopIpcError::Io(error.to_string()))?;
    let request_id = Uuid::new_v4().to_string();
    write_frame(
        &mut stream,
        &request_envelope(
            &request_id,
            INITIAL_CLIENT_ID,
            INITIALIZE_VERSION,
            METHOD_INITIALIZE,
            json!({ "clientType": "codepet-desktop-follower" }),
            None,
            u64::try_from(ROUTER_REQUEST_TIMEOUT.as_millis()).unwrap_or(5_000),
        ),
    )?;
    let deadline = Instant::now() + ROUTER_REQUEST_TIMEOUT;
    let mut buffered = Vec::new();
    let response = loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(DesktopIpcError::Timeout(METHOD_INITIALIZE.to_string()));
        }
        stream
            .set_read_timeout(Some(deadline.saturating_duration_since(now)))
            .map_err(|error| DesktopIpcError::Io(error.to_string()))?;
        let message = read_frame(&mut stream)?;
        if message.get("type").and_then(Value::as_str) == Some("client-discovery-request") {
            let discovery_request_id = required_string(&message, "requestId")?;
            write_frame(
                &mut stream,
                &json!({
                    "type": "client-discovery-response",
                    "requestId": discovery_request_id,
                    "response": { "canHandle": false },
                }),
            )?;
            continue;
        }
        if message.get("type").and_then(Value::as_str) == Some("response")
            && message.get("requestId").and_then(Value::as_str) == Some(request_id.as_str())
        {
            break IpcResponse::from_value(&message)?;
        }
        buffered.push(message);
    };
    let client_id = response
        .success_result(METHOD_INITIALIZE)?
        .get("clientId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value != INITIAL_CLIENT_ID)
        .map(str::to_string)
        .ok_or_else(|| {
            DesktopIpcError::Protocol("initialize response is missing unique clientId".to_string())
        })?;
    if response.handled_by_client_id.as_deref() != Some(client_id.as_str()) {
        return Err(DesktopIpcError::Protocol(
            "initialize response identity fields do not match".to_string(),
        ));
    }
    stream
        .set_read_timeout(None)
        .map_err(|error| DesktopIpcError::Io(error.to_string()))?;
    let writer = stream
        .try_clone()
        .map_err(|error| DesktopIpcError::Io(error.to_string()))?;
    Ok((stream, writer, client_id, buffered))
}

#[cfg(not(unix))]
fn connect_and_initialize() -> Result<(IpcStream, IpcStream, String, Vec<Value>), DesktopIpcError> {
    Err(super::transport::unsupported_platform_error())
}

#[cfg(unix)]
fn write_connection(
    connection: &mut ConnectionState,
    message: &Value,
) -> Result<(), DesktopIpcError> {
    let writer = connection.writer.as_mut().ok_or_else(|| {
        connection.error.clone().unwrap_or_else(|| {
            DesktopIpcError::Disconnected("socket writer is unavailable".to_string())
        })
    })?;
    write_frame(writer, message)
}

#[cfg(not(unix))]
fn write_connection(
    _connection: &mut ConnectionState,
    _message: &Value,
) -> Result<(), DesktopIpcError> {
    Err(super::transport::unsupported_platform_error())
}

#[cfg(unix)]
fn shutdown_stream(stream: Option<IpcStream>) {
    use std::net::Shutdown;
    if let Some(stream) = stream {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

#[cfg(not(unix))]
fn shutdown_stream(_stream: Option<IpcStream>) {}
