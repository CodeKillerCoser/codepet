use crate::settings::{
    load_app_settings, save_app_settings, AgentRuntimePreferenceSettings, AppSettings,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const CODEX_RUNTIME_PROVIDER_ID: &str = "codex";
pub const CLAUDE_RUNTIME_PROVIDER_ID: &str = "claude";
pub const OPENCODE_RUNTIME_PROVIDER_ID: &str = "opencode";

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const DISCOVERY_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentRuntimeStatus {
    Loading,
    Ready,
    Unavailable,
    InvalidConfiguredExecutable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentRuntimeSource {
    Configured,
    Environment,
    CurrentPath,
    LoginShell,
    MacosApplication,
    WindowsApplication,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeCandidate {
    pub executable_path: String,
    pub source: AgentRuntimeSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeInstallation {
    pub executable_path: String,
    pub version: String,
    pub source: AgentRuntimeSource,
}

impl AgentRuntimeDiagnostic {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntime {
    pub provider_id: String,
    pub display_name: String,
    pub status: AgentRuntimeStatus,
    pub resolved_executable: Option<String>,
    pub source: Option<AgentRuntimeSource>,
    pub configured_executable: Option<String>,
    pub version: Option<String>,
    pub diagnostic: Option<AgentRuntimeDiagnostic>,
    #[serde(default)]
    pub installed: Vec<AgentRuntimeInstallation>,
}

impl AgentRuntime {
    pub fn unavailable_reason(&self) -> String {
        self.diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.message.clone())
            .unwrap_or_else(|| format!("{} runtime is unavailable", self.display_name))
    }
}

#[derive(Debug)]
pub enum AgentRuntimeServiceError {
    Settings(io::Error),
    UnsupportedProvider(String),
    InvalidExecutable(AgentRuntimeDiagnostic),
}

impl fmt::Display for AgentRuntimeServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(error) => write!(formatter, "failed to access app settings: {error}"),
            Self::UnsupportedProvider(provider_id) => {
                write!(formatter, "unsupported agent runtime provider: {provider_id}")
            }
            Self::InvalidExecutable(diagnostic) => {
                write!(formatter, "{}: {}", diagnostic.code, diagnostic.message)
            }
        }
    }
}

impl std::error::Error for AgentRuntimeServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Settings(error) => Some(error),
            Self::UnsupportedProvider(_) | Self::InvalidExecutable(_) => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct AgentRuntimeService;

impl AgentRuntimeService {
    pub fn save_provider_selection(
        &self,
        provider_plugin_id: &str,
        executable: &str,
    ) -> Result<(), AgentRuntimeServiceError> {
        let mut settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        settings.agent_runtimes.by_provider
            .entry(provider_plugin_id.to_string())
            .or_insert_with(AgentRuntimePreferenceSettings::default)
            .configured_executable = Some(executable.to_string());
        save_app_settings(&settings).map_err(AgentRuntimeServiceError::Settings)
    }

    pub fn clear_provider_selection(
        &self,
        provider_plugin_id: &str,
    ) -> Result<(), AgentRuntimeServiceError> {
        let mut settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        settings.agent_runtimes.by_provider.remove(provider_plugin_id);
        save_app_settings(&settings).map_err(AgentRuntimeServiceError::Settings)
    }

    pub fn list(&self) -> Result<Vec<AgentRuntime>, AgentRuntimeServiceError> {
        let settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        Ok(runtime_descriptors()
            .iter()
            .map(|descriptor| resolve_descriptor(descriptor, &settings))
            .collect())
    }

    pub fn detect(
        &self,
        provider_id: &str,
    ) -> Result<AgentRuntime, AgentRuntimeServiceError> {
        let descriptor = runtime_descriptor(provider_id)?;
        let settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        Ok(resolve_descriptor(descriptor, &settings))
    }

    pub fn detect_automatic(
        &self,
        provider_id: &str,
    ) -> Result<AgentRuntime, AgentRuntimeServiceError> {
        let descriptor = runtime_descriptor(provider_id)?;
        Ok(resolve_descriptor_with(
            descriptor,
            None,
            automatic_candidates(descriptor),
            &SystemExecutableValidator,
        ))
    }

    pub fn candidates(
        &self,
        provider_id: &str,
    ) -> Result<Vec<AgentRuntimeCandidate>, AgentRuntimeServiceError> {
        let descriptor = runtime_descriptor(provider_id)?;
        let settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        let configured = settings.agent_runtimes.by_provider.get(provider_id)
            .and_then(|preference| preference.configured_executable.as_deref())
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| RuntimeCandidate {
                path: PathBuf::from(path),
                source: AgentRuntimeSource::Configured,
            });
        Ok(deduplicate_candidates(configured.into_iter().chain(automatic_candidates(descriptor)).collect())
            .into_iter()
            .map(|candidate| AgentRuntimeCandidate {
                executable_path: candidate.path.to_string_lossy().into_owned(),
                source: candidate.source,
            })
            .collect())
    }

    pub fn set_configured_executable(
        &self,
        provider_id: &str,
        executable: &str,
    ) -> Result<AgentRuntime, AgentRuntimeServiceError> {
        let descriptor = runtime_descriptor(provider_id)?;
        let configured = executable.trim();
        if configured.is_empty() {
            return Err(AgentRuntimeServiceError::InvalidExecutable(
                AgentRuntimeDiagnostic::new(
                    "executable-path-empty",
                    "selected executable path is empty",
                ),
            ));
        }

        let mut settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        let runtime = configure_runtime_in_settings(
            descriptor,
            &mut settings,
            configured,
            &SystemExecutableValidator,
        )
        .map_err(AgentRuntimeServiceError::InvalidExecutable)?;
        save_app_settings(&settings).map_err(AgentRuntimeServiceError::Settings)?;
        Ok(runtime)
    }

    pub fn clear_configured_executable(
        &self,
        provider_id: &str,
    ) -> Result<AgentRuntime, AgentRuntimeServiceError> {
        let descriptor = runtime_descriptor(provider_id)?;
        let mut settings = load_app_settings().map_err(AgentRuntimeServiceError::Settings)?;
        settings.agent_runtimes.by_provider.remove(provider_id);
        save_app_settings(&settings).map_err(AgentRuntimeServiceError::Settings)?;
        Ok(resolve_descriptor(descriptor, &settings))
    }
}

