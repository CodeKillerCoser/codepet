use crate::protocol::{
    OpenCodeActiveSessions, OpenCodeDataResponse, OpenCodeEvent, OpenCodeHealth,
    OpenCodePermissionReply, OpenCodePermissionReplyRequest, OpenCodePromptAdmission,
    OpenCodePromptRequest, OpenCodeServerError, OpenCodeSession, OpenCodeSessionCreate,
    OpenCodeSessionPage, OPENCODE_VERIFIED_SERVER_VERSION,
};
use reqwest::blocking::{Client, Response};
use reqwest::Url;
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_HEALTH_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_CONFIRM_DELAY: Duration = Duration::from_millis(100);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const STARTUP_RETRY_DELAY: Duration = Duration::from_millis(50);
const EVENT_QUEUE_CAPACITY: usize = 64;
const MAX_JSON_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_STARTUP_OUTPUT_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct OpenCodeClient {
    base_url: Url,
    requests: Client,
    waits: Client,
    events: Client,
    auth: Option<(String, String)>,
}

impl OpenCodeClient {
    fn new(
        base_url: Url,
        username: String,
        password: String,
    ) -> Result<Self, OpenCodeServerError> {
        Self::new_with_request_timeout(base_url, username, password, REQUEST_TIMEOUT)
    }

    fn new_with_request_timeout(
        base_url: Url,
        username: String,
        password: String,
        request_timeout: Duration,
    ) -> Result<Self, OpenCodeServerError> {
        let requests = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(request_timeout)
            .build()
            .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
        let waits = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
        let events = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
        Ok(Self {
            base_url,
            requests,
            waits,
            events,
            auth: Some((username, password)),
        })
    }

    pub fn health(&self, timeout: Duration) -> Result<OpenCodeHealth, OpenCodeServerError> {
        let response = self.request(self.requests.get(self.url(&["api", "health"])?))
            .timeout(timeout)
            .send()
            .map_err(transport_error)?;
        decode_json(response)
    }

    pub fn list_sessions(
        &self,
        cursor: Option<&str>,
        limit: Option<u64>,
    ) -> Result<OpenCodeSessionPage, OpenCodeServerError> {
        let mut request = self.request(self.requests.get(self.url(&["api", "session"])?));
        request = request.query(&[("order", "desc")]);
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        if let Some(limit) = limit {
            request = request.query(&[("limit", limit)]);
        }
        decode_json(request.send().map_err(transport_error)?)
    }

    pub fn active_sessions(&self) -> Result<OpenCodeActiveSessions, OpenCodeServerError> {
        let response: OpenCodeDataResponse<OpenCodeActiveSessions> = decode_json(
            self.request(self.requests.get(self.url(&["api", "session", "active"])?))
                .send()
                .map_err(transport_error)?,
        )?;
        Ok(response.data)
    }

    pub fn get_session(&self, session_id: &str) -> Result<OpenCodeSession, OpenCodeServerError> {
        let response: OpenCodeDataResponse<OpenCodeSession> = decode_json(
            self.request(self.requests.get(self.url(&["api", "session", session_id])?))
                .send()
                .map_err(transport_error)?,
        )?;
        Ok(response.data)
    }

    pub fn create_session(
        &self,
        request: &OpenCodeSessionCreate,
    ) -> Result<OpenCodeSession, OpenCodeServerError> {
        let response: OpenCodeDataResponse<OpenCodeSession> = decode_json(
            self.request(self.requests.post(self.url(&["api", "session"])?))
                .json(request)
                .send()
                .map_err(transport_error)?,
        )?;
        Ok(response.data)
    }

    pub fn prompt(
        &self,
        session_id: &str,
        request: &OpenCodePromptRequest,
    ) -> Result<OpenCodePromptAdmission, OpenCodeServerError> {
        let response: OpenCodeDataResponse<OpenCodePromptAdmission> = decode_json(
            self.request(
                self.requests
                    .post(self.url(&["api", "session", session_id, "prompt"])?),
            )
            .json(request)
            .send()
            .map_err(transport_error)?,
        )?;
        Ok(response.data)
    }

