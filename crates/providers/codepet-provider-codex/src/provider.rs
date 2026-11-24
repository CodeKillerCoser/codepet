mod project_membership;
use project_membership::*;
mod runtime;
use runtime::*;
mod dispatch;
mod workspaces;
use workspaces::{codex_home, prepare_conversation_workspace};
mod account_metadata;
use account_metadata::{codex_authentication, codex_usage};

mod directory_list;
mod hook_observation;
mod ordered_ids;
use crate::client::{CodexAppServerSession, CodexRequestOutcome};
use crate::mapper::{parse_permission_level, CodexProtocolMapper};
use crate::protocol::{
    approval_generation, approval_resource_id, CodexAppServerError, CodexApprovalRequest,
    CodexConversationSnapshot, CodexIncoming, CodexNotification, CodexProjectCreateRequest,
    CodexProjectRoot, CodexProjectUpdateRequest, CodexThreadListRequest, CodexThreadStartRequest,
    CodexTurn, CodexTurnItemsView, CodexTurnStartRequest, CodexTurnStatus, CodexTurnSteerRequest,
    CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID,
};
use codepet_provider_data::conversation_atoms::{self, ConversationAtoms};
use codepet_provider_sdk::local_runtime;
use codepet_provider_sdk::{
    Approval, ApprovalRequestedEvent, ApprovalResolveRequest, ApprovalResolveResponse,
    Conversation, ConversationAcquireInteractionRequest, ConversationAcquireInteractionResponse,
    ConversationCreateRequest, ConversationCreateResponse, ConversationGetRequest,
    ConversationGetResponse, ConversationListRequest, ConversationListResponse,
    ConversationProjectFilter, ConversationSearchRequest, ConversationSearchResponse,
    ConversationUpsertedEvent, FlatModelCatalogKind, FlatModelSelection, HarnessDescriptor,
    InstanceCapabilitiesRequest, InstanceCapabilitiesResponse, InstanceCreateRequest,
    InstanceCreateResponse, InstanceDestroyRequest, InstanceDestroyResponse, InstanceStartRequest,
    InstanceStartResponse, InstanceStatus, InstanceStatusChangedEvent, InstanceStopRequest,
    InstanceStopResponse, ModelCatalog, ModelSelection, PageInfo, ProjectCreateRequest,
    ProjectCreateResponse, ProjectDeleteRequest, ProjectDeleteResponse, ProjectGetRequest,
    ProjectGetResponse, ProjectListRequest, ProjectListResponse, ProjectUpdateRequest,
    ProjectUpdateResponse, ProtocolError, ProtocolEvent, ProtocolFuture, ProtocolVersionOffer,
    Provider, ProviderAuthentication, ProviderAuthenticationStatus, ProviderCapabilities,
    ProviderCapability, ProviderDescribeRequest, ProviderDescribeResponse, ProviderEventSink,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance, ProviderInstanceRoute,
    ProviderPluginDescriptor, ProviderResourceId, ProviderShutdownRequest,
    ProviderShutdownResponse, ProviderUsage, ProviderUsageDetail, RuntimeCandidate,
    RuntimeGetInstalledRequest, RuntimeGetInstalledResponse, RuntimeInstallation,
    RuntimeSelectRequest, RuntimeSelectResponse, TurnInterruptRequest, TurnInterruptResponse,
    TurnSelection, TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest,
    TurnSteerResponse, TurnTask, PROTOCOL_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod server_events;

static NEXT_EXECUTION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
// A forced global desktop shutdown must not race Codepet turn operations.
static DESKTOP_TAKEOVER_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());
static NEXT_INSTANCE_SESSION: AtomicU64 = AtomicU64::new(1);
static NEXT_MANAGED_WORKTREE: AtomicU64 = AtomicU64::new(1);
const DEFAULT_CONVERSATION_GET_TURN_LIMIT: u64 = 20;
const MAX_CONVERSATION_GET_TURN_LIMIT: u64 = 100;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct CodexInstanceSettings {
    app_server_executable: PathBuf,
    app_server_args: Vec<String>,
    #[serde(default, alias = "codexHome")]
    data_directory: Option<PathBuf>,
}

#[doc(hidden)]
pub trait ExecutionLifecycleHook: Send + Sync + 'static {
    fn close_desktop_for_takeover(
        &self,
        check_inactive: &mut dyn FnMut() -> Result<(), ProtocolError>,
    ) -> Result<(), ProtocolError> {
        crate::desktop_takeover::close_desktop(check_inactive)
    }
    fn after_handle_acquired(&self, _conversation_id: &str, _operation: &str) {}

    fn before_resume_linearization(&self, _conversation_id: &str) {}

    fn after_execution_cancelled(&self, _conversation_id: &str) {}
}

struct NoopExecutionLifecycleHook;

impl ExecutionLifecycleHook for NoopExecutionLifecycleHook {}

struct ProviderState {
    host_device_id: Option<String>,
    initialized_client_id: Option<String>,
    instances: HashMap<String, Arc<CodexInstanceRuntime>>,
}

pub struct CodexProvider {
    data: Arc<codepet_provider_data::ProviderData>,
    observation: Arc<codepet_observation::Observation>,
    scanner: local_runtime::RuntimeScanner,
    state: Arc<Mutex<ProviderState>>,
    events: Arc<dyn ProviderEventSink>,
    lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
    shutdown: Arc<AtomicBool>,
    shutdown_complete: AtomicBool,
    shutdown_changed: tokio::sync::Notify,
}

