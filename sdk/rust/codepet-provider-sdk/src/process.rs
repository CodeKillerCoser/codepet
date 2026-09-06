//! Owned process trees. Callers configure a standard command; this boundary owns cleanup.
use std::{
    ffi::OsStr,
    io,
    ops::{Deref, DerefMut},
    path::Path,
    process::{ExitStatus, Output, Stdio},
};

#[cfg(unix)]
#[path = "process/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "process/windows.rs"]
mod platform;

pub struct Command(std::process::Command);
impl Command {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self(std::process::Command::new(program))
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.0.arg(arg);
        self
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.0.args(args);
        self
    }
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.0.env(key, value);
        self
    }
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.envs(vars);
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.0.env_remove(key);
        self
    }
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.0.current_dir(dir);
        self
    }
    pub fn stdin(&mut self, value: impl Into<Stdio>) -> &mut Self {
        self.0.stdin(value);
        self
    }
    pub fn stdout(&mut self, value: impl Into<Stdio>) -> &mut Self {
        self.0.stdout(value);
        self
    }
    pub fn stderr(&mut self, value: impl Into<Stdio>) -> &mut Self {
        self.0.stderr(value);
        self
    }
    pub fn into_std(self) -> std::process::Command {
        self.0
    }
    pub fn spawn(&mut self) -> io::Result<Child> {
        platform::prepare_owner()?;
        let placeholder = std::process::Command::new(self.0.get_program());
        let command = std::mem::replace(&mut self.0, placeholder);
        let mut wrapped = process_wrap::std::CommandWrap::from(command);
        platform::configure_std(&mut wrapped);
        let result = wrapped.spawn();
        // Preserve the builder for repeated spawns, including environment and stdio configuration.
        self.0 = wrapped.into_command();
        result.map(Child::new)
    }
    pub fn status(&mut self) -> io::Result<ExitStatus> {
        self.spawn()?.wait()
    }
    pub fn output(&mut self) -> io::Result<Output> {
        self.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.spawn()?.wait_with_output()
    }
}
impl Deref for Command {
    type Target = std::process::Command;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Command {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug)]
struct ProcessTree {
    inner: Box<dyn process_wrap::std::ChildWrapper>,
    status: Option<ExitStatus>,
}
impl ProcessTree {
    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            // SAFETY: retain the wrapper until the native root and its descendants are cleaned up.
            self.status = unsafe { self.inner.try_inner_child_mut() }
                .expect("native child")
                .try_wait()?;
            if self.status.is_some() {
                let _ = self.inner.start_kill();
            }
        }
        Ok(self.status)
    }
}
/// A cloneable cancellation handle; keeps process identity rather than looking up a reusable PID.
#[derive(Clone, Debug)]
pub struct ProcessControl(std::sync::Arc<std::sync::Mutex<ProcessTree>>);
impl ProcessControl {
    pub fn kill(&self) -> io::Result<()> {
        let mut tree = self.0.lock().unwrap();
        if tree.poll()?.is_some() {
            return Ok(());
        }
        tree.inner.start_kill()
    }
    pub fn terminate(&self) -> io::Result<()> {
        platform::terminate(self)
    }
    pub fn interrupt(&self) -> io::Result<()> {
        platform::interrupt(self)
    }
}
#[derive(Debug)]
pub struct Child {
    control: ProcessControl,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
    pub stderr: Option<std::process::ChildStderr>,
}
impl Child {
    fn new(mut inner: Box<dyn process_wrap::std::ChildWrapper>) -> Self {
        Self {
            stdin: inner.stdin().take(),
            stdout: inner.stdout().take(),
            stderr: inner.stderr().take(),
            control: ProcessControl(std::sync::Arc::new(std::sync::Mutex::new(ProcessTree {
                inner,
                status: None,
            }))),
        }
    }
    pub fn control(&self) -> ProcessControl {
        self.control.clone()
    }
    pub fn id(&self) -> u32 {
        self.control.0.lock().unwrap().inner.id()
    }
    pub fn kill(&mut self) -> io::Result<()> {
        self.control.kill()
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.control.0.lock().unwrap().poll()
    }
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.stdin.take();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            // Do not hold the process lock while waiting: another thread can terminate the tree.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        use std::io::Read;
        self.stdin.take();
        fn read(mut pipe: impl Read) -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            pipe.read_to_end(&mut out)?;
            Ok(out)
        }
        let out = self
            .stdout
            .take()
            .map(|p| std::thread::spawn(move || read(p)));
        let err = self
            .stderr
            .take()
            .map(|p| std::thread::spawn(move || read(p)));
        let status = self.wait()?;
        let collect =
            |t: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>| -> io::Result<Vec<u8>> {
                t.map(|t| {
                    t.join()
                        .map_err(|_| io::Error::other("process output reader panicked"))?
                })
                .unwrap_or_else(|| Ok(Vec::new()))
            };
        Ok(Output {
            status,
            stdout: collect(out)?,
            stderr: collect(err)?,
        })
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