    pub fn interrupt(&self, session_id: &str) -> Result<(), OpenCodeServerError> {
        decode_no_content(
            self.request(
                self.requests
                    .post(self.url(&["api", "session", session_id, "interrupt"])?),
            )
            .send()
            .map_err(transport_error)?,
        )
    }

    pub fn wait_session(&self, session_id: &str) -> Result<(), OpenCodeServerError> {
        decode_no_content(
            self.request(
                self.waits
                    .post(self.url(&["api", "session", session_id, "wait"])?),
            )
            .send()
            .map_err(transport_error)?,
        )
    }

    pub fn reply_permission(
        &self,
        session_id: &str,
        request_id: &str,
        reply: OpenCodePermissionReply,
    ) -> Result<(), OpenCodeServerError> {
        decode_no_content(
            self.request(self.requests.post(self.url(&[
                "api",
                "session",
                session_id,
                "permission",
                request_id,
                "reply",
            ])?))
            .json(&OpenCodePermissionReplyRequest { reply })
            .send()
            .map_err(transport_error)?,
        )
    }

    fn event_response(&self) -> Result<Response, OpenCodeServerError> {
        let response = self.request(self.events.get(self.url(&["api", "event"])?))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .map_err(transport_error)?;
        ensure_success(response)
    }

    fn request(&self, request: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        if let Some((username, password)) = self.auth.as_ref() {
            request.basic_auth(username, Some(password))
        } else {
            request
        }
    }

    fn url(&self, segments: &[&str]) -> Result<Url, OpenCodeServerError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| OpenCodeServerError::Protocol("invalid OpenCode Server base URL".to_string()))?
            .clear()
            .extend(segments);
        Ok(url)
    }
}

