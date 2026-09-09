mod conversation_observer;
use codepet_provider_data::conversation_atoms::{self, ConversationAtoms};
use codepet_provider_sdk::local_runtime;
use crate::client::{
    ClaudeCliError, ClaudeProcessControl, ClaudeTurnLaunch, SpawnedClaudeTurn,
};
use crate::protocol::{ClaudeControlRequest, ClaudeOutput, ClaudeStreamDelta, ClaudeStreamEvent};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolveRequest, ApprovalResolveResponse, ApprovalResolvedEvent,
    ApprovalStatus, ConversationAcquireInteractionRequest,
    ConversationAcquireInteractionResponse,
    ConversationCreateCapabilities, ConversationCreateRequest,
    ChoiceOption, ChoiceSet, ConversationContentKind, ConversationCreateResponse, ConversationGetRequest,
    ConversationGetResponse, ConversationListRequest, ConversationListResponse,
    CommandConversationItem, CommandConversationItemKind, CommandToolInput, CommandToolInputKind,
    ContentBlock, ConversationItem, ConversationItemRole, ConversationItemStatus,
    ConversationStatus, HarnessDescriptor, PageInfo,
    ConversationUpsertedEvent, FlatModelCatalog, FlatModelCatalogKind, FlatModelSelection,
    InstanceCapabilitiesRequest, InstanceCapabilitiesResponse,
    InstanceCreateRequest, InstanceCreateResponse, InstanceDestroyRequest,
    InstanceDestroyResponse, InstanceStartRequest, InstanceStartResponse, InstanceStatus,
    InstanceStatusChangedEvent, InstanceStopRequest, InstanceStopResponse, ProtocolError,
    ModelCatalog, ModelSelection, ProtocolEvent, ProtocolFuture, Provider, ProviderCapabilities, ProviderCapability,
    Conversation, ProviderDescribeRequest, ProviderDescribeResponse, ProviderExtension,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance, ProviderInstanceRoute, ProviderResourceId,
    Approval, ProviderAuthentication, ProviderAuthenticationStatus, ProviderPluginDescriptor,
    ProviderShutdownRequest, ProviderShutdownResponse, TurnTask, ProviderUsage,
    ProviderUsageDetail,
    MessageConversationItem, MessageConversationItemKind, OpaqueToolInput, OpaqueToolInputKind,
    OutputContentBlock, OutputContentBlockKind, RoutedResourceId, RuntimeCandidate, RuntimeGetInstalledRequest, RuntimeGetInstalledResponse,
    RuntimeInstallation, RuntimeSelectRequest, RuntimeSelectResponse, ToolCategory,
    StructuredToolInput, StructuredToolInputKind, TextContentBlock, TextContentBlockKind,
    ToolConversationItem, ToolConversationItemKind, ToolExecutionError, ToolFailureOutcome,
    ToolFailureOutcomeKind, ToolInput, ToolInvocation, ToolOrigin, ToolOriginKind, ToolOutcome,
    ToolSuccessOutcome, ToolSuccessOutcomeKind,
    TurnInterruptRequest, TurnInterruptResponse, TurnOutputDeltaEvent,
    TurnSelection, TurnSendCapabilities, TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest,
    TurnSteerResponse,
    TurnUpsertedEvent, VersionRange, PROTOCOL_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const CLAUDE_PLUGIN_ID: &str = "dev.codepet.claude";
pub const CLAUDE_INSTANCE_KIND: &str = "claude";
const CLAUDE_EXTENSION_NAMESPACE: &str = "dev.codepet.claude";
const MAX_PROVIDER_TEXT_CHUNK_BYTES: usize = 64 * 1024;
const MAX_CLAUDE_METADATA_BYTES: usize = 4 * 1024;
const TURN_COMPLETION_WAIT: Duration = Duration::from_secs(4);
const CLAUDE_DEFAULT_MODEL: &str = "claude-default";
const CLAUDE_DEFAULT_ACCESS_MODE: &str = "manual";
const CLAUDE_DEFAULT_EFFORT: &str = "high";
const DEFAULT_CONVERSATION_GET_TURN_LIMIT: u64 = 40;
const MAX_CONVERSATION_GET_TURN_LIMIT: u64 = 100;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct ClaudeInstanceSettings {
    claude_executable: PathBuf,
    #[serde(default, alias = "dataDirectory")]
    claude_config_dir: Option<PathBuf>,
}

use codepet_provider_sdk::ProviderEventSink;

struct ManagedTurn {
    turn: TurnTask,
    control: ClaudeProcessControl,
    saw_text_delta: bool,
    pending_completion: Option<TurnCompletion>,
    requested_completion: Option<TurnCompletion>,
    stream_failure: Option<String>,
    finished: Arc<TurnFinished>,
    pending_approvals: HashMap<String, PendingClaudeApproval>,
}

#[derive(Clone)]
struct PendingClaudeApproval {
    approval: Approval,
    request_id: String,
    input: Value,
}

#[derive(Clone)]
struct TurnCompletion {
    status: TurnStatus,
    result: Option<String>,
    terminal_reason: Option<String>,
}

#[derive(Default)]
struct TurnFinished {
    outcome: Mutex<Option<Result<TurnTask, ProtocolError>>>,
    changed: Condvar,
}

impl TurnFinished {
    fn complete(&self, outcome: Result<TurnTask, ProtocolError>) {
        let mut stored = lock(&self.outcome);
        if stored.is_none() {
            *stored = Some(outcome);
            self.changed.notify_all();
        }
    }

    fn wait(&self, timeout: Duration) -> Option<Result<TurnTask, ProtocolError>> {
        let stored = lock(&self.outcome);
        if stored.is_some() {
            return stored.clone();
        }
        let (stored, _) = self
            .changed
            .wait_timeout_while(stored, timeout, |stored| stored.is_none())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        stored.clone()
    }
}

struct ManagedConversation {
    conversation: Conversation,
    workspace_root: PathBuf,
    title: Option<String>,
    access_mode: String,
    model: Option<String>,
    effort: Option<String>,
    materialized: bool,
    history_path: Option<PathBuf>,
    active_turn: Option<ManagedTurn>,
}

#[derive(Clone)]
struct DiscoveredConversation {
    conversation: Conversation,
    workspace_root: PathBuf,
    title: Option<String>,
    model: Option<String>,
    history_path: PathBuf,
}

struct InstanceMutable {
    atomic_task: Option<tokio::task::JoinHandle<()>>,
    atomic_facts_ready: bool,
    atomic_readiness_pending: bool,
    atomic_facts_epoch: u64,
    lifecycle_generation: u64,
    metadata_epoch: u64,
    metadata_task: Option<tokio::task::JoinHandle<()>>,
    status: InstanceStatus,
    conversations: HashMap<String, ManagedConversation>,
    version: Option<String>,
    authentication: Option<ProviderAuthentication>,
    usage: Option<ProviderUsage>,
}

impl Drop for InstanceMutable {
    fn drop(&mut self) { if let Some(task) = self.metadata_task.take() { task.abort(); } if let Some(task) = self.atomic_task.take() { task.abort(); } }
}

type HistorySummaryCache = HashMap<PathBuf, (SystemTime, u64, DiscoveredConversation)>;

struct ClaudeInstanceRuntime {
    history_cache: Mutex<HistorySummaryCache>,
    atoms: ConversationAtoms,
    route: ProviderInstanceRoute,
    instance_kind: String,
    display_name: String,
    settings: ClaudeInstanceSettings,
    capabilities: ProviderCapabilities,
    mutable: Mutex<InstanceMutable>,
    events: Arc<dyn ProviderEventSink>,
}

impl ClaudeInstanceRuntime {
    fn new(
        request: InstanceCreateRequest,
        settings: ClaudeInstanceSettings,
        events: Arc<dyn ProviderEventSink>,
    ) -> Self {
        let atoms = ConversationAtoms::default();
        let events = atoms.event_sink(request.route.clone(), events);
        Self {
            atoms,
            history_cache: Mutex::new(HashMap::new()),
            route: request.route,
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            capabilities: claude_capabilities(),
            mutable: Mutex::new(InstanceMutable {
                atomic_task: None, atomic_facts_ready: false, atomic_readiness_pending: false, atomic_facts_epoch: 0, lifecycle_generation: 0,
                metadata_epoch: 0, metadata_task: None,
                status: InstanceStatus::Created,
                conversations: HashMap::new(),
                version: None,
                authentication: None,
                usage: None,
            }),
            events,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        let mutable = lock(&self.mutable);
        self.snapshot_locked(&mutable)
    }

    fn snapshot_locked(&self, mutable: &InstanceMutable) -> ProviderInstance {
        ProviderInstance {
            route: self.route.clone(),
            plugin_id: CLAUDE_PLUGIN_ID.to_string(),
            instance_kind: self.instance_kind.clone(),
            display_name: self.display_name.clone(),
            harness: HarnessDescriptor {
                id: self.instance_kind.clone(),
                display_name: "Claude Code".to_string(),
                version: mutable.version.clone(),
                executable_path: Some(
                    self.settings
                        .claude_executable
                        .to_string_lossy()
                        .into_owned(),
                ),
            },
            status: mutable.status,
            authentication: mutable.authentication.clone(),
            usage: mutable.usage.clone(),
            capabilities: conversation_atoms::observed_capabilities(self.capabilities.clone(), mutable.atomic_facts_ready, mutable.atomic_facts_epoch),
        }
    }

    fn status(&self) -> InstanceStatus {
        lock(&self.mutable).status
    }

    fn set_status(&self, status: InstanceStatus) -> Result<ProviderInstance, ProtocolError> {
        let mut mutable = lock(&self.mutable);
        let previous = mutable.status;
        if status != InstanceStatus::Ready {
            mutable.metadata_epoch = mutable.metadata_epoch.wrapping_add(1);
            if let Some(task) = mutable.metadata_task.take() {
                task.abort();
            }
        }
        mutable.status = status;
        let instance = self.snapshot_locked(&mutable);
        if previous != status {
            self.events
                .publish(ProtocolEvent::EventInstanceStatusChanged {
                    jsonrpc: "2.0".into(),
                    params: InstanceStatusChangedEvent {
                        instance: instance.clone(),
                        previous_status: Some(previous),
                    },
                })?;
        }
        Ok(instance)
    }

    fn start(&self) -> Result<ProviderInstance, ProtocolError> {
        let mut mutable = lock(&self.mutable);
        if matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready) {
            return Ok(self.snapshot_locked(&mutable));
        }
        if mutable.status == InstanceStatus::Stopping {
            return Err(protocol_error(
                "provider_instance_starting",
                "Claude lifecycle transition is in progress".into(),
                true,
            ));
        }
        mutable.metadata_epoch = mutable.metadata_epoch.wrapping_add(1);
        if let Some(task) = mutable.metadata_task.take() {
            task.abort();
        }
        // There is no persistent Claude daemon. Publish this local transition
        // atomically with respect to stop; no CLI query belongs in this section.
        let previous = mutable.status;
        mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
        mutable.status = InstanceStatus::Starting;
        self.events.publish(ProtocolEvent::EventInstanceStatusChanged {
            jsonrpc: "2.0".into(),
            params: InstanceStatusChangedEvent {
                instance: self.snapshot_locked(&mutable),
                previous_status: Some(previous),
            },
        })?;
        Ok(self.snapshot_locked(&mutable))
    }