#[derive(Clone, Copy)]
struct AgentRuntimeDescriptor {
    provider_id: &'static str,
    display_name: &'static str,
    command_names: &'static [&'static str],
    environment_variable: Option<&'static str>,
    version_args: &'static [&'static str],
    macos_bundles: &'static [MacosBundleDescriptor],
    #[cfg(windows)]
    windows_layouts: &'static [WindowsInstallLayout],
}

#[derive(Clone, Copy)]
struct MacosBundleDescriptor {
    bundle_identifier: &'static str,
    relative_executables: &'static [&'static str],
}

#[derive(Clone, Copy)]
#[cfg(windows)]
enum WindowsInstallLayout {
    VersionedChildren {
        parent: &'static [&'static str],
        executable: &'static str,
    },
    PrefixedDirectory {
        parent: &'static [&'static str],
        prefix: &'static str,
        relative_executable: &'static [&'static str],
    },
}

const CODEX_MACOS_BUNDLES: &[MacosBundleDescriptor] = &[MacosBundleDescriptor {
    bundle_identifier: "com.openai.codex",
    relative_executables: &["Contents/Resources/codex"],
}];

#[cfg(windows)]
const CODEX_WINDOWS_LAYOUTS: &[WindowsInstallLayout] = &[
    WindowsInstallLayout::VersionedChildren {
        parent: &["OpenAI", "Codex", "bin"],
        executable: "codex.exe",
    },
    WindowsInstallLayout::PrefixedDirectory {
        parent: &["Packages"],
        prefix: "OpenAI.Codex_",
        relative_executable: &[
            "LocalCache",
            "Local",
            "OpenAI",
            "Codex",
            "bin",
            "codex.exe",
        ],
    },
];