struct SessionInner {
    generation: String,
    client: OpenCodeClient,
    child: Mutex<Option<Child>>,
    stopped: AtomicBool,
    subscriber: Mutex<Option<JoinHandle<()>>>,
    output: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct OpenCodeServerSession {
    inner: Arc<SessionInner>,
}

impl OpenCodeServerSession {
    pub fn spawn(
        executable: &Path,
        args: &[String],
        server_version: &str,
        generation: String,
    ) -> Result<Self, OpenCodeServerError> {
        validate_server_version(server_version)?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let username = "codepet".to_string();
        let password = Uuid::new_v4().to_string();
        let mut command = Command::new(executable);
        command
            .args(args)
            .arg("--hostname")
            .arg("127.0.0.1")
            .env("OPENCODE_SERVER_USERNAME", username.clone())
            .env("OPENCODE_SERVER_PASSWORD", password.clone())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command
            .spawn()
            .map_err(|error| OpenCodeServerError::Spawn(error.to_string()))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            OpenCodeServerError::Spawn("OpenCode Server stdout was not piped".to_string())
        })?;
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let output = thread::spawn(move || drain_server_output(stdout, startup_sender));
        let base_url = match wait_for_listening_address(&startup_receiver, &mut child, deadline) {
            Ok(base_url) => base_url,
            Err(error) => {
                let _ = terminate_child(&mut child, Instant::now() + SHUTDOWN_TIMEOUT);
                let _ = output.join();
                return Err(error);
            }
        };
        let client = match OpenCodeClient::new(base_url, username, password) {
            Ok(client) => client,
            Err(error) => {
                let _ = terminate_child(&mut child, Instant::now() + SHUTDOWN_TIMEOUT);
                let _ = output.join();
                return Err(error);
            }
        };
        if let Err(error) = wait_for_ready(&client, &mut child, deadline) {
            let _ = terminate_child(&mut child, Instant::now() + SHUTDOWN_TIMEOUT);
            let _ = output.join();
            return Err(error);
        }
        Ok(Self {
            inner: Arc::new(SessionInner {
                generation,
                client,
                child: Mutex::new(Some(child)),
                stopped: AtomicBool::new(false),
                subscriber: Mutex::new(None),
                output: Mutex::new(Some(output)),
            }),
        })
    }

    pub fn generation(&self) -> &str {
        &self.inner.generation
    }

    pub fn client(&self) -> OpenCodeClient {
        self.inner.client.clone()
    }

    pub fn subscribe(
        &self,
    ) -> Result<Receiver<Result<OpenCodeEvent, OpenCodeServerError>>, OpenCodeServerError> {
        let mut subscriber = lock(&self.inner.subscriber);
        if subscriber.is_some() {
            return Err(OpenCodeServerError::Protocol(
                "OpenCode Server events are already subscribed".to_string(),
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let inner = self.inner.clone();
        *subscriber = Some(thread::spawn(move || {
            if inner.stopped.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = inner.client.event_response().and_then(|response| {
                read_sse(response, &inner.stopped, |event| {
                    send_event(&sender, event)
                })
            }) {
                if !inner.stopped.load(Ordering::SeqCst) {
                    let _ = sender.try_send(Err(error));
                }
            }
        }));
        Ok(receiver)
    }

    pub fn shutdown(&self) -> Result<(), OpenCodeServerError> {
        if self.inner.stopped.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let child_result = {
            let mut child = lock(&self.inner.child);
            if let Some(mut child) = child.take() {
                terminate_child(&mut child, deadline)
            } else {
                Ok(())
            }
        };
        if let Some(subscriber) = lock(&self.inner.subscriber).take() {
            while !subscriber.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if subscriber.is_finished() {
                let _ = subscriber.join();
            } else if child_result.is_ok() {
                return Err(OpenCodeServerError::Timeout(
                    "OpenCode event subscriber did not stop before the shutdown deadline"
                        .to_string(),
                ));
            }
        }
        if let Some(output) = lock(&self.inner.output).take() {
            while !output.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if output.is_finished() {
                let _ = output.join();
            } else if child_result.is_ok() {
                return Err(OpenCodeServerError::Timeout(
                    "OpenCode output reader did not stop before the shutdown deadline".to_string(),
                ));
            }
        }
        child_result
    }
}

fn drain_server_output(
    stdout: ChildStdout,
    startup: SyncSender<Result<Url, OpenCodeServerError>>,
) {
    let mut reader = BufReader::new(stdout);
    let mut reported = false;
    loop {
        match read_bounded_line(&mut reader, MAX_STARTUP_OUTPUT_LINE_BYTES) {
            Ok(Some(line)) => {
                if reported {
                    continue;
                }
                let line = match std::str::from_utf8(&line) {
                    Ok(line) => line.trim(),
                    Err(error) => {
                        let _ = startup.try_send(Err(OpenCodeServerError::Protocol(format!(
                            "OpenCode startup output is not UTF-8: {error}"
                        ))));
                        return;
                    }
                };
                match parse_listening_address(line) {
                    Ok(Some(url)) => {
                        let _ = startup.try_send(Ok(url));
                        reported = true;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = startup.try_send(Err(error));
                        return;
                    }
                }
            }
            Ok(None) => {
                if !reported {
                    let _ = startup.try_send(Err(OpenCodeServerError::Protocol(
                        "OpenCode Server exited before reporting its listening address"
                            .to_string(),
                    )));
                }
                return;
            }
            Err(error) => {
                if !reported {
                    let _ = startup.try_send(Err(error));
                }
                return;
            }
        }
    }
}

fn parse_listening_address(line: &str) -> Result<Option<Url>, OpenCodeServerError> {
    let address = if let Some(address) = line.strip_prefix("opencode server listening on ") {
        address
    } else if let Some(address) = line.strip_prefix("server listening on ") {
        address
    } else {
        return Ok(None);
    };
    let url = Url::parse(address.trim()).map_err(|error| {
        OpenCodeServerError::Protocol(format!(
            "invalid OpenCode Server listening address: {error}"
        ))
    })?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none_or(|port| port == 0)
        || url.path() != "/"
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(OpenCodeServerError::Protocol(
            "OpenCode Server reported a non-loopback or malformed listening address".to_string(),
        ));
    }
    Ok(Some(url))
}

fn wait_for_listening_address(
    receiver: &Receiver<Result<Url, OpenCodeServerError>>,
    child: &mut Child,
    deadline: Instant,
) -> Result<Url, OpenCodeServerError> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| OpenCodeServerError::Io(error.to_string()))?
        {
            return Err(OpenCodeServerError::ProcessExited(status.to_string()));
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(OpenCodeServerError::Timeout(format!(
                "server did not report a listening address within {} seconds",
                STARTUP_TIMEOUT.as_secs()
            )));
        }
        match receiver.recv_timeout(
            deadline
                .saturating_duration_since(now)
                .min(STARTUP_RETRY_DELAY),
        ) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(OpenCodeServerError::Protocol(
                    "OpenCode startup output closed before a listening address was reported"
                        .to_string(),
                ))
            }
        }
    }
}

