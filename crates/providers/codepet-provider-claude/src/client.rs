use crate::protocol::{decode_claude_output, ClaudeOutput, ClaudeUserMessage};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

pub(crate) const MAX_CLAUDE_OUTPUT_LINE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLAUDE_STDERR_LINE_BYTES: usize = 64 * 1024;
const INTERRUPT_GRACE: Duration = Duration::from_millis(750);
const TERMINATE_GRACE: Duration = Duration::from_millis(500);
const KILL_WAIT: Duration = Duration::from_secs(2);
const STDOUT_DRAIN_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeCliError {
    Spawn(String),
    Io(String),
    Protocol(String),
    ProcessExited(Option<i32>),
    ProcessDidNotExit,
    InterruptUnsupported,
}

impl fmt::Display for ClaudeCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "failed to start Claude CLI: {message}"),
            Self::Io(message) => write!(formatter, "Claude CLI I/O failed: {message}"),
            Self::Protocol(message) => write!(formatter, "invalid Claude CLI stream: {message}"),
            Self::ProcessExited(code) => {
                write!(formatter, "Claude CLI exited before a result (code {code:?})")
            }
            Self::ProcessDidNotExit => {
                write!(formatter, "Claude CLI process group did not exit after forced termination")
            }
            Self::InterruptUnsupported => {
                write!(formatter, "Claude CLI interrupt is unsupported on this platform")
            }
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
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Default)]
struct ProcessExitState {
    exited: Mutex<bool>,
    changed: Condvar,
}

impl ProcessExitState {
    fn mark_exited(&self) {
        *lock(&self.exited) = true;
        self.changed.notify_all();
    }

    fn is_exited(&self) -> bool {
        *lock(&self.exited)
    }

    fn wait(&self, timeout: Duration) -> bool {
        let exited = lock(&self.exited);
        if *exited {
            return true;
        }
        let (exited, _) = self
            .changed
            .wait_timeout_while(exited, timeout, |exited| !*exited)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *exited
    }
}

#[derive(Clone)]
pub struct ClaudeProcessControl {
    process_id: u32,
    exit: Arc<ProcessExitState>,
}

impl ClaudeProcessControl {
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn is_exited(&self) -> bool {
        self.exit.is_exited()
    }

    pub fn wait_for_exit(&self, timeout: Duration) -> bool {
        self.exit.wait(timeout)
    }

    #[cfg(unix)]
    pub fn interrupt(&self) -> Result<(), ClaudeCliError> {
        signal_process_group(self.process_id, libc::SIGINT)?;
        if self.wait_for_exit(INTERRUPT_GRACE) {
            return Ok(());
        }
        self.force_kill()?;
        if self.wait_for_exit(KILL_WAIT) {
            Ok(())
        } else {
            Err(ClaudeCliError::ProcessDidNotExit)
        }
    }

    #[cfg(not(unix))]
    pub fn interrupt(&self) -> Result<(), ClaudeCliError> {
        Err(ClaudeCliError::InterruptUnsupported)
    }

    pub fn terminate(&self) -> Result<(), ClaudeCliError> {
        if !self.is_exited() {
            self.request_terminate()?;
        }
        if self.wait_for_exit(TERMINATE_GRACE) {
            return Ok(());
        }
        self.force_kill()?;
        if self.wait_for_exit(KILL_WAIT) {
            Ok(())
        } else {
            Err(ClaudeCliError::ProcessDidNotExit)
        }
    }

    #[cfg(unix)]
    fn request_terminate(&self) -> Result<(), ClaudeCliError> {
        signal_process_group(self.process_id, libc::SIGTERM)
    }

    #[cfg(windows)]
    fn request_terminate(&self) -> Result<(), ClaudeCliError> {
        self.force_kill()
    }

    #[cfg(unix)]
    fn force_kill(&self) -> Result<(), ClaudeCliError> {
        signal_process_group(self.process_id, libc::SIGKILL)
    }

    #[cfg(windows)]
    fn force_kill(&self) -> Result<(), ClaudeCliError> {
        let status = Command::new("taskkill")
            .args(["/PID", &self.process_id.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| ClaudeCliError::Io(error.to_string()))?;
        if status.success() || self.is_exited() {
            Ok(())
        } else {
            Err(ClaudeCliError::Io(format!(
                "taskkill failed for Claude process {}: {status}",
                self.process_id
            )))
        }
    }
}

pub(crate) struct SpawnedClaudeTurn {
    child: Child,
    control: ClaudeProcessControl,
    stdout: ChildStdout,
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
            .arg("--include-partial-messages");
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
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .current_dir(&self.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ClaudeCliError::Spawn(error.to_string()))?;
        let process_id = child.id();
        let mut stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => return abort_spawn(child, process_id, "stdin is unavailable"),
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return abort_spawn(child, process_id, "stdout is unavailable"),
        };
        let stderr = child.stderr.take();
        let input = ClaudeUserMessage::new(&self.user_message_id, &self.message);
        if let Err(error) = serde_json::to_writer(&mut stdin, &input)
            .map_err(|error| ClaudeCliError::Protocol(error.to_string()))
            .and_then(|_| {
                stdin
                    .write_all(b"\n")
                    .map_err(|error| ClaudeCliError::Io(error.to_string()))
            })
            .and_then(|_| {
                stdin
                    .flush()
                    .map_err(|error| ClaudeCliError::Io(error.to_string()))
            })
        {
            terminate_unstarted_child(&mut child, process_id);
            return Err(error);
        }
        drop(stdin);
        let exit = Arc::new(ProcessExitState::default());
        Ok(SpawnedClaudeTurn {
            child,
            control: ClaudeProcessControl { process_id, exit },
            stdout,
            stderr,
        })
    }
}