impl CodexProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self::new_with_execution_lifecycle_hook(events, Arc::new(NoopExecutionLifecycleHook))
    }

    #[doc(hidden)]
    pub fn new_with_execution_lifecycle_hook(
        events: Arc<dyn ProviderEventSink>,
        lifecycle_hook: Arc<dyn ExecutionLifecycleHook>,
    ) -> Self {
        let data = Arc::new(codepet_provider_data::ProviderData::default());
        let events: Arc<dyn ProviderEventSink> = Arc::new(codepet_provider_data::UsageSink {
            data: data.clone(),
            downstream: events,
            provider: "codex",
        });
        let state = Arc::new(Mutex::new(ProviderState {
            host_device_id: None,
            initialized_client_id: None,
            instances: HashMap::new(),
        }));
        let hook_events = Arc::new(hook_observation::HookEvents {
            state: Arc::downgrade(&state),
        });
        Self {
            data,
            scanner: local_runtime::RuntimeScanner::new(events.clone()),
            observation: Arc::new(
                codepet_observation::Observation::new(
                    codepet_observation::Definition {
                        windows_command_override: true,
                        name: "codex",
                        config: codepet_observation::config_home(
                            "CODEX_HOME",
                            codepet_observation::home().join(".codex"),
                        )
                        .join("hooks.json"),
                        events: &[
                            "SessionStart",
                            "SessionEnd",
                            "UserPromptSubmit",
                            "PreToolUse",
                            "PostToolUse",
                            "PermissionRequest",
                            "Stop",
                            "SubagentStart",
                            "SubagentStop",
                            "Interrupt",
                        ],
                        plugin: None,
                    },
                    events.clone(),
                )
                .with_observer(hook_events),
            ),
            state,
            events,
            lifecycle_hook,
            shutdown: Arc::new(AtomicBool::new(false)),
            shutdown_complete: AtomicBool::new(false),
            shutdown_changed: tokio::sync::Notify::new(),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn descriptor() -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            features: Some(vec![
                codepet_provider_sdk::ProviderFeature::Agent,
                codepet_provider_sdk::ProviderFeature::Observation,
                codepet_provider_sdk::ProviderFeature::RuntimeInventory,
            ]),
            plugin_id: CODEX_PLUGIN_ID.to_string(),
            display_name: "Codex".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_workspace_root: codepet_paths::remote_workspace_for_user("codex"),
            protocol: ProtocolVersionOffer {
                min_version: codepet_provider_sdk::MIN_PROTOCOL_VERSION,
                version: PROTOCOL_VERSION,
            },
            instance_kinds: vec![CODEX_INSTANCE_KIND.to_string()],
        }
    }

    fn instance(
        &self,
        route: &ProviderInstanceRoute,
    ) -> Result<Arc<CodexInstanceRuntime>, ProtocolError> {
        validate_route(route)?;
        let state = lock(&self.state);
        let expected_device = state.host_device_id.as_deref().ok_or_else(|| {
            protocol_error(
                "provider_not_initialized",
                "Provider must be initialized before using instances".to_string(),
                false,
            )
        })?;
        if route.device_id != expected_device {
            return Err(protocol_error(
                "wrong_device_route",
                "Provider route targets a different Host device".to_string(),
                false,
            ));
        }
        state
            .instances
            .get(&route.provider_instance_id)
            .cloned()
            .ok_or_else(|| {
                protocol_error(
                    "unknown_provider_instance",
                    format!(
                        "unknown Codex Provider instance: {}",
                        route.provider_instance_id
                    ),
                    false,
                )
            })
    }

    fn resource_instance(
        &self,
        resource: &ProviderResourceId,
    ) -> Result<Arc<CodexInstanceRuntime>, ProtocolError> {
        validate_resource(resource)?;
        self.instance(&ProviderInstanceRoute {
            device_id: resource.device_id.clone(),
            provider_plugin_id: resource.provider_plugin_id.clone(),
            provider_instance_id: resource.provider_instance_id.clone(),
        })
    }
}

impl CodexProvider {
    async fn start_runtime(
        &self,
        runtime: Arc<CodexInstanceRuntime>,
        explicit_resume: bool,
    ) -> Result<InstanceStartResponse, ProtocolError> {
        Self::start_runtime_owned(
            runtime,
            explicit_resume,
            self.observation.clone(),
            self.shutdown.clone(),
        )
        .await
    }