fn wait_for_ready(
    client: &OpenCodeClient,
    child: &mut Child,
    deadline: Instant,
) -> Result<(), OpenCodeServerError> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| OpenCodeServerError::Io(error.to_string()))?
        {
            return Err(OpenCodeServerError::ProcessExited(status.to_string()));
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(OpenCodeServerError::Timeout(format!(
                "server did not become healthy within {} seconds",
                STARTUP_TIMEOUT.as_secs()
            )));
        }
        let remaining = deadline.saturating_duration_since(now);
        match client.health(remaining.min(STARTUP_HEALTH_TIMEOUT)) {
            Ok(health) => {
                if !health.healthy {
                    return Err(OpenCodeServerError::Protocol(
                        "OpenCode health response reported healthy=false".to_string(),
                    ));
                }
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(STARTUP_CONFIRM_DELAY),
                );
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| OpenCodeServerError::Io(error.to_string()))?
                {
                    return Err(OpenCodeServerError::ProcessExited(status.to_string()));
                }
                return Ok(());
            }
            Err(_) if Instant::now() < deadline => thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(STARTUP_RETRY_DELAY),
            ),
            Err(error) => {
                return Err(OpenCodeServerError::Timeout(format!(
                    "server did not become healthy within {} seconds: {error}",
                    STARTUP_TIMEOUT.as_secs()
                )))
            }
        }
    }
}

fn validate_server_version(version: &str) -> Result<(), OpenCodeServerError> {
    if version.trim() != OPENCODE_VERIFIED_SERVER_VERSION {
        return Err(OpenCodeServerError::Protocol(format!(
            "OpenCode Server version {version:?} is unsupported; only {OPENCODE_VERIFIED_SERVER_VERSION} is verified"
        )));
    }
    Ok(())
}

fn terminate_child(
    child: &mut Child,
    deadline: Instant,
) -> Result<(), OpenCodeServerError> {
    if child
        .try_wait()
        .map_err(|error| OpenCodeServerError::Io(error.to_string()))?
        .is_some()
    {
        return Ok(());
    }
    child
        .kill()
        .map_err(|error| OpenCodeServerError::Io(error.to_string()))?;
    loop {
        if child
            .try_wait()
            .map_err(|error| OpenCodeServerError::Io(error.to_string()))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(OpenCodeServerError::Timeout(
                "OpenCode Server did not exit before the shutdown deadline".to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn send_event(
    sender: &SyncSender<Result<OpenCodeEvent, OpenCodeServerError>>,
    event: OpenCodeEvent,
) -> Result<(), OpenCodeServerError> {
    sender.try_send(Ok(event)).map_err(|error| match error {
        TrySendError::Full(_) => OpenCodeServerError::Protocol(format!(
            "OpenCode event queue exceeded its fixed capacity of {EVENT_QUEUE_CAPACITY}"
        )),
        TrySendError::Disconnected(_) => OpenCodeServerError::Shutdown,
    })
}

fn read_sse(
    response: impl Read,
    stopped: &AtomicBool,
    mut publish: impl FnMut(OpenCodeEvent) -> Result<(), OpenCodeServerError>,
) -> Result<(), OpenCodeServerError> {
    let mut reader = BufReader::new(response);
    let mut data = String::new();
    loop {
        if stopped.load(Ordering::SeqCst) {
            return Ok(());
        }
        let Some(line) = read_bounded_line(&mut reader, MAX_SSE_LINE_BYTES)? else {
            return Err(OpenCodeServerError::Protocol(
                "OpenCode event stream ended".to_string(),
            ));
        };
        let line = std::str::from_utf8(&line).map_err(|error| {
            OpenCodeServerError::Protocol(format!("OpenCode SSE is not UTF-8: {error}"))
        })?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !data.is_empty() {
                let event = serde_json::from_str::<OpenCodeEvent>(&data).map_err(|error| {
                    OpenCodeServerError::Protocol(format!("invalid OpenCode SSE event: {error}"))
                })?;
                data.clear();
                publish(event)?;
            }
            continue;
        }
        if line.starts_with(':') || line.starts_with("event:") {
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            if !data.is_empty() {
                data.push('\n');
            }
            if data.len().saturating_add(value.len()) > MAX_SSE_EVENT_BYTES {
                return Err(OpenCodeServerError::Protocol(format!(
                    "OpenCode SSE event exceeds {MAX_SSE_EVENT_BYTES} bytes"
                )));
            }
            data.push_str(value);
        }
    }
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    limit: usize,
) -> Result<Option<Vec<u8>>, OpenCodeServerError> {
    let mut captured = Vec::new();
    let mut total = 0usize;
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| OpenCodeServerError::Io(error.to_string()))?;
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
            OpenCodeServerError::Protocol("OpenCode SSE line length overflow".to_string())
        })?;
        if total > limit {
            reader.consume(consumed);
            return Err(OpenCodeServerError::Protocol(format!(
                "OpenCode SSE line exceeds {limit} bytes"
            )));
        }
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
    Ok(Some(captured))
}

fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T, OpenCodeServerError> {
    decode_json_with_limit(response, MAX_JSON_BODY_BYTES)
}

fn decode_json_with_limit<T: DeserializeOwned>(
    response: Response,
    limit: usize,
) -> Result<T, OpenCodeServerError> {
    let mut response = ensure_success(response)?;
    let body = read_response_body(&mut response, limit, "JSON response")?;
    serde_json::from_slice::<T>(&body)
        .map_err(|error| OpenCodeServerError::Protocol(format!("invalid JSON response: {error}")))
}

fn decode_no_content(response: Response) -> Result<(), OpenCodeServerError> {
    let mut response = ensure_success(response)?;
    if response.status() != reqwest::StatusCode::NO_CONTENT {
        return Err(OpenCodeServerError::Protocol(format!(
            "expected OpenCode 204 No Content, received {}",
            response.status()
        )));
    }
    let body = read_response_body(&mut response, MAX_ERROR_BODY_BYTES, "no-content response")?;
    if !body.is_empty() {
        return Err(OpenCodeServerError::Protocol(
            "OpenCode no-content response contained a body".to_string(),
        ));
    }
    Ok(())
}

fn ensure_success(mut response: Response) -> Result<Response, OpenCodeServerError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = read_response_body(&mut response, MAX_ERROR_BODY_BYTES, "error response")?;
    let message = String::from_utf8_lossy(&body).trim().to_string();
    Err(OpenCodeServerError::Http {
        status: status.as_u16(),
        message: if message.is_empty() {
            status.canonical_reason().unwrap_or("request failed").to_string()
        } else {
            message
        },
    })
}

fn read_response_body(
    response: &mut Response,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, OpenCodeServerError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(OpenCodeServerError::Protocol(format!(
            "OpenCode {label} exceeds {limit} bytes"
        )));
    }
    read_bounded_body(response, limit, label)
}

