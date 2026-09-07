//! Local filesystem and executable discovery. Product layouts remain in each provider.
use crate::{RuntimeCandidate, RuntimeCandidateSource};
use std::path::{Path, PathBuf};

/// Background runtime processes retain their stdio pipes but never create a console window.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> crate::process::Command {
    crate::process::Command::new(program)
}

/// Discover off the executor, then probe distinct executables concurrently.
pub async fn runtime_inventory<D, P>(
    name: &'static str,
    npm_package: &'static str,
    selected: Option<crate::RuntimeInstallation>,
    discover: D,
    probe: P,
) -> Result<crate::RuntimeGetInstalledResponse, crate::ProtocolError>
where
    D: FnOnce() -> Vec<RuntimeCandidate> + Send + 'static,
    P: Fn(
            RuntimeCandidate,
            std::time::Duration,
        ) -> Result<crate::RuntimeInstallation, crate::ProtocolError>
        + Send
        + Sync
        + 'static,
{
    runtime_inventory_controlled(
        name,
        npm_package,
        selected,
        discover,
        move |candidate, timeout, _| probe(candidate, timeout),
        RuntimeProbeControl::default(),
    )
    .await
}
async fn runtime_inventory_controlled<D, P>(
    name: &'static str,
    npm_package: &'static str,
    selected: Option<crate::RuntimeInstallation>,
    discover: D,
    probe: P,
    control: RuntimeProbeControl,
) -> Result<crate::RuntimeGetInstalledResponse, crate::ProtocolError>
where
    D: FnOnce() -> Vec<RuntimeCandidate> + Send + 'static,
    P: Fn(
            RuntimeCandidate,
            std::time::Duration,
            RuntimeProbeControl,
        ) -> Result<crate::RuntimeInstallation, crate::ProtocolError>
        + Send
        + Sync
        + 'static,
{
    use futures::{stream, StreamExt};
    let selected_candidate = selected.clone();
    let candidates = tokio::task::spawn_blocking(move || {
        let mut candidates = discover();
        if let Some(path) = std::env::var_os("CODEPET_RUNTIME_EXECUTABLE") {
            candidates.insert(
                0,
                candidate(PathBuf::from(path), RuntimeCandidateSource::Configured),
            );
        }
        if let Some(selected) = selected_candidate {
            candidates.insert(
                0,
                RuntimeCandidate {
                    executable_path: selected.executable_path,
                    source: selected.source,
                },
            );
        }
        let mut seen = std::collections::HashSet::new();
        candidates
            .into_iter()
            .filter_map(|mut candidate| {
                let path =
                    resolve_executable(Path::new(&candidate.executable_path), name, npm_package)
                        .ok()?;
                candidate.executable_path = path.to_string_lossy().into_owned();
                seen.insert(executable_key(&candidate.executable_path))
                    .then_some(candidate)
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| scan_error(e.to_string()))?;
    let probe = std::sync::Arc::new(probe);
    let mut results = stream::iter(
        candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| {
                let probe = probe.clone();
                let control = control.clone();
                async move {
                    let result = tokio::task::spawn_blocking(move || {
                        probe(candidate, std::time::Duration::from_secs(120), control)
                    })
                    .await;
                    (index, result)
                }
            }),
    )
    .buffer_unordered(4);
    let mut installed = Vec::new();
    let mut errors = Vec::new();
    while let Some((index, result)) = results.next().await {
        match result {
            Ok(Ok(runtime)) => installed.push((index, runtime)),
            Ok(Err(e)) => errors.push(e.message),
            Err(e) => errors.push(e.to_string()),
        }
    }
    installed.sort_by_key(|(index, _)| *index);
    let installed: Vec<_> = installed.into_iter().map(|(_, runtime)| runtime).collect();
    let selected = selected.and_then(|selected| {
        installed
            .iter()
            .find(|runtime| {
                executable_key(&runtime.executable_path)
                    == executable_key(&selected.executable_path)
            })
            .cloned()
    });
    Ok(crate::RuntimeGetInstalledResponse {
        installed,
        selected,
        scanning: Some(false),
        scan_error: (!errors.is_empty()).then(|| errors.join("; ")),
    })
}
fn scan_error(message: String) -> crate::ProtocolError {
    crate::ProtocolError {
        code: "runtime_scan_failed".into(),
        message,
        retryable: true,
        details: None,
    }
}

/// Owns all subprocesses launched by one blocking runtime scan.
#[derive(Clone, Default)]
pub struct RuntimeProbeControl(std::sync::Arc<std::sync::Mutex<ProbeProcesses>>);
#[derive(Default)]
struct ProbeProcesses {
    cancelled: bool,
    children: Vec<crate::process::ProcessControl>,
}
impl RuntimeProbeControl {
    pub fn track(&self, child: crate::process::ProcessControl) -> std::io::Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.cancelled {
            drop(state);
            let _ = child.kill();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "runtime scan cancelled",
            ));
        }
        state.children.push(child);
        Ok(())
    }
    fn cancel(&self) {
        let children = {
            let mut state = self.0.lock().unwrap();
            state.cancelled = true;
            std::mem::take(&mut state.children)
        };
        for child in children {
            let _ = child.kill();
        }
    }
}