    fn refresh_metadata(self: &Arc<Self>) {
        let mut mutable = lock(&self.mutable);
        if !matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready)
            || (mutable.status == InstanceStatus::Starting && mutable.metadata_task.is_some()) {
            return;
        }
        mutable.metadata_epoch = mutable.metadata_epoch.wrapping_add(1);
        if let Some(task) = mutable.metadata_task.take() {
            task.abort();
        }
        let epoch = mutable.metadata_epoch;
        let owner = Arc::downgrade(self);
        let executable = self.settings.claude_executable.clone();
        let config_dir = self.settings.claude_config_dir.clone();
        mutable.metadata_task = Some(tokio::spawn(async move {
            let probe = |args: &[&str]| {
                let mut command = tokio::process::Command::new(&executable);
                command.args(args);
                if let Some(directory) = &config_dir {
                    command.env("CLAUDE_CONFIG_DIR", directory);
                }
                codepet_provider_sdk::background_probe::run(command, Duration::from_secs(30))
            };
            let version = async {
                let result = probe(&["--version"]).await.ok();
                let version = result
                    .as_deref()
                    .and_then(|text| {
                        text.split_whitespace().find(|part| {
                            part.trim_start_matches('v')
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_digit())
                        })
                    })
                    .map(|s| s.trim_start_matches('v').to_string());
                version
            };
            let auth = async {
                let mut command = tokio::process::Command::new(&executable);
                command.args(["auth", "status", "--json"]);
                if let Some(directory) = &config_dir {
                    command.env("CLAUDE_CONFIG_DIR", directory);
                }
                let result = codepet_provider_sdk::background_probe::output(
                    command,
                    Duration::from_secs(30),
                )
                .await
                .map_err(|error| eprintln!("Claude auth status probe failed: {error}"))
                .ok();
                let text = result
                    .as_ref()
                    .and_then(|output| std::str::from_utf8(&output.stdout).ok());
                claude_authentication(text)
            };
            let (version, authentication) = tokio::join!(version, auth);
            if let Some(owner) = owner.upgrade() {
                owner.apply_metadata(epoch, version, Some(authentication));
            }
        }));
    }

    fn apply_metadata(
        &self,
        epoch: u64,
        version: Option<String>,
        authentication: Option<ProviderAuthentication>,
    ) {
        let mut mutable = lock(&self.mutable);
        if !matches!(mutable.status, InstanceStatus::Starting | InstanceStatus::Ready) || mutable.metadata_epoch != epoch {
            return;
        }
        if let Some(version) = version {
            mutable.version = Some(version);
        }
        if let Some(authentication) = authentication {
            mutable.authentication = Some(authentication);
        }
        let previous = mutable.status;
        mutable.status = InstanceStatus::Ready;
        let published = self
            .events
            .publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".into(),
                params: InstanceStatusChangedEvent {
                    instance: self.snapshot_locked(&mutable),
                    previous_status: Some(previous),
                },
            });
        if let Err(error) = published { eprintln!("Claude metadata notification failed: {error:?}"); }
    }

    fn create_conversation(
        &self,
        request: ConversationCreateRequest,
    ) -> Result<Conversation, ProtocolError> {
        if self.status() != InstanceStatus::Ready {
            return Err(provider_unavailable(&self.route));
        }
        if request.extension.is_some() {
            return Err(capability_error(
                "conversation.create extension data is unsupported by the Claude CLI adapter",
            ));
        }
        if request.workspace_mode.as_deref().unwrap_or("main") != "main" {
            return Err(protocol_error(
                "unsupported_workspace_mode",
                "Claude Provider only supports main workspace mode".to_string(),
                false,
            ));
        }
        validate_permission_level(&request.permission_level)?;
        let workspace_root = request.workspace_root.as_ref().ok_or_else(|| {
            protocol_error(
                "invalid_conversation_options",
                "Claude conversations require an absolute workspaceRoot".to_string(),
                false,
            )
        })?;
        let workspace_root = ensure_claude_workspace(workspace_root)?;
        if request.title.as_ref().is_some_and(|title| title.trim().is_empty()) {
            return Err(protocol_error(
                "invalid_conversation_options",
                "conversation title must not be empty".to_string(),
                false,
            ));
        }
        if request.model.as_ref().is_some_and(|model| model.trim().is_empty()) {
            return Err(protocol_error(
                "invalid_conversation_options",
                "model must not be empty".to_string(),
                false,
            ));
        }
        if let Some(effort) = request.reasoning_effort.as_deref() {
            if !["low", "medium", "high", "xhigh", "max"].contains(&effort) {
                return Err(protocol_error(
                    "invalid_conversation_options",
                    format!("unsupported Claude effort: {effort}"),
                    false,
                ));
            }
        }
        let session_id = Uuid::new_v4().to_string();
        let now = now_ms();
        let title = request
            .title
            .clone()
            .unwrap_or_else(|| format!("Claude {}", &session_id[..8]));
        let conversation = Conversation {
            resource: self.resource(session_id.clone()),
            project: None,
            title,
            preview: None,
            status: ConversationStatus::Idle,
            permission_level: Some(request.permission_level.clone()),
            model: request.model.clone(),
            reasoning_effort: request.reasoning_effort.clone(),
            selection: Some(claude_selection(
                CLAUDE_DEFAULT_ACCESS_MODE,
                request.reasoning_effort.as_deref().unwrap_or(CLAUDE_DEFAULT_EFFORT),
                request.model.as_deref().unwrap_or(CLAUDE_DEFAULT_MODEL),
            )),
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            created_at: Some(now),
            updated_at: Some(now),
            active_turn: None,
            read_state: None,
        };
        {
            let mut mutable = lock(&self.mutable);
            mutable.conversations.insert(
                session_id,
                ManagedConversation {
                    conversation: conversation.clone(),
                    workspace_root,
                    title: request.title,
                    access_mode: CLAUDE_DEFAULT_ACCESS_MODE.to_string(),
                    model: request.model,
                    effort: request.reasoning_effort,
                    materialized: false,
                    history_path: None,
                    active_turn: None,
                },
            );
        }
        self.publish_conversation(conversation.clone())?;
        Ok(conversation)
    }

    fn refresh_discovered_conversations(&self) -> Result<(), ProtocolError> {
        self.refresh_discovered_conversations_strict(false, None)
    }

    fn refresh_discovered_conversations_strict(&self, strict: bool, cancelled: Option<&AtomicBool>) -> Result<(), ProtocolError> {
        self.refresh_discovered_conversations_with_hook(strict, cancelled, || {})
    }

    fn refresh_discovered_conversations_with_hook(&self, strict: bool, cancelled: Option<&AtomicBool>, after_discovery: impl FnOnce()) -> Result<(), ProtocolError> {
        // Serialize discovery through installation, including legacy readers.
        // A newer RPC scan cannot install between an older poll's read and write.
        let mut history_cache = lock(&self.history_cache);
        let generation = lock(&self.mutable).lifecycle_generation;
        let event_epoch = self.atoms.event_epoch();
        let Some(config_dir) = self
            .settings
            .claude_config_dir
            .clone()
            .or_else(claude_config_dir)
        else {
            return Ok(());
        };
        let discovered = if strict {
            discover_claude_conversations_with_mode(&config_dir, &self.route, true, cancelled, Some(&mut *history_cache))?
        } else { discover_claude_conversations(&config_dir, &self.route)? };
        after_discovery();
        let mut mutable = lock(&self.mutable);
        if strict && (mutable.lifecycle_generation != generation || check_discovery_cancelled(cancelled).is_err()) { return Err(conversation_atoms::generation_changed()); }
        let mut install = || -> Result<(), ProtocolError> {
        if strict { check_discovery_cancelled(cancelled)?; }
        if strict {
            let mut deleted = Vec::new();
            for (id, managed) in &mutable.conversations {
                if managed.active_turn.is_some() || !managed.materialized { continue; }
                if let Some(path) = &managed.history_path {
                    match std::fs::metadata(path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => deleted.push(id.clone()),
                        Err(error) => return Err(discovery_error(path, error)),
                        Ok(_) => {}
                    }
                }
            }
            for id in deleted { mutable.conversations.remove(&id); }
        }
        for discovered in discovered {
            let conversation_id = discovered
                .conversation
                .resource
                .native_resource_id
                .clone();
            if let Some(managed) = mutable.conversations.get_mut(&conversation_id) {
                managed.history_path = Some(discovered.history_path);
                managed.materialized = true;
                if managed.active_turn.is_none() {
                    managed.conversation = discovered.conversation;
                    managed.workspace_root = discovered.workspace_root;
                    managed.title = discovered.title;
                    managed.model = discovered.model;
                }
            } else {
                mutable.conversations.insert(
                    conversation_id,
                    ManagedConversation {
                        conversation: discovered.conversation,
                        workspace_root: discovered.workspace_root,
                        title: discovered.title,
                        access_mode: CLAUDE_DEFAULT_ACCESS_MODE.to_string(),
                        model: discovered.model,
                        effort: None,
                        materialized: true,
                        history_path: Some(discovered.history_path),
                        active_turn: None,
                    },
                );
            }
        }
        Ok(())
        };
        if strict {
            self.atoms.with_fact_fence(event_epoch, install)?.ok_or_else(conversation_atoms::generation_changed)?
        } else { install() }
    }

    fn list_conversations(
        &self,
        request: ConversationListRequest,
    ) -> Result<ConversationListResponse, ProtocolError> {
        if self.status() != InstanceStatus::Ready {
            return Err(provider_unavailable(&self.route));
        }
        if request.route != self.route {
            return Err(protocol_error(
                "resource_route_mismatch",
                "conversation.list targets a different Claude instance".to_string(),
                false,
            ));
        }
        self.refresh_discovered_conversations()?;
        let mut conversations = lock(&self.mutable)
            .conversations
            .values()
            .map(|managed| managed.conversation.clone())
            .collect::<Vec<_>>();
        conversations.sort_by(|left, right| {
            right
                .updated_at
                .unwrap_or_default()
                .cmp(&left.updated_at.unwrap_or_default())
                .then_with(|| {
                    left.resource
                        .native_resource_id
                        .cmp(&right.resource.native_resource_id)
                })
        });
        let offset = request
            .cursor
            .as_deref()
            .map(|cursor| {
                cursor.parse::<usize>().map_err(|_| {
                    protocol_error(
                        "invalid_cursor",
                        "Claude conversation cursor is invalid".to_string(),
                        false,
                    )
                })
            })
            .transpose()?
            .unwrap_or_default();
        let limit = request.limit.unwrap_or(50).clamp(1, 200) as usize;
        if offset > conversations.len() {
            return Err(protocol_error(
                "invalid_cursor",
                "Claude conversation cursor is outside the result set".to_string(),
                false,
            ));
        }
        let end = offset.saturating_add(limit).min(conversations.len());
        let next_cursor = (end < conversations.len()).then(|| end.to_string());
        Ok(ConversationListResponse {
            conversations: conversations[offset..end].to_vec(),
            page_info: PageInfo { next_cursor },
        })
    }

    fn get_conversation(
        &self,
        request: ConversationGetRequest,
    ) -> Result<ConversationGetResponse, ProtocolError> {
        if self.status() != InstanceStatus::Ready {
            return Err(provider_unavailable(&self.route));
        }
        validate_resource_for_instance(&request.conversation, &self.route)?;
        self.refresh_discovered_conversations()?;
        let (conversation, history_path) = {
            let mutable = lock(&self.mutable);
            let managed = mutable
                .conversations
                .get(&request.conversation.native_resource_id)
                .ok_or_else(|| {
                    protocol_error(
                        "unknown_conversation",
                        "unknown Claude conversation".to_string(),
                        false,
                    )
                })?;
            (managed.conversation.clone(), managed.history_path.clone())
        };
        let items = match history_path {
            Some(path) => read_claude_history_items(&path, &self.route, &conversation.resource)?,
            None => Vec::new(),
        };
        let (items, next_cursor) = paginate_claude_history(
            items,
            request.cursor.as_deref(),
            request.limit,
        )?;
        Ok(ConversationGetResponse {
            conversation,
            items,
            page_info: Some(PageInfo { next_cursor }),
        })
    }

    fn start_turn(
        self: &Arc<Self>,
        request: TurnStartRequest,
    ) -> Result<(TurnTask, TurnSelection), ProtocolError> {
        validate_resource_for_instance(&request.conversation, &self.route)?;
        if request.input.text.trim().is_empty() || request.client_request_id.trim().is_empty() {
            return Err(protocol_error(
                "invalid_turn_request",
                "turn message and clientMessageId must not be empty".to_string(),
                false,
            ));
        }
        let conversation_id = request.conversation.native_resource_id.clone();
        let turn_id = Uuid::new_v4().to_string();
        if request.capability_revision != "claude-cli-stream-json-controls-v1" && request.capability_revision != self.snapshot().capabilities.revision {
            return Err(protocol_error(
                "stale_capability_revision",
                "turn.start capabilityRevision no longer matches the Provider instance"
                    .to_string(),
                true,
            ));
        }
        let user_message_id = request.client_request_id.clone();
        let (spawned, turn, conversation, effective_selection) = {
            let mut mutable = lock(&self.mutable);
            if mutable.status != InstanceStatus::Ready {
                return Err(provider_unavailable(&self.route));
            }
            let managed = mutable.conversations.get_mut(&conversation_id).ok_or_else(|| {
                protocol_error(
                    "unknown_conversation",
                    "Claude conversation is not managed by this Provider process".to_string(),
                    false,
                )
            })?;
            if managed.active_turn.is_some() {
                return Err(protocol_error(
                    "turn_already_active",
                    "Claude conversation already has an active turn".to_string(),
                    true,
                ));
            }
            let effective_selection = resolve_claude_selection(&request.selection, managed)?;
            managed.access_mode = effective_selection
                .access_mode_id
                .clone()
                .unwrap_or_else(|| CLAUDE_DEFAULT_ACCESS_MODE.to_string());
            managed.effort = effective_selection.reasoning_effort_id.clone();
            managed.model = selected_claude_model(&effective_selection);
            managed.conversation.permission_level = Some(managed.access_mode.clone());
            managed.conversation.reasoning_effort = managed.effort.clone();
            managed.conversation.model = managed.model.clone();
            managed.conversation.selection = Some(effective_selection.clone());
            let spawned = ClaudeTurnLaunch {
                executable: self.settings.claude_executable.clone(),
                config_directory: self.settings.claude_config_dir.clone(),
                workspace_root: managed.workspace_root.clone(),
                session_id: conversation_id.clone(),
                resume: managed.materialized,
                user_message_id,
                message: request.input.text,
                title: managed.title.clone(),
                permission_mode: managed.access_mode.clone(),
                model: managed.model.clone(),
                effort: managed.effort.clone(),
            }
            .spawn()
            .map_err(cli_protocol_error)?;
            let now = now_ms();
            let turn = TurnTask {
                resource: self.resource(turn_id.clone()),
                conversation: self.resource(conversation_id.clone()),
                status: TurnStatus::Running,
                display_summary: None,
                started_at: Some(now),
                updated_at: Some(now),
                completed_at: None,
            };
            let finished = Arc::new(TurnFinished::default());
            managed.active_turn = Some(ManagedTurn {
                turn: turn.clone(),
                control: spawned.control(),
                saw_text_delta: false,
                pending_completion: None,
                requested_completion: None,
                stream_failure: None,
                finished,
                pending_approvals: HashMap::new(),
            });
            managed.conversation.status = ConversationStatus::Running;
            managed.conversation.active_turn = Some(turn.clone());
            managed.conversation.updated_at = Some(now);
            (spawned, turn, managed.conversation.clone(), effective_selection)
        };
        let control = spawned.control();
        if let Err(error) = self
            .publish_turn(turn.clone())
            .and_then(|_| self.publish_conversation(conversation))
        {
            self.record_stream_failure(&conversation_id, &turn_id, error.message.clone());
            self.monitor_turn(spawned, conversation_id, turn_id);
            let _ = control.terminate();
            return Err(error);
        }
        self.monitor_turn(spawned, conversation_id, turn_id);
        Ok((turn, effective_selection))
    }

    fn monitor_turn(
        self: &Arc<Self>,
        spawned: SpawnedClaudeTurn,
        conversation_id: String,
        turn_id: String,
    ) {
        let weak_for_output = Arc::downgrade(self);
        let output_conversation = conversation_id.clone();
        let output_turn = turn_id.clone();
        let weak_for_error = Arc::downgrade(self);
        let error_conversation = conversation_id.clone();
        let error_turn = turn_id.clone();
        let weak_for_exit = Arc::downgrade(self);
        spawned.start(
            move |output| {
                let Some(runtime) = weak_for_output.upgrade() else {
                    return Ok(());
                };
                runtime
                    .handle_output(&output_conversation, &output_turn, output)
                    .map_err(|error| ClaudeCliError::Protocol(error.message))
            },
            move |error| {
                if let Some(runtime) = weak_for_error.upgrade() {
                    runtime.record_stream_failure(
                        &error_conversation,
                        &error_turn,
                        error.to_string(),
                    );
                }
            },
            move |outcome| {
                if let Some(runtime) = weak_for_exit.upgrade() {
                    runtime.handle_exit(&conversation_id, &turn_id, outcome);
                }
            },
        );
    }

    fn handle_output(
        &self,
        conversation_id: &str,
        turn_id: &str,
        output: ClaudeOutput,
    ) -> Result<(), ProtocolError> {
        match output {
            ClaudeOutput::ControlRequest {
                request_id,
                request:
                    ClaudeControlRequest::CanUseTool {
                        tool_name,
                        input,
                        tool_use_id: _,
                        title,
                        display_name,
                        description,
                    },
            } => {
                let approval = {
                    let mut mutable = lock(&self.mutable);
                    let managed = active_conversation_mut(&mut mutable, conversation_id, turn_id)?;
                    let active = managed.active_turn.as_mut().expect("active turn checked");
                    let approval_id = format!("{turn_id}:approval:{request_id}");
                    let approval = Approval {
                        resource: self.resource(approval_id.clone()),
                        conversation: managed.conversation.resource.clone(),
                        turn: active.turn.resource.clone(),
                        kind: tool_name.clone(),
                        title: title.or(display_name).unwrap_or_else(|| format!("Allow {tool_name}")),
                        description,
                        status: ApprovalStatus::Pending,
                        decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
                        requested_at: Some(now_ms()),
                        resolved_at: None,
                        decision: None,
                    };
                    active.pending_approvals.insert(
                        approval_id,
                        PendingClaudeApproval {
                            approval: approval.clone(),
                            request_id,
                            input,
                        },
                    );
                    let now = now_ms();
                    active.turn.status = TurnStatus::WaitingApproval;
                    active.turn.updated_at = Some(now);
                    managed.conversation.status = ConversationStatus::WaitingApproval;
                    managed.conversation.active_turn = Some(active.turn.clone());
                    managed.conversation.updated_at = Some(now);
                    approval
                };
                self.events.publish(ProtocolEvent::EventApprovalRequested {
                    jsonrpc: "2.0".to_string(),
                    params: ApprovalRequestedEvent { approval },
                })
            }
            ClaudeOutput::System {
                subtype,
                session_id,
                cwd,
                model,
                ..
            } if subtype == "init" => {
                validate_claude_session(conversation_id, session_id.as_deref())?;
                if cwd
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_CLAUDE_METADATA_BYTES)
                    || model
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_CLAUDE_METADATA_BYTES)
                {
                    return Err(protocol_error(
                        "claude_metadata_too_large",
                        "Claude init metadata exceeds the Provider limit".to_string(),
                        false,
                    ));
                }
                let conversation = {
                    let mut mutable = lock(&self.mutable);
                    let managed = active_conversation_mut(&mut mutable, conversation_id, turn_id)?;
                    managed.materialized = true;
                    if let Some(cwd) = cwd {
                        managed.conversation.workspace_root = Some(cwd);
                    }
                    if let Some(model) = model {
                        managed.conversation.model = Some(model);
                    }
                    managed.conversation.updated_at = Some(now_ms());
                    managed.conversation.clone()
                };
                self.publish_conversation(conversation)
            }
            ClaudeOutput::StreamEvent {
                session_id,
                parent_tool_use_id,
                event:
                    ClaudeStreamEvent::ContentBlockDelta {
                        index,
                        delta: ClaudeStreamDelta::TextDelta { text },
                    },
            } => {
                validate_claude_session(conversation_id, session_id.as_deref())?;
                if parent_tool_use_id.is_some() || text.is_empty() {
                    return Ok(());
                }
                let (turn, conversation) = {
                    let mut mutable = lock(&self.mutable);
                    let managed = active_conversation_mut(&mut mutable, conversation_id, turn_id)?;
                    let active = managed.active_turn.as_mut().expect("active turn checked");
                    active.saw_text_delta = true;
                    let now = now_ms();
                    active.turn.updated_at = Some(now);
                    managed.conversation.updated_at = Some(now);
                    (active.turn.clone(), managed.conversation.resource.clone())
                };
                let item_id = format!("{turn_id}:text:{index}");
                let content_id = format!("{item_id}:text");
                self.publish_text_chunks(
                    &turn.resource,
                    &conversation,
                    &item_id,
                    &content_id,
                    ConversationContentKind::Text,
                    &text,
                    "text_delta",
                )
            }
            ClaudeOutput::Assistant {
                session_id,
                aborted: Some(true),
                ..
            } => {
                validate_claude_session(conversation_id, session_id.as_deref())?;
                self.set_pending_completion(
                    conversation_id,
                    turn_id,
                    TurnCompletion {
                        status: TurnStatus::Interrupted,
                        result: None,
                        terminal_reason: None,
                    },
                )
            }
            ClaudeOutput::Result {
                subtype,
                is_error,
                session_id,
                result,
                stop_reason: _,
                terminal_reason,
                usage,
                total_cost_usd,
            } => {
                validate_claude_session(conversation_id, session_id.as_deref())?;
                self.update_usage(usage, total_cost_usd);
                let status = result_status(&subtype, is_error, terminal_reason.as_deref());
                self.set_pending_completion(
                    conversation_id,
                    turn_id,
                    TurnCompletion {
                        status,
                        result,
                        terminal_reason,
                    },
                )
            }
            _ => Ok(()),
        }
    }

    fn update_usage(&self, usage: Option<Value>, total_cost_usd: Option<f64>) {
        let Some(usage) = usage else { return };
        let mut data = usage.as_object().map(|value| value.iter()
            .map(|(key, value)| (key.clone(), value.clone())).collect::<BTreeMap<_, _>>())
            .unwrap_or_default();
        if let Some(cost) = total_cost_usd {
            data.insert("totalCostUsd".to_string(), json!(cost));
        }
        let tokens = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
            .into_iter()
            .filter_map(|key| data.get(key).and_then(Value::as_u64))
            .sum::<u64>();
        lock(&self.mutable).usage = Some(ProviderUsage {
            display_text: match total_cost_usd {
                Some(cost) => format!("Last turn: {tokens} tokens · ${cost:.4}"),
                None => format!("Last turn: {tokens} tokens"),
            },
            observed_at: Some(now_ms()),
            details: Some(vec![ProviderUsageDetail {
                namespace: "anthropic.claude.turn-usage".to_string(),
                schema_version: "1".to_string(),
                data,
            }]),
        });
        let _ = self.events.publish(ProtocolEvent::EventInstanceStatusChanged {
            jsonrpc: "2.0".to_string(),
            params: InstanceStatusChangedEvent { instance: self.snapshot(), previous_status: None },
        });
    }

    fn set_pending_completion(
        &self,
        conversation_id: &str,
        turn_id: &str,
        completion: TurnCompletion,
    ) -> Result<(), ProtocolError> {
        let mut mutable = lock(&self.mutable);
        let managed = active_conversation_mut(&mut mutable, conversation_id, turn_id)?;
        let active = managed.active_turn.as_mut().expect("active turn checked");
        if active.pending_completion.is_none() {
            active.pending_completion = Some(completion);
        }
        Ok(())
    }

    fn record_stream_failure(&self, conversation_id: &str, turn_id: &str, message: String) {
        let mut mutable = lock(&self.mutable);
        let Some(managed) = mutable.conversations.get_mut(conversation_id) else {
            return;
        };
        let Some(active) = managed.active_turn.as_mut() else {
            return;
        };
        if active.turn.resource.native_resource_id == turn_id && active.stream_failure.is_none() {
            active.stream_failure = Some(truncate_text(&message, MAX_CLAUDE_METADATA_BYTES));
        }
    }

    fn handle_exit(
        &self,
        conversation_id: &str,
        turn_id: &str,
        outcome: Result<std::process::ExitStatus, ClaudeCliError>,
    ) {
        self.finish_turn_after_exit(conversation_id, turn_id, outcome);
    }

    fn finish_turn_after_exit(
        &self,
        conversation_id: &str,
        turn_id: &str,
        outcome: Result<std::process::ExitStatus, ClaudeCliError>,
    ) {
        let snapshot = {
            let mut mutable = lock(&self.mutable);
            let Some(managed) = mutable.conversations.get_mut(conversation_id) else {
                return;
            };
            let Some(active) = managed.active_turn.as_mut() else {
                return;
            };
            if active.turn.resource.native_resource_id != turn_id {
                return;
            }
            let completion = completion_for_exit(active, &outcome);
            let expired_approvals = active
                .pending_approvals
                .drain()
                .map(|(_, pending)| {
                    let mut approval = pending.approval;
                    approval.status = ApprovalStatus::Expired;
                    approval.resolved_at = Some(now_ms());
                    approval
                })
                .collect::<Vec<_>>();
            (
                active.turn.clone(),
                managed.conversation.clone(),
                active.saw_text_delta,
                completion,
                active.finished.clone(),
                expired_approvals,
            )
        };
        let (
            base_turn,
            base_conversation,
            saw_text_delta,
            mut completion,
            finished,
            expired_approvals,
        ) = snapshot;
        for approval in expired_approvals {
            if let Err(error) = self.events.publish(ProtocolEvent::EventApprovalResolved {
                jsonrpc: "2.0".to_string(),
                params: ApprovalResolvedEvent { approval },
            }) {
                eprintln!("Claude Provider approval expiration event failed: {error:?}");
            }
        }
        let fallback = (!saw_text_delta)
            .then_some(completion.result.as_deref())
            .flatten()
            .filter(|value| !value.is_empty());
        if let Some(delta) = fallback {
            let (kind, content_suffix) = if completion.status == TurnStatus::Failed {
                (ConversationContentKind::ActivitySummary, "summary")
            } else {
                (ConversationContentKind::Text, "text")
            };
            let item_id = format!("{turn_id}:result");
            let content_id = format!("{item_id}:{content_suffix}");
            if let Err(error) = self.publish_text_chunks(
                &base_turn.resource,
                &base_turn.conversation,
                &item_id,
                &content_id,
                kind,
                delta,
                "result",
            ) {
                completion = TurnCompletion {
                    status: TurnStatus::Failed,
                    result: Some("Provider failed to publish Claude output".to_string()),
                    terminal_reason: Some("provider_event_publish_failed".to_string()),
                };
                eprintln!("Claude Provider output event failed: {error:?}");
            }
        }
        let (turn, conversation) = terminal_snapshot(base_turn, base_conversation, &completion);
        if let Err(error) = self.publish_turn(turn.clone()) {
            finished.complete(Err(error));
            return;
        }
        {
            let mut mutable = lock(&self.mutable);
            let Some(managed) = mutable.conversations.get_mut(conversation_id) else {
                finished.complete(Err(protocol_error(
                    "unknown_conversation",
                    "Claude conversation disappeared before terminal commit".to_string(),
                    false,
                )));
                return;
            };
            if managed
                .active_turn
                .as_ref()
                .is_none_or(|active| active.turn.resource.native_resource_id != turn_id)
            {
                return;
            }
            managed.conversation = conversation.clone();
            managed.active_turn = None;
        }
        if let Err(error) = self.publish_conversation(conversation) {
            eprintln!("Claude Provider terminal conversation event failed: {error:?}");
        }
        finished.complete(Ok(turn));
    }

    fn publish_text_chunks(
        &self,
        turn: &RoutedResourceId,
        conversation: &RoutedResourceId,
        item_id: &str,
        content_id: &str,
        kind: ConversationContentKind,
        text: &str,
        native_event: &str,
    ) -> Result<(), ProtocolError> {
        for chunk in text_chunks(text) {
            self.events.publish(ProtocolEvent::EventTurnOutputDelta {
                jsonrpc: "2.0".to_string(),
                params: TurnOutputDeltaEvent {
                    turn: provider_resource(
                        &self.route,
                        turn.native_resource_id.clone(),
                    ),
                    conversation: provider_resource(
                        &self.route,
                        conversation.native_resource_id.clone(),
                    ),
                    item_id: item_id.to_string(),
                    content_id: content_id.to_string(),
                    kind,
                    delta: chunk.to_string(),
                    extension: Some(extension([("nativeEvent", json!(native_event))])),
                },
            })?;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn interrupt_turn(&self, request: TurnInterruptRequest) -> Result<TurnTask, ProtocolError> {
        validate_resource_for_instance(&request.conversation, &self.route)?;
        validate_resource_for_instance(&request.turn, &self.route)?;
        let conversation_id = request.conversation.native_resource_id.clone();
        let (control, finished) = {
            let mut mutable = lock(&self.mutable);
            let managed = mutable.conversations.get_mut(&conversation_id).ok_or_else(|| {
                protocol_error("unknown_conversation", "unknown Claude conversation".to_string(), false)
            })?;
            let active = managed.active_turn.as_mut().ok_or_else(|| {
                protocol_error("turn_not_active", "Claude turn is not active".to_string(), false)
            })?;
            if active.turn.resource.native_resource_id != request.turn.native_resource_id
                || active.turn.conversation.native_resource_id
                    != request.conversation.native_resource_id
            {
                return Err(protocol_error(
                    "turn_route_mismatch",
                    "turn and conversation resources do not identify the active Claude turn".to_string(),
                    false,
                ));
            }
            active.requested_completion = Some(TurnCompletion {
                status: TurnStatus::Interrupted,
                result: None,
                terminal_reason: Some("interrupt_requested".to_string()),
            });
            (active.control.clone(), active.finished.clone())
        };
        control.interrupt().map_err(cli_protocol_error)?;
        finished.wait(TURN_COMPLETION_WAIT).ok_or_else(|| {
            protocol_error(
                "turn_completion_timeout",
                "Claude process exited but the interrupted turn did not publish a terminal event"
                    .to_string(),
                true,
            )
        })?
    }

    fn resolve_approval(
        &self,
        request: ApprovalResolveRequest,
    ) -> Result<Approval, ProtocolError> {
        validate_resource_for_instance(&request.approval, &self.route)?;
        let approval_id = request.approval.native_resource_id.clone();
        let (conversation_id, control, pending) = {
            let mutable = lock(&self.mutable);
            mutable
                .conversations
                .iter()
                .find_map(|(conversation_id, managed)| {
                    let active = managed.active_turn.as_ref()?;
                    let pending = active.pending_approvals.get(&approval_id)?;
                    (pending.approval.resource.native_resource_id
                        == request.approval.native_resource_id)
                    .then(|| {
                        (conversation_id.clone(), active.control.clone(), pending.clone())
                    })
                })
                .ok_or_else(|| {
                    protocol_error(
                        "approval_not_found",
                        format!("approval {approval_id} is not pending in this Provider instance"),
                        false,
                    )
                })?
        };
        control
            .resolve_permission(
                &pending.request_id,
                request.decision == ApprovalDecision::Approve,
                &pending.input,
            )
            .map_err(cli_protocol_error)?;
        let (approval, turn, conversation) = {
            let mut mutable = lock(&self.mutable);
            let managed = mutable.conversations.get_mut(&conversation_id).ok_or_else(|| {
                protocol_error("unknown_conversation", "unknown Claude conversation".to_string(), false)
            })?;
            let active = managed.active_turn.as_mut().ok_or_else(|| {
                protocol_error("turn_not_active", "Claude turn is not active".to_string(), false)
            })?;
            let mut approval = active
                .pending_approvals
                .remove(&approval_id)
                .ok_or_else(|| {
                    protocol_error(
                        "approval_not_found",
                        format!("approval {approval_id} is no longer pending"),
                        false,
                    )
                })?
                .approval;
            let now = now_ms();
            approval.status = match request.decision {
                ApprovalDecision::Approve => ApprovalStatus::Approved,
                ApprovalDecision::Deny => ApprovalStatus::Denied,
            };
            approval.decision = Some(request.decision);
            approval.resolved_at = Some(now);
            active.turn.status = TurnStatus::Running;
            active.turn.updated_at = Some(now);
            managed.conversation.status = ConversationStatus::Running;
            managed.conversation.active_turn = Some(active.turn.clone());
            managed.conversation.updated_at = Some(now);
            (approval, active.turn.clone(), managed.conversation.clone())
        };
        self.events.publish(ProtocolEvent::EventApprovalResolved {
            jsonrpc: "2.0".to_string(),
            params: ApprovalResolvedEvent {
                approval: approval.clone(),
            },
        })?;
        self.publish_turn(turn)?;
        self.publish_conversation(conversation)?;
        Ok(approval)
    }

    fn stop(&self) -> Result<ProviderInstance, ProtocolError> {
        if matches!(self.status(), InstanceStatus::Stopped | InstanceStatus::Created) {
            return self.set_status(InstanceStatus::Stopped);
        }
        let mut first_error = self.set_status(InstanceStatus::Stopping).err();
        let active_turns = {
            let mut mutable = lock(&self.mutable);
            mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
            mutable.atomic_facts_ready = false;
            if let Some(task) = mutable.atomic_task.take() { task.abort(); }
            let mut active_turns = Vec::new();
            for managed in mutable.conversations.values_mut() {
                if let Some(active) = managed.active_turn.as_mut() {
                    active.requested_completion = Some(TurnCompletion {
                        status: TurnStatus::Interrupted,
                        result: None,
                        terminal_reason: Some("instance_stop".to_string()),
                    });
                    active_turns.push((active.control.clone(), active.finished.clone()));
                }
            }
            active_turns
        };
        for (control, _) in &active_turns {
            if let Err(error) = control.terminate() {
                first_error.get_or_insert_with(|| cli_protocol_error(error));
            }
        }
        for (_, finished) in active_turns {
            match finished.wait(TURN_COMPLETION_WAIT) {
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    first_error.get_or_insert(error);
                }
                None => {
                    first_error.get_or_insert_with(|| {
                        protocol_error(
                            "turn_completion_timeout",
                            "Claude process was terminated but its turn did not reach terminal"
                                .to_string(),
                            true,
                        )
                    });
                }
            }
        }
        if let Some(error) = first_error {
            let _ = self.set_status(InstanceStatus::Error);
            return Err(error);
        }
        self.set_status(InstanceStatus::Stopped)
    }

    fn reap_active_processes(&self) -> Result<(), ProtocolError> {
        let controls = {
            let mut mutable = lock(&self.mutable);
            mutable
                .conversations
                .values_mut()
                .filter_map(|managed| {
                    let active = managed.active_turn.as_mut()?;
                    active.requested_completion = Some(TurnCompletion {
                        status: TurnStatus::Interrupted,
                        result: None,
                        terminal_reason: Some("provider_exit".to_string()),
                    });
                    Some(active.control.clone())
                })
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for control in controls {
            if let Err(error) = control.terminate() {
                first_error.get_or_insert_with(|| cli_protocol_error(error));
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn resource(&self, native_resource_id: String) -> RoutedResourceId {
        RoutedResourceId {
            provider_id: self.route.provider_instance_id.clone(),
            native_resource_id,
        }
    }

    fn publish_conversation(&self, conversation: Conversation) -> Result<(), ProtocolError> {
        self.events.publish(ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".to_string(),
            params: ConversationUpsertedEvent { conversation },
        })
    }

    fn publish_turn(&self, turn: TurnTask) -> Result<(), ProtocolError> {
        self.events.publish(ProtocolEvent::EventTurnUpserted {
            jsonrpc: "2.0".to_string(),
            params: TurnUpsertedEvent { turn },
        })
    }
}

struct ProviderState {
    host_device_id: Option<String>,
    initialized_client_id: Option<String>,
    instances: HashMap<String, Arc<ClaudeInstanceRuntime>>,
    selected_runtime: Option<RuntimeInstallation>,
}

pub struct ClaudeProvider {
    data: Arc<codepet_provider_data::ProviderData>,
    observation: codepet_observation::Observation,
    scanner: local_runtime::RuntimeScanner,
    state: Mutex<ProviderState>,
    events: Arc<dyn ProviderEventSink>,
    shutdown: AtomicBool,
}

impl ClaudeProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        let data=Arc::new(codepet_provider_data::ProviderData::default());
        let events: Arc<dyn ProviderEventSink>=Arc::new(codepet_provider_data::UsageSink { data:data.clone(), downstream:events, provider:"claude" });
        Self {
            data,
            scanner: local_runtime::RuntimeScanner::new(events.clone()),
            observation: codepet_observation::Observation::new(codepet_observation::Definition {
                windows_command_override: false,
                name: "claude", config: codepet_observation::config_home("CLAUDE_CONFIG_DIR", codepet_observation::home().join(".claude")).join("settings.json"), events: &["SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop", "SubagentStart", "SubagentStop", "PostToolUseFailure", "StopFailure", "Elicitation", "ElicitationResult"], plugin: None,
            }, events.clone()),
            state: Mutex::new(ProviderState {
                host_device_id: None,
                initialized_client_id: None,
                instances: HashMap::new(),
                selected_runtime: None,
            }),
            events,
            shutdown: AtomicBool::new(false),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub fn reap_active_processes(&self) -> Result<(), ProtocolError> {
        let instances = lock(&self.state)
            .instances
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut first_error = None;
        for instance in instances {
            if let Err(error) = instance.reap_active_processes() {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn descriptor() -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            plugin_id: CLAUDE_PLUGIN_ID.to_string(),
            display_name: "Claude".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_workspace_root: default_remote_workspace_root("claude"),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
            instance_kinds: vec![CLAUDE_INSTANCE_KIND.to_string()],
        }
    }

    fn instance(&self, route: &ProviderInstanceRoute) -> Result<Arc<ClaudeInstanceRuntime>, ProtocolError> {
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
                    format!("unknown Claude Provider instance: {}", route.provider_instance_id),
                    false,
                )
            })
    }

    fn resource_instance(&self, resource: &ProviderResourceId) -> Result<Arc<ClaudeInstanceRuntime>, ProtocolError> {
        validate_resource(resource)?;
        self.instance(&ProviderInstanceRoute {
            device_id: resource.device_id.clone(),
            provider_plugin_id: resource.provider_plugin_id.clone(),
            provider_instance_id: resource.provider_instance_id.clone(),
        })
    }
}

impl Provider for ClaudeProvider {
    fn event_subscribe<'a>(&'a self, request: codepet_provider_sdk::EventSubscribeRequest) -> codepet_provider_sdk::ProtocolFuture<'a, codepet_provider_sdk::EventSubscribeResponse> {
        Box::pin(async move {
            if self.is_shutdown() { return Err(codepet_provider_sdk::ProtocolError { code: "provider_shutdown".into(), message: "Provider stopped".into(), retryable: false, details: None }); }
            if lock(&self.state).host_device_id.is_none() { return Err(protocol_error("provider_not_initialized", "Initialize Provider before subscribing".into(), false)); }
            self.observation.subscribe(request.subscription_id).await
        })
    }
    fn event_unsubscribe<'a>(&'a self, request: codepet_provider_sdk::EventUnsubscribeRequest) -> codepet_provider_sdk::ProtocolFuture<'a, codepet_provider_sdk::EventUnsubscribeResponse> {
        Box::pin(async move { self.observation.unsubscribe(request.subscription_id).await })
    }

    fn provider_initialize<'a>(
        &'a self,
        request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async move {
            if request.supported_versions.min_version > request.supported_versions.max_version {
                return Err(protocol_error(
                    "invalid_protocol_range",
                    "Host protocol range is invalid".to_string(),
                    false,
                ));
            }
            if PROTOCOL_VERSION < request.supported_versions.min_version
                || PROTOCOL_VERSION > request.supported_versions.max_version
            {
                return Err(protocol_error(
                    "unsupported_protocol_version",
                    "Provider protocol v1 is outside the Host-supported range".to_string(),
                    false,
                ));
            }
            if request.host_client_id.trim().is_empty()
                || request.host_device_id.trim().is_empty()
                || request.host_version.trim().is_empty()
            {
                return Err(protocol_error(
                    "invalid_host_identity",
                    "Host client, device, and version must not be empty".to_string(),
                    false,
                ));
            }
            let mut state = lock(&self.state);
            if let Some(device_id) = state.host_device_id.as_ref() {
                if device_id != &request.host_device_id
                    || state.initialized_client_id.as_ref() != Some(&request.host_client_id)
                {
                    return Err(protocol_error(
                        "provider_already_initialized",
                        "Provider process is already bound to another Host identity".to_string(),
                        false,
                    ));
                }
            } else {
                state.host_device_id = Some(request.host_device_id);
                state.initialized_client_id = Some(request.host_client_id);
            }
            drop(state);
            self.data.initialize(request.directories.as_ref())?;
            self.data.start_collection()?;
            eprintln!("provider.initialize completed; business storage configured={}", self.data.configured());
            self.scanner.start_cancellable("claude", "@anthropic-ai/claude-code", || discover_path_candidates("claude"), |candidate, timeout, control| inspect_runtime_candidate(candidate, "claude", timeout, control));
            Ok(ProviderInitializeResponse {
                selected_version: PROTOCOL_VERSION,
                plugin: Self::descriptor(),
            })
        })
    }

    fn usage_query<'a>(&'a self, request: codepet_provider_sdk::UsageQueryRequest) -> ProtocolFuture<'a, codepet_provider_sdk::UsageQueryResponse> {
        Box::pin(async move {
            let _runtime = self.instance(&request.route)?;
            let data=self.data.clone();
            let result=tokio::task::spawn_blocking(move || data.query_observed(&request.route.provider_instance_id, request.query)).await.map_err(|e|codepet_provider_data::error("usage_query_failed",e))??;
            Ok(codepet_provider_sdk::UsageQueryResponse { result })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        _request: ProviderDescribeRequest,
    ) -> ProtocolFuture<'a, ProviderDescribeResponse> {
        Box::pin(async move {
            Ok(ProviderDescribeResponse {
                plugin: Self::descriptor(),
            })
        })
    }

    fn runtime_get_installed<'a>(&'a self, request: RuntimeGetInstalledRequest) -> ProtocolFuture<'a, RuntimeGetInstalledResponse> {
        Box::pin(async move {
            if request.refresh == Some(true) && self.scanner.snapshot().scanning != Some(true) {
                self.scanner.stop();
                self.scanner.start_cancellable("claude", "@anthropic-ai/claude-code", || discover_path_candidates("claude"), |candidate, timeout, control| inspect_runtime_candidate(candidate, "claude", timeout, control));
            }
            if request.refresh == Some(true) {
                let instances = lock(&self.state).instances.values().cloned().collect::<Vec<_>>();
                for instance in instances { instance.refresh_metadata(); }
            }
            Ok(self.scanner.snapshot())
        })
    }
    fn runtime_select<'a>(&'a self, request: RuntimeSelectRequest) -> ProtocolFuture<'a, RuntimeSelectResponse> {
        Box::pin(async move {
            let mut candidate = request.candidate;
            candidate.executable_path = local_runtime::resolve_executable(std::path::Path::new(&candidate.executable_path), "claude", "@anthropic-ai/claude-code")
                .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?.to_string_lossy().into_owned();
            let selected=self.scanner.select(&candidate)?;
            lock(&self.state).selected_runtime=Some(selected.clone());
            Ok(RuntimeSelectResponse {selected})
        })
    }

    fn instance_create<'a>(
        &'a self,
        mut request: InstanceCreateRequest,
    ) -> ProtocolFuture<'a, InstanceCreateResponse> {
        Box::pin(async move {
            Self::descriptor().validate_instance_kind(&request.instance_kind)?;
            validate_route(&request.route)?;
            if request.display_name.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_provider_instance",
                    "Provider instance display name must not be empty".to_string(),
                    false,
                ));
            }
            if std::env::var("CODEPET_RUNTIME_MIN_VERSION").is_ok_and(|value| !value.is_empty()) {
                if let Some(path) = request.settings.get("claudeExecutable").and_then(Value::as_str) {
                    let runtime = self.scanner.select(&RuntimeCandidate {
                        executable_path: path.to_string(),
                        source: codepet_provider_sdk::RuntimeCandidateSource::Configured,
                    })?;
                    if runtime.version.is_empty() {
                        return Err(protocol_error("runtime_scanning", "Runtime version validation is still in progress".into(), true));
                    }
                    local_runtime::require_compatible_runtime(&local_runtime::apply_runtime_requirement(runtime))?;
                }
            }
            let selected = if request.settings.contains_key("claudeExecutable") { None } else {
                let current = { lock(&self.state).selected_runtime.clone() };
                let installation = match current {
                    Some(selected) => Some(selected),
                    None => {
                        let inventory=self.scanner.snapshot();
                        if inventory.scanning==Some(true) {return Err(protocol_error("runtime_scanning", "Runtime discovery is still in progress".into(), true));}
                        inventory.installed.into_iter().find(|runtime| runtime.incompatibility_reason.is_none())
                    },
                };
                Some(installation.ok_or_else(|| protocol_error("provider_unavailable", "Claude Provider did not find a local runtime".to_string(), true))?)
            };
            if let Some(selected) = selected.as_ref() {
                if selected.version.is_empty() {
                    return Err(protocol_error("runtime_scanning", "Runtime version validation is still in progress".into(), true));
                }
                local_runtime::require_compatible_runtime(&local_runtime::apply_runtime_requirement(selected.clone()))?;
                request.settings.insert("claudeExecutable".to_string(), json!(selected.executable_path.clone()));
            }
            let settings = decode_settings(request.settings.clone())?;
            let mut state = lock(&self.state);
            if let Some(selected) = selected { state.selected_runtime = Some(selected); }
            let host_device = state.host_device_id.as_deref().ok_or_else(|| {
                protocol_error(
                    "provider_not_initialized",
                    "Provider must be initialized before creating instances".to_string(),
                    false,
                )
            })?;
            if request.route.device_id != host_device {
                return Err(protocol_error(
                    "wrong_device_route",
                    "Provider instance targets a different Host device".to_string(),
                    false,
                ));
            }
            if let Some(existing) = state.instances.get(&request.route.provider_instance_id) {
                if existing.route != request.route
                    || existing.instance_kind != request.instance_kind
                    || existing.display_name != request.display_name
                    || existing.settings != settings
                {
                    return Err(protocol_error(
                        "provider_instance_conflict",
                        "Provider instance id was reused with different configuration".to_string(),
                        false,
                    ));
                }
                return Ok(InstanceCreateResponse {
                    instance: existing.snapshot(),
                });
            }
            let runtime = Arc::new(ClaudeInstanceRuntime::new(request, settings, self.events.clone()));
            let instance = runtime.snapshot();
            state
                .instances
                .insert(runtime.route.provider_instance_id.clone(), runtime);
            Ok(InstanceCreateResponse { instance })
        })
    }

    fn instance_start<'a>(
        &'a self,
        request: InstanceStartRequest,
    ) -> ProtocolFuture<'a, InstanceStartResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if matches!(runtime.status(), InstanceStatus::Starting | InstanceStatus::Ready) {
                return Ok(InstanceStartResponse {
                    instance: runtime.snapshot(),
                });
            }
            let instance = runtime.start()?;
            runtime.refresh_metadata();
            runtime.start_atomic_poll();
            Ok(InstanceStartResponse { instance })
        })
    }

    fn instance_stop<'a>(
        &'a self,
        request: InstanceStopRequest,
    ) -> ProtocolFuture<'a, InstanceStopResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            Ok(InstanceStopResponse {
                instance: runtime.stop()?,
            })
        })
    }

    fn instance_destroy<'a>(
        &'a self,
        request: InstanceDestroyRequest,
    ) -> ProtocolFuture<'a, InstanceDestroyResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            runtime.stop()?;
            let mut state = lock(&self.state);
            let destroyed = state
                .instances
                .remove(&request.route.provider_instance_id)
                .is_some();
            Ok(InstanceDestroyResponse { destroyed })
        })
    }

    fn instance_capabilities<'a>(
        &'a self,
        request: InstanceCapabilitiesRequest,
    ) -> ProtocolFuture<'a, InstanceCapabilitiesResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            Ok(InstanceCapabilitiesResponse {
                capabilities: runtime.snapshot().capabilities,
            })
        })
    }

    fn conversation_active_list<'a>(&'a self, request: codepet_provider_sdk::ConversationActiveListRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationActiveListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if !lock(&runtime.mutable).atomic_facts_ready { return Err(protocol_error("unsupported", "complete native observation is not ready".into(), true)); }
            let generation = runtime.query_generation()?;
            if let Some(page) = runtime.atoms.active_cached(&generation, &request)? { return Ok(page); }
            let epoch = runtime.atoms.event_epoch();
            let rows = runtime.collect_atomic_summaries().await?;
            if generation != runtime.query_generation()? || epoch != runtime.atoms.event_epoch() { return Err(conversation_atoms::generation_changed()); }
            runtime.atoms.active(&generation, &request, rows)
        })
    }

    fn conversation_unread_list<'a>(&'a self, request: codepet_provider_sdk::ConversationUnreadListRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationUnreadListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            runtime.atoms.unread(&runtime.query_generation()?, &request)
        })
    }

    fn conversation_mark_read<'a>(&'a self, request: codepet_provider_sdk::ConversationMarkReadRequest) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationMarkReadResponse> {
        Box::pin(async move {
            let runtime = self.instance(&conversation_atoms::resource_route(&request.conversation))?;
            runtime.query_generation()?;
            runtime.atoms.mark_read(&request, runtime.events.as_ref())
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            if request.query.is_some() {
                let runtime = self.instance(&request.route)?;
                let generation = runtime.query_generation()?;
                if let Some(page) = runtime.atoms.list_cached(&generation, &request)? { return Ok(page); }
                let event_epoch = runtime.atoms.event_epoch();
            let rows = self.complete_conversation_summaries(&request.route).await?;
                if generation != runtime.query_generation()? || event_epoch != runtime.atoms.event_epoch() { return Err(conversation_atoms::generation_changed()); }
                return runtime.atoms.list(&generation, &request, rows);
            }
            if let Some(scope) = request.reader_scope.clone() {
                let mut native_request = request;
                native_request.reader_scope = None;
                let mut response = self.conversation_list(native_request).await?;
                codepet_provider_data::conversation_state::SharedConversationStateStore::from_env()?.decorate_many(&scope, &mut response.conversations)?;
                return Ok(response);
            }

            let runtime = self.instance(&request.route)?;
            runtime.list_conversations(request)
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProtocolFuture<'a, ConversationGetResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            runtime.get_conversation(request)
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: ConversationAcquireInteractionRequest,
    ) -> ProtocolFuture<'a, ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            validate_resource_for_instance(&request.conversation, &runtime.route)?;
            Ok(ConversationAcquireInteractionResponse {
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
                lease_expires_at: None,
            })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
            if request.project.is_some() {
                return Err(protocol_error(
                    "capability_unsupported",
                    "Claude Provider does not support project-owned conversations".to_string(),
                    false,
                ));
            }
            let runtime = self.instance(&request.route)?;
            Ok(ConversationCreateResponse {
                conversation: runtime.create_conversation(request)?,
            })
        })
    }

    fn turn_start<'a>(
        &'a self,
        request: TurnStartRequest,
    ) -> ProtocolFuture<'a, TurnStartResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation = runtime.resource(request.conversation.native_resource_id.clone());
            let item_id = request.client_request_id.clone();
            let input_text = request.input.text.clone();
            let (turn, effective_selection) = runtime.start_turn(request)?;
            Ok(TurnStartResponse {
                accepted: true,
                user_item: Some(ConversationItem::MessageConversationItem(MessageConversationItem {
                    meta: None,
                    resource: runtime.resource(item_id.clone()),
                    turn: turn.resource.clone(),
                    conversation,
                    kind: MessageConversationItemKind::Message,
                    status: ConversationItemStatus::Completed,
                    role: ConversationItemRole::User,
                    contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                        content_id: format!("{item_id}:text"),
                        kind: TextContentBlockKind::Text,
                        text: input_text,
                        truncation: None,
                    })],
                })),
                turn,
                effective_selection,
            })
        })
    }

    fn turn_steer<'a>(
        &'a self,
        _request: TurnSteerRequest,
    ) -> ProtocolFuture<'a, TurnSteerResponse> {
        Box::pin(async { Err(capability_error("Claude CLI queued input is not equivalent to turn.steer")) })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProtocolFuture<'a, TurnInterruptResponse> {
        #[cfg(unix)]
        {
            Box::pin(async move {
                let runtime = self.resource_instance(&request.conversation)?;
                Ok(TurnInterruptResponse {
                    turn: runtime.interrupt_turn(request)?,
                })
            })
        }
        #[cfg(not(unix))]
        {
            let _ = request;
            Box::pin(async { Err(capability_error("Claude CLI turn interrupt requires Unix SIGINT")) })
        }
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.approval)?;
            Ok(ApprovalResolveResponse {
                approval: runtime.resolve_approval(request)?,
            })
        })
    }

    fn provider_shutdown<'a>(
        &'a self,
        _request: ProviderShutdownRequest,
    ) -> ProtocolFuture<'a, ProviderShutdownResponse> {
        Box::pin(async move {
            self.scanner.stop();
            self.observation.shutdown().await;
            let instances = lock(&self.state).instances.values().cloned().collect::<Vec<_>>();
            let mut first_error = None;
            for instance in instances {
                if let Err(error) = instance.stop() {
                    first_error.get_or_insert(error);
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
            self.shutdown.store(true, Ordering::SeqCst);
            Ok(ProviderShutdownResponse { accepted: true })
        })
    }
}