fn read_bounded_body(
    reader: &mut impl Read,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, OpenCodeServerError> {
    let read_limit = u64::try_from(limit)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut body = Vec::with_capacity(limit.min(16 * 1024));
    reader
        .take(read_limit)
        .read_to_end(&mut body)
        .map_err(|error| OpenCodeServerError::Io(error.to_string()))?;
    if body.len() > limit {
        return Err(OpenCodeServerError::Protocol(format!(
            "OpenCode {label} exceeds {limit} bytes"
        )));
    }
    Ok(body)
}

fn transport_error(error: reqwest::Error) -> OpenCodeServerError {
    if error.is_timeout() {
        OpenCodeServerError::Timeout(error.to_string())
    } else {
        OpenCodeServerError::Io(error.to_string())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{
        decode_json_with_limit, parse_listening_address, read_bounded_body,
        read_bounded_line, read_sse, send_event, validate_server_version,
    };
    use crate::protocol::OpenCodeEvent;
    use serde_json::json;
    use std::io::{BufReader, Cursor};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn accepts_only_the_verified_server_version() {
        assert!(validate_server_version("1.18.25").is_ok());
        assert!(validate_server_version("1.18.26").is_err());
        assert!(validate_server_version("v1.18.25").is_err());
        assert!(validate_server_version("1.18.24").is_err());
        assert!(validate_server_version("development").is_err());
    }

    #[test]
    fn parses_official_sse_framing_and_heartbeats() {
        let input = b": heartbeat\n\nevent: message\ndata: {\"id\":\"evt_1\",\"type\":\"server.connected\",\"data\":{}}\n\n";
        let stopped = AtomicBool::new(false);
        let mut events = Vec::<OpenCodeEvent>::new();
        let error = read_sse(Cursor::new(input.to_vec()), &stopped, |event| {
            events.push(event);
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("stream ended"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "server.connected");
    }

    #[test]
    fn oversized_sse_line_fails_without_draining_the_unbounded_line() {
        let mut reader = BufReader::with_capacity(3, Cursor::new(b"123456789\nnext\n"));
        assert!(read_bounded_line(&mut reader, 5).is_err());
        assert_eq!(
            read_bounded_line(&mut reader, 5).unwrap(),
            Some(b"789\n".to_vec())
        );
    }

    #[test]
    fn bounded_body_rejects_content_beyond_the_limit() {
        let mut accepted = Cursor::new(b"12345".to_vec());
        assert_eq!(
            read_bounded_body(&mut accepted, 5, "fixture").unwrap(),
            b"12345"
        );
        let mut oversized = Cursor::new(b"123456".to_vec());
        assert!(read_bounded_body(&mut oversized, 5, "fixture").is_err());
    }

    #[test]
    fn accepts_only_owned_loopback_startup_addresses() {
        assert_eq!(
            parse_listening_address(
                "opencode server listening on http://127.0.0.1:4097/"
            )
            .unwrap()
            .unwrap()
            .as_str(),
            "http://127.0.0.1:4097/"
        );
        assert!(parse_listening_address("server listening on http://127.0.0.1:4098/")
            .unwrap()
            .is_some());
        assert!(parse_listening_address("server listening on http://0.0.0.0:4096/")
            .is_err());
        assert!(parse_listening_address("unrelated log line").unwrap().is_none());
    }

    #[test]
    fn chunked_json_response_is_bounded_after_decoding_transport_chunks() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n{\"hea\r\nA\r\nlthy\":true\r\n1\r\n}\r\n0\r\n\r\n",
                )
                .unwrap();
        });
        let response = reqwest::blocking::get(format!("http://127.0.0.1:{port}/api/health"))
            .unwrap();
        let error = decode_json_with_limit::<serde_json::Value>(response, 8).unwrap_err();
        assert!(error.to_string().contains("exceeds 8 bytes"));
        server.join().unwrap();
    }

    #[test]
    fn session_wait_does_not_inherit_the_normal_request_timeout() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read])
                .starts_with("POST /api/session/ses_wait/wait "));
            std::thread::sleep(std::time::Duration::from_millis(100));
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let client = super::OpenCodeClient::new_with_request_timeout(
            reqwest::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
            "codepet".to_string(),
            "wait-secret".to_string(),
            std::time::Duration::from_millis(20),
        )
        .unwrap();
        let started = std::time::Instant::now();
        client.wait_session("ses_wait").unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(80));
        server.join().unwrap();
    }

    #[test]
    fn event_queue_fails_closed_when_its_fixed_capacity_is_full() {
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        let event = OpenCodeEvent {
            id: "evt_queue".to_string(),
            kind: "server.connected".to_string(),
            data: json!({}),
            location: None,
        };
        send_event(&sender, event.clone()).unwrap();
        let error = send_event(&sender, event).unwrap_err();
        assert!(error.to_string().contains("fixed capacity"));
    }

    #[cfg(unix)]
    #[test]
    fn startup_health_probe_obeys_the_total_deadline() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
        });
        let client = super::OpenCodeClient::new(
            reqwest::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
            "codepet".to_string(),
            "deadline-secret".to_string(),
        )
        .unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 5"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let error = super::wait_for_ready(
            &client,
            &mut child,
            started + std::time::Duration::from_millis(120),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timeout"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        super::terminate_child(
            &mut child,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap();
        server.join().unwrap();
    }

}