type RuntimeProbe = dyn Fn(
        RuntimeCandidate,
        std::time::Duration,
        RuntimeProbeControl,
    ) -> Result<crate::RuntimeInstallation, crate::ProtocolError>
    + Send
    + Sync;

/// The RPC reads a snapshot. Only initialization starts the owned background scan.
pub struct RuntimeScanner {
    control: std::sync::Mutex<RuntimeProbeControl>,
    snapshot: std::sync::Arc<std::sync::Mutex<crate::RuntimeGetInstalledResponse>>,
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    events: std::sync::Arc<dyn crate::ProviderEventSink>,
    probe: std::sync::Mutex<Option<std::sync::Arc<RuntimeProbe>>>,
}
impl RuntimeScanner {
    pub fn new(events: std::sync::Arc<dyn crate::ProviderEventSink>) -> Self {
        Self {
            control: std::sync::Mutex::new(RuntimeProbeControl::default()),
            snapshot: std::sync::Arc::new(std::sync::Mutex::new(
                crate::RuntimeGetInstalledResponse {
                    installed: vec![],
                    selected: None,
                    scanning: Some(true),
                    scan_error: None,
                },
            )),
            task: std::sync::Mutex::new(None),
            probe: std::sync::Mutex::new(None),
            events,
        }
    }
    pub fn snapshot(&self) -> crate::RuntimeGetInstalledResponse {
        self.snapshot.lock().unwrap().clone()
    }
    pub fn start<D, P>(&self, name: &'static str, package: &'static str, discover: D, probe: P)
    where
        D: FnOnce() -> Vec<RuntimeCandidate> + Send + 'static,
        P: Fn(
                RuntimeCandidate,
                std::time::Duration,
            ) -> Result<crate::RuntimeInstallation, crate::ProtocolError>
            + Send
            + Sync
            + 'static,
    {
        self.start_cancellable(name, package, discover, move |candidate, timeout, _| {
            probe(candidate, timeout)
        });
    }