    // The release task owns this context so disconnecting its caller cannot
    // cancel cleanup or strand the instance between stop and restart.
    async fn start_runtime_owned(
        runtime: Arc<CodexInstanceRuntime>,
        explicit_resume: bool,
        observation: Arc<codepet_observation::Observation>,
        shutdown: Arc<AtomicBool>,
    ) -> Result<InstanceStartResponse, ProtocolError> {
        let (slot, starting_event_error) = {
            let _transition = lock(&runtime.lifecycle_transition);
            let mut mutable = lock(&runtime.mutable);
            if shutdown.load(Ordering::SeqCst) {
                return Err(protocol_error(
                    "provider_shutdown",
                    "Provider is shutting down".into(),
                    false,
                ));
            }
            if mutable.destroyed {
                return Err(protocol_error(
                    "unknown_provider_instance",
                    format!(
                        "unknown Codex Provider instance: {}",
                        runtime.route.provider_instance_id
                    ),
                    false,
                ));
            }
            if mutable.cleanup_in_progress {
                return Err(protocol_error(
                    "provider_instance_stopping",
                    "Codex Provider instance is cleaning up a failed lifecycle".to_string(),
                    true,
                ));
            }
            if !mutable.failed_shutdown_sessions.is_empty() {
                return Err(protocol_error(
                    "interaction_release_failed",
                    "Previous harness termination failed; retry release first".into(),
                    true,
                ));
            }
            match mutable.status.clone() {
                InstanceStatus::Ready => {
                    if explicit_resume {
                        mutable.interaction_released = false;
                    }
                    drop(mutable);
                    return Ok(InstanceStartResponse {
                        instance: runtime.snapshot(),
                    });
                }
                InstanceStatus::Starting => {
                    if explicit_resume {
                        mutable.interaction_released = false;
                    }
                    drop(mutable);
                    return Ok(InstanceStartResponse {
                        instance: runtime.snapshot(),
                    });
                }
                InstanceStatus::Stopping => {
                    return Err(protocol_error(
                        "provider_instance_stopping",
                        "Codex Provider instance is stopping".to_string(),
                        true,
                    ));
                }
                _ => {}
            }
            if explicit_resume {
                mutable.interaction_released = false;
            }
            let previous = mutable.status.clone();
            CodexInstanceRuntime::cancel_metadata(&mut mutable);
            mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.status = InstanceStatus::Starting;
            mutable.server_session_id = None;
            mutable.server_generation = None;
            mutable.pending_approvals.clear();
            mutable.approval_history.clear();
            mutable.pending_materialization.clear();
            mutable.observed_thread_ids.clear();
            mutable.auto_title_attempted.clear();
            let slot = Arc::new(InstanceSessionSlot::new(mutable.lifecycle_generation));
            mutable.sessions.insert(slot.id, slot.clone());
            drop(mutable);
            let event_error = runtime.publish_status_change(previous).err();
            (slot, event_error)
        };
        if let Some(error) = starting_event_error {
            let _ = runtime.mark_start_failed(&slot);
            return Err(error);
        }
        // Reserve the cancellable instance slot before installing the
        // independent Hook source; stop must win while installation awaits.
        let hook_result = observation
            .subscribe(hook_observation::INTERNAL_SUBSCRIPTION.into())
            .await;
        let hooks_ready = hook_result.is_ok();
        if let Err(error) = hook_result {
            eprintln!("Codex activity observation unavailable: {}", error.message);
        }
        if runtime.hook_ready.swap(hooks_ready, Ordering::SeqCst) != hooks_ready {
            let status = {
                let mut state = lock(&runtime.mutable);
                state.atomic_facts_epoch = state.atomic_facts_epoch.saturating_add(1);
                state.status.clone()
            };
            if status == InstanceStatus::Ready {
                let _ = runtime.publish_status_change(status);
            }
        }
        if !slot.begin_spawn() {
            runtime.unregister_session(&slot);
            return Err(instance_session_cancelled_error("server session"));
        }
        let executable = runtime.settings.app_server_executable.clone();
        let args = runtime.settings.app_server_args.clone();
        let data_directory = runtime.settings.data_directory.clone();
        let server = match tokio::task::spawn_blocking(move || {
            CodexAppServerSession::spawn_uninitialized(
                &executable,
                &args,
                data_directory.as_deref(),
            )
        })
        .await
        {
            Ok(Ok(server)) => server,
            Ok(Err(error)) => {
                let mapped = CodexProtocolMapper::error(error);
                if runtime.mark_start_failed(&slot)? {
                    return Err(mapped);
                }
                return Err(instance_session_cancelled_error("server session"));
            }
            Err(error) => {
                let mapped = protocol_error(
                    "provider_task_failed",
                    format!("Codex App Server start task failed: {error}"),
                    true,
                );
                if runtime.mark_start_failed(&slot)? {
                    return Err(mapped);
                }
                return Err(instance_session_cancelled_error("server session"));
            }
        };
        if !slot.register_spawned(server.clone()) || !runtime.session_is_current(&slot) {
            let _ = server.shutdown();
            runtime.unregister_session(&slot);
            return Err(instance_session_cancelled_error("server session"));
        }
        let initialize_session = server.clone();
        let initialize_result =
            tokio::task::spawn_blocking(move || initialize_session.initialize())
                .await
                .map_err(provider_task_error)?;
        if let Err(error) = initialize_result {
            let mapped = CodexProtocolMapper::error(error);
            let _ = server.shutdown();
            if runtime.mark_start_failed(&slot)? {
                return Err(mapped);
            }
            return Err(instance_session_cancelled_error("server session"));
        }
        let harness = HarnessDescriptor {
            id: CODEX_INSTANCE_KIND.to_string(),
            display_name: "Codex".to_string(),
            version: server.harness_version(),
            executable_path: Some(
                runtime
                    .settings
                    .app_server_executable
                    .to_string_lossy()
                    .into_owned(),
            ),
        };
        let incoming = match server.subscribe() {
            Ok(incoming) => incoming,
            Err(error) => {
                let mapped = CodexProtocolMapper::error(error);
                let _ = server.shutdown();
                if runtime.mark_start_failed(&slot)? {
                    return Err(mapped);
                }
                return Err(instance_session_cancelled_error("server session"));
            }
        };
        let server_generation = server.generation().to_string();
        let installed = {
            let _transition = lock(&runtime.lifecycle_transition);
            let mut mutable = lock(&runtime.mutable);
            let current = mutable.status == InstanceStatus::Starting
                && mutable.lifecycle_generation == slot.lifecycle_generation
                && mutable
                    .sessions
                    .get(&slot.id)
                    .is_some_and(|current| Arc::ptr_eq(current, &slot))
                && !slot.cancelled.load(Ordering::SeqCst)
                && mutable.server_session_id.is_none()
                && mutable.executions.is_empty();
            if !current {
                false
            } else {
                mutable.capabilities =
                    CodexProtocolMapper::unavailable_capabilities(server_generation.clone());
                mutable.harness = harness;
                mutable.authentication = None;
                mutable.usage = None;
                mutable.server_session_id = Some(slot.id);
                mutable.server_generation = Some(server_generation.clone());
                mutable.pending_approvals.clear();
                mutable.approval_history.clear();
                mutable.pending_materialization.clear();
                mutable.observed_thread_ids.clear();
                mutable.auto_title_attempted.clear();
                runtime.lifecycle_changed.notify_all();
                true
            }
        };
        if !installed {
            let _ = server.shutdown();
            runtime.unregister_session(&slot);
            return Err(instance_session_cancelled_error("server session"));
        }
        runtime.start_server_forwarder(server_generation, server, incoming);
        let instance = runtime.snapshot();
        runtime.refresh_metadata();

        Ok(InstanceStartResponse { instance })
    }
}

