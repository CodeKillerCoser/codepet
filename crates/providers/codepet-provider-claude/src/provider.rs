use crate::client::{
    ClaudeCliError, ClaudeProcessControl, ClaudeTurnLaunch, SpawnedClaudeTurn,
};
use crate::protocol::{ClaudeOutput, ClaudeStreamDelta, ClaudeStreamEvent};
use codepet_provider_sdk::{
    ApprovalResolveRequest, ApprovalResolveResponse, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ConversationStatus,
    ConversationUpsertedEvent, InstanceCapabilitiesRequest, InstanceCapabilitiesResponse,
    InstanceCreateRequest, InstanceCreateResponse, InstanceDestroyRequest,
    InstanceDestroyResponse, InstanceStartRequest, InstanceStartResponse, InstanceStatus,
    InstanceStatusChangedEvent, InstanceStopRequest, InstanceStopResponse, ProtocolError,
    ProtocolEvent, ProtocolFuture, ProtocolServer, ProviderCapabilities, ProviderCapability,
    ProviderConversation, ProviderDescribeRequest, ProviderDescribeResponse, ProviderExtension,
    ProviderInitializeRequest, ProviderInitializeResponse, ProviderInstance, ProviderInstanceRoute,
    ProviderPluginDescriptor, ProviderShutdownRequest, ProviderShutdownResponse, ProviderTurn,
    RoutedResourceId, TurnInterruptRequest, TurnInterruptResponse, TurnOutputDeltaEvent,
    TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest, TurnSteerResponse,
    TurnUpsertedEvent, VersionRange, PROTOCOL_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::{Command, Stdio};
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct ClaudeInstanceSettings {
    claude_executable: PathBuf,
}

pub trait ProviderEventSink: Send + Sync + 'static {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError>;
}

impl<F> ProviderEventSink for F
where
    F: Fn(ProtocolEvent) -> Result<(), ProtocolError> + Send + Sync + 'static,
{
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        self(event)
    }
}

struct ManagedTurn {
    turn: ProviderTurn,
    control: ClaudeProcessControl,
    saw_text_delta: bool,
    pending_completion: Option<TurnCompletion>,
    requested_completion: Option<TurnCompletion>,
    stream_failure: Option<String>,
    finished: Arc<TurnFinished>,
}

#[derive(Clone)]
struct TurnCompletion {
    status: TurnStatus,
    result: Option<String>,
    stop_reason: Option<String>,
    terminal_reason: Option<String>,
}

#[derive(Default)]
struct TurnFinished {
    outcome: Mutex<Option<Result<ProviderTurn, ProtocolError>>>,
    changed: Condvar,
}

impl TurnFinished {
    fn complete(&self, outcome: Result<ProviderTurn, ProtocolError>) {
        let mut stored = lock(&self.outcome);
        if stored.is_none() {
            *stored = Some(outcome);
            self.changed.notify_all();
        }
    }

    fn wait(&self, timeout: Duration) -> Option<Result<ProviderTurn, ProtocolError>> {
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
    conversation: ProviderConversation,
    workspace_root: PathBuf,
    title: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    materialized: bool,
    active_turn: Option<ManagedTurn>,
}

struct InstanceMutable {
    status: InstanceStatus,
    conversations: HashMap<String, ManagedConversation>,
}

struct ClaudeInstanceRuntime {
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
        Self {
            route: request.route,
            instance_kind: request.instance_kind,
            display_name: request.display_name,
            settings,
            capabilities: claude_capabilities(),
            mutable: Mutex::new(InstanceMutable {
                status: InstanceStatus::Created,
                conversations: HashMap::new(),
            }),
            events,
        }
    }

    fn snapshot(&self) -> ProviderInstance {
        ProviderInstance {
            route: self.route.clone(),
            plugin_id: CLAUDE_PLUGIN_ID.to_string(),
            instance_kind: self.instance_kind.clone(),
            display_name: self.display_name.clone(),
            status: lock(&self.mutable).status,
            capabilities: self.capabilities.clone(),
        }
    }

    fn status(&self) -> InstanceStatus {
        lock(&self.mutable).status
    }

    fn set_status(&self, status: InstanceStatus) -> Result<ProviderInstance, ProtocolError> {
        let previous_status = {
            let mut mutable = lock(&self.mutable);
            if mutable.status == status {
                None
            } else {
                let previous = mutable.status;
                mutable.status = status;
                Some(previous)
            }
        };
        let instance = self.snapshot();
        if let Some(previous_status) = previous_status {
            self.events.publish(ProtocolEvent::EventInstanceStatusChanged {
                jsonrpc: "2.0".to_string(),
                params: InstanceStatusChangedEvent {
                    instance: instance.clone(),
                    previous_status: Some(previous_status),
                },
            })?;
        }
        Ok(instance)
    }

