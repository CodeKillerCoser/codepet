//! Local filesystem and executable discovery. Product layouts remain in each provider.
use crate::{RuntimeCandidate, RuntimeCandidateSource};
use std::path::{Path, PathBuf};

/// Background runtime processes retain their stdio pipes but never create a console window.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> crate::process::Command {
    let mut command = crate::process::Command::new(program);
    runtime_environment(&mut command);
    command
}

#[cfg(not(windows))]
static RUNTIME_PATH: std::sync::RwLock<Option<std::ffi::OsString>> = std::sync::RwLock::new(None);

/// Reuse the PATH captured by background discovery for probes and runtime children.
/// Never start a shell here: callers may run on the async executor.
pub fn runtime_environment(command: &mut std::process::Command) {
    #[cfg(not(windows))]
    if let Some(path) = RUNTIME_PATH.read().unwrap().as_ref() {
        if !command.get_envs().any(|(key, _)| key == "PATH") {
            command.env("PATH", path);
        }
    }
    #[cfg(windows)]
    let _ = command;
}

thread_local! {
    // Discovery runs on one blocking worker. Keep its diagnostics with that scan,
    // rather than leaking a failed shell lookup into another scanner's result.
    static DISCOVERY_ERRORS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
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
    let (candidates, mut errors) = tokio::task::spawn_blocking(move || {
        DISCOVERY_ERRORS.with(|errors| errors.borrow_mut().clear());
        let mut candidates = discover();
        if let Some(selected) = selected_candidate.filter(|runtime| Path::new(&runtime.executable_path).is_file()) {
            candidates.insert(
                0,
                RuntimeCandidate {
                    executable_path: selected.executable_path,
                    source: selected.source,
                },
            );
        }
        let mut seen = std::collections::HashSet::new();
        let mut resolved = Vec::new();
        let mut errors = DISCOVERY_ERRORS.with(|errors| std::mem::take(&mut *errors.borrow_mut()));
        for mut candidate in candidates {
            match resolve_executable(Path::new(&candidate.executable_path), name, npm_package) {
                Ok(path) => {
                    candidate.executable_path = path.to_string_lossy().into_owned();
                    if seen.insert(executable_key(&candidate.executable_path)) { resolved.push(candidate); }
                }
                Err(error) if matches!(candidate.source, RuntimeCandidateSource::Configured | RuntimeCandidateSource::Environment)
                    || Path::new(&candidate.executable_path).exists() => {
                    errors.push(format!("{}: {error}", candidate.executable_path));
                }
                Err(_) => {}
            }
        }
        (resolved, errors)
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
                let path = candidate.executable_path.clone();
                async move {
                    let result = tokio::task::spawn_blocking(move || {
                        probe(candidate, std::time::Duration::from_secs(120), control)
                    })
                    .await;
                    (index, path, result)
                }
            }),
    )
    .buffer_unordered(4);
    let mut harness_list = Vec::new();
    while let Some((index, path, result)) = results.next().await {
        match result {
            Ok(Ok(runtime)) => harness_list.push((index, apply_runtime_requirement(runtime))),
            Ok(Err(e)) => errors.push(format!("{path}: {}", e.message)),
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    harness_list.sort_by_key(|(index, _)| *index);
    let harness_list: Vec<_> = harness_list.into_iter().map(|(_, runtime)| runtime).collect();
    let selected = select_installation(&harness_list, selected.as_ref().map(|r| r.executable_path.as_str()));
    Ok(crate::RuntimeGetInstalledResponse {
        harness_list,
        selected,
        scanning: Some(false),
        scan_error: (!errors.is_empty()).then(|| errors.join("; ")),
    })
}

/// Prefer the previous executable if it is still compatible; otherwise use the
/// highest semantic version. Equal versions use the path for a stable tie-break.
pub fn select_installation(
    harness_list: &[crate::RuntimeInstallation],
    last_selected: Option<&str>,
) -> Option<crate::RuntimeInstallation> {
    let compatible = || harness_list.iter().filter(|r| r.incompatibility_reason.is_none());
    if let Some(path) = last_selected {
        if let Some(runtime) = compatible().find(|r| executable_key(&r.executable_path) == executable_key(path)) {
            return Some(runtime.clone());
        }
    }
    compatible().max_by(|a, b| {
        let a_version = semver::Version::parse(a.version.trim_start_matches('v')).ok();
        let b_version = semver::Version::parse(b.version.trim_start_matches('v')).ok();
        match (&a_version, &b_version) {
            (Some(a), Some(b)) => a.cmp_precedence(b),
            _ => a_version.cmp(&b_version),
        }
            .then_with(|| executable_key(&b.executable_path).cmp(&executable_key(&a.executable_path)))
    }).cloned()
}

/// Persistence belongs to the Provider business database, not the SDK or Host.
pub trait RuntimeSelectionStorage: Send + Sync {
    fn load_last_selected(&self) -> Result<Option<String>, crate::ProtocolError>;
    fn save_last_selected(&self, path: Option<&str>) -> Result<(), crate::ProtocolError>;
}
/// Preserve detected versions even when a Provider's configured version floor rejects them.
pub fn apply_runtime_requirement(mut runtime: crate::RuntimeInstallation) -> crate::RuntimeInstallation {
    let minimum = std::env::var("CODEPET_RUNTIME_MIN_VERSION").ok().filter(|value| !value.is_empty());
    apply_minimum_version(&mut runtime, minimum.as_deref());
    runtime
}

fn apply_minimum_version(runtime: &mut crate::RuntimeInstallation, minimum: Option<&str>) {
    runtime.minimum_version = minimum.map(str::to_string);
    runtime.incompatibility_reason = minimum.and_then(|minimum| {
        match (semver::Version::parse(&runtime.version), semver::Version::parse(minimum)) {
            (Ok(version), Ok(required)) if version >= required => None,
            (Ok(_), Ok(_)) => Some(format!("Detected version {} is below Provider minimum {minimum}", runtime.version)),
            (_, Err(_)) => Some(format!("Invalid Provider minimum version: {minimum}")),
            (Err(_), _) => Some(format!("Cannot compare detected version {} with Provider minimum {minimum}", runtime.version)),
        }
    });
}

pub fn require_compatible_runtime(runtime: &crate::RuntimeInstallation) -> Result<(), crate::ProtocolError> {
    match runtime.incompatibility_reason.as_ref() {
        None => Ok(()),
        Some(reason) => Err(scan_error(format!("{}: {reason}", runtime.executable_path))),
    }
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
    storage: std::sync::Mutex<Option<std::sync::Arc<dyn RuntimeSelectionStorage>>>,
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl RuntimeScanner {
    pub fn new(events: std::sync::Arc<dyn crate::ProviderEventSink>) -> Self {
        Self {
            control: std::sync::Mutex::new(RuntimeProbeControl::default()),
            snapshot: std::sync::Arc::new(std::sync::Mutex::new(
                crate::RuntimeGetInstalledResponse {
                    harness_list: vec![],
                    selected: None,
                    scanning: Some(true),
                    scan_error: None,
                },
            )),
            task: std::sync::Mutex::new(None),
            probe: std::sync::Mutex::new(None),
            storage: std::sync::Mutex::new(None),
            generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            events,
        }
    }
    pub fn set_selection_storage(&self, storage: std::sync::Arc<dyn RuntimeSelectionStorage>) {
        *self.storage.lock().unwrap() = Some(storage);
    }
    pub fn snapshot(&self) -> crate::RuntimeGetInstalledResponse {
        self.snapshot.lock().unwrap().clone()
    }
    /// Validate an instance-specific executable without changing the Provider's
    /// default selection or its persisted lastSelected record.
    pub async fn inspect(&self, candidate: RuntimeCandidate) -> Result<crate::RuntimeInstallation, crate::ProtocolError> {
        let probe = self.probe.lock().unwrap().clone()
            .ok_or_else(|| scan_error("Provider is not initialized".into()))?;
        let control = self.control.lock().unwrap().clone();
        let runtime = tokio::task::spawn_blocking(move || {
            probe(candidate, std::time::Duration::from_secs(120), control)
        }).await.map_err(|e| scan_error(e.to_string()))??;
        let runtime = apply_runtime_requirement(runtime);
        require_compatible_runtime(&runtime)?;
        Ok(runtime)
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
        let storage = self.storage.lock().unwrap().clone();
        let generation = self.generation.clone();
        let epoch = generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        *task = Some(tokio::spawn(async move {
            let load_storage = storage.clone();
            let result = async {
                let selected = tokio::task::spawn_blocking(move || {
                    let path = match load_storage {
                        Some(storage) => storage.load_last_selected()?,
                        None => selected.map(|r| r.executable_path),
                    };
                    Ok::<_, crate::ProtocolError>(path.map(|executable_path| crate::RuntimeInstallation {
                        executable_path, source: RuntimeCandidateSource::Configured,
                        version: String::new(), minimum_version: None, incompatibility_reason: None,
                    }))
                }).await.map_err(|e| scan_error(e.to_string()))??;
                runtime_inventory_controlled(name, package, selected, discover,
                    move |candidate, timeout, control| probe(candidate, timeout, control), control).await
            }.await;
            // Database writes run off the executor and finish before publishing the
            // snapshot. Cancelled generations cannot overwrite a newer scan.
            let _ = tokio::task::spawn_blocking(move || {
                let mut current = snapshot.lock().unwrap();
                if generation.load(std::sync::atomic::Ordering::SeqCst) != epoch { return; }
                match result {
                    Ok(mut result) => {
                        if let (Some(storage), Some(selected)) = (&storage, &result.selected) {
                            if let Err(error) = storage.save_last_selected(Some(&selected.executable_path)) {
                                result.scan_error = Some(error.message);
                                result.selected = None;
                            }
                        }
                        *current = result;
                    }
                    Err(error) => {
                        current.scanning = Some(false);
                        current.selected = None;
                        current.scan_error = Some(error.message);
                    }
                }
                let params = current.clone();
                drop(current);
                let _ = events.publish(crate::ProtocolEvent::RuntimeInventoryChanged {
                    jsonrpc: "2.0".into(), params,
                });
            }).await;
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
            if current.scanning == Some(true) {
                return Err(crate::ProtocolError { code: "runtime_scanning".into(),
                    message: "Wait for runtime discovery before selecting an executable".into(),
                    retryable: true, details: None });
            }
            let cached = current
                .harness_list
                .iter()
                .find(|r| {
                    executable_key(&r.executable_path) == executable_key(&path.to_string_lossy())
                })
                .cloned();
            let selected = if let Some(mut selected) = cached {
                require_compatible_runtime(&selected)?;
                selected.source = candidate.source;
                if let Some(storage) = self.storage.lock().unwrap().as_ref() {
                    storage.save_last_selected(Some(&selected.executable_path))?;
                }
                if let Some(runtime) = current.harness_list.iter_mut().find(|r| r.executable_path == selected.executable_path) {
                    *runtime = selected.clone();
                }
                current.scan_error = None;
                selected
            } else {
                let probe = self
                    .probe
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| scan_error("Provider is not initialized".into()))?;
                // Return a pending result; publish and persist selection only after validation.
                let selected = crate::RuntimeInstallation { minimum_version: None, incompatibility_reason: None,
                    executable_path: path.to_string_lossy().into_owned(),
                    source: candidate.source,
                    version: String::new(),
                };
                let candidate = RuntimeCandidate {
                    executable_path: selected.executable_path.clone(),
                    source: selected.source,
                };
                let snapshot = self.snapshot.clone();
                let events = self.events.clone();
                let storage = self.storage.lock().unwrap().clone();
                let generation = self.generation.clone();
                let epoch = generation.load(std::sync::atomic::Ordering::SeqCst);
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
                    let _ = tokio::task::spawn_blocking(move || {
                        let mut current = snapshot.lock().unwrap();
                        if generation.load(std::sync::atomic::Ordering::SeqCst) != epoch { return; }
                        current.scanning = Some(false);
                        match result {
                            Ok(Ok(runtime)) => {
                                let runtime = apply_runtime_requirement(runtime);
                                let accepted = require_compatible_runtime(&runtime).and_then(|()| {
                                    match &storage {
                                        Some(storage) => storage.save_last_selected(Some(&runtime.executable_path)),
                                        None => Ok(()),
                                    }
                                });
                                match accepted {
                                    Ok(()) => current.selected = Some(runtime.clone()),
                                    Err(error) => current.scan_error = Some(error.message),
                                }
                                current.harness_list.push(runtime);
                            }
                            Ok(Err(error)) => current.scan_error = Some(error.message),
                            Err(error) => current.scan_error = Some(error.to_string()),
                        }
                        let params = current.clone();
                        drop(current);
                        let _ = events.publish(crate::ProtocolEvent::RuntimeInventoryChanged {
                            jsonrpc: "2.0".into(), params,
                        });
                    }).await;
                }));
                selected
            };
            if !selected.version.is_empty() { current.selected = Some(selected.clone()); }
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
        {
            let _snapshot = self.snapshot.lock().unwrap();
            self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
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
    candidates.extend(login_shell_candidates(command, npm_package).into_iter()
        .map(|path| candidate(path, RuntimeCandidateSource::LoginShell)));
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
fn login_shell_candidates(_command: &str, _npm_package: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(not(windows))]
fn login_shell_candidates(command: &str, npm_package: &str) -> Vec<PathBuf> {
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
    let paths = login_shell_path(crate::process::Command::new(shell));
    if paths.is_none() {
        DISCOVERY_ERRORS.with(|errors| errors.borrow_mut().push(
            "Login shell PATH discovery failed: shell startup, output, or 5-second timeout; runtime inventory may be incomplete".into()));
    }
    *RUNTIME_PATH.write().unwrap() = paths.clone();
    paths.map(|paths| candidates_from_path(&paths, command, npm_package)).unwrap_or_default()
}

#[cfg(not(windows))]
fn candidates_from_path(paths: &std::ffi::OsStr, command: &str, npm_package: &str) -> Vec<PathBuf> {
    std::env::split_paths(paths)
        .flat_map(|directory| candidates_in(&directory, command, npm_package))
        .collect()
}

#[cfg(not(windows))]
fn login_shell_path(
    mut shell: crate::process::Command,
) -> Option<std::ffi::OsString> {
    use std::{process::Stdio, time::{Duration, Instant}};
    use std::os::unix::ffi::OsStringExt;
    // Interactive startup files commonly own npm/nvm PATH entries. Read the PATH
    // itself so aliases/functions and startup banners cannot masquerade as a file.
    let mut child = shell
        .args(["-lic", "command printf '\\0%s\\0' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Drain while the shell runs. Even PATH output can fill a platform pipe;
    // waiting for exit before reading misreports a healthy shell as a timeout.
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut bytes = Vec::new();
        stdout.take(1024 * 1024).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = child.wait();
                let output = reader.join().ok()?.ok()?;
                if !status.success() { return None; }
                let paths = output.split(|byte| *byte == 0).nth(1)?;
                return Some(std::ffi::OsString::from_vec(paths.to_vec()));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
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
    #[test]
    fn selection_prefers_matching_last_selected_then_highest_semver() {
        let runtime = |path: &str, version: &str| crate::RuntimeInstallation {
            executable_path: path.into(), version: version.into(),
            source: RuntimeCandidateSource::CurrentPath,
            minimum_version: None, incompatibility_reason: None,
        };
        let old = runtime("/old.exe", "1.9.0");
        let latest = runtime("/latest.exe", "1.10.0");
        let prerelease = runtime("/preview.exe", "1.10.0-rc.1");
        let list = vec![old.clone(), prerelease, latest.clone()];
        assert_eq!(select_installation(&list, Some("/old.exe")), Some(old));
        assert_eq!(select_installation(&list, None), Some(latest.clone()));
        assert_eq!(select_installation(&list, Some("/removed.exe")), Some(latest));
        let mut incompatible = runtime("/blocked.exe", "9.0.0");
        incompatible.incompatibility_reason = Some("unsupported".into());
        let list = vec![list[0].clone(), incompatible.clone()];
        assert_eq!(select_installation(&list, Some("/blocked.exe")), Some(list[0].clone()));
        assert!(select_installation(&[incompatible], None).is_none());
        assert!(select_installation(&[], None).is_none());
    }

    #[tokio::test]
    async fn new_scanner_discovers_relocated_installation_without_restoring_selection() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old.exe");
        for name in ["old.exe", "new.exe"] {
            let path = temp.path().join(name);
            std::fs::write(&path, b"fixture").unwrap();
            let expected = std::fs::canonicalize(&path).unwrap().to_string_lossy().into_owned();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let scanner = RuntimeScanner::new(std::sync::Arc::new(move |event| {
                tx.send(event).unwrap();
                Ok(())
            }));
            scanner.start("agent", "missing",
                move || vec![candidate(path, RuntimeCandidateSource::CurrentPath)],
                |candidate, _| Ok(crate::RuntimeInstallation {
                    executable_path: candidate.executable_path, source: candidate.source,
                    version: "1.0.0".into(), minimum_version: None, incompatibility_reason: None,
                }));
            tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            let inventory = scanner.snapshot();
            assert_eq!(inventory.harness_list.len(), 1);
            assert_eq!(inventory.harness_list[0].executable_path, expected);
            assert_eq!(inventory.selected.as_ref().unwrap().executable_path, expected);
            assert!(inventory.scan_error.is_none());
            if name == "old.exe" {
                let selected = scanner.select(&RuntimeCandidate {
                    executable_path: expected, source: RuntimeCandidateSource::Configured,
                }).unwrap();
                assert_eq!(selected.source, RuntimeCandidateSource::Configured);
                std::fs::remove_file(&old).unwrap();
            }
        }
    }

    #[test]
    fn startup_inventory_ignores_previous_host_executable() {
        let output = command(std::env::current_exe().unwrap())
            .env("CODEPET_RUNTIME_EXECUTABLE", "C:/removed-installation/codex.exe")
            .args(["--ignored", "--exact", "local_runtime::tests::fresh_inventory_child", "--nocapture"])
            .output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[tokio::test]
    #[ignore = "isolated environment fixture for startup_inventory_ignores_previous_host_executable"]
    async fn fresh_inventory_child() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("current-install.exe");
        std::fs::write(&binary, b"fixture").unwrap();
        let expected = std::fs::canonicalize(&binary).unwrap().to_string_lossy().into_owned();
        let inventory = runtime_inventory("agent", "missing", None,
            move || vec![candidate(binary, RuntimeCandidateSource::CurrentPath)],
            |candidate, _| Ok(crate::RuntimeInstallation {
                executable_path: candidate.executable_path, source: candidate.source,
                version: "1.0.0".into(), minimum_version: None, incompatibility_reason: None,
            })).await.unwrap();
        assert_eq!(inventory.harness_list.len(), 1);
        assert_eq!(inventory.harness_list[0].executable_path, expected);
        assert!(inventory.scan_error.is_none(), "{:?}", inventory.scan_error);
        assert_eq!(inventory.selected.as_ref().unwrap().executable_path, expected);
    }
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn discovered_shell_path_reaches_version_and_runtime_children() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("npm prefix 中文").join("bin");
        std::fs::create_dir_all(&directory).unwrap();
        for (name, content) in [
            ("codepet_fixture_agent", "#!/usr/bin/env codepet_fixture_node\n"),
            ("codepet_fixture_node", "#!/bin/sh\nprintf 'fixture-runtime %s\\n' \"$2\"\n"),
        ] {
            let path = directory.join(name);
            std::fs::write(&path, content).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(temp.path().join(".zshrc"),
            "export PATH=\"$ZDOTDIR/npm prefix 中文/bin:/usr/bin:/bin\"\n").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "local_runtime::tests::runtime_path_child", "--nocapture"])
            .env("SHELL", "/bin/zsh")
            .env("ZDOTDIR", temp.path())
            .env("PATH", "/usr/bin:/bin")
            .output().unwrap();
        assert!(output.status.success(), "{}\n{}",
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "isolated process for runtime environment regression"]
    async fn runtime_path_child() {
        let parent_path = std::env::var_os("PATH").unwrap();
        let candidates = tokio::task::spawn_blocking(|| discover("codepet_fixture_agent", "missing"))
            .await.unwrap();
        let executable = &candidates.first().expect("shell runtime discovered").executable_path;
        let original = std::process::Command::new(executable).arg("--version").output().unwrap();
        assert_eq!(original.status.code(), Some(127));
        for arg in ["--version", "app-server"] {
            let output = command(executable).arg(arg).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), format!("fixture-runtime {arg}"));
        }
        let mut probe = tokio::process::Command::new(executable);
        probe.arg("auth");
        let output = crate::background_probe::run(probe, std::time::Duration::from_secs(5)).await.unwrap();
        assert_eq!(output.trim(), "fixture-runtime auth");
        let mut explicit = tokio::process::Command::new(executable);
        explicit.env("PATH", &parent_path);
        let output = crate::background_probe::output(explicit, std::time::Duration::from_secs(5)).await.unwrap();
        assert_eq!(output.status.code(), Some(127), "explicit child PATH must be preserved");
        assert_eq!(std::env::var_os("PATH").unwrap(), parent_path);
    }

    #[test]
    fn minimum_version_keeps_old_installations_visible_but_rejects_selection() {
        let mut runtime = crate::RuntimeInstallation {
            executable_path: "/test/codex".into(), version: "0.148.0".into(),
            source: RuntimeCandidateSource::CurrentPath,
            minimum_version: None, incompatibility_reason: None,
        };
        apply_minimum_version(&mut runtime, Some("0.151.0"));
        assert_eq!(runtime.version, "0.148.0");
        assert_eq!(runtime.minimum_version.as_deref(), Some("0.151.0"));
        assert!(require_compatible_runtime(&runtime).is_err());
        let file = tempfile::NamedTempFile::new().unwrap();
        runtime.executable_path = std::fs::canonicalize(file.path()).unwrap().to_string_lossy().into_owned();
        let scanner = RuntimeScanner::new(std::sync::Arc::new(|_| Ok(())));
        scanner.snapshot.lock().unwrap().harness_list.push(runtime.clone());
        assert!(scanner.select(&RuntimeCandidate {
            executable_path: runtime.executable_path.clone(), source: runtime.source,
        }).is_err());
        assert!(scanner.snapshot().selected.is_none());
        for version in ["0.151.0", "0.153.4"] {
            runtime.version = version.into();
            apply_minimum_version(&mut runtime, Some("0.151.0"));
            assert!(require_compatible_runtime(&runtime).is_ok());
        }
        runtime.version = "0.151.0-beta.1".into();
        apply_minimum_version(&mut runtime, Some("0.151.0"));
        assert!(require_compatible_runtime(&runtime).is_err());
        apply_minimum_version(&mut runtime, Some("bad-version"));
        assert!(require_compatible_runtime(&runtime).is_err());
        apply_minimum_version(&mut runtime, None);
        assert!(require_compatible_runtime(&runtime).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn login_shell_discovers_interactive_path_despite_function_and_banner() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("npm prefix 中文").join("bin");
        std::fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("codepet_fixture_agent");
        std::fs::write(&binary, b"fixture").unwrap();
        std::fs::write(temp.path().join(".zshrc"),
            "export PATH=\"$ZDOTDIR/npm prefix 中文/bin:$PATH\"\ncommand printf '%131072s\\n' 'shell startup banner'\ncodepet_fixture_agent() { echo wrapper; }\n").unwrap();
        let mut shell = crate::process::Command::new("/bin/zsh");
        shell.env("ZDOTDIR", temp.path());
        let paths = login_shell_path(shell).unwrap();
        let second = temp.path().join("second install");
        std::fs::create_dir_all(&second).unwrap();
        let second_binary = second.join("codepet_fixture_agent");
        std::fs::write(&second_binary, b"fixture").unwrap();
        let combined = std::env::join_paths(std::env::split_paths(&paths).chain([second])).unwrap();
        assert_eq!(candidates_from_path(&combined, "codepet_fixture_agent", "missing"), vec![binary.clone(), second_binary]);
        assert_eq!(std::env::split_paths(&paths)
            .flat_map(|directory| candidates_in(&directory, "codepet_fixture_agent", "missing"))
            .next(), Some(binary));
    }

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
                Ok(crate::RuntimeInstallation { minimum_version: None, incompatibility_reason: None,
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
        assert_eq!(inventory.unwrap().harness_list.len(), 1);
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
                Ok(crate::RuntimeInstallation { minimum_version: None, incompatibility_reason: None,
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
        assert_eq!(params.harness_list.len(), 2);
        assert!(params.harness_list[0].executable_path.ends_with("one.exe"));
        assert_eq!(scanner.snapshot().harness_list.len(), 2);
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
            Ok(crate::RuntimeInstallation { minimum_version: None, incompatibility_reason: None,
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
            Ok(crate::RuntimeInstallation { minimum_version: None, incompatibility_reason: None,
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