fn claude_capabilities() -> ProviderCapabilities {
    let mut methods = vec![
        ProviderCapability::ConversationList,
        ProviderCapability::ConversationGet,
        ProviderCapability::ConversationCreate,
        ProviderCapability::TurnStart,
    ];
    #[cfg(unix)]
    methods.push(ProviderCapability::TurnInterrupt);
    methods.push(ProviderCapability::ApprovalResolve);
    let turn_send = TurnSendCapabilities {
        access_mode: Some(ChoiceSet {
            options: vec![
                choice("manual", "Ask before changes", Some("Claude asks before protected tool actions.")),
                choice("acceptEdits", "Accept edits", Some("Automatically accepts file edits while retaining other permission checks.")),
                choice("plan", "Plan mode", Some("Read-only planning mode.")),
                choice("dontAsk", "Don't ask", Some("Declines actions that would require an approval prompt.")),
                choice("auto", "Auto", Some("Uses Claude Code's automatic permission policy.")),
            ],
            default_id: Some(CLAUDE_DEFAULT_ACCESS_MODE.to_string()),
        }),
        reasoning_effort: Some(ChoiceSet {
            options: ["low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .map(|effort| choice(effort, &choice_display_name(effort), None))
                .collect(),
            default_id: Some(CLAUDE_DEFAULT_EFFORT.to_string()),
        }),
        model_catalog: Some(ModelCatalog::FlatModelCatalog(FlatModelCatalog {
            kind: FlatModelCatalogKind::Flat,
            models: vec![
                choice(CLAUDE_DEFAULT_MODEL, "Default", Some("Uses the model selected by Claude Code configuration.")),
                choice("sonnet", "Sonnet", None),
                choice("opus", "Opus", None),
                choice("fable", "Fable", None),
            ],
            default_selection: Some(FlatModelSelection {
                kind: FlatModelCatalogKind::Flat,
                model_id: CLAUDE_DEFAULT_MODEL.to_string(),
            }),
        })),
    };
    let create_selection = TurnSendCapabilities {
        access_mode: Some(ChoiceSet {
            options: vec![choice(
                "workspace-write",
                "Workspace write",
                Some("Creates the conversation in the selected workspace."),
            )],
            default_id: Some("workspace-write".to_string()),
        }),
        reasoning_effort: turn_send.reasoning_effort.clone(),
        model_catalog: turn_send.model_catalog.clone(),
    };
    codepet_provider_data::with_usage_capabilities(ProviderCapabilities { usage_datasets: Some(codepet_provider_data::datasets(false)),
            conversation_list_query: Some(codepet_provider_sdk::ConversationListQueryCapabilities { updated_after: true, ids: true }),
            revision: "claude-cli-stream-json-controls-v1".to_string(),
        methods,
        conversation_create: Some(ConversationCreateCapabilities {
            supports_title: true,
            selection: Some(create_selection),
            workspace_mode: Some(ChoiceSet {
                options: vec![choice("main", "Main workspace", None)],
                default_id: Some("main".to_string()),
            }),
        }),
        turn_send: Some(turn_send),
        extensions: vec![extension([
            ("nativeInterface", json!("claude-print-stream-json")),
            (
                "nativeMethods",
                json!(["--session-id", "--resume", "stream-json", "SIGINT"]),
            ),
            (
                "unsupportedMethods",
                json!(["turn.steer", "approval.resolve"]),
            ),
            ("permissionModeMapping", json!({ "workspace-write": "inherited" })),
        ])],
    })
}

fn choice(id: &str, display_name: &str, description: Option<&str>) -> ChoiceOption {
    ChoiceOption {
        id: id.to_string(),
        display_name: display_name.to_string(),
        description: description.map(str::to_string),
        enabled: Some(true),
        disabled_reason: None,
    }
}

fn choice_display_name(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn claude_selection(access_mode: &str, effort: &str, model: &str) -> TurnSelection {
    TurnSelection {
        access_mode_id: Some(access_mode.to_string()),
        reasoning_effort_id: Some(effort.to_string()),
        model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
            kind: FlatModelCatalogKind::Flat,
            model_id: model.to_string(),
        })),
    }
}

