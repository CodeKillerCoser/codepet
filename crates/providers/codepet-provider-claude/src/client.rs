use crate::protocol::{decode_claude_output, ClaudeOutput, ClaudeUserMessage};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

const MAX_CLAUDE_OUTPUT_LINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CLAUDE_STDERR_LINE_BYTES: usize = 64 * 1024;
const DISABLE_HOOKS_SETTINGS: &str = r#"{"disableAllHooks":true}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeCliError {
    Spawn(String),
    Io(String),
    Protocol(String),
    ProcessExited(Option<i32>),
    InterruptUnsupported,
}

impl fmt::Display for ClaudeCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "failed to start Claude CLI: {message}"),
            Self::Io(message) => write!(formatter, "Claude CLI I/O failed: {message}"),
            Self::Protocol(message) => write!(formatter, "invalid Claude CLI stream: {message}"),
            Self::ProcessExited(code) => write!(formatter, "Claude CLI exited before a result (code {code:?})"),
            Self::InterruptUnsupported => write!(formatter, "Claude CLI interrupt is unsupported on this platform"),
        }
    }
}

impl std::error::Error for ClaudeCliError {}

pub struct ClaudeTurnLaunch {
    pub executable: PathBuf,
    pub workspace_root: PathBuf,
    pub session_id: String,
    pub resume: bool,
    pub user_message_id: String,
    pub message: String,
    pub title: Option<String>,
    pub permission_mode: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

struct ProcessInner {
    child: Mutex<Child>,
}

impl Drop for ProcessInner {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[derive(Clone)]
pub struct ClaudeProcessControl {
    inner: Arc<ProcessInner>,
}

impl ClaudeProcessControl {
    #[cfg(unix)]
    pub fn interrupt(&self) -> Result<(), ClaudeCliError> {
        let process_id = lock(&self.inner.child).id();
        let result = unsafe { libc::kill(process_id as libc::pid_t, libc::SIGINT) };
        if result == 0 {
            Ok(())
        } else {
            Err(ClaudeCliError::Io(std::io::Error::last_os_error().to_string()))
        }
    }

    #[cfg(not(unix))]
    pub fn interrupt(&self) -> Result<(), ClaudeCliError> {
        Err(ClaudeCliError::InterruptUnsupported)
    }

    pub fn terminate(&self) -> Result<(), ClaudeCliError> {
        let mut child = lock(&self.inner.child);
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(error) => return Err(ClaudeCliError::Io(error.to_string())),
        }
        child
            .kill()
            .map_err(|error| ClaudeCliError::Io(error.to_string()))?;
        child
            .wait()
            .map_err(|error| ClaudeCliError::Io(error.to_string()))?;
        Ok(())
    }

    fn wait(&self) -> Result<ExitStatus, ClaudeCliError> {
        lock(&self.inner.child)
            .wait()
            .map_err(|error| ClaudeCliError::Io(error.to_string()))
    }
}

pub(crate) struct SpawnedClaudeTurn {
    control: ClaudeProcessControl,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
}

impl ClaudeTurnLaunch {
    pub(crate) fn spawn(self) -> Result<SpawnedClaudeTurn, ClaudeCliError> {
        let mut command = Command::new(&self.executable);
        command
            .arg("--print")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--settings")
            .arg(DISABLE_HOOKS_SETTINGS)
            .arg("--permission-mode")
            .arg(&self.permission_mode);
        if self.resume {
            command.arg("--resume").arg(&self.session_id);
        } else {
            command.arg("--session-id").arg(&self.session_id);
            if let Some(title) = self.title.as_ref() {
                command.arg("--name").arg(title);
            }
        }
        if let Some(model) = self.model.as_ref() {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = self.effort.as_ref() {
            command.arg("--effort").arg(effort);
        }
        let mut child = command
            .current_dir(&self.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ClaudeCliError::Spawn(error.to_string()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClaudeCliError::Spawn("stdin is unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClaudeCliError::Spawn("stdout is unavailable".to_string()))?;
        let stderr = child.stderr.take();
        let input = ClaudeUserMessage::new(&self.user_message_id, &self.message);
        if let Err(error) = serde_json::to_writer(&mut stdin, &input)
            .map_err(|error| ClaudeCliError::Protocol(error.to_string()))
            .and_then(|_| stdin.write_all(b"\n").map_err(|error| ClaudeCliError::Io(error.to_string())))
            .and_then(|_| stdin.flush().map_err(|error| ClaudeCliError::Io(error.to_string())))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        drop(stdin);
        Ok(SpawnedClaudeTurn {
            control: ClaudeProcessControl {
                inner: Arc::new(ProcessInner {
                    child: Mutex::new(child),
                }),
            },
            stdout: Some(stdout),
            stderr,
        })
    }
}

impl SpawnedClaudeTurn {
    pub(crate) fn control(&self) -> ClaudeProcessControl {
        self.control.clone()
    }

    pub(crate) fn start<F, G>(mut self, on_output: F, on_exit: G)
    where
        F: Fn(ClaudeOutput) -> Result<(), ClaudeCliError> + Send + 'static,
        G: Fn(Result<ExitStatus, ClaudeCliError>) + Send + 'static,
    {
        if let Some(stderr) = self.stderr.take() {
            thread::spawn(move || drain_stderr(stderr));
        }
        let Some(stdout) = self.stdout.take() else {
            on_exit(Err(ClaudeCliError::Spawn("stdout is unavailable".to_string())));
            return;
        };
        let control = self.control.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let line = match read_bounded_line(&mut reader, MAX_CLAUDE_OUTPUT_LINE_BYTES) {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(error) => {
                        let _ = control.terminate();
                        on_exit(Err(error));
                        return;
                    }
                };
                let output = match decode_claude_output(&line) {
                    Ok(output) => output,
                    Err(error) => {
                        let _ = control.terminate();
                        on_exit(Err(ClaudeCliError::Protocol(format!("invalid JSON: {error}"))));
                        return;
                    }
                };
                if let Err(error) = on_output(output) {
                    let _ = control.terminate();
                    on_exit(Err(error));
                    return;
                }
            }
            on_exit(control.wait());
        });
    }
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    limit: usize,
) -> Result<Option<Vec<u8>>, ClaudeCliError> {
    let mut captured = Vec::new();
    let mut total = 0usize;
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| ClaudeCliError::Io(error.to_string()))?;
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
        total = total
            .checked_add(consumed)
            .ok_or_else(|| ClaudeCliError::Protocol("line length overflow".to_string()))?;
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
        return Err(ClaudeCliError::Protocol(format!(
            "Claude output line exceeds {limit} bytes"
        )));
    }
    Ok(Some(captured))
}

fn drain_stderr(stderr: impl Read) {
    let mut reader = BufReader::new(stderr);
    loop {
        match read_bounded_line(&mut reader, MAX_CLAUDE_STDERR_LINE_BYTES) {
            Ok(Some(line)) => {
                let line = String::from_utf8_lossy(&line);
                eprintln!("Claude CLI: {}", line.trim_end());
            }
            Ok(None) => return,
            Err(error) => {
                eprintln!("Claude CLI stderr stopped: {error}");
                return;
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