const RUNTIME_DESCRIPTORS: &[AgentRuntimeDescriptor] = &[
    AgentRuntimeDescriptor {
        provider_id: CODEX_RUNTIME_PROVIDER_ID,
        display_name: "Codex",
        command_names: &["codex"],
        environment_variable: Some("CODE_PET_CODEX_BIN"),
        version_args: &["--version"],
        macos_bundles: CODEX_MACOS_BUNDLES,
        #[cfg(windows)]
        windows_layouts: CODEX_WINDOWS_LAYOUTS,
    },
    AgentRuntimeDescriptor {
        provider_id: CLAUDE_RUNTIME_PROVIDER_ID,
        display_name: "Claude Code",
        command_names: &["claude"],
        environment_variable: None,
        version_args: &["--version"],
        macos_bundles: &[],
        #[cfg(windows)]
        windows_layouts: &[],
    },
    AgentRuntimeDescriptor {
        provider_id: OPENCODE_RUNTIME_PROVIDER_ID,
        display_name: "OpenCode",
        command_names: &["opencode"],
        environment_variable: None,
        version_args: &["--version"],
        macos_bundles: &[],
        #[cfg(windows)]
        windows_layouts: &[],
    },
];

fn runtime_descriptors() -> &'static [AgentRuntimeDescriptor] {
    RUNTIME_DESCRIPTORS
}

fn runtime_descriptor(
    provider_id: &str,
) -> Result<&'static AgentRuntimeDescriptor, AgentRuntimeServiceError> {
    runtime_descriptors()
        .iter()
        .find(|descriptor| descriptor.provider_id == provider_id)
        .ok_or_else(|| AgentRuntimeServiceError::UnsupportedProvider(provider_id.to_string()))
}