fn resolve_claude_selection(
    requested: &TurnSelection,
    managed: &ManagedConversation,
) -> Result<TurnSelection, ProtocolError> {
    let capabilities = claude_capabilities();
    let controls = capabilities.turn_send.expect("Claude turn controls");
    let access_mode = resolve_choice(
        controls.access_mode.as_ref().expect("Claude access modes"),
        requested.access_mode_id.as_deref(),
        Some(managed.access_mode.as_str()),
        "accessModeId",
    )?;
    let effort = resolve_choice(
        controls.reasoning_effort.as_ref().expect("Claude reasoning efforts"),
        requested.reasoning_effort_id.as_deref(),
        managed.effort.as_deref(),
        "reasoningEffortId",
    )?;
    let catalog = match controls.model_catalog.expect("Claude model catalog") {
        ModelCatalog::FlatModelCatalog(catalog) => catalog,
        ModelCatalog::GroupedModelCatalog(_) => unreachable!("Claude uses a flat model catalog"),
    };
    let requested_model = match requested.model.as_ref() {
        Some(ModelSelection::FlatModelSelection(selection)) => Some(selection.model_id.as_str()),
        Some(ModelSelection::GroupedModelSelection(_)) => {
            return Err(protocol_error(
                "invalid_turn_selection",
                "Claude requires a flat model selection".to_string(),
                false,
            ));
        }
        None => None,
    };
    let current_model = managed.model.as_deref().unwrap_or(CLAUDE_DEFAULT_MODEL);
    let model = requested_model
        .or_else(|| catalog.models.iter().any(|option| option.id == current_model).then_some(current_model))
        .or_else(|| catalog.default_selection.as_ref().map(|selection| selection.model_id.as_str()))
        .ok_or_else(|| protocol_error(
            "provider_capability_invalid",
            "Claude model catalog has no default".to_string(),
            false,
        ))?;
    validate_choice(&catalog.models, model, "model")?;
    Ok(claude_selection(&access_mode, &effort, model))
}