fn execution_outcome_error<T>(
    operation: &str,
    conversation_id: Option<&str>,
    outcome: CodexRequestOutcome<T>,
) -> ProtocolError {
    match outcome {
        CodexRequestOutcome::ExplicitRpcReject(error @ CodexAppServerError::Rpc { .. })
            if operation == "thread/resume"
                && conversation_id.is_some_and(|conversation_id| {
                    is_active_writer_conflict(&error, conversation_id)
                }) =>
        {
            ProtocolError {
                code: "conversation_write_conflict".to_string(),
                message: "Conversation is being written by another runtime".to_string(),
                retryable: true,
                details: Some(
                    [
                        ("operation".to_string(), json!(operation)),
                        ("reason".to_string(), json!("owned-by-other-runtime")),
                    ]
                    .into_iter()
                    .collect(),
                ),
            }
        }
        CodexRequestOutcome::SentOutcomeUnknown(error)
            if matches!(operation, "turn/start" | "turn/steer") =>
        {
            let mut error = CodexProtocolMapper::error(error);
            error
                .details
                .get_or_insert_with(Default::default)
                .insert("outcome".into(), json!("unknown"));
            error
        }
        CodexRequestOutcome::NotSent(error)
        | CodexRequestOutcome::ExplicitRpcReject(error)
        | CodexRequestOutcome::SentOutcomeUnknown(error) => CodexProtocolMapper::error(error),
        CodexRequestOutcome::Success(_) => unreachable!("successful outcome is not an error"),
    }
}

struct LoadedConversationTurns {
    turns: Vec<CodexTurn>,
    next_cursor: Option<String>,
    materialized: bool,
}

fn load_conversation_turns(
    session: &CodexAppServerSession,
    conversation_id: &str,
    cursor: Option<String>,
    requested_limit: u64,
    used_pending_snapshot: bool,
) -> Result<LoadedConversationTurns, ProtocolError> {
    let page = match session.thread_turns_list_with_view(
        conversation_id,
        cursor.clone(),
        requested_limit as u32,
        CodexTurnItemsView::Full,
    ) {
        Ok(page) => page,
        Err(error)
            if cursor.is_none()
                && (error
                    .is_thread_turns_unavailable_before_first_user_message(conversation_id)
                    || used_pending_snapshot
                        && (error.is_thread_not_loaded(conversation_id)
                            || is_created_conversation_not_ready(&error, conversation_id))) =>
        {
            return Ok(LoadedConversationTurns {
                turns: Vec::new(),
                next_cursor: None,
                materialized: false,
            });
        }
        Err(error) => return Err(CodexProtocolMapper::error(error)),
    };
    if page.data.len() as u64 > requested_limit {
        return Err(protocol_error(
            "provider_protocol_error",
            format!(
                "thread/turns/list returned {} turns for limit {requested_limit}",
                page.data.len(),
            ),
            false,
        ));
    }
    Ok(LoadedConversationTurns {
        turns: page.data,
        next_cursor: page.next_cursor,
        materialized: true,
    })
}