pub struct AsyncChild {
    inner: Box<dyn process_wrap::tokio::ChildWrapper>,
    pub stdin: Option<tokio::process::ChildStdin>,
    pub stdout: Option<tokio::process::ChildStdout>,
    pub stderr: Option<tokio::process::ChildStderr>,
}
pub fn spawn_async(command: tokio::process::Command) -> io::Result<AsyncChild> {
    platform::prepare_owner()?;
    let mut wrapped = process_wrap::tokio::CommandWrap::from(command);
    wrapped.wrap(process_wrap::tokio::KillOnDrop);
    platform::configure_async(&mut wrapped);
    let mut inner = wrapped.spawn()?;
    Ok(AsyncChild {
        stdin: inner.stdin().take(),
        stdout: inner.stdout().take(),
        stderr: inner.stderr().take(),
        inner,
    })
}
impl AsyncChild {
    pub fn id(&self) -> Option<u32> {
        self.inner.id()
    }
    pub fn start_kill(&mut self) -> io::Result<()> {
        self.inner.start_kill()
    }
    pub async fn kill(&mut self) -> io::Result<()> {
        self.start_kill()?;
        self.wait().await.map(|_| ())
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        unsafe { self.inner.try_inner_child_mut() }
            .expect("native child")
            .try_wait()
    }
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = unsafe { self.inner.try_inner_child_mut() }
            .expect("native child")
            .wait()
            .await?;
        let _ = self.inner.start_kill();
        Ok(status)
    }
}
impl Drop for AsyncChild {
    fn drop(&mut self) {
        let _ = self.inner.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn fixture(command: &mut Command, role: &str, file: &Path) {
        command
            .args(["--ignored", "--exact", "process::tests::tree_fixture"])
            .env("CODEPET_PROCESS_FIXTURE", role)
            .env("CODEPET_PROCESS_PID_FILE", file);
    }
    fn wait_pid(file: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(text) = fs::read_to_string(file) {
                if let Ok(pid) = text.parse() {
                    return pid;
                }
            }
            assert!(Instant::now() < deadline, "fixture did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[cfg(windows)]
    fn alive(pid: u32) -> bool {
        use windows::Win32::{Foundation::CloseHandle, System::Threading::*};
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0;
            let ok = GetExitCodeProcess(handle, &mut code).is_ok();
            let _ = CloseHandle(handle);
            ok && code == 259
        }
    }
    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success())
    }
    fn assert_exited(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) {
            assert!(
                Instant::now() < deadline,
                "descendant {pid} survived its owner"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn root_exit_cleans_descendants_and_inherited_output_pipes() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        fixture(&mut command, "root", &file);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert_exited(wait_pid(&file));
    }
    #[test]
    fn cancellation_handle_can_stop_a_process_while_another_thread_waits() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        fixture(&mut command, "sleeper", &file);
        let mut child = command.spawn().unwrap();
        let control = child.control();
        let waiter = std::thread::spawn(move || child.wait());
        let pid = wait_pid(&file);
        control.kill().unwrap();
        assert!(!waiter.join().unwrap().unwrap().success());
        assert_exited(pid);
        control.kill().unwrap(); // An old handle must never signal a reused process ID.
    }

    #[cfg(windows)]
    #[test]
    fn owner_crash_kills_owned_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        fixture(&mut command, "owner", &file);
        // Deliberately bypass our boundary to simulate abrupt application termination.
        let mut owner = command.into_std().spawn().unwrap();
        let pid = wait_pid(&file);
        owner.kill().unwrap();
        owner.wait().unwrap();
        assert_exited(pid);
    }
    #[test]
    #[ignore = "child fixture, invoked by process lifecycle tests"]
    fn tree_fixture() {
        let role = std::env::var("CODEPET_PROCESS_FIXTURE").unwrap();
        let file = std::env::var_os("CODEPET_PROCESS_PID_FILE").unwrap();
        if role == "sleeper" {
            fs::write(&file, std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        fixture(&mut command, "sleeper", Path::new(&file));
        if role == "owner" {
            let _child = command.spawn().unwrap();
            std::thread::sleep(Duration::from_secs(60));
        } else {
            let _child = command.into_std().spawn().unwrap();
            wait_pid(Path::new(&file));
        }
    }
}