fn resolve_choice(
    choices: &ChoiceSet,
    requested: Option<&str>,
    current: Option<&str>,
    field: &str,
) -> Result<String, ProtocolError> {
    let selected = requested
        .or_else(|| current.filter(|value| choices.options.iter().any(|option| option.id == *value)))
        .or(choices.default_id.as_deref())
        .ok_or_else(|| protocol_error(
            "provider_capability_invalid",
            format!("Claude {field} has no default"),
            false,
        ))?;
    validate_choice(&choices.options, selected, field)?;
    Ok(selected.to_string())
}

fn validate_choice(
    options: &[ChoiceOption],
    selected: &str,
    field: &str,
) -> Result<(), ProtocolError> {
    match options.iter().find(|option| option.id == selected) {
        Some(option) if option.enabled != Some(false) => Ok(()),
        Some(option) => Err(protocol_error(
            "invalid_turn_selection",
            option.disabled_reason.clone().unwrap_or_else(|| format!("{field} is disabled: {selected}")),
            false,
        )),
        None => Err(protocol_error(
            "invalid_turn_selection",
            format!("unknown {field}: {selected}"),
            false,
        )),
    }
}

fn selected_claude_model(selection: &TurnSelection) -> Option<String> {
    match selection.model.as_ref() {
        Some(ModelSelection::FlatModelSelection(selection))
            if selection.model_id != CLAUDE_DEFAULT_MODEL => Some(selection.model_id.clone()),
        _ => None,
    }
}