fn discover_codex_candidates() -> Vec<RuntimeCandidate> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("CODE_PET_CODEX_BIN").filter(|value| !value.is_empty()) {
        candidates.push(local_runtime::candidate(
            PathBuf::from(path),
            codepet_provider_sdk::RuntimeCandidateSource::Environment,
        ));
    }
    candidates.extend(local_runtime::discover("codex", "@openai/codex"));
    #[cfg(target_os = "macos")]
    candidates.extend(macos_codex_candidates(Path::new("/Applications")));
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let mut roots = vec![local.join("OpenAI").join("Codex").join("bin")];
        if let Ok(packages) = std::fs::read_dir(local.join("Packages")) {
            for package in packages.flatten().filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("OpenAI.Codex_")
            }) {
                roots.push(
                    package
                        .path()
                        .join("LocalCache")
                        .join("Local")
                        .join("OpenAI")
                        .join("Codex")
                        .join("bin"),
                );
            }
        }
        for root in roots {
            candidates.push(local_runtime::candidate(
                root.join("codex.exe"),
                codepet_provider_sdk::RuntimeCandidateSource::WindowsApplication,
            ));
            if let Ok(versions) = std::fs::read_dir(root) {
                let mut versions: Vec<_> = versions.flatten().collect();
                versions.sort_by_key(|entry| {
                    std::cmp::Reverse(entry.metadata().and_then(|m| m.modified()).ok())
                });
                for version in versions {
                    candidates.push(local_runtime::candidate(
                        version.path().join("codex.exe"),
                        codepet_provider_sdk::RuntimeCandidateSource::WindowsApplication,
                    ));
                }
            }
        }
    }
    candidates
}

// Product bundle layouts belong to the Provider; shared discovery still owns
// PATH/npm lookup, executable validation and canonical-path deduplication.
#[cfg(target_os = "macos")]
fn macos_codex_candidates(applications: &Path) -> Vec<RuntimeCandidate> {
    ["Codex.app", "ChatGPT.app"]
        .into_iter()
        .flat_map(|app| {
            let resources = applications.join(app).join("Contents/Resources");
            ["codex", "codex-cli/CodexCLI.app/Contents/MacOS/codex"]
                .into_iter()
                .map(move |relative| {
                    local_runtime::candidate(
                        resources.join(relative),
                        codepet_provider_sdk::RuntimeCandidateSource::MacosApplication,
                    )
                })
        })
        .collect()
}

#[cfg(all(test, target_os = "macos"))]
mod macos_runtime_layout_tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn executable(path: &Path, version: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\n"),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    async fn inventory(root: PathBuf) -> RuntimeGetInstalledResponse {
        local_runtime::runtime_inventory(
            "codex",
            "@openai/codex",
            None,
            move || macos_codex_candidates(&root),
            |candidate, timeout| {
                inspect_runtime_candidate(
                    candidate,
                    "codex",
                    timeout,
                    local_runtime::RuntimeProbeControl::default(),
                )
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn old_codex_old_chatgpt_and_nested_cli_coexist() {
        let dir = tempfile::tempdir().unwrap();
        for (relative, version) in [
            ("Codex.app/Contents/Resources/codex", "0.150.0"),
            ("ChatGPT.app/Contents/Resources/codex", "0.151.0"),
            (
                "ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
                "0.166.0",
            ),
        ] {
            executable(&dir.path().join(relative), version);
        }
        let result = inventory(dir.path().to_path_buf()).await;
        assert_eq!(result.harness_list.len(), 3);
        assert!(result.scan_error.is_none(), "{:?}", result.scan_error);
        for version in ["0.150.0", "0.151.0", "0.166.0"] {
            assert!(result.harness_list.iter().any(|r| r.version == version));
        }
    }

    #[tokio::test]
    async fn nested_cli_is_found_without_legacy_entry_and_aliases_are_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("ChatGPT.app/Contents/Resources");
        let native = resources.join("codex-cli/CodexCLI.app/Contents/MacOS/codex");
        executable(&native, "0.166.0-alpha.5.1");
        let result = inventory(dir.path().to_path_buf()).await;
        assert_eq!(result.harness_list.len(), 1);
        assert_eq!(result.harness_list[0].version, "0.166.0-alpha.5.1");
        symlink(&native, resources.join("codex")).unwrap();
        assert_eq!(
            inventory(dir.path().to_path_buf()).await.harness_list.len(),
            1
        );
    }

    #[tokio::test]
    async fn missing_bundles_are_not_reported_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let result = inventory(dir.path().to_path_buf()).await;
        assert!(result.harness_list.is_empty());
        assert!(result.scan_error.is_none());
    }

    #[tokio::test]
    #[ignore = "requires locally installed ChatGPT; inventory only, no settings or model requests"]
    async fn native_chatgpt_discovery_with_minimal_path() {
        if std::env::var_os("CODEPET_MACOS_LAYOUT_PROBE").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", "provider::macos_runtime_layout_tests::native_chatgpt_discovery_with_minimal_path", "--nocapture"])
                .env("CODEPET_MACOS_LAYOUT_PROBE", "1")
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env_remove("CODE_PET_CODEX_BIN")
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout));
            return;
        }
        let result = local_runtime::runtime_inventory(
            "codex",
            "@openai/codex",
            None,
            discover_codex_candidates,
            |candidate, timeout| {
                inspect_runtime_candidate(
                    candidate,
                    "codex",
                    timeout,
                    local_runtime::RuntimeProbeControl::default(),
                )
            },
        )
        .await
        .unwrap();
        let native = std::fs::canonicalize("/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex").unwrap();
        assert!(result
            .harness_list
            .iter()
            .any(|r| Path::new(&r.executable_path) == native));
        for runtime in result.harness_list {
            println!("{} {}", runtime.executable_path, runtime.version);
        }
    }
}