    pub fn start_cancellable<D, P>(
        &self,
        name: &'static str,
        package: &'static str,
        discover: D,
        probe: P,
    ) where
        D: FnOnce() -> Vec<RuntimeCandidate> + Send + 'static,
        P: Fn(
                RuntimeCandidate,
                std::time::Duration,
                RuntimeProbeControl,
            ) -> Result<crate::RuntimeInstallation, crate::ProtocolError>
            + Send
            + Sync
            + 'static,
    {
        let mut task = self.task.lock().unwrap();
        if task.is_some() {
            return;
        }
        let control = RuntimeProbeControl::default();
        *self.control.lock().unwrap() = control.clone();
        self.snapshot.lock().unwrap().scanning = Some(true);
        let probe = std::sync::Arc::new(probe);
        *self.probe.lock().unwrap() = Some(probe.clone());
        let snapshot = self.snapshot.clone();
        let events = self.events.clone();
        let selected = self.snapshot().selected;
        *task = Some(tokio::spawn(async move {
            let result = runtime_inventory_controlled(
                name,
                package,
                selected,
                discover,
                move |candidate, timeout, control| probe(candidate, timeout, control),
                control,
            )
            .await;
            let params = {
                let mut current = snapshot.lock().unwrap();
                match result {
                    Ok(mut result) => {
                        result.selected = current.selected.as_ref().and_then(|selected| {
                            result
                                .installed
                                .iter()
                                .find(|runtime| {
                                    executable_key(&runtime.executable_path)
                                        == executable_key(&selected.executable_path)
                                })
                                .cloned()
                        });
                        *current = result;
                    }
                    Err(e) => {
                        current.scanning = Some(false);
                        current.scan_error = Some(e.message);
                    }
                }
                current.clone()
            };
            let _ = events.publish(crate::ProtocolEvent::RuntimeInventoryChanged {
                jsonrpc: "2.0".into(),
                params,
            });
        }));
    }
    pub fn select(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<crate::RuntimeInstallation, crate::ProtocolError> {
        let path = std::fs::canonicalize(&candidate.executable_path)
            .map_err(|e| scan_error(e.to_string()))?;
        let mut release = None;
        let mut selection_task = None;
        let (selected, params) = {
            let mut current = self.snapshot.lock().unwrap();
            let cached = current
                .installed
                .iter()
                .find(|r| {
                    executable_key(&r.executable_path) == executable_key(&path.to_string_lossy())
                })
                .cloned();
            let selected = if let Some(selected) = cached {
                selected
            } else {
                if current.scanning == Some(true) {
                    return Err(crate::ProtocolError {
                        code: "runtime_scanning".into(),
                        message: "Wait for the current scan before selecting a new executable"
                            .into(),
                        retryable: true,
                        details: None,
                    });
                }
                let probe = self
                    .probe
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| scan_error("Provider is not initialized".into()))?;
                // Selection records configuration immediately; an empty version is pending detection.
                let selected = crate::RuntimeInstallation {
                    executable_path: path.to_string_lossy().into_owned(),
                    source: candidate.source,
                    version: String::new(),
                };
                let candidate = RuntimeCandidate {
                    executable_path: selected.executable_path.clone(),
                    source: selected.source,
                };
                let selected_path = selected.executable_path.clone();
                let snapshot = self.snapshot.clone();
                let events = self.events.clone();
                current.scanning = Some(true);
                current.scan_error = None;
                let (ready, started) = tokio::sync::oneshot::channel::<()>();
                release = Some(ready);
                let control = RuntimeProbeControl::default();
                *self.control.lock().unwrap() = control.clone();
                selection_task = Some(tokio::spawn(async move {
                    if started.await.is_err() {
                        return;
                    }
                    let result = tokio::task::spawn_blocking(move || {
                        probe(candidate, std::time::Duration::from_secs(120), control)
                    })
                    .await;
                    let params =
                        {
                            let mut current = snapshot.lock().unwrap();
                            current.scanning = Some(false);
                            match result {
                                Ok(Ok(runtime)) => {
                                    if current.selected.as_ref().is_some_and(|r| {
                                        r.executable_path == runtime.executable_path
                                    }) {
                                        current.selected = Some(runtime.clone());
                                    }
                                    current.installed.push(runtime);
                                }
                                result => {
                                    if current.selected.as_ref().is_some_and(|runtime| {
                                        runtime.executable_path == selected_path
                                    }) {
                                        current.selected = None;
                                    }
                                    current.scan_error = Some(match result {
                                        Ok(Err(error)) => error.message,
                                        Err(error) => error.to_string(),
                                        _ => unreachable!(),
                                    });
                                }
                            }
                            current.clone()
                        };
                    let _ = events.publish(crate::ProtocolEvent::RuntimeInventoryChanged {
                        jsonrpc: "2.0".into(),
                        params,
                    });
                }));
                selected
            };
            current.selected = Some(selected.clone());
            (selected, current.clone())
        };
        if let Some(task) = selection_task {
            *self.task.lock().unwrap() = Some(task);
        }
        let _ = self
            .events
            .publish(crate::ProtocolEvent::RuntimeInventoryChanged {
                jsonrpc: "2.0".into(),
                params,
            });
        if let Some(ready) = release {
            let _ = ready.send(());
        }
        Ok(selected)
    }
    pub fn stop(&self) {
        self.control.lock().unwrap().cancel();
        if let Some(task) = self.task.lock().unwrap().take() {
            task.abort();
        }
    }
}
impl Drop for RuntimeScanner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn executable_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn data_dir(variable: &str, default_name: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(default_name)))
}