fn decode_settings(
    settings: codepet_provider_sdk::JsonObject,
) -> Result<ClaudeInstanceSettings, ProtocolError> {
    let value = Value::Object(settings.into_iter().collect());
    let settings: ClaudeInstanceSettings = serde_json::from_value(value).map_err(|error| {
        protocol_error(
            "invalid_instance_settings",
            format!("invalid Claude instance settings: {error}"),
            false,
        )
    })?;
    if !settings.claude_executable.is_absolute() {
        return Err(protocol_error(
            "invalid_instance_settings",
            "claudeExecutable must be an absolute path resolved by the Host".to_string(),
            false,
        ));
    }
    if settings
        .claude_config_dir
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(protocol_error(
            "invalid_instance_settings",
            "claudeConfigDir must be absolute when provided".to_string(),
            false,
        ));
    }
    Ok(settings)
}

fn claude_authentication(output: Option<&str>) -> ProviderAuthentication {
    let value = output.and_then(|text| serde_json::from_str::<Value>(text).ok());
    let signed_in = value
        .as_ref()
        .and_then(|value| value.get("loggedIn"))
        .and_then(Value::as_bool);
    ProviderAuthentication {
        status: match signed_in {
            Some(true) => ProviderAuthenticationStatus::SignedIn,
            Some(false) => ProviderAuthenticationStatus::SignedOut,
            None => ProviderAuthenticationStatus::Unknown,
        },
        display_text: Some(match signed_in {
            Some(true) => value
                .as_ref()
                .and_then(|v| v.get("subscriptionType"))
                .and_then(Value::as_str)
                .map(|plan| format!("Signed in · {plan}"))
                .unwrap_or_else(|| "Signed in".into()),
            Some(false) => "Signed out".into(),
            None => "Authentication status unavailable".into(),
        }),
    }
}

fn verify_claude_executable(executable: PathBuf, timeout: Duration, control: local_runtime::RuntimeProbeControl) -> Result<String, ProtocolError> {
    if !executable.is_file() {
        return Err(protocol_error(
            "provider_unavailable",
            format!("Host-resolved Claude executable is unavailable: {}", executable.display()),
            true,
        ));
    }
    let mut child = codepet_provider_sdk::local_runtime::command(&executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| protocol_error(
            "provider_unavailable",
            format!("start Host-resolved Claude executable: {error}"),
            true,
        ))?;
    control.track(child.control()).map_err(|error| protocol_error("runtime_scan_cancelled", error.to_string(), true))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let output = child.wait_with_output().map_err(|error| protocol_error(
                    "provider_unavailable", format!("read Claude version output: {error}"), true))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let line = stdout.lines().chain(stderr.lines()).find(|line| !line.trim().is_empty())
                    .unwrap_or_default().trim();
                if !line.to_ascii_lowercase().contains("claude") {
                    return Err(protocol_error("provider_unavailable", "Executable does not identify itself as Claude".to_string(), false));
                }
                return Ok(line.split_whitespace()
                    .find(|part| part.trim_start_matches('v').chars().next().is_some_and(|character| character.is_ascii_digit()))
                    .unwrap_or(line).trim_start_matches('v').to_string());
            }
            Ok(Some(status)) => {
                return Err(protocol_error(
                    "provider_unavailable",
                    format!("Host-resolved Claude executable rejected --version: {status}"),
                    true,
                ))
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(protocol_error(
                    "provider_unavailable",
                    "Host-resolved Claude executable timed out during --version".to_string(),
                    true,
                ));
            }
            Err(error) => {
                return Err(protocol_error(
                    "provider_unavailable",
                    format!("inspect Host-resolved Claude executable: {error}"),
                    true,
                ))
            }
        }
    }
}

fn discover_path_candidates(command: &str) -> Vec<RuntimeCandidate> {
    let mut candidates = local_runtime::discover(command, "@anthropic-ai/claude-code");
    if let Some(path) = std::env::var_os("CODE_PET_CLAUDE_BIN").filter(|value| !value.is_empty()) {
        candidates.insert(0, local_runtime::candidate(PathBuf::from(path), codepet_provider_sdk::RuntimeCandidateSource::Environment));
    }
    if let Some(home) = local_runtime::home_dir() {
        for directory in [home.join(".local").join("bin")] {
            candidates.extend(local_runtime::candidates_in(&directory, command, "@anthropic-ai/claude-code").into_iter().map(|path| local_runtime::candidate(path, codepet_provider_sdk::RuntimeCandidateSource::CurrentPath)));
        }
    }
    candidates
}

fn inspect_runtime_candidate(candidate: RuntimeCandidate, product: &str, timeout: Duration, control: local_runtime::RuntimeProbeControl) -> Result<RuntimeInstallation, ProtocolError> {
    let canonical = local_runtime::resolve_executable(Path::new(&candidate.executable_path), "claude", "@anthropic-ai/claude-code")
        .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?;
    let version = verify_claude_executable(canonical.clone(), timeout, control).map_err(|error| protocol_error(
        "invalid_runtime_selection", format!("{product} runtime validation failed: {}", error.message), error.retryable))?;
    Ok(RuntimeInstallation { minimum_version: None, incompatibility_reason: None, executable_path: canonical.to_string_lossy().into_owned(), version, source: candidate.source })
}

fn active_conversation_mut<'a>(
    mutable: &'a mut InstanceMutable,
    conversation_id: &str,
    turn_id: &str,
) -> Result<&'a mut ManagedConversation, ProtocolError> {
    let managed = mutable.conversations.get_mut(conversation_id).ok_or_else(|| {
        protocol_error("unknown_conversation", "unknown Claude conversation".to_string(), false)
    })?;
    if managed
        .active_turn
        .as_ref()
        .map(|active| active.turn.resource.native_resource_id.as_str())
        != Some(turn_id)
    {
        return Err(protocol_error(
            "stale_claude_output",
            "Claude output does not target the active turn".to_string(),
            false,
        ));
    }
    Ok(managed)
}

fn validate_claude_session(expected: &str, actual: Option<&str>) -> Result<(), ProtocolError> {
    match actual {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(protocol_error(
            "claude_session_mismatch",
            format!("Claude output session {actual} does not match {expected}"),
            false,
        )),
        None => Err(protocol_error(
            "claude_session_missing",
            "Claude output omitted session_id".to_string(),
            false,
        )),
    }
}

fn completion_for_exit(
    active: &ManagedTurn,
    outcome: &Result<std::process::ExitStatus, ClaudeCliError>,
) -> TurnCompletion {
    if let Some(completion) = active.requested_completion.clone() {
        return completion;
    }
    if let Some(message) = active.stream_failure.as_ref() {
        return TurnCompletion {
            status: TurnStatus::Failed,
            result: Some(message.clone()),
            terminal_reason: Some("claude_stream_failed".to_string()),
        };
    }
    if let Some(mut completion) = active.pending_completion.clone() {
        if completion.status == TurnStatus::Completed
            && !matches!(outcome, Ok(status) if status.success())
        {
            completion.status = TurnStatus::Failed;
            completion.result = Some(process_exit_reason(outcome));
            completion.terminal_reason = Some("process_exit".to_string());
        }
        return completion;
    }
    TurnCompletion {
        status: TurnStatus::Failed,
        result: Some(process_exit_reason(outcome)),
        terminal_reason: Some("process_exit".to_string()),
    }
}

fn process_exit_reason(
    outcome: &Result<std::process::ExitStatus, ClaudeCliError>,
) -> String {
    match outcome {
        Ok(status) if status.success() => "Claude CLI exited without a result frame".to_string(),
        Ok(status) => format!("Claude CLI exited before a result frame: {:?}", status.code()),
        Err(error) => error.to_string(),
    }
}

fn terminal_snapshot(
    mut turn: TurnTask,
    mut conversation: Conversation,
    completion: &TurnCompletion,
) -> (TurnTask, Conversation) {
    let now = now_ms();
    turn.status = completion.status;
    turn.updated_at = Some(now);
    turn.completed_at = Some(now);
    conversation.status = match completion.status {
        TurnStatus::Failed => ConversationStatus::Error,
        _ => ConversationStatus::Idle,
    };
    conversation.active_turn = None;
    conversation.updated_at = Some(now);
    if let Some(result) = completion.result.as_ref().filter(|value| !value.is_empty()) {
        conversation.preview = Some(result.chars().take(240).collect());
    }
    (turn, conversation)
}

fn text_chunks(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let mut end = text.len().min(MAX_PROVIDER_TEXT_CHUNK_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let (chunk, remaining) = text.split_at(end);
        text = remaining;
        Some(chunk)
    })
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn claude_config_dir() -> Option<PathBuf> {
    local_runtime::data_dir("CLAUDE_CONFIG_DIR", ".claude")
}

fn default_remote_workspace_root(harness_name: &str) -> Option<String> {
    local_runtime::home_dir()
        .filter(|path| path.is_absolute())
        .map(|path| {
            path.join(".codepet")
                .join("remote_workspace")
                .join(harness_name)
                .to_string_lossy()
                .into_owned()
        })
}

fn ensure_claude_workspace(path: &str) -> Result<PathBuf, ProtocolError> {
    let workspace = PathBuf::from(path);
    if !workspace.is_absolute() {
        return Err(protocol_error(
            "invalid_conversation_options",
            "workspaceRoot must be an absolute directory".to_string(),
            false,
        ));
    }
    std::fs::create_dir_all(&workspace).map_err(|error| {
        protocol_error(
            "workspace_create_failed",
            format!("failed to create Claude workspaceRoot: {error}"),
            false,
        )
    })?;
    Ok(workspace)
}

fn discover_claude_conversations(
    config_dir: &Path, route: &ProviderInstanceRoute,
) -> Result<Vec<DiscoveredConversation>, ProtocolError> {
    discover_claude_conversations_with_mode(config_dir, route, false, None, None)
}

fn discovery_error(path: &Path, error: impl std::fmt::Display) -> ProtocolError {
    protocol_error("conversation_query_incomplete", format!("cannot completely read Claude history {}: {error}", path.display()), true)
}

fn discover_claude_conversations_with_mode(
    config_dir: &Path, route: &ProviderInstanceRoute, strict: bool, cancelled: Option<&AtomicBool>, mut cache: Option<&mut HistorySummaryCache>,
) -> Result<Vec<DiscoveredConversation>, ProtocolError> {
    let projects_dir = config_dir.join("projects");
    let projects = match std::fs::read_dir(&projects_dir) {
        Ok(projects) => projects,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound || !strict => return Ok(Vec::new()),
        Err(error) => return Err(discovery_error(&projects_dir, error)),
    };
    let mut discovered = Vec::new();
    let mut seen_paths = HashSet::new();
    for project in projects {
        check_discovery_cancelled(cancelled)?;
        let project = match project { Ok(project) => project, Err(error) if strict => return Err(discovery_error(&projects_dir, error)), Err(_) => continue };
        let file_type = match project.file_type() { Ok(kind) => kind, Err(error) if strict => return Err(discovery_error(&project.path(), error)), Err(_) => continue };
        if !file_type.is_dir() { continue; }
        let histories = match std::fs::read_dir(project.path()) { Ok(histories) => histories, Err(error) if strict => return Err(discovery_error(&project.path(), error)), Err(_) => continue };
        for history in histories {
            check_discovery_cancelled(cancelled)?;
            let history = match history { Ok(history) => history, Err(error) if strict => return Err(discovery_error(&project.path(), error)), Err(_) => continue };
            let path = history.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") { continue; }
            seen_paths.insert(path.clone());
            if let Some(cache) = cache.as_deref_mut() {
                let metadata = std::fs::metadata(&path).map_err(|error| discovery_error(&path, error))?;
                let modified = metadata.modified().map_err(|error| discovery_error(&path, error))?;
                if let Some((cached_time, cached_size, row)) = cache.get(&path) {
                    if *cached_time == modified && *cached_size == metadata.len() { discovered.push(row.clone()); continue; }
                }
                if let Some(row) = summarize_claude_history_with_mode(&path, route, strict, cancelled)? {
                    cache.insert(path, (modified, metadata.len(), row.clone())); discovered.push(row);
                }
            } else if let Some(row) = summarize_claude_history_with_mode(&path, route, strict, cancelled)? { discovered.push(row); }
        }
    }
    if let Some(cache) = cache { cache.retain(|path, _| seen_paths.contains(path)); }
    Ok(discovered)
}

fn summarize_claude_history(
    path: &Path,
    route: &ProviderInstanceRoute,
) -> Result<Option<DiscoveredConversation>, ProtocolError> {
    summarize_claude_history_with_mode(path, route, false, None)
}