fn inspect_runtime_candidate(
    candidate: RuntimeCandidate,
    product: &str,
    timeout: Duration,
    control: local_runtime::RuntimeProbeControl,
) -> Result<RuntimeInstallation, ProtocolError> {
    let canonical = local_runtime::resolve_executable(
        Path::new(&candidate.executable_path),
        "codex",
        "@openai/codex",
    )
    .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?;
    let line = bounded_runtime_version(&canonical, timeout, control)?;
    if line.is_empty() || !line.to_ascii_lowercase().contains(product) {
        return Err(protocol_error(
            "invalid_runtime_selection",
            format!("Runtime executable does not identify itself as {product}"),
            false,
        ));
    }
    let version = line
        .split_whitespace()
        .find(|part| {
            part.chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        })
        .unwrap_or(&line)
        .trim_start_matches('v')
        .to_string();
    Ok(RuntimeInstallation {
        minimum_version: None,
        incompatibility_reason: None,
        executable_path: canonical.to_string_lossy().into_owned(),
        version,
        source: candidate.source,
    })
}

fn bounded_runtime_version(
    executable: &Path,
    timeout: Duration,
    control: local_runtime::RuntimeProbeControl,
) -> Result<String, ProtocolError> {
    let mut child = codepet_provider_sdk::local_runtime::command(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            protocol_error(
                "invalid_runtime_selection",
                format!("Run runtime executable {}: {error}", executable.display()),
                false,
            )
        })?;
    control
        .track(child.control())
        .map_err(|error| protocol_error("runtime_scan_cancelled", error.to_string(), true))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().map_err(|error| {
                    protocol_error(
                        "invalid_runtime_selection",
                        format!("Read runtime version: {error}"),
                        false,
                    )
                })?;
                if !status.success() {
                    return Err(protocol_error(
                        "invalid_runtime_selection",
                        format!(
                            "Runtime executable {} rejected --version: {status}; {}",
                            executable.display(),
                            String::from_utf8_lossy(&output.stderr)
                                .chars()
                                .take(1024)
                                .collect::<String>()
                                .trim()
                        ),
                        false,
                    ));
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Ok(stdout
                    .lines()
                    .chain(stderr.lines())
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or_default()
                    .trim()
                    .to_string());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(protocol_error(
                    "invalid_runtime_selection",
                    "Runtime version probe timed out".to_string(),
                    true,
                ));
            }
            Err(error) => {
                return Err(protocol_error(
                    "invalid_runtime_selection",
                    format!("Inspect runtime version probe: {error}"),
                    false,
                ))
            }
        }
    }
}

fn is_created_conversation_not_ready(error: &CodexAppServerError, conversation_id: &str) -> bool {
    if error.is_thread_not_loaded(conversation_id) {
        return true;
    }
    let CodexAppServerError::Rpc { message, .. } = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    (message.contains("failed to read session metadata")
        && (message.contains("is empty")
            || message.contains("not found")
            || message.contains("no such file")))
        || message.contains("thread not found")
}

fn is_active_writer_conflict(error: &CodexAppServerError, conversation_id: &str) -> bool {
    let CodexAppServerError::Rpc {
        code,
        message,
        data,
    } = error
    else {
        return false;
    };
    *code == -32600
        && data.is_none()
        && message == &format!("thread {conversation_id} already has an active writer")
}

fn execution_start_cancelled_error() -> ProtocolError {
    protocol_error(
        "provider_unavailable",
        "Codex Provider stopped while the execution session was starting".to_string(),
        true,
    )
}

fn instance_session_cancelled_error(session: &str) -> ProtocolError {
    protocol_error(
        "provider_unavailable",
        format!("Codex Provider stopped while the {session} was starting"),
        true,
    )
}

fn incoming_conversation_id(incoming: &CodexIncoming) -> Option<&str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::ThreadStarted { snapshot }) => {
            Some(&snapshot.thread.id)
        }
        CodexIncoming::Notification(
            CodexNotification::ThreadNameUpdated { thread_id, .. }
            | CodexNotification::ThreadStatusChanged { thread_id, .. }
            | CodexNotification::TurnStarted { thread_id, .. }
            | CodexNotification::TurnError { thread_id, .. }
            | CodexNotification::TurnCompleted { thread_id, .. }
            | CodexNotification::ItemUpserted { thread_id, .. }
            | CodexNotification::OutputDelta { thread_id, .. }
            | CodexNotification::ServerRequestResolved { thread_id, .. },
        ) => Some(thread_id),
        CodexIncoming::ApprovalRequested(request) => Some(&request.thread_id),
        CodexIncoming::Notification(CodexNotification::ProjectChanged { .. })
        | CodexIncoming::Notification(CodexNotification::Unknown { .. })
        | CodexIncoming::UnsupportedServerRequest { .. } => None,
    }
}

fn incoming_active_turn_id<'a>(
    incoming: &'a CodexIncoming,
    session_generation: &str,
) -> Option<&'a str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::TurnStarted { turn, .. }) => Some(&turn.id),
        CodexIncoming::Notification(
            CodexNotification::ItemUpserted { turn_id, .. }
            | CodexNotification::OutputDelta { turn_id, .. },
        ) => Some(turn_id),
        CodexIncoming::ApprovalRequested(request)
            if request.session_generation == session_generation =>
        {
            Some(&request.turn_id)
        }
        _ => None,
    }
}