/// npm on Windows exposes shell shims in PATH, not the native executable.
/// Resolve the package's declared binary without executing/parsing shell text.
pub fn npm_binary(directory: &Path, package: &str, command: &str) -> Option<PathBuf> {
    let root = directory.join("node_modules").join(package);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("package.json")).ok()?).ok()?;
    let bin = manifest.get("bin")?;
    let relative = Path::new(bin.as_str().or_else(|| bin.get(command)?.as_str())?);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let binary = root.join(relative);
    native_file(&binary).then_some(binary)
}

fn native_file(path: &Path) -> bool {
    if !path.is_absolute() || !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    return path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com"));
    #[cfg(not(windows))]
    {
        true
    }
}

pub fn candidates_in(directory: &Path, command: &str, npm_package: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let mut candidates = vec![
        directory.join(command).with_extension("exe"),
        directory.join(command).with_extension("com"),
    ];
    #[cfg(not(windows))]
    let mut candidates = vec![directory.join(command)];
    if let Some(binary) = npm_binary(directory, npm_package, command) {
        candidates.push(binary);
    }
    candidates
        .into_iter()
        .filter(|path| native_file(path))
        .collect()
}

pub fn discover(command: &str, npm_package: &str) -> Vec<RuntimeCandidate> {
    let mut candidates = Vec::new();
    let mut directories = std::collections::HashSet::new();
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            if !directories.insert(executable_key(&directory.to_string_lossy())) {
                continue;
            }
            candidates.extend(
                candidates_in(&directory, command, npm_package)
                    .into_iter()
                    .map(|path| candidate(path, RuntimeCandidateSource::CurrentPath)),
            );
        }
    }
    #[cfg(windows)]
    for directory in [
        std::env::var_os("NPM_CONFIG_PREFIX").map(PathBuf::from),
        std::env::var_os("APPDATA").map(|path| PathBuf::from(path).join("npm")),
    ]
    .into_iter()
    .flatten()
    {
        candidates.extend(
            candidates_in(&directory, command, npm_package)
                .into_iter()
                .map(|path| candidate(path, RuntimeCandidateSource::WindowsApplication)),
        );
    }
    if let Some(path) = login_shell_command(command) {
        candidates.push(candidate(path, RuntimeCandidateSource::LoginShell));
    }
    candidates
}

pub fn candidate(path: PathBuf, source: RuntimeCandidateSource) -> RuntimeCandidate {
    RuntimeCandidate {
        executable_path: path.to_string_lossy().into_owned(),
        source,
    }
}