impl SpawnedClaudeTurn {
    pub(crate) fn control(&self) -> ClaudeProcessControl {
        self.control.clone()
    }

    pub(crate) fn start<F, G, H>(self, on_output: F, on_stream_error: G, on_exit: H)
    where
        F: Fn(ClaudeOutput) -> Result<(), ClaudeCliError> + Send + 'static,
        G: Fn(ClaudeCliError) + Send + Sync + 'static,
        H: Fn(Result<ExitStatus, ClaudeCliError>) + Send + 'static,
    {
        let SpawnedClaudeTurn {
            mut child,
            control,
            stdout,
            stderr,
        } = self;
        if let Some(stderr) = stderr {
            thread::spawn(move || drain_stderr(stderr));
        }

        let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
        let reaper_exit = control.exit.clone();
        let process_id = control.process_id;
        let stream_error: Arc<dyn Fn(ClaudeCliError) + Send + Sync> = Arc::new(on_stream_error);
        let reader_error = stream_error.clone();
        let reader_control = control;
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut saw_terminal_frame = false;
            loop {
                let line = match read_bounded_line(&mut reader, MAX_CLAUDE_OUTPUT_LINE_BYTES) {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        if !saw_terminal_frame {
                            reader_error(ClaudeCliError::Protocol(
                                "Claude stdout closed before a terminal frame".to_string(),
                            ));
                            let _ = reader_control.force_kill();
                        }
                        break;
                    }
                    Err(error) => {
                        reader_error(error);
                        let _ = reader_control.force_kill();
                        break;
                    }
                };
                let output = match decode_claude_output(&line) {
                    Ok(output) => output,
                    Err(error) => {
                        reader_error(ClaudeCliError::Protocol(format!(
                            "invalid JSON: {error}"
                        )));
                        let _ = reader_control.force_kill();
                        break;
                    }
                };
                let terminal = matches!(
                    &output,
                    ClaudeOutput::Result { .. }
                        | ClaudeOutput::Assistant {
                            aborted: Some(true),
                            ..
                        }
                );
                if let Err(error) = on_output(output) {
                    reader_error(error);
                    let _ = reader_control.force_kill();
                    break;
                }
                saw_terminal_frame |= terminal;
            }
            let _ = stdout_sender.send(());
        });

        let reaper_error = stream_error;
        thread::spawn(move || {
            let outcome = child
                .wait()
                .map_err(|error| ClaudeCliError::Io(error.to_string()));
            cleanup_process_group(process_id);
            reaper_exit.mark_exited();
            if stdout_receiver.recv_timeout(STDOUT_DRAIN_WAIT).is_err() {
                reaper_error(ClaudeCliError::Protocol(
                    "Claude stdout did not close after process exit".to_string(),
                ));
            }
            on_exit(outcome);
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
        if total > limit {
            return Err(ClaudeCliError::Protocol(format!(
                "Claude output line exceeds {limit} bytes"
            )));
        }
        captured.extend_from_slice(&available[..consumed]);
        let ended = available[consumed - 1] == b'\n';
        reader.consume(consumed);
        if ended {
            break;
        }
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

fn abort_spawn<T>(
    mut child: Child,
    process_id: u32,
    message: &str,
) -> Result<T, ClaudeCliError> {
    terminate_unstarted_child(&mut child, process_id);
    Err(ClaudeCliError::Spawn(message.to_string()))
}

fn terminate_unstarted_child(child: &mut Child, process_id: u32) {
    #[cfg(unix)]
    let _ = signal_process_group(process_id, libc::SIGKILL);
    #[cfg(windows)]
    let _ = child.kill();
    let _ = child.wait();
    cleanup_process_group(process_id);
}

#[cfg(unix)]
fn signal_process_group(process_id: u32, signal: libc::c_int) -> Result<(), ClaudeCliError> {
    let process_group = -(process_id as libc::pid_t);
    let result = unsafe { libc::kill(process_group, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(ClaudeCliError::Io(error.to_string()))
    }
}

#[cfg(unix)]
fn cleanup_process_group(process_id: u32) {
    let _ = signal_process_group(process_id, libc::SIGKILL);
}

#[cfg(windows)]
fn cleanup_process_group(_process_id: u32) {}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