fn shutdown_sessions(sessions: Vec<CodexAppServerSession>) -> Result<(), CodexAppServerError> {
    let mut first_error = None;
    for session in sessions {
        if let Err(error) = session.shutdown() {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn record_approval(mutable: &mut InstanceMutable, item_id: String, approval: Approval) {
    if let Some(observed) = mutable.approval_history.iter_mut().find(|observed| {
        observed.approval.resource.native_resource_id == approval.resource.native_resource_id
    }) {
        observed.item_id = item_id;
        observed.approval = approval;
    } else {
        mutable
            .approval_history
            .push(ObservedApproval { item_id, approval });
    }
}

fn update_recorded_approval(mutable: &mut InstanceMutable, approval: &Approval) {
    if let Some(observed) = mutable.approval_history.iter_mut().find(|observed| {
        observed.approval.resource.native_resource_id == approval.resource.native_resource_id
    }) {
        observed.approval = approval.clone();
    }
}

fn permission_level_id(permission: crate::protocol::CodexPermissionLevel) -> &'static str {
    match permission {
        crate::protocol::CodexPermissionLevel::ReadOnly => "read-only",
        crate::protocol::CodexPermissionLevel::WorkspaceWrite => "workspace-write",
        crate::protocol::CodexPermissionLevel::FullAccess => "full-access",
    }
}

fn resolve_turn_selection(
    capabilities: &ProviderCapabilities,
    requested: TurnSelection,
    current: TurnSelection,
) -> Result<TurnSelection, ProtocolError> {
    let turn_send = capabilities.turn_send.as_ref().ok_or_else(|| {
        protocol_error(
            "capability_unsupported",
            "Provider does not advertise turn.start controls".to_string(),
            false,
        )
    })?;
    let access_mode_id = resolve_choice(
        turn_send.access_mode.as_ref(),
        requested.access_mode_id,
        current.access_mode_id,
        "accessModeId",
    )?;
    let reasoning_effort_id = resolve_choice(
        turn_send.reasoning_effort.as_ref(),
        requested.reasoning_effort_id,
        current.reasoning_effort_id,
        "reasoningEffortId",
    )?;
    let model = match turn_send.model_catalog.as_ref() {
        Some(ModelCatalog::Unrecognized(_)) => {
            return Err(protocol_error(
                "unsupported_model_catalog",
                "Unknown model catalog".into(),
                false,
            ))
        }
        None => {
            if requested.model.is_some() {
                return Err(protocol_error(
                    "unsupported_turn_control",
                    "model must be omitted when modelCatalog is unavailable".to_string(),
                    false,
                ));
            }
            None
        }
        Some(ModelCatalog::FlatModelCatalog(catalog)) => {
            let requested_model = match requested.model {
                Some(ModelSelection::Unrecognized(_)) => {
                    return Err(protocol_error(
                        "unsupported_model_selection",
                        "Unknown model selection".into(),
                        false,
                    ))
                }
                Some(ModelSelection::FlatModelSelection(selection)) => Some(selection.model_id),
                Some(ModelSelection::GroupedModelSelection(_)) => {
                    return Err(protocol_error(
                        "invalid_model_selection",
                        "flat modelCatalog requires modelId without providerId".to_string(),
                        false,
                    ));
                }
                None => None,
            };
            let current_model = match current.model {
                Some(ModelSelection::FlatModelSelection(selection)) => Some(selection.model_id),
                _ => None,
            };
            let model_id = requested_model
                .or_else(|| {
                    current_model.filter(|id| catalog.models.iter().any(|option| option.id == *id))
                })
                .or_else(|| {
                    catalog
                        .default_selection
                        .as_ref()
                        .map(|selection| selection.model_id.clone())
                });
            match model_id {
                Some(model_id) => {
                    validate_choice(&catalog.models, &model_id, "model")?;
                    Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                        kind: FlatModelCatalogKind::Flat,
                        model_id,
                    }))
                }
                None => None,
            }
        }
        Some(ModelCatalog::GroupedModelCatalog(_)) => {
            return Err(protocol_error(
                "provider_capability_invalid",
                "Codex Provider cannot execute a grouped modelCatalog".to_string(),
                false,
            ));
        }
    };
    Ok(TurnSelection {
        access_mode_id,
        reasoning_effort_id,
        model,
    })
}

fn resolve_choice(
    control: Option<&codepet_provider_sdk::ChoiceSet>,
    requested: Option<String>,
    current: Option<String>,
    field: &str,
) -> Result<Option<String>, ProtocolError> {
    let Some(control) = control else {
        if requested.is_some() {
            return Err(protocol_error(
                "unsupported_turn_control",
                format!("{field} must be omitted when its control is unavailable"),
                false,
            ));
        }
        return Ok(None);
    };
    let selected = requested
        .or_else(|| current.filter(|id| control.options.iter().any(|option| option.id == *id)))
        .or_else(|| control.default_id.clone());
    if let Some(selected) = selected.as_deref() {
        validate_choice(&control.options, selected, field)?;
    }
    Ok(selected)
}

fn validate_choice(
    options: &[codepet_provider_sdk::ChoiceOption],
    selected: &str,
    field: &str,
) -> Result<(), ProtocolError> {
    let option = options
        .iter()
        .find(|option| option.id == selected)
        .ok_or_else(|| {
            protocol_error(
                "invalid_turn_selection",
                format!("unknown {field} selection: {selected}"),
                false,
            )
        })?;
    if option.enabled == Some(false) {
        return Err(protocol_error(
            "disabled_turn_selection",
            option
                .disabled_reason
                .clone()
                .unwrap_or_else(|| format!("{field} selection {selected} is disabled")),
            false,
        ));
    }
    Ok(())
}

