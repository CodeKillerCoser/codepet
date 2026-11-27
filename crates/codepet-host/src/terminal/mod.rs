//! Host-owned PTYs, independent from Providers and from individual connections.
//! Only short, bounded reads cross Gateway; blocking OS work stays off Tokio.
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use codepet_gateway_sdk as gateway;
use portable_pty::{native_pty_system, MasterPty, PtySize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod output;
mod platform;
use output::Output;

type Error = gateway::ProtocolError;
type Child = Arc<Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>>;
const MAX_SESSIONS: usize = 32;
const MAX_CLIENT_SESSIONS: usize = 8;
const FINISHED_TTL: Duration = Duration::from_secs(30 * 60);

pub(crate) fn error(code: &str, message: &str, retryable: bool) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
        retryable,
        details: None,
    }
}
fn io_error(_: impl std::fmt::Display) -> Error {
    error(
        "terminal_io_error",
        "Terminal operating system operation failed",
        false,
    )
}
fn size(columns: u64, rows: u64) -> Result<PtySize, Error> {
    if !(2..=500).contains(&columns) || !(1..=300).contains(&rows) {
        return Err(error(
            "terminal_invalid_size",
            "Invalid terminal dimensions",
            false,
        ));
    }
    Ok(PtySize {
        rows: rows as u16,
        cols: columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    })
}

struct Input {
    writer: Box<dyn Write + Send>,
    last_sequence: u64,
    last_data: Vec<u8>,
    failed: bool,
}

struct Session {
    owner: String,
    key: String,
    id: String,
    cwd: PathBuf,
    shell: PathBuf,
    master: Mutex<Box<dyn MasterPty + Send>>,
    input: Mutex<Input>,
    child: Child,
    output: Arc<Mutex<Output>>,
    touched: Mutex<Instant>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.terminate();
    }
}

