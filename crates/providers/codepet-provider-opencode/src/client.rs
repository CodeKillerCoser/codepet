use crate::protocol::{
    OpenCodeActiveSessions, OpenCodeDataResponse, OpenCodeEvent, OpenCodeHealth,
    OpenCodePermissionReply, OpenCodePermissionReplyRequest, OpenCodePromptAdmission,
    OpenCodePromptRequest, OpenCodeServerError, OpenCodeSession, OpenCodeSessionCreate,
    OpenCodeSessionPage, OPENCODE_MINIMUM_SERVER_VERSION,
};
use reqwest::blocking::{Client, Response};
use reqwest::Url;
use semver::Version;
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const STARTUP_RETRY_DELAY: Duration = Duration::from_millis(50);
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;
static NEXT_SESSION_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct OpenCodeClient {
    base_url: Url,
    requests: Client,
    events: Client,
    auth: Option<(String, String)>,
}

impl OpenCodeClient {
    fn new(base_url: Url) -> Result<Self, OpenCodeServerError> {
        let requests = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
        let events = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
        let auth = std::env::var("OPENCODE_SERVER_PASSWORD")
            .ok()
            .map(|password| {
                let username = std::env::var("OPENCODE_SERVER_USERNAME")
                    .unwrap_or_else(|_| "opencode".to_string());
                (username, password)
            });
        Ok(Self {
            base_url,
            requests,
            events,
            auth,
        })
    }

    pub fn health(&self) -> Result<OpenCodeHealth, OpenCodeServerError> {
        let response = self.request(self.requests.get(self.url(&["global", "health"])?))
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

    fn dispose(&self) -> Result<(), OpenCodeServerError> {
        decode_no_content(
            self.request(self.requests.post(self.url(&["global", "dispose"])?))
                .send()
                .map_err(transport_error)?,
        )
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
}

#[derive(Clone)]
pub struct OpenCodeServerSession {
    inner: Arc<SessionInner>,
}

impl OpenCodeServerSession {
    pub fn spawn(executable: &Path, args: &[String]) -> Result<Self, OpenCodeServerError> {
        let port = reserve_loopback_port()?;
        let base_url = Url::parse(&format!("http://127.0.0.1:{port}/"))
            .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
        let client = OpenCodeClient::new(base_url)?;
        let mut command = Command::new(executable);
        command
            .args(args)
            .arg("--hostname")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = command
            .spawn()
            .map_err(|error| OpenCodeServerError::Spawn(error.to_string()))?;
        if let Err(error) = wait_for_ready(&client, &mut child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let generation = NEXT_SESSION_GENERATION
            .fetch_add(1, Ordering::SeqCst)
            .to_string();
        Ok(Self {
            inner: Arc::new(SessionInner {
                generation,
                client,
                child: Mutex::new(Some(child)),
                stopped: AtomicBool::new(false),
                subscriber: Mutex::new(None),
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
        let (sender, receiver) = mpsc::channel();
        let inner = self.inner.clone();
        *subscriber = Some(thread::spawn(move || {
            if inner.stopped.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = inner.client.event_response().and_then(|response| {
                read_sse(response, &inner.stopped, |event| {
                    sender.send(Ok(event)).map_err(|_| OpenCodeServerError::Shutdown)
                })
            }) {
                if !inner.stopped.load(Ordering::SeqCst) {
                    let _ = sender.send(Err(error));
                }
            }
        }));
        Ok(receiver)
    }

    pub fn shutdown(&self) -> Result<(), OpenCodeServerError> {
        if self.inner.stopped.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.inner.client.dispose();
        let child_result = {
            let mut child = lock(&self.inner.child);
            if let Some(mut child) = child.take() {
                match child.try_wait() {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => child
                        .kill()
                        .and_then(|_| child.wait().map(|_| ()))
                        .map_err(|error| OpenCodeServerError::Io(error.to_string())),
                    Err(error) => Err(OpenCodeServerError::Io(error.to_string())),
                }
            } else {
                Ok(())
            }
        };
        if let Some(subscriber) = lock(&self.inner.subscriber).take() {
            let _ = subscriber.join();
        }
        child_result
    }
}

fn wait_for_ready(client: &OpenCodeClient, child: &mut Child) -> Result<(), OpenCodeServerError> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| OpenCodeServerError::Io(error.to_string()))?
        {
            return Err(OpenCodeServerError::ProcessExited(status.to_string()));
        }
        match client.health() {
            Ok(health) => {
                if !health.healthy {
                    return Err(OpenCodeServerError::Protocol(
                        "OpenCode health response reported healthy=false".to_string(),
                    ));
                }
                validate_server_version(&health.version)?;
                return Ok(());
            }
            Err(_) if Instant::now() < deadline => thread::sleep(STARTUP_RETRY_DELAY),
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
    let actual = Version::parse(version.trim_start_matches('v')).map_err(|error| {
        OpenCodeServerError::Protocol(format!("invalid OpenCode Server version {version:?}: {error}"))
    })?;
    let minimum = Version::parse(OPENCODE_MINIMUM_SERVER_VERSION)
        .map_err(|error| OpenCodeServerError::Protocol(error.to_string()))?;
    if actual < minimum {
        return Err(OpenCodeServerError::Protocol(format!(
            "OpenCode Server {actual} is unsupported; {minimum} or newer is required"
        )));
    }
    Ok(())
}

fn reserve_loopback_port() -> Result<u16, OpenCodeServerError> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr().map(|address| address.port()))
        .map_err(|error| OpenCodeServerError::Io(error.to_string()))
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
        return Err(OpenCodeServerError::Protocol(format!(
            "OpenCode SSE line exceeds {limit} bytes"
        )));
    }
    Ok(Some(captured))
}

fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T, OpenCodeServerError> {
    ensure_success(response)?
        .json::<T>()
        .map_err(|error| OpenCodeServerError::Protocol(format!("invalid JSON response: {error}")))
}

fn decode_no_content(response: Response) -> Result<(), OpenCodeServerError> {
    ensure_success(response).map(|_| ())
}

fn ensure_success(mut response: Response) -> Result<Response, OpenCodeServerError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(MAX_ERROR_BODY_BYTES as u64)
        .read_to_end(&mut body)
        .map_err(|error| OpenCodeServerError::Io(error.to_string()))?;
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
    use super::{read_bounded_line, read_sse, validate_server_version};
    use crate::protocol::OpenCodeEvent;
    use std::io::{BufReader, Cursor};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn validates_minimum_server_version() {
        assert!(validate_server_version("1.18.25").is_ok());
        assert!(validate_server_version("v1.19.0").is_ok());
        assert!(validate_server_version("1.18.24").is_err());
        assert!(validate_server_version("development").is_err());
    }

    #[test]
    fn parses_official_sse_framing_and_heartbeats() {
        let input = b": heartbeat\n\nevent: message\ndata: {\"id\":\"evt_1\",\"type\":\"session.idle\",\"data\":{\"sessionID\":\"ses_1\"}}\n\n";
        let stopped = AtomicBool::new(false);
        let mut events = Vec::<OpenCodeEvent>::new();
        let error = read_sse(Cursor::new(input.to_vec()), &stopped, |event| {
            events.push(event);
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("stream ended"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "session.idle");
    }

    #[test]
    fn oversized_sse_line_is_drained_before_failure() {
        let mut reader = BufReader::new(Cursor::new(b"12345\nnext\n"));
        assert!(read_bounded_line(&mut reader, 5).is_err());
        assert_eq!(
            read_bounded_line(&mut reader, 5).unwrap(),
            Some(b"next\n".to_vec())
        );
    }
}