#[derive(Clone, Debug)]
struct RuntimeCandidate {
    path: PathBuf,
    source: AgentRuntimeSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ValidatedExecutable {
    path: PathBuf,
    version: Option<String>,
}

trait ExecutableValidator {
    fn validate(
        &self,
        descriptor: &AgentRuntimeDescriptor,
        path: &Path,
    ) -> Result<ValidatedExecutable, AgentRuntimeDiagnostic>;
}

struct SystemExecutableValidator;

impl ExecutableValidator for SystemExecutableValidator {
    fn validate(
        &self,
        descriptor: &AgentRuntimeDescriptor,
        path: &Path,
    ) -> Result<ValidatedExecutable, AgentRuntimeDiagnostic> {
        let metadata = fs::metadata(path).map_err(|error| {
            AgentRuntimeDiagnostic::new(
                "executable-not-found",
                format!("executable does not exist at {}: {error}", path.display()),
            )
        })?;
        if !metadata.is_file() {
            return Err(AgentRuntimeDiagnostic::new(
                "executable-not-file",
                format!("selected path is not a file: {}", path.display()),
            ));
        }
        if !file_is_executable(&metadata) {
            return Err(AgentRuntimeDiagnostic::new(
                "executable-permission-denied",
                format!("selected file is not executable: {}", path.display()),
            ));
        }

        let canonical_path = path.canonicalize().map_err(|error| {
            AgentRuntimeDiagnostic::new(
                "executable-path-unresolved",
                format!("failed to resolve executable {}: {error}", path.display()),
            )
        })?;
        let version = probe_version(&canonical_path, descriptor.version_args)?;
        Ok(ValidatedExecutable {
            path: canonical_path,
            version,
        })
    }
}

#[cfg(unix)]
fn file_is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn file_is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

fn resolve_descriptor(
    descriptor: &AgentRuntimeDescriptor,
    settings: &AppSettings,
) -> AgentRuntime {
    let configured = settings
        .agent_runtimes
        .by_provider
        .get(descriptor.provider_id)
        .and_then(|preference| preference.configured_executable.as_deref())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    resolve_descriptor_with(
        descriptor,
        configured,
        automatic_candidates(descriptor),
        &SystemExecutableValidator,
    )
}

fn resolve_descriptor_with(
    descriptor: &AgentRuntimeDescriptor,
    configured: Option<String>,
    automatic_candidates: Vec<RuntimeCandidate>,
    validator: &dyn ExecutableValidator,
) -> AgentRuntime {
    if let Some(configured_path) = configured.as_deref() {
        return match validator.validate(descriptor, Path::new(configured_path)) {
            Ok(validated) => ready_runtime(
                descriptor,
                validated,
                AgentRuntimeSource::Configured,
                configured,
            ),
            Err(diagnostic) => AgentRuntime {
                provider_id: descriptor.provider_id.to_string(),
                display_name: descriptor.display_name.to_string(),
                status: AgentRuntimeStatus::InvalidConfiguredExecutable,
                resolved_executable: None,
                source: Some(AgentRuntimeSource::Configured),
                configured_executable: configured,
                version: None,
                diagnostic: Some(diagnostic),
                installed: Vec::new(),
            },
        };
    }

    let mut rejected_candidates = Vec::new();
    for candidate in deduplicate_candidates(automatic_candidates) {
        match validator.validate(descriptor, &candidate.path) {
            Ok(validated) => {
                return ready_runtime(descriptor, validated, candidate.source, None)
            }
            Err(diagnostic) => rejected_candidates.push((candidate.path, diagnostic)),
        }
    }

    let diagnostic = if rejected_candidates.is_empty() {
        AgentRuntimeDiagnostic::new(
            "runtime-not-found",
            format!(
                "{} was not found in the current PATH, login shell, or supported application installations",
                descriptor.display_name
            ),
        )
    } else {
        let attempts = rejected_candidates
            .iter()
            .take(4)
            .map(|(path, diagnostic)| format!("{} ({})", path.display(), diagnostic.message))
            .collect::<Vec<_>>()
            .join("; ");
        AgentRuntimeDiagnostic::new(
            "runtime-candidates-invalid",
            format!(
                "{} candidates were found but failed validation: {attempts}",
                descriptor.display_name
            ),
        )
    };
    AgentRuntime {
        provider_id: descriptor.provider_id.to_string(),
        display_name: descriptor.display_name.to_string(),
        status: AgentRuntimeStatus::Unavailable,
        resolved_executable: None,
        source: None,
        configured_executable: None,
        version: None,
        diagnostic: Some(diagnostic),
        installed: Vec::new(),
    }
}

fn configure_runtime_in_settings(
    descriptor: &AgentRuntimeDescriptor,
    settings: &mut AppSettings,
    executable: &str,
    validator: &dyn ExecutableValidator,
) -> Result<AgentRuntime, AgentRuntimeDiagnostic> {
    let validated = validator.validate(descriptor, Path::new(executable))?;
    let configured = validated.path.to_string_lossy().to_string();
    settings
        .agent_runtimes
        .by_provider
        .entry(descriptor.provider_id.to_string())
        .or_insert_with(AgentRuntimePreferenceSettings::default)
        .configured_executable = Some(configured.clone());
    Ok(ready_runtime(
        descriptor,
        validated,
        AgentRuntimeSource::Configured,
        Some(configured),
    ))
}

fn ready_runtime(
    descriptor: &AgentRuntimeDescriptor,
    validated: ValidatedExecutable,
    source: AgentRuntimeSource,
    configured_executable: Option<String>,
) -> AgentRuntime {
    AgentRuntime {
        provider_id: descriptor.provider_id.to_string(),
        display_name: descriptor.display_name.to_string(),
        status: AgentRuntimeStatus::Ready,
        resolved_executable: Some(validated.path.to_string_lossy().to_string()),
        source: Some(source),
        configured_executable,
        version: validated.version,
        diagnostic: None,
        installed: Vec::new(),
    }
}

fn automatic_candidates(descriptor: &AgentRuntimeDescriptor) -> Vec<RuntimeCandidate> {
    let mut candidates = Vec::new();
    if let Some(variable) = descriptor.environment_variable {
        if let Some(path) = env::var_os(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            candidates.push(RuntimeCandidate {
                path,
                source: AgentRuntimeSource::Environment,
            });
        }
    }
    append_current_path_candidates(&mut candidates, descriptor);
    append_login_shell_candidates(&mut candidates, descriptor);
    append_macos_application_candidates(&mut candidates, descriptor);
    append_windows_application_candidates(&mut candidates, descriptor);
    candidates
}

fn append_current_path_candidates(
    candidates: &mut Vec<RuntimeCandidate>,
    descriptor: &AgentRuntimeDescriptor,
) {
    let Some(path_value) = env::var_os("PATH") else {
        return;
    };
    for directory in env::split_paths(&path_value) {
        for command_name in descriptor.command_names {
            for filename in executable_filenames(command_name) {
                let path = directory.join(filename);
                if path.exists() {
                    candidates.push(RuntimeCandidate {
                        path,
                        source: AgentRuntimeSource::CurrentPath,
                    });
                }
            }
        }
    }
}

#[cfg(windows)]
fn executable_filenames(command_name: &str) -> Vec<String> {
    [".exe", ".cmd", ".bat", ""]
        .iter()
        .map(|extension| format!("{command_name}{extension}"))
        .collect()
}

#[cfg(not(windows))]
fn executable_filenames(command_name: &str) -> Vec<String> {
    vec![command_name.to_string()]
}

#[cfg(unix)]
fn append_login_shell_candidates(
    candidates: &mut Vec<RuntimeCandidate>,
    descriptor: &AgentRuntimeDescriptor,
) {
    let Some(shell) = login_shell_path() else {
        return;
    };
    for command_name in descriptor.command_names {
        if !command_name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            continue;
        }
        let mut command = Command::new(&shell);
        command.args(["-l", "-c", &format!("command -v -- {command_name}")]);
        let Ok(output) = run_captured_command(command, DISCOVERY_COMMAND_TIMEOUT) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Some(path) = output
            .stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
        {
            let path = PathBuf::from(path);
            if path.exists() {
                candidates.push(RuntimeCandidate {
                    path,
                    source: AgentRuntimeSource::LoginShell,
                });
            }
        }
    }
}