impl Session {
    fn terminate(&self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut process) = child.take() {
                let _ = process.kill();
                let _ = process.wait();
            }
        }
    }
    fn open(owner: String, request: gateway::TerminalOpenRequest) -> Result<Self, Error> {
        let dimensions = size(request.columns, request.rows)?;
        let cwd = request
            .cwd
            .map(PathBuf::from)
            .or_else(platform::home)
            .ok_or_else(|| error("terminal_cwd_unavailable", "电脑用户目录不可用", false))?;
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(error(
                "terminal_invalid_cwd",
                "电脑上的终端工作目录不存在",
                false,
            ));
        }
        let cwd = cwd.canonicalize().map_err(io_error)?;
        let pair = native_pty_system().openpty(dimensions).map_err(io_error)?;
        let reader = pair.master.try_clone_reader().map_err(io_error)?;
        let writer = pair.master.take_writer().map_err(io_error)?;
        let mut command = platform::command();
        command.cwd(&cwd);
        let child = pair.slave.spawn_command(command).map_err(io_error)?;
        drop(pair.slave);
        let session = Self {
            owner,
            key: request.session_key,
            id: uuid::Uuid::new_v4().to_string(),
            cwd,
            shell: platform::shell(),
            master: Mutex::new(pair.master),
            input: Mutex::new(Input {
                writer,
                last_sequence: 0,
                last_data: Vec::new(),
                failed: false,
            }),
            child: Arc::new(Mutex::new(Some(child))),
            output: Arc::new(Mutex::new(Output::default())),
            touched: Mutex::new(Instant::now()),
        };
        Output::start_waiter(session.output.clone(), session.child.clone()).map_err(io_error)?;
        Output::start_reader(session.output.clone(), reader).map_err(io_error)?;
        Ok(session)
    }

    fn info(&self) -> Result<gateway::TerminalSession, Error> {
        let dimensions = self
            .master
            .lock()
            .map_err(io_error)?
            .get_size()
            .map_err(io_error)?;
        Ok(gateway::TerminalSession {
            terminal_id: self.id.clone(),
            cwd: self.cwd.to_string_lossy().into(),
            shell: self.shell.to_string_lossy().into(),
            exited: self.output.lock().map_err(io_error)?.finished(),
            columns: dimensions.cols as u64,
            rows: dimensions.rows as u64,
        })
    }

    fn write(&self, sequence: u64, data: &str) -> Result<(), Error> {
        let bytes = BASE64.decode(data).map_err(|_| {
            error(
                "terminal_invalid_input",
                "Invalid base64 terminal input",
                false,
            )
        })?;
        if bytes.is_empty() || bytes.len() > 16 * 1024 {
            return Err(error(
                "terminal_invalid_input",
                "Terminal input exceeds the allowed size",
                false,
            ));
        }
        let mut input = self.input.lock().map_err(io_error)?;
        if sequence == input.last_sequence && bytes == input.last_data {
            return Ok(());
        }
        if input.failed || sequence != input.last_sequence + 1 {
            return Err(error(
                "terminal_input_sequence",
                "终端输入状态不确定，请新建终端",
                false,
            ));
        }
        if input
            .writer
            .write_all(&bytes)
            .and_then(|_| input.writer.flush())
            .is_err()
        {
            input.failed = true;
            return Err(error(
                "terminal_write_failed",
                "终端输入未确认，请新建终端",
                false,
            ));
        }
        input.last_sequence = sequence;
        input.last_data = bytes;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct TerminalManager {
    sessions: Arc<Mutex<BTreeMap<String, Arc<Session>>>>,
}

impl TerminalManager {
    pub(crate) fn new() -> Self {
        let sessions: Arc<Mutex<BTreeMap<String, Arc<Session>>>> = Arc::default();
        let weak = Arc::downgrade(&sessions);
        // Weak ownership lets Host shutdown drop all PTYs. Reaping does not
        // depend on a new request and never touches Provider or transport state.
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(30));
            let Some(sessions) = weak.upgrade() else {
                break;
            };
            Self::reap_finished(&sessions);
        });
        Self { sessions }
    }

    fn reap_finished(sessions: &Mutex<BTreeMap<String, Arc<Session>>>) {
        if let Ok(mut sessions) = sessions.lock() {
            sessions.retain(|_, session| {
                // An offline phone must not kill a running shell/job.
                // Only completed PTYs expire; live sessions require explicit close.
                let finished = session.output.lock().map(|o| o.finished()).unwrap_or(false);
                let expired = session
                    .touched
                    .lock()
                    .map(|t| t.elapsed() >= FINISHED_TTL)
                    .unwrap_or(false);
                let keep = !finished || !expired;
                if !keep {
                    session.terminate();
                }
                keep
            });
        };
    }

    pub(crate) fn shutdown(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for session in sessions.values() {
                session.terminate();
            }
            sessions.clear();
        }
    }

    fn get(&self, owner: &str, id: &str) -> Result<Arc<Session>, Error> {
        let session = self
            .sessions
            .lock()
            .map_err(io_error)?
            .get(id)
            .filter(|s| s.owner == owner)
            .cloned()
            .ok_or_else(|| {
                error(
                    "terminal_not_found",
                    "终端会话已结束或失效，请新建终端",
                    false,
                )
            })?;
        *session.touched.lock().map_err(io_error)? = Instant::now();
        Ok(session)
    }

    pub(crate) fn open(
        &self,
        owner: &str,
        request: gateway::TerminalOpenRequest,
    ) -> Result<gateway::TerminalOpenResponse, Error> {
        if owner.is_empty() || request.session_key.is_empty() || request.session_key.len() > 256 {
            return Err(error(
                "terminal_invalid_key",
                "Invalid terminal session key",
                false,
            ));
        }
        size(request.columns, request.rows)?;
        let mut sessions = self.sessions.lock().map_err(io_error)?;
        if let Some(session) = sessions
            .values()
            .find(|s| s.owner == owner && s.key == request.session_key)
        {
            *session.touched.lock().map_err(io_error)? = Instant::now();
            return Ok(gateway::TerminalOpenResponse {
                session: session.info()?,
            });
        }
        if sessions.len() >= MAX_SESSIONS
            || sessions.values().filter(|s| s.owner == owner).count() >= MAX_CLIENT_SESSIONS
        {
            return Err(error(
                "terminal_capacity",
                "终端数量已达上限，请先结束不再使用的终端",
                false,
            ));
        }
        let session = Arc::new(Session::open(owner.into(), request)?);
        let info = session.info()?;
        sessions.insert(session.id.clone(), session);
        Ok(gateway::TerminalOpenResponse { session: info })
    }

    pub(crate) fn read(
        &self,
        owner: &str,
        request: gateway::TerminalReadRequest,
    ) -> Result<gateway::TerminalReadResponse, Error> {
        let session = self.get(owner, &request.terminal_id)?;
        let result = session
            .output
            .lock()
            .map_err(io_error)?
            .read(request.offset);
        result
    }
    pub(crate) fn write(
        &self,
        owner: &str,
        request: gateway::TerminalWriteRequest,
    ) -> Result<gateway::TerminalWriteResponse, Error> {
        self.get(owner, &request.terminal_id)?
            .write(request.sequence, &request.data)?;
        Ok(gateway::TerminalWriteResponse {})
    }
    pub(crate) fn resize(
        &self,
        owner: &str,
        request: gateway::TerminalResizeRequest,
    ) -> Result<gateway::TerminalResizeResponse, Error> {
        let session = self.get(owner, &request.terminal_id)?;
        session
            .master
            .lock()
            .map_err(io_error)?
            .resize(size(request.columns, request.rows)?)
            .map_err(io_error)?;
        Ok(gateway::TerminalResizeResponse {})
    }
    pub(crate) fn close(
        &self,
        owner: &str,
        request: gateway::TerminalCloseRequest,
    ) -> Result<gateway::TerminalCloseResponse, Error> {
        let mut sessions = self.sessions.lock().map_err(io_error)?;
        if sessions
            .get(&request.terminal_id)
            .is_some_and(|s| s.owner == owner)
        {
            if let Some(session) = sessions.remove(&request.terminal_id) {
                drop(sessions);
                session.terminate();
            }
        }
        Ok(gateway::TerminalCloseResponse {})
    }
}

#[cfg(test)]
mod tests;