fn summarize_claude_history_with_mode(path: &Path, route: &ProviderInstanceRoute, strict: bool, cancelled: Option<&AtomicBool>) -> Result<Option<DiscoveredConversation>, ProtocolError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if !strict || error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(discovery_error(path, error)),
    };
    let mut session_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string();
    if session_id.is_empty() {
        return Ok(None);
    }
    let mut workspace_root = None;
    let mut generated_title = None;
    let mut first_user_text = None;
    let mut latest_assistant_text = None;
    let mut model = None;
    for line in BufReader::new(file).lines() {
        check_discovery_cancelled(cancelled)?;
        let line = match line { Ok(line) => line, Err(error) if strict => return Err(discovery_error(path, error)), Err(_) => break };
        if line.trim().is_empty() { continue; }
        let record = match serde_json::from_str::<Value>(&line) {
            Ok(record) => record,
            Err(error) if strict => return Err(discovery_error(path, error)),
            Err(_) => continue,
        };
        if let Some(value) = record.get("sessionId").and_then(Value::as_str) {
            if !value.trim().is_empty() {
                session_id = value.to_string();
            }
        }
        if workspace_root.is_none() {
            workspace_root = record
                .get("cwd")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from);
        }
        if record.get("type").and_then(Value::as_str) == Some("ai-title") {
            generated_title = record
                .get("aiTitle")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned);
        }
        let Some(message) = record.get("message") else {
            continue;
        };
        let Some(role) = message.get("role").and_then(Value::as_str) else {
            continue;
        };
        let Some(text) = message.get("content").and_then(claude_message_text) else {
            continue;
        };
        match role {
            "user" if first_user_text.is_none() => first_user_text = Some(text),
            "assistant" => {
                latest_assistant_text = Some(text);
                if let Some(value) = message.get("model").and_then(Value::as_str) {
                    if !value.trim().is_empty() {
                        model = Some(value.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let Some(workspace_root) = workspace_root.filter(|path| path.is_absolute()) else {
        return Ok(None);
    };
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if strict => return Err(discovery_error(path, error)),
        Err(_) => None,
    };
    if strict { metadata.as_ref().expect("metadata checked").modified().map_err(|error| discovery_error(path, error))?; }
    let updated_at = metadata
        .as_ref()
        .and_then(|value| value.modified().ok())
        .and_then(system_time_ms)
        .unwrap_or_default();
    let created_at = metadata
        .as_ref()
        .and_then(|value| value.created().ok())
        .and_then(system_time_ms)
        .unwrap_or(updated_at);
    let title = generated_title
        .or_else(|| first_user_text.as_deref().map(|value| truncate_text(value, 120)))
        .unwrap_or_else(|| session_id.clone());
    let resource = RoutedResourceId {
        provider_id: route.provider_instance_id.clone(),
        native_resource_id: session_id,
    };
    Ok(Some(DiscoveredConversation {
        conversation: Conversation {
            resource,
            project: None,
            title: title.clone(),
            preview: latest_assistant_text.map(|value| truncate_text(&value, 240)),
            status: ConversationStatus::Idle,
            permission_level: Some("workspace-write".to_string()),
            model: model.clone(),
            reasoning_effort: None,
            selection: Some(claude_selection(
                CLAUDE_DEFAULT_ACCESS_MODE,
                CLAUDE_DEFAULT_EFFORT,
                model
                    .as_deref()
                    .filter(|model| ["sonnet", "opus", "fable"].contains(model))
                    .unwrap_or(CLAUDE_DEFAULT_MODEL),
            )),
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            created_at: Some(created_at),
            updated_at: Some(updated_at),
            active_turn: None,
            read_state: None,
        },
        workspace_root,
        title: Some(title),
        model,
        history_path: path.to_path_buf(),
    }))
}

fn read_claude_history_items(
    path: &Path,
    route: &ProviderInstanceRoute,
    conversation: &RoutedResourceId,
) -> Result<Vec<ConversationItem>, ProtocolError> {
    let file = File::open(path).map_err(|error| {
        protocol_error(
            "claude_history_unavailable",
            format!("open Claude conversation history: {error}"),
            true,
        )
    })?;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut tool_indexes = HashMap::<String, usize>::new();
    for (index, line) in BufReader::new(file).lines().map_while(Result::ok).enumerate() {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(message) = record.get("message") else {
            continue;
        };
        let role = match message.get("role").and_then(Value::as_str) {
            Some("user") => ConversationItemRole::User,
            Some("assistant") => ConversationItemRole::Assistant,
            _ => continue,
        };
        let native_id = record
            .get("uuid")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("history-{index}"));
        if !seen.insert(native_id.clone()) {
            continue;
        }
        let turn = routed_resource(route, format!("{native_id}:turn"));
        if let Some(text) = message.get("content").and_then(claude_message_text) {
            items.push(ConversationItem::MessageConversationItem(MessageConversationItem {
                meta: None,
                resource: routed_resource(route, native_id.clone()),
                turn: turn.clone(),
                conversation: conversation.clone(),
                kind: MessageConversationItemKind::Message,
                status: ConversationItemStatus::Completed,
                role,
                contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                    content_id: format!("{native_id}:text"),
                    kind: TextContentBlockKind::Text,
                    text,
                    truncation: None,
                })],
            }));
        }
        let Some(parts) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for (part_index, part) in parts.iter().enumerate() {
            match part.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let Some(call_id) = part.get("id").and_then(Value::as_str) else { continue };
                    let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let input_value = part.get("input").cloned().unwrap_or_else(|| json!({}));
                    let command = input_value.as_object().and_then(|input| input.get("command")).and_then(Value::as_str);
                    let input = if let Some(command) = command {
                        ToolInput::CommandToolInput(CommandToolInput {
                            kind: CommandToolInputKind::Command,
                            command: command.to_string(),
                            cwd: input_value.as_object().and_then(|input| input.get("cwd")).and_then(Value::as_str).map(str::to_string),
                            shell: None,
                            truncation: None,
                            actions: None,
                        })
                    } else if let Some(object) = input_value.as_object() {
                        ToolInput::StructuredToolInput(StructuredToolInput {
                            kind: StructuredToolInputKind::Structured,
                            value: object.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
                            truncation: None,
                        })
                    } else {
                        ToolInput::OpaqueToolInput(OpaqueToolInput {
                            kind: OpaqueToolInputKind::Opaque,
                            value: input_value.to_string(),
                            mime_type: Some("application/json".to_string()),
                            truncation: None,
                        })
                    };
                    let title = command.map(concise_claude_title).unwrap_or_else(|| name.to_string());
                    let tool = ToolInvocation {
                        call_id: call_id.to_string(),
                        name: name.to_string(),
                        namespace: None,
                        category: claude_tool_category(name, command.is_some()),
                        origin: ToolOrigin { kind: ToolOriginKind::Server, name: Some("claude".to_string()) },
                        input,
                        outcome: None,
                        timing: None,
                        annotations: None,
                    };
                    let item = if command.is_some() {
                        ConversationItem::CommandConversationItem(CommandConversationItem {
                        meta: None,
                        resource: routed_resource(route, call_id.to_string()),
                        turn: turn.clone(),
                        conversation: conversation.clone(),
                        kind: CommandConversationItemKind::Command,
                        status: ConversationItemStatus::Completed,
                        title: Some(title),
                        tool,
                    })
                    } else {
                        ConversationItem::ToolConversationItem(ToolConversationItem {
                            meta: None,
                            resource: routed_resource(route, call_id.to_string()),
                            turn: turn.clone(), conversation: conversation.clone(),
                            kind: ToolConversationItemKind::Tool,
                            status: ConversationItemStatus::Completed,
                            title: Some(title), tool,
                        })
                    };
                    tool_indexes.insert(call_id.to_string(), items.len());
                    items.push(item);
                }
                Some("tool_result") => {
                    let Some(call_id) = part.get("tool_use_id").and_then(Value::as_str) else { continue };
                    let Some(item_index) = tool_indexes.get(call_id).copied() else { continue };
                    let is_error = part.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    let text = part.get("content").and_then(claude_result_text);
                    let content = text.as_ref().map(|text| ContentBlock::OutputContentBlock(OutputContentBlock {
                            content_id: format!("{call_id}:result:{part_index}"),
                            kind: OutputContentBlockKind::Output,
                            text: text.clone(),
                            truncation: None,
                        })).into_iter().collect();
                    let outcome = if is_error {
                        ToolOutcome::ToolFailureOutcome(ToolFailureOutcome {
                            kind: ToolFailureOutcomeKind::Failure,
                            content,
                            error: ToolExecutionError {
                                code: None,
                                message: "Claude tool execution failed".to_string(),
                                retryable: None,
                            },
                            exit_code: None,
                            process_id: None,
                        })
                    } else {
                        ToolOutcome::ToolSuccessOutcome(ToolSuccessOutcome {
                            kind: ToolSuccessOutcomeKind::Success,
                            content,
                            exit_code: None,
                            process_id: None,
                        })
                    };
                    match &mut items[item_index] {
                        ConversationItem::CommandConversationItem(item) => {
                            item.tool.outcome = Some(outcome);
                            if is_error { item.status = ConversationItemStatus::Failed; }
                        }
                        ConversationItem::ToolConversationItem(item) => {
                            item.tool.outcome = Some(outcome);
                            if is_error { item.status = ConversationItemStatus::Failed; }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    for item in &mut items {
        codepet_provider_sdk::truncate_tool_item_text(item, codepet_provider_sdk::DEFAULT_TOOL_TEXT_BYTES);
    }
    Ok(items)
}

fn paginate_claude_history(
    items: Vec<ConversationItem>,
    cursor: Option<&str>,
    requested_limit: Option<u64>,
) -> Result<(Vec<ConversationItem>, Option<String>), ProtocolError> {
    let limit = requested_limit.unwrap_or(DEFAULT_CONVERSATION_GET_TURN_LIMIT);
    if !(1..=MAX_CONVERSATION_GET_TURN_LIMIT).contains(&limit) {
        return Err(protocol_error(
            "invalid_request",
            format!(
                "conversation.get limit must be between 1 and {MAX_CONVERSATION_GET_TURN_LIMIT}"
            ),
            false,
        ));
    }
    let mut turn_starts = Vec::new();
    let mut previous_turn = None;
    for (index, item) in items.iter().enumerate() {
        let turn = conversation_item_turn(item);
        if previous_turn != Some(turn) {
            turn_starts.push(index);
            previous_turn = Some(turn);
        }
    }
    if turn_starts.is_empty() {
        if cursor.is_some() {
            return Err(protocol_error(
                "invalid_cursor",
                "Claude conversation cursor is outside the history".to_string(),
                false,
            ));
        }
        return Ok((Vec::new(), None));
    }
    let turn_count = turn_starts.len();
    let end_turn = match cursor {
        None => turn_count,
        Some(cursor) => turn_starts
            .iter()
            .position(|start| {
                conversation_item_turn(&items[*start]).native_resource_id == cursor
            })
            .map(|index| index + 1)
            .ok_or_else(|| {
                protocol_error(
                    "invalid_cursor",
                    "Claude conversation cursor is outside the history".to_string(),
                    false,
                )
            })?,
    };
    let start_turn = end_turn.saturating_sub(limit as usize);
    let start_item = turn_starts[start_turn];
    let end_item = turn_starts.get(end_turn).copied().unwrap_or(items.len());
    let next_cursor = (start_turn > 0).then(|| {
        conversation_item_turn(&items[turn_starts[start_turn - 1]])
            .native_resource_id
            .clone()
    });
    Ok((items[start_item..end_item].to_vec(), next_cursor))
}

fn conversation_item_turn(item: &ConversationItem) -> &RoutedResourceId {
    match item {
        ConversationItem::MessageConversationItem(item) => &item.turn,
        ConversationItem::ReasoningConversationItem(item) => &item.turn,
        ConversationItem::CommandConversationItem(item) => &item.turn,
        ConversationItem::FileChangeConversationItem(item) => &item.turn,
        ConversationItem::ToolConversationItem(item) => &item.turn,
        ConversationItem::ApprovalConversationItem(item) => &item.turn,
        ConversationItem::UnknownConversationItem(item) => &item.turn,
    }
}

fn claude_result_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let text = value.as_array()?.iter().filter_map(|part| {
        part.get("text").and_then(Value::as_str)
    }).collect::<Vec<_>>().join("\n");
    (!text.is_empty()).then_some(text)
}

fn claude_tool_category(name: &str, command: bool) -> ToolCategory {
    if command {
        return ToolCategory::Command;
    }
    let name = name.to_ascii_lowercase();
    if name.contains("read") || name.contains("list") {
        ToolCategory::Read
    } else if name.contains("write") || name.contains("edit") {
        ToolCategory::Write
    } else if name.contains("search") || name.contains("glob") || name.contains("grep") {
        ToolCategory::Search
    } else if name.contains("web") {
        ToolCategory::Web
    } else if name.contains("agent") || name.contains("task") {
        ToolCategory::Agent
    } else {
        ToolCategory::Other
    }
}

fn concise_claude_title(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or(command).trim();
    if first_line.chars().count() <= 80 {
        first_line.to_string()
    } else {
        format!("{}…", first_line.chars().take(79).collect::<String>())
    }
}

fn claude_message_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.trim().is_empty()).then(|| text.to_string());
    }
    let parts = value.as_array()?.iter().filter_map(|part| {
        if part.get("type").and_then(Value::as_str) != Some("text") {
            return None;
        }
        part.get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
    });
    let text = parts.collect::<Vec<_>>().join("\n");
    (!text.is_empty()).then_some(text)
}

fn routed_resource(route: &ProviderInstanceRoute, native_resource_id: String) -> RoutedResourceId {
    RoutedResourceId {
        provider_id: route.provider_instance_id.clone(),
        native_resource_id,
    }
}

fn provider_resource(
    route: &ProviderInstanceRoute,
    native_resource_id: String,
) -> ProviderResourceId {
    ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id,
    }
}

fn system_time_ms(value: SystemTime) -> Option<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn result_status(subtype: &str, is_error: bool, terminal_reason: Option<&str>) -> TurnStatus {
    if terminal_reason.is_some_and(|reason| {
        reason.eq_ignore_ascii_case("interrupted")
            || reason.eq_ignore_ascii_case("cancelled")
            || reason.eq_ignore_ascii_case("canceled")
    }) {
        return TurnStatus::Interrupted;
    }
    if is_error || subtype != "success" {
        TurnStatus::Failed
    } else {
        TurnStatus::Completed
    }
}