fn decode_settings(
    settings: codepet_provider_sdk::JsonObject,
) -> Result<CodexInstanceSettings, ProtocolError> {
    let value = Value::Object(settings.into_iter().collect());
    let settings: CodexInstanceSettings = serde_json::from_value(value).map_err(|error| {
        protocol_error(
            "invalid_instance_settings",
            format!("invalid Codex instance settings: {error}"),
            false,
        )
    })?;
    if !settings.app_server_executable.is_absolute() {
        return Err(protocol_error(
            "invalid_instance_settings",
            "appServerExecutable must be an absolute path resolved by the Host".to_string(),
            false,
        ));
    }
    if settings
        .data_directory
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(protocol_error(
            "invalid_instance_settings",
            "dataDirectory must be absolute".into(),
            false,
        ));
    }
    Ok(settings)
}

fn validate_route(route: &ProviderInstanceRoute) -> Result<(), ProtocolError> {
    if route.device_id.trim().is_empty()
        || route.provider_plugin_id.trim().is_empty()
        || route.provider_instance_id.trim().is_empty()
    {
        return Err(protocol_error(
            "invalid_provider_route",
            "deviceId, providerPluginId, and providerInstanceId must not be empty".to_string(),
            false,
        ));
    }
    if route.provider_plugin_id != CODEX_PLUGIN_ID {
        return Err(protocol_error(
            "wrong_provider_plugin_route",
            format!(
                "Codex Provider cannot serve plugin {}",
                route.provider_plugin_id
            ),
            false,
        ));
    }
    Ok(())
}

fn validate_resource(resource: &ProviderResourceId) -> Result<(), ProtocolError> {
    validate_route(&ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
        provider_plugin_id: resource.provider_plugin_id.clone(),
        provider_instance_id: resource.provider_instance_id.clone(),
    })?;
    if resource.native_resource_id.trim().is_empty() {
        return Err(protocol_error(
            "invalid_provider_resource",
            "nativeResourceId must not be empty".to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_same_resource_route(
    left: &ProviderResourceId,
    right: &ProviderResourceId,
) -> Result<(), ProtocolError> {
    validate_resource(left)?;
    validate_resource(right)?;
    if left.device_id != right.device_id
        || left.provider_plugin_id != right.provider_plugin_id
        || left.provider_instance_id != right.provider_instance_id
    {
        return Err(protocol_error(
            "mismatched_provider_route",
            "resources must target the same device, Provider plugin, and Provider instance"
                .to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_resource_route(
    resource: &ProviderResourceId,
    route: &ProviderInstanceRoute,
) -> Result<(), ProtocolError> {
    validate_resource(resource)?;
    validate_route(route)?;
    if resource.device_id != route.device_id
        || resource.provider_plugin_id != route.provider_plugin_id
        || resource.provider_instance_id != route.provider_instance_id
    {
        return Err(protocol_error(
            "mismatched_provider_route",
            "resource must target the requested device, Provider plugin, and Provider instance"
                .to_string(),
            false,
        ));
    }
    Ok(())
}

fn require_capability(
    runtime: &CodexInstanceRuntime,
    capability: ProviderCapability,
) -> Result<(), ProtocolError> {
    if lock(&runtime.mutable)
        .capabilities
        .methods
        .contains(&capability)
    {
        return Ok(());
    }
    Err(protocol_error(
        "capability_unsupported",
        format!("Codex Provider does not advertise {capability:?}"),
        false,
    ))
}

fn validate_project_fields(
    name: &str,
    roots: &[codepet_provider_sdk::ProjectRoot],
) -> Result<(), ProtocolError> {
    if name.trim().is_empty() {
        return Err(protocol_error(
            "invalid_request",
            "project name must not be empty".to_string(),
            false,
        ));
    }
    if roots.is_empty() || roots.iter().any(|root| root.path.trim().is_empty()) {
        return Err(protocol_error(
            "invalid_request",
            "project roots must contain non-empty paths".to_string(),
            false,
        ));
    }
    Ok(())
}

fn provider_task_error(error: tokio::task::JoinError) -> ProtocolError {
    protocol_error(
        "provider_task_failed",
        format!("Codex Provider task failed: {error}"),
        true,
    )
}

pub(crate) fn protocol_error(code: &str, message: String, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.to_string(),
        message,
        retryable,
        details: None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod storage_path_tests {
    use super::*;
    #[test]
    fn executable_and_data_directory_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("custom data 中文");
        let executable = std::env::current_exe().unwrap();
        let settings = decode_settings(
            serde_json::from_value(json!({
                "appServerExecutable": executable, "appServerArgs": [],
                "dataDirectory": data,
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(settings.data_directory.as_deref(), Some(data.as_path()));
        let invalid = decode_settings(
            serde_json::from_value(json!({
                "appServerExecutable": executable, "appServerArgs": [],
                "dataDirectory": "relative-storage",
            }))
            .unwrap(),
        );
        assert!(invalid.is_err());
    }
}

impl CodexInstanceRuntime {
    fn query_generation(&self) -> Result<String, ProtocolError> {
        self.ready_server()?;
        Ok(lock(&self.mutable).lifecycle_generation.to_string())
    }
}