    fn create_conversation(
        &self,
        request: ConversationCreateRequest,
    ) -> Result<ProviderConversation, ProtocolError> {
        if self.status() != InstanceStatus::Ready {
            return Err(provider_unavailable(&self.route));
        }
        if request.extension.is_some() {
            return Err(capability_error(
                "conversation.create extension data is unsupported by the Claude CLI adapter",
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
        let workspace_root = PathBuf::from(workspace_root);
        if !workspace_root.is_absolute() || !workspace_root.is_dir() {
            return Err(protocol_error(
                "invalid_conversation_options",
                "workspaceRoot must be an existing absolute directory".to_string(),
                false,
            ));
        }
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
        let conversation = ProviderConversation {
            resource: self.resource(session_id.clone()),
            title,
            preview: None,
            status: ConversationStatus::Idle,
            permission_level: Some(request.permission_level.clone()),
            model: request.model.clone(),
            reasoning_effort: request.reasoning_effort.clone(),
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            created_at: Some(now),
            updated_at: Some(now),
            active_turn: None,
            extension: Some(extension([
                ("nativeInterface", json!("claude-print-stream-json")),
                ("sessionScope", json!("provider-managed")),
                ("configurationMode", json!("inherit-claude-defaults")),
                ("externalSessionDiscovery", json!(false)),
            ])),
        };
        {
            let mut mutable = lock(&self.mutable);
            mutable.conversations.insert(
                session_id,
                ManagedConversation {
                    conversation: conversation.clone(),
                    workspace_root,
                    title: request.title,
                    model: request.model,
                    effort: request.reasoning_effort,
                    materialized: false,
                    active_turn: None,
                },
            );
        }
        self.publish_conversation(conversation.clone())?;
        Ok(conversation)
    }

    fn start_turn(
        self: &Arc<Self>,
        request: TurnStartRequest,
    ) -> Result<ProviderTurn, ProtocolError> {
        validate_resource_for_instance(&request.conversation, &self.route)?;
        if request.message.trim().is_empty() || request.client_message_id.trim().is_empty() {
            return Err(protocol_error(
                "invalid_turn_request",
                "turn message and clientMessageId must not be empty".to_string(),
                false,
            ));
        }
        let conversation_id = request.conversation.native_resource_id.clone();
        let turn_id = Uuid::new_v4().to_string();
        let user_message_id = Uuid::new_v4().to_string();
        let (spawned, turn, conversation) = {
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
            let spawned = ClaudeTurnLaunch {
                executable: self.settings.claude_executable.clone(),
                workspace_root: managed.workspace_root.clone(),
                session_id: conversation_id.clone(),
                resume: managed.materialized,
                user_message_id,
                message: request.message,
                title: managed.title.clone(),
                model: managed.model.clone(),
                effort: managed.effort.clone(),
            }
            .spawn()
            .map_err(cli_protocol_error)?;
            let now = now_ms();
            let turn = ProviderTurn {
                resource: self.resource(turn_id.clone()),
                conversation: request.conversation.clone(),
                status: TurnStatus::Running,
                display_summary: None,
                started_at: Some(now),
                updated_at: Some(now),
                completed_at: None,
                extension: Some(extension([
                    ("nativeInterface", json!("claude-print-stream-json")),
                    ("clientMessageId", json!(request.client_message_id)),
                ])),
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
            });
            managed.conversation.status = ConversationStatus::Running;
            managed.conversation.active_turn = Some(turn.clone());
            managed.conversation.updated_at = Some(now);
            (spawned, turn, managed.conversation.clone())
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
        Ok(turn)
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
                self.publish_text_chunks(
                    &turn.resource,
                    &conversation,
                    &format!("{turn_id}:text:{index}"),
                    "text",
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
                        stop_reason: None,
                        terminal_reason: None,
                    },
                )
            }
            ClaudeOutput::Result {
                subtype,
                is_error,
                session_id,
                result,
                stop_reason,
                terminal_reason,
                usage: _,
                total_cost_usd: _,
            } => {
                validate_claude_session(conversation_id, session_id.as_deref())?;
                let status = result_status(&subtype, is_error, terminal_reason.as_deref());
                self.set_pending_completion(
                    conversation_id,
                    turn_id,
                    TurnCompletion {
                        status,
                        result,
                        stop_reason,
                        terminal_reason,
                    },
                )
            }
            _ => Ok(()),
        }
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
            let mutable = lock(&self.mutable);
            let Some(managed) = mutable.conversations.get(conversation_id) else {
                return;
            };
            let Some(active) = managed.active_turn.as_ref() else {
                return;
            };
            if active.turn.resource.native_resource_id != turn_id {
                return;
            }
            let completion = completion_for_exit(active, &outcome);
            (
                active.turn.clone(),
                managed.conversation.clone(),
                active.saw_text_delta,
                completion,
                active.finished.clone(),
            )
        };
        let (base_turn, base_conversation, saw_text_delta, mut completion, finished) = snapshot;
        let fallback = (!saw_text_delta)
            .then_some(completion.result.as_deref())
            .flatten()
            .filter(|value| !value.is_empty());
        if let Some(delta) = fallback {
            let kind = if completion.status == TurnStatus::Failed {
                "error"
            } else {
                "text"
            };
            if let Err(error) = self.publish_text_chunks(
                &base_turn.resource,
                &base_turn.conversation,
                &format!("{turn_id}:result"),
                kind,
                delta,
                "result",
            ) {
                completion = TurnCompletion {
                    status: TurnStatus::Failed,
                    result: Some("Provider failed to publish Claude output".to_string()),
                    stop_reason: None,
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
        output_id: &str,
        kind: &str,
        text: &str,
        native_event: &str,
    ) -> Result<(), ProtocolError> {
        for (chunk_index, chunk) in text_chunks(text).enumerate() {
            self.events.publish(ProtocolEvent::EventTurnOutputDelta {
                jsonrpc: "2.0".to_string(),
                params: TurnOutputDeltaEvent {
                    turn: turn.clone(),
                    conversation: conversation.clone(),
                    output_id: format!("{output_id}:{chunk_index}"),
                    kind: kind.to_string(),
                    delta: chunk.to_string(),
                    extension: Some(extension([("nativeEvent", json!(native_event))])),
                },
            })?;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn interrupt_turn(&self, request: TurnInterruptRequest) -> Result<ProviderTurn, ProtocolError> {
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
            if active.turn.resource != request.turn || active.turn.conversation != request.conversation {
                return Err(protocol_error(
                    "turn_route_mismatch",
                    "turn and conversation resources do not identify the active Claude turn".to_string(),
                    false,
                ));
            }
            active.requested_completion = Some(TurnCompletion {
                status: TurnStatus::Interrupted,
                result: None,
                stop_reason: None,
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

    fn stop(&self) -> Result<ProviderInstance, ProtocolError> {
        if matches!(self.status(), InstanceStatus::Stopped | InstanceStatus::Created) {
            return self.set_status(InstanceStatus::Stopped);
        }
        let mut first_error = self.set_status(InstanceStatus::Stopping).err();
        let active_turns = {
            let mut mutable = lock(&self.mutable);
            let mut active_turns = Vec::new();
            for managed in mutable.conversations.values_mut() {
                if let Some(active) = managed.active_turn.as_mut() {
                    active.requested_completion = Some(TurnCompletion {
                        status: TurnStatus::Interrupted,
                        result: None,
                        stop_reason: None,
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
                        stop_reason: None,
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
            device_id: self.route.device_id.clone(),
            provider_plugin_id: self.route.provider_plugin_id.clone(),
            provider_instance_id: self.route.provider_instance_id.clone(),
            native_resource_id,
        }
    }

    fn publish_conversation(&self, conversation: ProviderConversation) -> Result<(), ProtocolError> {
        self.events.publish(ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".to_string(),
            params: ConversationUpsertedEvent { conversation },
        })
    }

    fn publish_turn(&self, turn: ProviderTurn) -> Result<(), ProtocolError> {
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
}

pub struct ClaudeProvider {
    state: Mutex<ProviderState>,
    events: Arc<dyn ProviderEventSink>,
    shutdown: AtomicBool,
}

impl ClaudeProvider {
    pub fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self {
            state: Mutex::new(ProviderState {
                host_device_id: None,
                initialized_client_id: None,
                instances: HashMap::new(),
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

    fn resource_instance(&self, resource: &RoutedResourceId) -> Result<Arc<ClaudeInstanceRuntime>, ProtocolError> {
        validate_resource(resource)?;
        self.instance(&ProviderInstanceRoute {
            device_id: resource.device_id.clone(),
            provider_plugin_id: resource.provider_plugin_id.clone(),
            provider_instance_id: resource.provider_instance_id.clone(),
        })
    }
}

impl ProtocolServer for ClaudeProvider {
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
            Ok(ProviderInitializeResponse {
                selected_version: PROTOCOL_VERSION,
                plugin: Self::descriptor(),
            })
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

    fn instance_create<'a>(
        &'a self,
        request: InstanceCreateRequest,
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
            let settings = decode_settings(request.settings.clone())?;
            let mut state = lock(&self.state);
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
            if runtime.status() == InstanceStatus::Ready {
                return Ok(InstanceStartResponse {
                    instance: runtime.snapshot(),
                });
            }
            runtime.set_status(InstanceStatus::Starting)?;
            let executable = runtime.settings.claude_executable.clone();
            let outcome = tokio::task::spawn_blocking(move || verify_claude_executable(executable))
                .await
                .map_err(|error| protocol_error(
                    "provider_task_failed",
                    format!("Claude executable validation task failed: {error}"),
                    true,
                ))?;
            if let Err(error) = outcome {
                runtime.set_status(InstanceStatus::Error)?;
                return Err(error);
            }
            Ok(InstanceStartResponse {
                instance: runtime.set_status(InstanceStatus::Ready)?,
            })
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
                capabilities: runtime.capabilities.clone(),
            })
        })
    }

    fn conversation_list<'a>(
        &'a self,
        _request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async { Err(capability_error("Claude CLI does not expose a stable conversation list API")) })
    }

    fn conversation_get<'a>(
        &'a self,
        _request: ConversationGetRequest,
    ) -> ProtocolFuture<'a, ConversationGetResponse> {
        Box::pin(async { Err(capability_error("Claude CLI does not expose a stable conversation get API")) })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
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
            Ok(TurnStartResponse {
                turn: runtime.start_turn(request)?,
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
        _request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async { Err(capability_error("Claude approval callbacks require an SDK or permission prompt tool")) })
    }

    fn provider_shutdown<'a>(
        &'a self,
        _request: ProviderShutdownRequest,
    ) -> ProtocolFuture<'a, ProviderShutdownResponse> {
        Box::pin(async move {
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
        ProviderCapability::ConversationCreate,
        ProviderCapability::TurnStart,
    ];
    #[cfg(unix)]
    methods.push(ProviderCapability::TurnInterrupt);
    ProviderCapabilities {
        methods,
        permission_levels: vec!["workspace-write".to_string()],
        models: vec![
            "sonnet".to_string(),
            "opus".to_string(),
            "haiku".to_string(),
            "fable".to_string(),
        ],
        reasoning_efforts: vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
            "max".to_string(),
        ],
        extensions: vec![extension([
            ("nativeInterface", json!("claude-print-stream-json")),
            (
                "nativeMethods",
                json!(["--session-id", "--resume", "stream-json", "SIGINT"]),
            ),
            (
                "unsupportedMethods",
                json!(["conversation.list", "conversation.get", "turn.steer", "approval.resolve"]),
            ),
            ("permissionModeMapping", json!({ "workspace-write": "inherited" })),
        ])],
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
    Ok(settings)
}

fn verify_claude_executable(executable: PathBuf) -> Result<(), ProtocolError> {
    if !executable.is_file() {
        return Err(protocol_error(
            "provider_unavailable",
            format!("Host-resolved Claude executable is unavailable: {}", executable.display()),
            true,
        ));
    }
    let mut child = Command::new(&executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| protocol_error(
            "provider_unavailable",
            format!("start Host-resolved Claude executable: {error}"),
            true,
        ))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
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
            stop_reason: None,
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
        stop_reason: None,
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
    mut turn: ProviderTurn,
    mut conversation: ProviderConversation,
    completion: &TurnCompletion,
) -> (ProviderTurn, ProviderConversation) {
    let now = now_ms();
    turn.status = completion.status;
    turn.updated_at = Some(now);
    turn.completed_at = Some(now);
    let native_subtype = match completion.status {
        TurnStatus::Completed => "success",
        TurnStatus::Interrupted => "interrupted",
        _ => "error",
    };
    let stop_reason = completion
        .stop_reason
        .as_deref()
        .map(|value| truncate_text(value, MAX_CLAUDE_METADATA_BYTES));
    let terminal_reason = completion
        .terminal_reason
        .as_deref()
        .map(|value| truncate_text(value, MAX_CLAUDE_METADATA_BYTES));
    turn.extension = Some(extension([
        ("nativeSubtype", json!(native_subtype)),
        ("stopReason", json!(stop_reason)),
        ("terminalReason", json!(terminal_reason)),
    ]));
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

fn validate_resource(resource: &RoutedResourceId) -> Result<(), ProtocolError> {
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
    resource: &RoutedResourceId,
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