fn validate_permission_level(permission_level: &str) -> Result<(), ProtocolError> {
    if permission_level == "workspace-write" {
        Ok(())
    } else {
        Err(protocol_error(
            "invalid_permission_level",
            format!("unsupported Claude permission level: {permission_level}"),
            false,
        ))
    }
}

fn validate_route(route: &ProviderInstanceRoute) -> Result<(), ProtocolError> {
    if route.device_id.trim().is_empty()
        || route.provider_plugin_id.trim().is_empty()
        || route.provider_instance_id.trim().is_empty()
    {
        return Err(protocol_error(
            "invalid_provider_route",
            "Provider route fields must not be empty".to_string(),
            false,
        ));
    }
    if route.provider_plugin_id != CLAUDE_PLUGIN_ID {
        return Err(protocol_error(
            "wrong_provider_route",
            "Provider route targets a different plugin".to_string(),
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
            "Provider native resource id must not be empty".to_string(),
            false,
        ));
    }
    Ok(())
}

fn validate_resource_for_instance(
    resource: &ProviderResourceId,
    route: &ProviderInstanceRoute,
) -> Result<(), ProtocolError> {
    validate_resource(resource)?;
    if resource.device_id != route.device_id
        || resource.provider_plugin_id != route.provider_plugin_id
        || resource.provider_instance_id != route.provider_instance_id
    {
        return Err(protocol_error(
            "resource_route_mismatch",
            "Provider resource targets a different instance route".to_string(),
            false,
        ));
    }
    Ok(())
}

fn provider_unavailable(route: &ProviderInstanceRoute) -> ProtocolError {
    protocol_error(
        "provider_unavailable",
        format!("Claude Provider instance {} is not ready", route.provider_instance_id),
        true,
    )
}

fn cli_protocol_error(error: ClaudeCliError) -> ProtocolError {
    protocol_error("claude_cli_error", error.to_string(), true)
}

fn capability_error(message: &str) -> ProtocolError {
    protocol_error("capability_unsupported", message.to_string(), false)
}

fn protocol_error(code: &str, message: String, retryable: bool) -> ProtocolError {
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
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn extension<const N: usize>(entries: [(&str, Value); N]) -> ProviderExtension {
    ProviderExtension {
        namespace: CLAUDE_EXTENSION_NAMESPACE.to_string(),
        data: entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_missing_standalone_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("task-1");

        let prepared = ensure_claude_workspace(workspace.to_str().unwrap()).unwrap();

        assert_eq!(prepared, workspace);
        assert!(workspace.is_dir());
    }

    #[test]
    fn discovers_and_reads_persisted_claude_jsonl_conversations() {
        let config = tempfile::tempdir().unwrap();
        let project = config.path().join("projects/project-fixture");
        std::fs::create_dir_all(&project).unwrap();
        let history = project.join("session-fixture.jsonl");
        let records = [
            json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": "session-fixture",
                "cwd": config.path(),
                "message": { "role": "user", "content": "first question" }
            }),
            json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "sessionId": "session-fixture",
                "cwd": config.path(),
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet",
                    "content": [
                        { "type": "text", "text": "final answer" },
                        {
                            "type": "tool_use",
                            "id": "tool-1",
                            "name": "Bash",
                            "input": { "command": "npm test", "cwd": "/workspace" }
                        }
                    ]
                }
            }),
            json!({
                "type": "user",
                "uuid": "tool-result-1",
                "sessionId": "session-fixture",
                "cwd": config.path(),
                "message": {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "tool-1",
                        "content": "tests passed",
                        "is_error": false
                    }]
                }
            }),
            json!({
                "type": "ai-title",
                "sessionId": "session-fixture",
                "cwd": config.path(),
                "aiTitle": "Recovered title"
            }),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&history, records).unwrap();
        let route = ProviderInstanceRoute {
            device_id: "device".to_string(),
            provider_plugin_id: CLAUDE_PLUGIN_ID.to_string(),
            provider_instance_id: "claude".to_string(),
        };

        let discovered = discover_claude_conversations(config.path(), &route).unwrap();

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].conversation.title, "Recovered title");
        assert_eq!(discovered[0].conversation.preview.as_deref(), Some("final answer"));
        assert_eq!(discovered[0].conversation.model.as_deref(), Some("claude-sonnet"));
        assert_eq!(
            discovered[0].conversation.resource.native_resource_id,
            "session-fixture"
        );
        let items = read_claude_history_items(
            &history,
            &route,
            &discovered[0].conversation.resource,
        )
        .unwrap();
        assert_eq!(items.len(), 3);
        let ConversationItem::MessageConversationItem(user) = &items[0] else { panic!("user message") };
        assert_eq!(user.role, ConversationItemRole::User);
        let ContentBlock::TextContentBlock(user_text) = &user.contents[0] else { panic!("text") };
        assert_eq!(user_text.text, "first question");
        let ConversationItem::MessageConversationItem(assistant) = &items[1] else { panic!("assistant message") };
        assert_eq!(assistant.role, ConversationItemRole::Assistant);
        let ContentBlock::TextContentBlock(assistant_text) = &assistant.contents[0] else { panic!("text") };
        assert_eq!(assistant_text.text, "final answer");
        let ConversationItem::CommandConversationItem(command) = &items[2] else { panic!("command") };
        assert_eq!(command.title.as_deref(), Some("npm test"));
        let tool = &command.tool;
        assert_eq!(tool.name, "Bash");
        let ToolInput::CommandToolInput(input) = &tool.input else { panic!("command input") };
        assert_eq!(input.cwd.as_deref(), Some("/workspace"));
        assert!(input.truncation.is_none());
        let Some(ToolOutcome::ToolSuccessOutcome(outcome)) = &tool.outcome else { panic!("success") };
        let ContentBlock::OutputContentBlock(output) = &outcome.content[0] else { panic!("output") };
        assert_eq!(output.text, "tests passed");
    }

    #[test]
    fn paginates_persisted_history_by_turn_and_preserves_the_cursor_boundary() {
        let route = ProviderInstanceRoute {
            device_id: "device".to_string(),
            provider_plugin_id: CLAUDE_PLUGIN_ID.to_string(),
            provider_instance_id: "claude".to_string(),
        };
        let conversation = routed_resource(&route, "conversation".to_string());
        let items = ["oldest", "middle", "newest"]
            .into_iter()
            .map(|id| ConversationItem::MessageConversationItem(MessageConversationItem {
                meta: None,
                resource: routed_resource(&route, id.to_string()),
                turn: routed_resource(&route, format!("{id}:turn")),
                conversation: conversation.clone(),
                kind: MessageConversationItemKind::Message,
                status: ConversationItemStatus::Completed,
                role: ConversationItemRole::Assistant,
                contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                    content_id: format!("{id}:text"),
                    kind: TextContentBlockKind::Text,
                    text: id.to_string(),
                    truncation: None,
                })],
            }))
            .collect::<Vec<_>>();

        let (narrow, narrow_cursor) =
            paginate_claude_history(items.clone(), None, Some(1)).unwrap();
        let (wide, wide_cursor) =
            paginate_claude_history(items.clone(), None, Some(2)).unwrap();
        let (continued, continued_cursor) =
            paginate_claude_history(items, narrow_cursor.as_deref(), Some(1)).unwrap();

        assert_eq!(narrow.len(), 1);
        assert_eq!(item_resource(&narrow[0]).native_resource_id, "newest");
        assert_eq!(narrow_cursor.as_deref(), Some("middle:turn"));
        assert_eq!(wide.len(), 2);
        assert_eq!(item_resource(&wide[0]).native_resource_id, "middle");
        assert_eq!(item_resource(&wide[1]).native_resource_id, "newest");
        assert_eq!(wide_cursor.as_deref(), Some("oldest:turn"));
        assert_eq!(continued.len(), 1);
        assert_eq!(item_resource(&continued[0]).native_resource_id, "middle");
        assert_eq!(continued_cursor.as_deref(), Some("oldest:turn"));
    }

    fn item_resource(item: &ConversationItem) -> &RoutedResourceId {
        match item {
            ConversationItem::MessageConversationItem(item) => &item.resource,
            _ => panic!("message item"),
        }
    }
}

#[cfg(test)]
mod storage_path_tests {
    use super::*;
    #[test]
    fn executable_and_data_directory_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("custom data 中文");
        let executable = std::env::current_exe().unwrap();
        let settings = decode_settings(serde_json::from_value(json!({
            "claudeExecutable": executable,
            "dataDirectory": data,
        })).unwrap()).unwrap();
        assert_eq!(settings.claude_config_dir.as_deref(), Some(data.as_path()));
        let invalid = decode_settings(serde_json::from_value(json!({
            "claudeExecutable": executable,
            "dataDirectory": "relative-storage",
        })).unwrap());
        assert!(invalid.is_err());
    }
}

impl ClaudeInstanceRuntime {
    fn query_generation(&self) -> Result<String, ProtocolError> {
        if self.status() != InstanceStatus::Ready { return Err(provider_unavailable(&self.route)); }
        Ok(lock(&self.mutable).lifecycle_generation.to_string())
    }
}

impl ClaudeProvider {
    async fn complete_conversation_summaries(&self, route: &ProviderInstanceRoute) -> Result<Vec<Conversation>, ProtocolError> {
        self.instance(route)?.collect_atomic_summaries().await
    }
}
impl ClaudeInstanceRuntime {
    async fn collect_atomic_summaries(self: &Arc<Self>) -> Result<Vec<Conversation>, ProtocolError> {
        if self.status() != InstanceStatus::Ready { return Err(provider_unavailable(&self.route)); }
        let runtime = self.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelDiscovery(cancelled.clone());
        tokio::task::spawn_blocking(move || {
            runtime.refresh_discovered_conversations_strict(true, Some(&cancelled))?;
            let rows = lock(&runtime.mutable).conversations.values().map(|entry| entry.conversation.clone()).collect();
            Ok(rows)
        }).await.map_err(|error| protocol_error("provider_task_failed", error.to_string(), false))?
    }
}

#[cfg(test)]
mod atomic_query_tests {
    use super::*;
    #[test]
    fn concurrent_scan_cannot_install_newer_history_before_older_scan_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("projects/workspace");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("session.jsonl");
        let record = |text| serde_json::to_vec(&json!({"sessionId":"session","cwd":directory.path(),"message":{"role":"assistant","content":text}})).unwrap();
        std::fs::write(&path, record("old summary")).unwrap();
        let route = ProviderInstanceRoute { device_id: "device".into(), provider_plugin_id: CLAUDE_PLUGIN_ID.into(), provider_instance_id: "claude".into() };
        let runtime = Arc::new(ClaudeInstanceRuntime::new(
            InstanceCreateRequest { route, instance_kind: CLAUDE_INSTANCE_KIND.into(), display_name: "scan fixture".into(), settings: Default::default() },
            ClaudeInstanceSettings { claude_executable: PathBuf::from("unused"), claude_config_dir: Some(directory.path().to_path_buf()) },
            Arc::new(|_| Ok(())),
        ));
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first_runtime = runtime.clone();
        let first = std::thread::spawn(move || first_runtime.refresh_discovered_conversations_with_hook(true, None, || {
            read_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        read_rx.recv().unwrap();
        std::fs::write(&path, record("newer summary with changed size")).unwrap();
        assert!(matches!(runtime.history_cache.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let second_runtime = runtime.clone();
        let second = std::thread::spawn(move || {
            attempt_tx.send(()).unwrap();
            second_runtime.refresh_discovered_conversations_strict(true, None)
        });
        attempt_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert_eq!(lock(&runtime.mutable).conversations["session"].conversation.preview.as_deref(), Some("newer summary with changed size"));
    }

    #[test]
    fn history_summary_cache_refreshes_changed_files_and_removes_confirmed_missing_paths() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("projects").join("workspace");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("session.jsonl");
        let route = ProviderInstanceRoute { device_id: "device".into(), provider_plugin_id: CLAUDE_PLUGIN_ID.into(), provider_instance_id: "claude".into() };
        let record = |text| serde_json::to_vec(&json!({"sessionId":"session","cwd":directory.path(),"message":{"role":"assistant","content":text}})).unwrap();
        std::fs::write(&path, record("first")).unwrap();
        let mut cache = HashMap::new();
        let first = discover_claude_conversations_with_mode(directory.path(), &route, true, None, Some(&mut cache)).unwrap();
        assert_eq!(first[0].conversation.preview.as_deref(), Some("first"));
        std::fs::write(&path, record("second response with a different size")).unwrap();
        let second = discover_claude_conversations_with_mode(directory.path(), &route, true, None, Some(&mut cache)).unwrap();
        assert_eq!(second[0].conversation.preview.as_deref(), Some("second response with a different size"));
        std::fs::remove_file(path).unwrap();
        assert!(discover_claude_conversations_with_mode(directory.path(), &route, true, None, Some(&mut cache)).unwrap().is_empty());
        assert!(cache.is_empty());
        let cancelled = AtomicBool::new(true);
        assert!(discover_claude_conversations_with_mode(directory.path(), &route, true, Some(&cancelled), Some(&mut cache)).is_err());
    }

    #[test]
    fn strict_history_discovery_enumerates_all_and_does_not_hide_corrupt_files() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("projects").join("workspace");
        std::fs::create_dir_all(&project).unwrap();
        let route = ProviderInstanceRoute { device_id: "device".into(), provider_plugin_id: CLAUDE_PLUGIN_ID.into(), provider_instance_id: "claude".into() };
        for index in 0..237 {
            std::fs::write(project.join(format!("session-{index}.jsonl")), serde_json::to_vec(&json!({"sessionId":format!("session-{index}"),"cwd":directory.path(),"message":{"role":"assistant","content":"answer"}})).unwrap()).unwrap();
        }
        assert_eq!(discover_claude_conversations_with_mode(directory.path(), &route, true, None, None).unwrap().len(), 237);
        std::fs::write(project.join("broken.jsonl"), "{partial").unwrap();
        assert!(discover_claude_conversations_with_mode(directory.path(), &route, true, None, None).is_err());
        assert_eq!(discover_claude_conversations(directory.path(), &route).unwrap().len(), 237);
    }
}

struct CancelDiscovery(Arc<AtomicBool>);
impl Drop for CancelDiscovery { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
fn check_discovery_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), ProtocolError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        return Err(protocol_error("conversation_query_cancelled", "history discovery was cancelled".into(), true));
    }
    Ok(())
}