#[cfg(not(unix))]
fn append_login_shell_candidates(
    _candidates: &mut Vec<RuntimeCandidate>,
    _descriptor: &AgentRuntimeDescriptor,
) {
}

#[cfg(unix)]
fn login_shell_path() -> Option<PathBuf> {
    if let Some(shell) = env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Some(shell);
    }
    macos_login_shell_path()
}

#[cfg(target_os = "macos")]
fn macos_login_shell_path() -> Option<PathBuf> {
    let user = env::var("USER").ok()?.trim().to_string();
    if user.is_empty() {
        return None;
    }
    let mut command = Command::new("/usr/bin/dscl");
    command.args([
        ".",
        "-read",
        &format!("/Users/{user}"),
        "UserShell",
    ]);
    let output = run_captured_command(command, DISCOVERY_COMMAND_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    output
        .stdout
        .split_whitespace()
        .last()
        .map(PathBuf::from)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn macos_login_shell_path() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn append_macos_application_candidates(
    candidates: &mut Vec<RuntimeCandidate>,
    descriptor: &AgentRuntimeDescriptor,
) {
    for bundle in descriptor.macos_bundles {
        let bundle_identifier = bundle.bundle_identifier.replace('\'', "\\'");
        let mut command = Command::new("/usr/bin/mdfind");
        command.arg(format!(
            "kMDItemCFBundleIdentifier == '{bundle_identifier}'"
        ));
        let Ok(output) = run_captured_command(command, DISCOVERY_COMMAND_TIMEOUT) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for application_path in output.stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
            for relative_executable in bundle.relative_executables {
                let path = Path::new(application_path).join(relative_executable);
                if path.exists() {
                    candidates.push(RuntimeCandidate {
                        path,
                        source: AgentRuntimeSource::MacosApplication,
                    });
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn append_macos_application_candidates(
    _candidates: &mut Vec<RuntimeCandidate>,
    _descriptor: &AgentRuntimeDescriptor,
) {
}

#[cfg(windows)]
fn append_windows_application_candidates(
    candidates: &mut Vec<RuntimeCandidate>,
    descriptor: &AgentRuntimeDescriptor,
) {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return;
    };
    append_windows_layout_candidates(candidates, descriptor, &local_app_data);
}

#[cfg(not(windows))]
fn append_windows_application_candidates(
    _candidates: &mut Vec<RuntimeCandidate>,
    _descriptor: &AgentRuntimeDescriptor,
) {
}

#[cfg(windows)]
fn append_windows_layout_candidates(
    candidates: &mut Vec<RuntimeCandidate>,
    descriptor: &AgentRuntimeDescriptor,
    local_app_data: &Path,
) {
    for layout in descriptor.windows_layouts {
        match layout {
            WindowsInstallLayout::VersionedChildren { parent, executable } => {
                let parent = join_segments(local_app_data, parent);
                for child in child_directories_newest_first(&parent) {
                    let path = child.join(executable);
                    if path.exists() {
                        candidates.push(RuntimeCandidate {
                            path,
                            source: AgentRuntimeSource::WindowsApplication,
                        });
                    }
                }
            }
            WindowsInstallLayout::PrefixedDirectory {
                parent,
                prefix,
                relative_executable,
            } => {
                let parent = join_segments(local_app_data, parent);
                for child in child_directories_newest_first(&parent) {
                    let matches_prefix = child
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(prefix));
                    if matches_prefix {
                        let path = join_segments(&child, relative_executable);
                        if path.exists() {
                            candidates.push(RuntimeCandidate {
                                path,
                                source: AgentRuntimeSource::WindowsApplication,
                            });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn join_segments(base: &Path, segments: &[&str]) -> PathBuf {
    segments
        .iter()
        .fold(base.to_path_buf(), |path, segment| path.join(segment))
}

#[cfg(windows)]
fn child_directories_newest_first(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut directories = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        let left_modified = left.metadata().and_then(|metadata| metadata.modified()).ok();
        let right_modified = right.metadata().and_then(|metadata| metadata.modified()).ok();
        right_modified.cmp(&left_modified)
    });
    directories
}

fn deduplicate_candidates(candidates: Vec<RuntimeCandidate>) -> Vec<RuntimeCandidate> {
    let mut deduplicated = Vec::<RuntimeCandidate>::new();
    for candidate in candidates {
        if !deduplicated
            .iter()
            .any(|existing| existing.path == candidate.path)
        {
            deduplicated.push(candidate);
        }
    }
    deduplicated
}

fn probe_version(
    executable: &Path,
    version_args: &[&str],
) -> Result<Option<String>, AgentRuntimeDiagnostic> {
    let mut command = Command::new(executable);
    command.args(version_args);
    let output = run_captured_command(command, VERSION_PROBE_TIMEOUT).map_err(|error| {
        let code = if error.kind() == io::ErrorKind::TimedOut {
            "version-probe-timeout"
        } else {
            "version-probe-failed"
        };
        AgentRuntimeDiagnostic::new(
            code,
            format!(
                "failed to run {} {}: {error}",
                executable.display(),
                version_args.join(" ")
            ),
        )
    })?;
    if !output.status.success() {
        let detail = first_nonempty_line(&output.stderr)
            .or_else(|| first_nonempty_line(&output.stdout))
            .unwrap_or("no diagnostic output");
        return Err(AgentRuntimeDiagnostic::new(
            "version-probe-rejected",
            format!(
                "{} {} exited with {}: {}",
                executable.display(),
                version_args.join(" "),
                output.status,
                truncate_text(detail, 240)
            ),
        ));
    }
    let version = first_nonempty_line(&output.stdout)
        .or_else(|| first_nonempty_line(&output.stderr))
        .map(|line| truncate_text(line, 240));
    Ok(version)
}

fn first_nonempty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

fn truncate_text(value: &str, max_characters: usize) -> String {
    let mut characters = value.chars();
    let truncated = characters.by_ref().take(max_characters).collect::<String>();
    if characters.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

struct CapturedCommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_captured_command(
    mut command: Command,
    timeout: Duration,
) -> io::Result<CapturedCommandOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = read_child_pipe(child.stdout.take())?;
            let stderr = read_child_pipe(child.stderr.take())?;
            return Ok(CapturedCommandOutput {
                status,
                stdout,
                stderr,
            });
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command did not finish within {} ms", timeout.as_millis()),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_child_pipe<T: Read>(pipe: Option<T>) -> io::Result<String> {
    let Some(mut pipe) = pipe else {
        return Ok(String::new());
    };
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeValidator {
        outcomes: HashMap<PathBuf, Result<ValidatedExecutable, AgentRuntimeDiagnostic>>,
    }

    impl FakeValidator {
        fn valid(mut self, path: &str, version: &str) -> Self {
            self.outcomes.insert(
                PathBuf::from(path),
                Ok(ValidatedExecutable {
                    path: PathBuf::from(path),
                    version: Some(version.to_string()),
                }),
            );
            self
        }

        fn invalid(mut self, path: &str, message: &str) -> Self {
            self.outcomes.insert(
                PathBuf::from(path),
                Err(AgentRuntimeDiagnostic::new(
                    "fixture-invalid",
                    message,
                )),
            );
            self
        }
    }

    impl ExecutableValidator for FakeValidator {
        fn validate(
            &self,
            _descriptor: &AgentRuntimeDescriptor,
            path: &Path,
        ) -> Result<ValidatedExecutable, AgentRuntimeDiagnostic> {
            self.outcomes.get(path).cloned().unwrap_or_else(|| {
                Err(AgentRuntimeDiagnostic::new(
                    "fixture-missing",
                    format!("no fixture for {}", path.display()),
                ))
            })
        }
    }

    fn candidate(path: &str, source: AgentRuntimeSource) -> RuntimeCandidate {
        RuntimeCandidate {
            path: PathBuf::from(path),
            source,
        }
    }

    #[test]
    fn configured_executable_has_priority_over_automatic_candidates() {
        let descriptor = runtime_descriptor(CODEX_RUNTIME_PROVIDER_ID).unwrap();
        let validator = FakeValidator::default()
            .valid("/manual/codex", "codex 2.0")
            .valid("/auto/codex", "codex 1.0");

        let runtime = resolve_descriptor_with(
            descriptor,
            Some("/manual/codex".to_string()),
            vec![candidate(
                "/auto/codex",
                AgentRuntimeSource::CurrentPath,
            )],
            &validator,
        );

        assert_eq!(runtime.status, AgentRuntimeStatus::Ready);
        assert_eq!(runtime.source, Some(AgentRuntimeSource::Configured));
        assert_eq!(
            runtime.resolved_executable.as_deref(),
            Some("/manual/codex")
        );
        assert_eq!(runtime.version.as_deref(), Some("codex 2.0"));
    }

    #[test]
    fn no_automatic_candidate_returns_a_diagnostic_runtime() {
        let descriptor = runtime_descriptor(OPENCODE_RUNTIME_PROVIDER_ID).unwrap();

        let runtime = resolve_descriptor_with(
            descriptor,
            None,
            Vec::new(),
            &FakeValidator::default(),
        );

        assert_eq!(runtime.status, AgentRuntimeStatus::Unavailable);
        assert!(runtime.resolved_executable.is_none());
        assert_eq!(
            runtime.diagnostic.as_ref().map(|value| value.code.as_str()),
            Some("runtime-not-found")
        );
    }

    #[test]
    fn invalid_configured_executable_does_not_fall_back_to_automatic_detection() {
        let descriptor = runtime_descriptor(CODEX_RUNTIME_PROVIDER_ID).unwrap();
        let validator = FakeValidator::default()
            .invalid("/manual/broken-codex", "manual runtime is broken")
            .valid("/auto/codex", "codex 1.0");

        let runtime = resolve_descriptor_with(
            descriptor,
            Some("/manual/broken-codex".to_string()),
            vec![candidate(
                "/auto/codex",
                AgentRuntimeSource::CurrentPath,
            )],
            &validator,
        );

        assert_eq!(
            runtime.status,
            AgentRuntimeStatus::InvalidConfiguredExecutable
        );
        assert_eq!(runtime.source, Some(AgentRuntimeSource::Configured));
        assert_eq!(
            runtime.configured_executable.as_deref(),
            Some("/manual/broken-codex")
        );
        assert!(runtime.resolved_executable.is_none());
    }

    #[test]
    fn invalid_manual_path_returns_a_specific_file_diagnostic() {
        let descriptor = runtime_descriptor(CLAUDE_RUNTIME_PROVIDER_ID).unwrap();
        let missing = tempfile::tempdir()
            .unwrap()
            .path()
            .join("missing-claude");

        let runtime = resolve_descriptor_with(
            descriptor,
            Some(missing.to_string_lossy().to_string()),
            Vec::new(),
            &SystemExecutableValidator,
        );

        assert_eq!(
            runtime.status,
            AgentRuntimeStatus::InvalidConfiguredExecutable
        );
        assert_eq!(
            runtime.diagnostic.as_ref().map(|value| value.code.as_str()),
            Some("executable-not-found")
        );
    }

    #[test]
    fn rejected_manual_path_does_not_replace_the_existing_setting() {
        let descriptor = runtime_descriptor(CODEX_RUNTIME_PROVIDER_ID).unwrap();
        let mut settings = AppSettings::default();
        settings
            .agent_runtimes
            .by_provider
            .entry(CODEX_RUNTIME_PROVIDER_ID.to_string())
            .or_default()
            .configured_executable = Some("/existing/codex".to_string());
        let validator = FakeValidator::default()
            .invalid("/replacement/broken-codex", "replacement is invalid");

        let result = configure_runtime_in_settings(
            descriptor,
            &mut settings,
            "/replacement/broken-codex",
            &validator,
        );

        assert!(result.is_err());
        assert_eq!(
            settings.agent_runtimes.by_provider[CODEX_RUNTIME_PROVIDER_ID]
                .configured_executable
                .as_deref(),
            Some("/existing/codex")
        );
    }

    #[test]
    fn automatic_detection_skips_invalid_candidates() {
        let descriptor = runtime_descriptor(CODEX_RUNTIME_PROVIDER_ID).unwrap();
        let validator = FakeValidator::default()
            .invalid("/path/codex", "broken PATH wrapper")
            .valid("/app/codex", "codex 3.0");

        let runtime = resolve_descriptor_with(
            descriptor,
            None,
            vec![
                candidate("/path/codex", AgentRuntimeSource::CurrentPath),
                candidate("/app/codex", AgentRuntimeSource::MacosApplication),
            ],
            &validator,
        );

        assert_eq!(runtime.status, AgentRuntimeStatus::Ready);
        assert_eq!(
            runtime.source,
            Some(AgentRuntimeSource::MacosApplication)
        );
        assert_eq!(runtime.resolved_executable.as_deref(), Some("/app/codex"));
    }

    #[test]
    #[cfg(windows)]
    fn windows_descriptor_discovers_versioned_codex_installations() {
        let descriptor = runtime_descriptor(CODEX_RUNTIME_PROVIDER_ID).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let executable = temp
            .path()
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("hash")
            .join("codex.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"").unwrap();
        let mut candidates = Vec::new();

        append_windows_layout_candidates(&mut candidates, descriptor, temp.path());

        assert!(candidates.iter().any(|candidate| candidate.path == executable));
    }
}