/// User-selected npm shims resolve to the same native binary as automatic discovery.
pub fn resolve_executable(
    path: &Path,
    command: &str,
    npm_package: &str,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    if path.is_absolute()
        && path.is_file()
        && path
            .file_stem()
            .is_some_and(|stem| stem.eq_ignore_ascii_case(command))
        && (path.extension().is_none()
            || path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("ps1")
            }))
    {
        if let Some(binary) = path
            .parent()
            .and_then(|parent| npm_binary(parent, npm_package, command))
        {
            return std::fs::canonicalize(binary).map_err(|error| error.to_string());
        }
    }
    let _ = (command, npm_package);
    if !native_file(path) {
        return Err(format!(
            "Select an existing native executable: {}",
            path.display()
        ));
    }
    std::fs::canonicalize(path).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn login_shell_command(_command: &str) -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn login_shell_command(command: &str) -> Option<PathBuf> {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    if !command
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/sh"
            })
        });
    let mut child = crate::process::Command::new(shell)
        .args(["-lc", &format!("command -v {command}")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
                return (status.success() && native_file(&path)).then_some(path);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Isolated OpenCode storage uses XDG roots; these are not executable directories.
pub fn opencode_environment(command: &mut std::process::Command, directory: Option<&Path>) {
    if let Some(directory) = directory {
        for (variable, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_STATE_HOME", "state"),
        ] {
            command.env(variable, directory.join(name));
        }
        // Explicit storage must not accidentally load an inherited custom config.
        for variable in [
            "OPENCODE_CONFIG",
            "OPENCODE_CONFIG_DIR",
            "OPENCODE_CONFIG_CONTENT",
        ] {
            command.env_remove(variable);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn inventory_probes_each_canonical_executable_once_without_blocking_runtime() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("agent.exe");
        std::fs::write(&binary, b"fixture").unwrap();
        let candidate = candidate(binary, RuntimeCandidateSource::CurrentPath);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        let inventory = runtime_inventory(
            "agent",
            "missing",
            None,
            move || vec![candidate.clone(), candidate.clone(), candidate],
            move |candidate, budget| {
                assert_eq!(budget, std::time::Duration::from_secs(120));
                counter.fetch_add(1, Ordering::SeqCst);
                released
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .expect("probe blocked executor");
                Ok(crate::RuntimeInstallation {
                    executable_path: candidate.executable_path,
                    source: candidate.source,
                    version: "fixture".into(),
                })
            },
        );
        let heartbeat = async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            release.send(()).unwrap();
        };
        let (inventory, ()) = tokio::join!(inventory, heartbeat);
        assert_eq!(inventory.unwrap().installed.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scanner_returns_snapshot_while_probes_run_concurrently_and_notifies_completion() {
        use std::sync::{Arc, Barrier};
        let temp = tempfile::tempdir().unwrap();
        let candidates = ["one.exe", "two.exe"].map(|name| {
            let path = temp.path().join(name);
            std::fs::write(&path, b"fixture").unwrap();
            candidate(path, RuntimeCandidateSource::CurrentPath)
        });
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let scanner = RuntimeScanner::new(Arc::new(move |event| {
            events.send(event).unwrap();
            Ok(())
        }));
        let barrier = Arc::new(Barrier::new(3));
        let probes = barrier.clone();
        scanner.start(
            "agent",
            "missing",
            move || candidates.to_vec(),
            move |candidate, _| {
                probes.wait();
                Ok(crate::RuntimeInstallation {
                    executable_path: candidate.executable_path,
                    source: candidate.source,
                    version: "test".into(),
                })
            },
        );
        assert_eq!(scanner.snapshot().scanning, Some(true));
        // Both blocking probes must reach the barrier, without occupying this executor.
        let release = tokio::task::spawn_blocking(move || barrier.wait());
        tokio::time::timeout(std::time::Duration::from_secs(5), release)
            .await
            .unwrap()
            .unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap();
        let crate::ProtocolEvent::RuntimeInventoryChanged { params, .. } = event else {
            panic!("wrong event");
        };
        assert_eq!(params.scanning, Some(false));
        assert_eq!(params.installed.len(), 2);
        assert!(params.installed[0].executable_path.ends_with("one.exe"));
        assert_eq!(scanner.snapshot().installed.len(), 2);
        scanner.stop();
    }

    #[tokio::test]
    async fn manual_selection_probes_in_background_and_replaces_pending_version() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("manual.exe");
        std::fs::write(&binary, b"fixture").unwrap();
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let scanner = RuntimeScanner::new(std::sync::Arc::new(move |event| {
            events.send(event).unwrap();
            Ok(())
        }));
        scanner.start("agent", "missing", Vec::new, |candidate, _| {
            Ok(crate::RuntimeInstallation {
                executable_path: candidate.executable_path,
                source: candidate.source,
                version: "detected".into(),
            })
        });
        received.recv().await.unwrap();
        let selection = scanner
            .select(&candidate(binary, RuntimeCandidateSource::Configured))
            .unwrap();
        assert!(selection.version.is_empty());
        let crate::ProtocolEvent::RuntimeInventoryChanged { params, .. } =
            received.recv().await.unwrap()
        else {
            panic!("wrong event");
        };
        assert_eq!(params.scanning, Some(true));
        let crate::ProtocolEvent::RuntimeInventoryChanged { params, .. } =
            received.recv().await.unwrap()
        else {
            panic!("wrong event");
        };
        assert_eq!(params.scanning, Some(false));
        assert_eq!(params.selected.unwrap().version, "detected");
        scanner.stop();
        scanner.start("agent", "missing", Vec::new, |candidate, _| {
            Ok(crate::RuntimeInstallation {
                executable_path: candidate.executable_path,
                source: candidate.source,
                version: "rescanned".into(),
            })
        });
        assert_eq!(scanner.snapshot().scanning, Some(true));
        received.recv().await.unwrap();
        assert_eq!(scanner.snapshot().selected.unwrap().version, "rescanned");
    }

    #[cfg(windows)]
    #[test]
    fn background_command_has_no_console() {
        let output = command(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "local_runtime::tests::console_probe_child",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "child process assertion for background_command_has_no_console"]
    fn console_probe_child() {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetConsoleWindow() -> *mut std::ffi::c_void;
        }
        assert!(
            unsafe { GetConsoleWindow() }.is_null(),
            "background process owns a console window"
        );
    }
    #[test]
    fn npm_manifest_resolves_native_binary_in_path_with_spaces() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("npm prefix 中文");
        let package = prefix.join("node_modules").join("@example").join("runtime");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        let executable = package.join("bin").join("agent.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"bin":{"agent":"bin/agent.exe"}}"#,
        )
        .unwrap();
        assert_eq!(
            npm_binary(&prefix, "@example/runtime", "agent"),
            Some(executable)
        );
        std::fs::write(
            package.join("package.json"),
            r#"{"bin":{"agent":"../../escape.exe"}}"#,
        )
        .unwrap();
        assert!(npm_binary(&prefix, "@example/runtime", "agent").is_none());
    }
    #[test]
    fn opencode_storage_is_separate_and_does_not_change_parent_environment() {
        let temp = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("opencode");
        opencode_environment(&mut command, Some(temp.path()));
        let data = command
            .get_envs()
            .find(|(key, _)| *key == "XDG_DATA_HOME")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(Path::new(data), temp.path().join("data"));
    }
    #[cfg(windows)]
    #[test]
    fn windows_path_discovery_ignores_unix_shim_and_finds_exe() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("agent"), b"#!/bin/sh").unwrap();
        std::fs::write(temp.path().join("agent.exe"), b"fixture").unwrap();
        assert_eq!(
            candidates_in(temp.path(), "agent", "missing"),
            vec![temp.path().join("agent.exe")]
        );
    }
    #[cfg(windows)]
    #[test]
    fn selected_npm_shim_resolves_to_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("node_modules").join("agent-package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        let binary = package.join("bin").join("agent.exe");
        std::fs::write(&binary, b"fixture").unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"bin":{"agent":"bin/agent.exe"}}"#,
        )
        .unwrap();
        let shim = temp.path().join("agent.cmd");
        std::fs::write(&shim, b"not executed").unwrap();
        assert_eq!(
            resolve_executable(&shim, "agent", "agent-package").unwrap(),
            binary.canonicalize().unwrap()
        );
    }
}

#[cfg(test)]
mod probe_cancellation_tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc, Mutex},
        time::Duration,
    };
    #[tokio::test]
    async fn stopping_scan_kills_blocking_probe_even_when_registration_is_late() {
        for late_registration in [false, true] {
            let executable = std::env::current_exe().unwrap();
            let candidate = candidate(executable, RuntimeCandidateSource::Configured);
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let entered = Arc::new(Mutex::new(Some(entered_tx)));
            let (release_tx, release_rx) = mpsc::channel();
            let release = Arc::new(Mutex::new(release_rx));
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let done = Arc::new(Mutex::new(Some(done_tx)));
            let scanner = RuntimeScanner::new(Arc::new(|_| Ok(())));
            scanner.start_cancellable(
                "agent",
                "missing",
                move || vec![candidate],
                move |candidate, _, control| {
                    let mut child = command(&candidate.executable_path)
                        .args([
                            "--ignored",
                            "--exact",
                            "background_probe::tests::slow_child",
                        ])
                        .stdout(std::process::Stdio::null())
                        .spawn()
                        .unwrap();
                    if !late_registration {
                        control.track(child.control()).unwrap();
                    }
                    entered.lock().unwrap().take().unwrap().send(()).unwrap();
                    if late_registration {
                        release.lock().unwrap().recv().unwrap();
                        assert!(control.track(child.control()).is_err());
                    }
                    let _ = child.wait();
                    done.lock().unwrap().take().unwrap().send(()).unwrap();
                    Err(scan_error("cancelled fixture".into()))
                },
            );
            tokio::time::timeout(Duration::from_secs(3), entered_rx)
                .await
                .unwrap()
                .unwrap();
            scanner.stop();
            if late_registration {
                release_tx.send(()).unwrap();
            }
            tokio::time::timeout(Duration::from_secs(2), done_rx)
                .await
                .expect("blocking version probe survived scanner.stop")
                .unwrap();
        }
    }
}
