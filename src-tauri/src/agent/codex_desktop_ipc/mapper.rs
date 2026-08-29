use super::state::ThreadSnapshot;
use crate::runtime_gateway::generated::{
    Approval, ApprovalDecision, ApprovalStatus, Conversation, ConversationStatus, JsonObject,
    PermissionLevel, Provider, ProviderCapabilities, ProviderExtension, ProviderStatus, TurnTask,
    QuickReply, TurnTaskStatus,
};
use chrono::DateTime;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CODEX_PROVIDER_ID: &str = "codex";
pub(crate) const CONTINUE_QUICK_REPLY_ID: &str = "continue";
const EXTENSION_NAMESPACE: &str = "codepet.codex-desktop";

#[derive(Clone, Debug, PartialEq)]
pub struct MappedThread {
    pub revision: u64,
    pub conversation: Conversation,
    pub latest_turn: Option<TurnTask>,
    pub(crate) approvals: Vec<MappedApproval>,
    pub(crate) native_pending_approval_ids: HashSet<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MappedApproval {
    pub(crate) approval: Approval,
    pub(crate) raw_request_id: Value,
    pub(crate) native_method: String,
    pub(crate) owner_client_id: String,
    pub(crate) revision: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) item_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitingKind {
    Approval,
    UserInput,
    Unsupported,
}

pub fn provider(status: ProviderStatus, unavailable_reason: Option<&str>) -> Provider {
    let mut data = JsonObject::new();
    data.insert("source".to_string(), json!("codex-desktop-private-ipc"));
    data.insert(
        "discoveryScope".to_string(),
        json!("desktop-followed-or-explicitly-known"),
    );
    if let Some(reason) = unavailable_reason.filter(|reason| !reason.trim().is_empty()) {
        data.insert("unavailableReason".to_string(), json!(reason));
    }
    Provider {
        id: CODEX_PROVIDER_ID.to_string(),
        provider_type: "codex".to_string(),
        display_name: "Codex Desktop".to_string(),
        version: None,
        status,
        capabilities: ProviderCapabilities {
            methods: vec![
                "conversation.get".to_string(),
                "turn.send".to_string(),
                "turn.interrupt".to_string(),
                "approval.resolve".to_string(),
            ],
            permission_levels: Vec::new(),
            models: Vec::new(),
            reasoning_efforts: Vec::new(),
            quick_replies: vec![QuickReply {
                id: CONTINUE_QUICK_REPLY_ID.to_string(),
                label: "继续".to_string(),
                text: "请继续。".to_string(),
            }],
            can_steer: true,
            can_interrupt: true,
            extension: None,
        },
        extension: Some(ProviderExtension {
            namespace: EXTENSION_NAMESPACE.to_string(),
            data,
        }),
    }
}

pub fn map_thread(snapshot: &ThreadSnapshot) -> MappedThread {
    let state = &snapshot.state;
    let mut diagnostics = Vec::new();
    let (approvals, native_pending_approval_ids) =
        map_pending_approvals(snapshot, &mut diagnostics);
    let created_at = timestamp_ms(state.get("createdAt"))
        .or_else(|| timestamp_ms(state.get("updatedAt")))
        .unwrap_or_else(now_ms);
    let updated_at = timestamp_ms(state.get("updatedAt"))
        .or_else(|| timestamp_ms(state.get("recencyAt")))
        .unwrap_or(created_at);
    let turns = ordered_turns(state);
    let latest_native_turn = turns.last().copied();
    let runtime_type = state
        .get("threadRuntimeStatus")
        .and_then(|status| status.get("type"))
        .and_then(Value::as_str);
    let waiting = if runtime_type == Some("systemError") {
        None
    } else {
        waiting_kind(state, &mut diagnostics)
    };
    let mut latest_turn = latest_native_turn.and_then(|turn| {
        map_turn(
            &snapshot.conversation_id,
            turn,
            waiting,
            updated_at,
            &mut diagnostics,
        )
    });

    if runtime_type == Some("systemError") {
        let summary = system_error_summary(state);
        if let Some(turn) = latest_turn.as_mut() {
            if !is_terminal(turn.status) {
                turn.status = TurnTaskStatus::Failed;
                turn.display_summary = turn.display_summary.clone().or(summary);
                turn.updated_at = updated_at;
                turn.completed_at = Some(updated_at);
            }
        } else {
            latest_turn = Some(TurnTask {
                id: format!("{}:desktop-system-error", snapshot.conversation_id),
                provider_id: CODEX_PROVIDER_ID.to_string(),
                conversation_id: snapshot.conversation_id.clone(),
                status: TurnTaskStatus::Failed,
                display_summary: summary,
                started_at: None,
                updated_at,
                completed_at: Some(updated_at),
                extension: None,
            });
        }
    }
    if latest_turn.is_none() && matches!(runtime_type, Some("active")) {
        latest_turn = Some(TurnTask {
            id: format!("{}:desktop-active", snapshot.conversation_id),
            provider_id: CODEX_PROVIDER_ID.to_string(),
            conversation_id: snapshot.conversation_id.clone(),
            status: if waiting.is_some() {
                TurnTaskStatus::WaitingApproval
            } else {
                TurnTaskStatus::Running
            },
            display_summary: waiting_summary(waiting),
            started_at: None,
            updated_at,
            completed_at: None,
            extension: None,
        });
    }

    let conversation_status = conversation_status(runtime_type, latest_turn.as_ref(), waiting);
    if let Some(other) = runtime_type.filter(|value| {
        !matches!(*value, "active" | "idle" | "notLoaded" | "systemError")
    }) {
        diagnostics.push(format!("unknown Desktop thread runtime status {other}"));
    }
    let active_turn = latest_turn
        .as_ref()
        .filter(|turn| !is_terminal(turn.status))
        .cloned();
    let title = nonempty_string(state.get("title"))
        .or_else(|| nonempty_string(state.get("name")))
        .unwrap_or_else(|| "Codex Desktop task".to_string());
    let preview = nonempty_string(state.get("preview"))
        .or_else(|| latest_native_turn.and_then(last_agent_message));
    let (permission_level, permission_diagnostic) =
        permission_level(state, latest_native_turn);
    if let Some(diagnostic) = permission_diagnostic {
        diagnostics.push(diagnostic);
    }
    let model = nonempty_string(state.get("latestModel"));
    let reasoning_effort = nonempty_string(state.get("latestReasoningEffort"));
    let workspace_root = nonempty_string(state.get("cwd"))
        .or_else(|| nonempty_string(state.get("workspaceBrowserRoot")));

    MappedThread {
        revision: snapshot.revision,
        conversation: Conversation {
            id: snapshot.conversation_id.clone(),
            provider_id: CODEX_PROVIDER_ID.to_string(),
            title,
            preview,
            status: conversation_status,
            permission_level,
            model,
            reasoning_effort,
            workspace_root,
            created_at,
            updated_at,
            active_turn,
            extension: None,
        },
        latest_turn,
        approvals,
        native_pending_approval_ids,
        diagnostics,
    }
}

pub(crate) fn map_pending_approvals(
    snapshot: &ThreadSnapshot,
    diagnostics: &mut Vec<String>,
) -> (Vec<MappedApproval>, HashSet<String>) {
    let Some(requests) = snapshot.state.get("requests").and_then(Value::as_array) else {
        return (Vec::new(), HashSet::new());
    };
    let mut approvals = Vec::new();
    let mut native_pending_approval_ids = HashSet::new();
    for request in requests {
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            continue;
        };
        let (kind, title, id_kind) = match method {
            "item/commandExecution/requestApproval" => (
                "command-execution",
                "Allow command execution?",
                "command",
            ),
            "item/fileChange/requestApproval" => {
                ("file-change", "Allow file changes?", "file")
            }
            _ => continue,
        };
        let Some(raw_request_id) = request.get("id").filter(|id| valid_request_id(id)) else {
            invalid_approval_diagnostic(diagnostics, method, "missing string or number request id");
            continue;
        };
        if requests
            .iter()
            .filter(|candidate| candidate.get("id") == Some(raw_request_id))
            .count()
            != 1
        {
            invalid_approval_diagnostic(diagnostics, method, "request id is not unique");
            continue;
        }
        let Some(params) = request.get("params").and_then(Value::as_object) else {
            invalid_approval_diagnostic(diagnostics, method, "missing params object");
            continue;
        };
        let Some(thread_id) = native_id(params.get("threadId")) else {
            invalid_approval_diagnostic(diagnostics, method, "missing threadId");
            continue;
        };
        let Some(turn_id) = native_id(params.get("turnId")) else {
            invalid_approval_diagnostic(diagnostics, method, "missing turnId");
            continue;
        };
        let Some(item_id) = native_id(params.get("itemId")) else {
            invalid_approval_diagnostic(diagnostics, method, "missing itemId");
            continue;
        };
        let approval_id = approval_public_id(
            snapshot,
            id_kind,
            raw_request_id,
            thread_id,
            turn_id,
            item_id,
        );
        native_pending_approval_ids.insert(approval_id.clone());
        if thread_id != snapshot.conversation_id {
            invalid_approval_diagnostic(diagnostics, method, "threadId does not match snapshot");
            continue;
        }
        if active_turn_id(&snapshot.state) != Some(turn_id) {
            invalid_approval_diagnostic(
                diagnostics,
                method,
                "turnId is not the current inProgress turn",
            );
            continue;
        }
        let requested_at = approval_requested_at(&snapshot.state, turn_id);
        approvals.push(MappedApproval {
            approval: Approval {
                id: approval_id,
                provider_id: CODEX_PROVIDER_ID.to_string(),
                conversation_id: snapshot.conversation_id.clone(),
                turn_id: turn_id.to_string(),
                kind: kind.to_string(),
                title: title.to_string(),
                description: nonempty_string(params.get("reason")),
                status: ApprovalStatus::Pending,
                decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
                requested_at,
                resolved_at: None,
                decision: None,
                extension: None,
            },
            raw_request_id: raw_request_id.clone(),
            native_method: method.to_string(),
            owner_client_id: snapshot.owner_client_id.clone(),
            revision: snapshot.revision,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            item_id: item_id.to_string(),
        });
    }
    (approvals, native_pending_approval_ids)
}

fn approval_public_id(
    snapshot: &ThreadSnapshot,
    id_kind: &str,
    raw_request_id: &Value,
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
) -> String {
    format!(
        "desktop-approval:{}:{id_kind}:{}:{}:{}:{}:{}",
        encode_request_id(&Value::String(snapshot.conversation_id.clone())),
        encode_request_id(&Value::String(snapshot.owner_client_id.clone())),
        encode_request_id(raw_request_id),
        encode_request_id(&Value::String(thread_id.to_string())),
        encode_request_id(&Value::String(turn_id.to_string())),
        encode_request_id(&Value::String(item_id.to_string()))
    )
}

fn valid_request_id(value: &Value) -> bool {
    value.is_string() || value.is_number()
}

fn native_id(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn invalid_approval_diagnostic(diagnostics: &mut Vec<String>, method: &str, reason: &str) {
    diagnostics.push(format!(
        "invalid_pending_request: Desktop {method} approval is not actionable: {reason}"
    ));
}

fn approval_requested_at(state: &Value, turn_id: &str) -> u64 {
    ordered_turns(state)
        .into_iter()
        .find(|turn| turn.get("turnId").and_then(Value::as_str) == Some(turn_id))
        .and_then(|turn| timestamp_ms(turn.get("turnStartedAtMs")))
        .or_else(|| timestamp_ms(state.get("createdAt")))
        .or_else(|| timestamp_ms(state.get("updatedAt")))
        .unwrap_or_else(now_ms)
}

fn active_turn_id(state: &Value) -> Option<&str> {
    if state
        .get("threadRuntimeStatus")
        .and_then(|status| status.get("type"))
        .and_then(Value::as_str)
        != Some("active")
    {
        return None;
    }
    ordered_turns(state)
        .into_iter()
        .rev()
        .find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))
        .and_then(|turn| turn.get("turnId"))
        .and_then(Value::as_str)
        .filter(|turn_id| !turn_id.trim().is_empty())
}

fn encode_request_id(value: &Value) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let serialized = value.to_string();
    let mut encoded = String::with_capacity(serialized.len().saturating_mul(2));
    for byte in serialized.bytes() {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn conversation_status(
    runtime_type: Option<&str>,
    latest_turn: Option<&TurnTask>,
    waiting: Option<WaitingKind>,
) -> ConversationStatus {
    if runtime_type == Some("systemError") {
        return ConversationStatus::Error;
    }
    if waiting.is_some()
        || latest_turn.is_some_and(|turn| turn.status == TurnTaskStatus::WaitingApproval)
    {
        return ConversationStatus::WaitingApproval;
    }
    match runtime_type {
        Some("active") => ConversationStatus::Running,
        Some("idle" | "notLoaded") => terminal_or_idle_status(latest_turn),
        Some(_) | None => latest_turn.map_or(ConversationStatus::Idle, |turn| match turn.status {
            TurnTaskStatus::Queued | TurnTaskStatus::Running => ConversationStatus::Running,
            TurnTaskStatus::WaitingApproval => ConversationStatus::WaitingApproval,
            TurnTaskStatus::Failed => ConversationStatus::Error,
            TurnTaskStatus::Completed | TurnTaskStatus::Interrupted => ConversationStatus::Idle,
        }),
    }
}

fn system_error_summary(state: &Value) -> Option<String> {
    let _ = state;
    Some("Codex Desktop task entered a system error state".to_string())
}

fn terminal_or_idle_status(latest_turn: Option<&TurnTask>) -> ConversationStatus {
    match latest_turn.map(|turn| turn.status) {
        Some(TurnTaskStatus::Failed) => ConversationStatus::Error,
        Some(TurnTaskStatus::Queued | TurnTaskStatus::Running) => ConversationStatus::Running,
        Some(TurnTaskStatus::WaitingApproval) => ConversationStatus::WaitingApproval,
        _ => ConversationStatus::Idle,
    }
}

fn map_turn(
    conversation_id: &str,
    turn: &Value,
    waiting: Option<WaitingKind>,
    conversation_updated_at: u64,
    diagnostics: &mut Vec<String>,
) -> Option<TurnTask> {
    let native_status = turn.get("status").and_then(Value::as_str)?;
    let mut status = match native_status {
        "inProgress" => TurnTaskStatus::Running,
        "completed" => TurnTaskStatus::Completed,
        "failed" => TurnTaskStatus::Failed,
        "interrupted" => TurnTaskStatus::Interrupted,
        other => {
            diagnostics.push(format!("ignored Desktop turn with unknown status {other}"));
            return None;
        }
    };
    if status == TurnTaskStatus::Running && waiting.is_some() {
        status = TurnTaskStatus::WaitingApproval;
    }
    let started_at = timestamp_ms(turn.get("turnStartedAtMs"));
    let duration = finite_u64(turn.get("durationMs"));
    let completed_at = if is_terminal(status) {
        started_at
            .zip(duration)
            .map(|(started, duration)| started.saturating_add(duration))
            .or(Some(conversation_updated_at))
    } else {
        None
    };
    let id = nonempty_string(turn.get("turnId")).unwrap_or_else(|| {
        format!(
            "{conversation_id}:desktop-turn:{}",
            started_at.unwrap_or(conversation_updated_at)
        )
    });
    let display_summary = waiting_summary(waiting)
        .or_else(|| error_message(turn.get("error")))
        .or_else(|| last_agent_message(turn));
    Some(TurnTask {
        id,
        provider_id: CODEX_PROVIDER_ID.to_string(),
        conversation_id: conversation_id.to_string(),
        status,
        display_summary,
        started_at,
        updated_at: completed_at.unwrap_or(conversation_updated_at),
        completed_at,
        extension: None,
    })
}

fn ordered_turns(state: &Value) -> Vec<&Value> {
    let mut ordered = Vec::new();
    if state
        .get("turnHistory")
        .and_then(|history| history.get("kind"))
        .and_then(Value::as_str)
        == Some("canonical")
    {
        if let Some(history) = state.get("turnHistory").and_then(|value| value.get("history")) {
            if let (Some(islands), Some(entities)) = (
                history.get("islands").and_then(Value::as_array),
                history.get("entitiesByKey").and_then(Value::as_object),
            ) {
                for island in islands {
                    if let Some(entries) = island.get("entries").and_then(Value::as_array) {
                        for entry in entries {
                            if let Some(key) = entry.get("value").and_then(Value::as_str) {
                                if let Some(turn) = entities.get(key) {
                                    ordered.push(turn);
                                }
                            }
                        }
                    }
                }
            }
        }
        return ordered;
    }
    state
        .get("turns")
        .and_then(Value::as_array)
        .map(|turns| turns.iter().collect())
        .unwrap_or_default()
}

fn waiting_kind(state: &Value, diagnostics: &mut Vec<String>) -> Option<WaitingKind> {
    let flags = state
        .get("threadRuntimeStatus")
        .and_then(|status| status.get("activeFlags"))
        .and_then(Value::as_array);
    let mut waiting = if flags.is_some_and(|flags| {
        flags
            .iter()
            .any(|flag| flag.as_str() == Some("waitingOnApproval"))
    }) {
        Some(WaitingKind::Approval)
    } else if flags.is_some_and(|flags| {
        flags
            .iter()
            .any(|flag| flag.as_str() == Some("waitingOnUserInput"))
    }) {
        Some(WaitingKind::UserInput)
    } else {
        None
    };
    let mut unknown_requests = 0_usize;
    for request in state
        .get("requests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let method = request.get("method").and_then(Value::as_str);
        if matches!(
            method,
            Some(
                "item/commandExecution/requestApproval"
                    | "item/fileChange/requestApproval"
            )
        ) {
            waiting = Some(WaitingKind::Approval);
            continue;
        }
        if method == Some("item/permissions/requestApproval") {
            waiting = Some(WaitingKind::Approval);
            capability_unsupported_diagnostic(diagnostics, method.unwrap_or_default());
            continue;
        }
        if method == Some("mcpServer/elicitation/request") {
            if !request_is_completed(request) {
                waiting = Some(WaitingKind::Approval);
                capability_unsupported_diagnostic(diagnostics, method.unwrap_or_default());
            }
            continue;
        }
        let is_user_input = matches!(
            method,
            Some(
                "item/tool/requestUserInput"
                    | "item/tool/requestOptionPicker"
                    | "item/tool/requestSetupCodexContextPicker"
            )
        );
        if is_user_input {
            if waiting != Some(WaitingKind::Approval) {
                waiting = Some(WaitingKind::UserInput);
            }
            capability_unsupported_diagnostic(diagnostics, method.unwrap_or_default());
            continue;
        }
        if method == Some("item/tool/call") {
            let tool = request
                .get("params")
                .and_then(|params| params.get("tool"))
                .and_then(Value::as_str);
            let known_input_tool = matches!(
                tool,
                Some(
                    "request_onboarding_input"
                        | "request_option_picker"
                        | "setup_codex_context_picker"
                )
            ) || (tool == Some("setup_codex_step") && setup_step_requires_input(request));
            if known_input_tool {
                if waiting != Some(WaitingKind::Approval) {
                    waiting = Some(WaitingKind::UserInput);
                }
            } else {
                if waiting.is_none() {
                    waiting = Some(WaitingKind::Unsupported);
                }
                unknown_requests = unknown_requests.saturating_add(1);
            }
            capability_unsupported_diagnostic(diagnostics, method.unwrap_or_default());
            continue;
        }
        if method == Some("item/plan/requestImplementation") {
            if waiting.is_none() {
                waiting = Some(WaitingKind::Unsupported);
            }
            capability_unsupported_diagnostic(diagnostics, method.unwrap_or_default());
            continue;
        }
        if waiting.is_none() {
            waiting = Some(WaitingKind::Unsupported);
        }
        unknown_requests = unknown_requests.saturating_add(1);
        capability_unsupported_diagnostic(diagnostics, method.unwrap_or("unknown"));
    }
    if unknown_requests > 0 {
        diagnostics.push(format!(
            "ignored {unknown_requests} unrecognized pending Desktop request(s)"
        ));
    }
    waiting
}

fn request_is_completed(request: &Value) -> bool {
    request.get("completed").and_then(Value::as_bool) == Some(true)
        || request
            .get("params")
            .and_then(|params| params.get("completed"))
            .and_then(Value::as_bool)
            == Some(true)
}

fn capability_unsupported_diagnostic(diagnostics: &mut Vec<String>, method: &str) {
    diagnostics.push(format!(
        "capability_unsupported: pending Desktop request {method} cannot be resolved through Standard Protocol v0"
    ));
}

fn permission_level(
    state: &Value,
    latest_native_turn: Option<&Value>,
) -> (PermissionLevel, Option<String>) {
    let candidates = [
        state.pointer("/latestThreadSettings/sandboxPolicy/type"),
        latest_native_turn.and_then(|turn| turn.pointer("/params/sandboxPolicy/type")),
        state.pointer("/currentPermissions/sandboxPolicy/type"),
    ];
    for candidate in candidates.into_iter().flatten().filter_map(Value::as_str) {
        match candidate {
            "readOnly" => return (PermissionLevel::ReadOnly, None),
            "dangerFullAccess" => {
                return (PermissionLevel::FullAccess, None)
            }
            "workspaceWrite" => return (PermissionLevel::WorkspaceWrite, None),
            "externalSandbox" => {
                return (
                    PermissionLevel::ReadOnly,
                    Some(
                        "Desktop externalSandbox permission has no Standard Protocol mapping; projected read-only"
                            .to_string(),
                    ),
                )
            }
            other => {
                return (
                    PermissionLevel::ReadOnly,
                    Some(format!(
                        "unknown Desktop sandbox policy {other}; projected read-only"
                    )),
                )
            }
        }
    }
    (
        PermissionLevel::ReadOnly,
        Some("Desktop snapshot is missing sandbox policy; projected read-only".to_string()),
    )
}

fn setup_step_requires_input(request: &Value) -> bool {
    let Some(arguments) = request
        .get("params")
        .and_then(|params| params.get("arguments"))
    else {
        return false;
    };
    let parsed;
    let arguments = if let Some(text) = arguments.as_str() {
        parsed = serde_json::from_str::<Value>(text).ok();
        parsed.as_ref().unwrap_or(arguments)
    } else {
        arguments
    };
    arguments
        .get("step")
        .and_then(Value::as_str)
        .is_some_and(|step| step != "complete")
}

fn waiting_summary(waiting: Option<WaitingKind>) -> Option<String> {
    waiting.map(|kind| match kind {
        WaitingKind::Approval => "等待审批".to_string(),
        WaitingKind::UserInput => "等待输入".to_string(),
        WaitingKind::Unsupported => "等待 Desktop 操作".to_string(),
    })
}

fn last_agent_message(turn: &Value) -> Option<String> {
    turn.get("items")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
        .and_then(|item| nonempty_string(item.get("text")))
        .map(|message| concise(&message, 240))
}

fn error_message(error: Option<&Value>) -> Option<String> {
    let error = error?;
    nonempty_string(Some(error)).or_else(|| nonempty_string(error.get("message")))
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn timestamp_ms(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(number) = value.as_f64().filter(|number| number.is_finite() && *number >= 0.0) {
        return Some(number.round() as u64);
    }
    let text = value.as_str()?;
    if let Ok(number) = text.parse::<f64>() {
        return timestamp_ms(Some(&json!(number)));
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .and_then(|date| u64::try_from(date.timestamp_millis()).ok())
}

fn concise(value: &str, max_characters: usize) -> String {
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(max_characters).collect::<String>();
    if characters.next().is_some() {
        format!("{}…", prefix.trim_end())
    } else {
        prefix
    }
}

fn finite_u64(value: Option<&Value>) -> Option<u64> {
    value?
        .as_f64()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number.round() as u64)
}

fn is_terminal(status: TurnTaskStatus) -> bool {
    matches!(
        status,
        TurnTaskStatus::Completed | TurnTaskStatus::Failed | TurnTaskStatus::Interrupted
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(runtime_status: Value, turn_status: &str) -> ThreadSnapshot {
        ThreadSnapshot {
            conversation_id: "thread-one".to_string(),
            owner_client_id: "owner-one".to_string(),
            revision: 3,
            state: json!({
                "id": "thread-one",
                "title": "Desktop task",
                "createdAt": 1_700_000_000_000_u64,
                "updatedAt": 1_700_000_001_000_u64,
                "threadRuntimeStatus": runtime_status,
                "currentPermissions": {
                    "sandboxPolicy": { "type": "workspaceWrite" }
                },
                "turnHistory": {
                    "kind": "canonical",
                    "history": {
                        "islands": [{ "entries": [{ "value": "turn:one" }] }],
                        "entitiesByKey": {
                            "turn:one": {
                                "turnId": "turn-one",
                                "status": turn_status,
                                "turnStartedAtMs": 1_700_000_000_000_u64,
                                "durationMs": 900,
                                "items": []
                            }
                        }
                    }
                },
                "turns": []
            }),
        }
    }

    #[test]
    fn maps_running_waiting_and_terminal_states() {
        let cases = [
            (
                json!({ "type": "active", "activeFlags": [] }),
                "inProgress",
                ConversationStatus::Running,
                TurnTaskStatus::Running,
            ),
            (
                json!({ "type": "active", "activeFlags": ["waitingOnApproval"] }),
                "inProgress",
                ConversationStatus::WaitingApproval,
                TurnTaskStatus::WaitingApproval,
            ),
            (
                json!({ "type": "active", "activeFlags": ["waitingOnUserInput"] }),
                "inProgress",
                ConversationStatus::WaitingApproval,
                TurnTaskStatus::WaitingApproval,
            ),
            (
                json!({ "type": "idle" }),
                "completed",
                ConversationStatus::Idle,
                TurnTaskStatus::Completed,
            ),
            (
                json!({ "type": "idle" }),
                "failed",
                ConversationStatus::Error,
                TurnTaskStatus::Failed,
            ),
            (
                json!({ "type": "idle" }),
                "interrupted",
                ConversationStatus::Idle,
                TurnTaskStatus::Interrupted,
            ),
        ];
        for (runtime, native_turn, conversation, turn) in cases {
            let mapped = map_thread(&snapshot(runtime, native_turn));
            assert_eq!(mapped.conversation.status, conversation);
            assert_eq!(mapped.conversation.created_at, 1_700_000_000_000_u64);
            assert_eq!(mapped.conversation.permission_level, PermissionLevel::WorkspaceWrite);
            assert_eq!(mapped.latest_turn.unwrap().status, turn);
        }
    }

    #[test]
    fn unknown_turn_status_is_ignored_and_diagnosed() {
        let mapped = map_thread(&snapshot(json!({ "type": "idle" }), "futureStatus"));
        assert!(mapped.latest_turn.is_none());
        assert_eq!(mapped.conversation.status, ConversationStatus::Idle);
        assert!(mapped.diagnostics[0].contains("futureStatus"));
    }

    #[test]
    fn system_error_overrides_stale_waiting_flags_and_in_progress_turn() {
        let mut source = snapshot(
            json!({
                "type": "systemError",
                "activeFlags": ["waitingOnApproval"]
            }),
            "inProgress",
        );
        source.state["turnHistory"]["history"]["entitiesByKey"]["turn:one"]["error"] =
            json!({ "message": "owner failed" });
        let mapped = map_thread(&source);

        assert_eq!(mapped.conversation.status, ConversationStatus::Error);
        assert!(mapped.conversation.active_turn.is_none());
        let turn = mapped.latest_turn.unwrap();
        assert_eq!(turn.status, TurnTaskStatus::Failed);
        assert_eq!(turn.display_summary.as_deref(), Some("owner failed"));
        assert_eq!(turn.completed_at, Some(1_700_000_001_000_u64));
    }

    #[test]
    fn system_error_does_not_rewrite_a_terminal_turn() {
        let mapped = map_thread(&snapshot(json!({ "type": "systemError" }), "completed"));

        assert_eq!(mapped.conversation.status, ConversationStatus::Error);
        assert!(mapped.conversation.active_turn.is_none());
        assert_eq!(
            mapped.latest_turn.unwrap().status,
            TurnTaskStatus::Completed
        );
    }

    #[test]
    fn latest_thread_settings_take_permission_precedence() {
        let mut source = snapshot(json!({ "type": "idle" }), "completed");
        source.state["latestThreadSettings"] = json!({
            "sandboxPolicy": { "type": "readOnly" }
        });
        source.state["currentPermissions"] = json!({
            "sandboxPolicy": { "type": "dangerFullAccess" }
        });
        let mapped = map_thread(&source);
        assert_eq!(mapped.conversation.permission_level, PermissionLevel::ReadOnly);
    }

    #[test]
    fn maps_command_and_file_approvals_with_native_routing_metadata() {
        let mut source = snapshot(json!({ "type": "active" }), "inProgress");
        source.state["requests"] = json!([
            {
                "id": 17,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "command-one",
                    "reason": "Run a harmless check"
                }
            },
            {
                "id": "file-request",
                "method": "item/fileChange/requestApproval",
                "params": {
                    "threadId": "thread-one",
                    "turnId": "turn-one",
                    "itemId": "file-one"
                }
            }
        ]);

        let mapped = map_thread(&source);

        assert_eq!(mapped.conversation.status, ConversationStatus::WaitingApproval);
        assert_eq!(mapped.approvals.len(), 2);
        let command = &mapped.approvals[0];
        assert_eq!(command.raw_request_id, json!(17));
        assert_eq!(command.native_method, "item/commandExecution/requestApproval");
        assert_eq!(command.owner_client_id, "owner-one");
        assert_eq!(command.revision, 3);
        assert_eq!(command.thread_id, "thread-one");
        assert_eq!(command.turn_id, "turn-one");
        assert_eq!(command.item_id, "command-one");
        assert_eq!(command.approval.kind, "command-execution");
        assert_eq!(command.approval.status, ApprovalStatus::Pending);
        assert_eq!(
            command.approval.decisions,
            vec![ApprovalDecision::Approve, ApprovalDecision::Deny]
        );
        assert_eq!(command.approval.requested_at, 1_700_000_000_000_u64);
        assert_eq!(
            command.approval.description.as_deref(),
            Some("Run a harmless check")
        );
        let file = &mapped.approvals[1];
        assert_eq!(file.raw_request_id, json!("file-request"));
        assert_eq!(file.native_method, "item/fileChange/requestApproval");
        assert_eq!(file.approval.kind, "file-change");
        assert_ne!(command.approval.id, file.approval.id);
    }

    #[test]
    fn unsupported_pending_requests_remain_waiting_with_diagnostics() {
        let mut source = snapshot(json!({ "type": "active" }), "inProgress");
        source.state["requests"] = json!([
            {
                "id": "permissions-one",
                "method": "item/permissions/requestApproval",
                "params": { "threadId": "thread-one", "turnId": "turn-one" }
            },
            {
                "id": "plan-one",
                "method": "item/plan/requestImplementation",
                "params": { "threadId": "thread-one", "turnId": "turn-one" }
            }
        ]);

        let mapped = map_thread(&source);

        assert!(mapped.approvals.is_empty());
        assert_eq!(mapped.conversation.status, ConversationStatus::WaitingApproval);
        assert_eq!(
            mapped.latest_turn.as_ref().map(|turn| turn.status),
            Some(TurnTaskStatus::WaitingApproval)
        );
        assert_eq!(
            mapped
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.contains("capability_unsupported"))
                .count(),
            2
        );
    }

    #[test]
    fn mismatched_approval_thread_is_not_actionable() {
        let mut source = snapshot(json!({ "type": "active" }), "inProgress");
        source.state["requests"] = json!([{
            "id": "command-one",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-other",
                "turnId": "turn-one",
                "itemId": "command-one"
            }
        }]);

        let mapped = map_thread(&source);

        assert!(mapped.approvals.is_empty());
        assert!(mapped
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("threadId does not match snapshot")));
        assert_eq!(mapped.conversation.status, ConversationStatus::WaitingApproval);
    }

    #[test]
    fn approval_for_non_active_turn_is_not_actionable() {
        let mut source = snapshot(json!({ "type": "active" }), "completed");
        source.state["requests"] = json!([{
            "id": "command-one",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-one",
                "turnId": "turn-one",
                "itemId": "command-one"
            }
        }]);

        let mapped = map_thread(&source);

        assert!(mapped.approvals.is_empty());
        assert!(mapped.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("turnId is not the current inProgress turn")
        }));
        assert_eq!(mapped.conversation.status, ConversationStatus::WaitingApproval);
    }

    #[test]
    fn provider_advertises_only_verified_companion_methods() {
        let provider = provider(ProviderStatus::Ready, None);
        assert_eq!(
            provider.capabilities.methods,
            vec![
                "conversation.get",
                "turn.send",
                "turn.interrupt",
                "approval.resolve"
            ]
        );
        assert!(provider.capabilities.can_steer);
        assert!(provider.capabilities.can_interrupt);
        assert_eq!(provider.capabilities.quick_replies.len(), 1);
        assert_eq!(provider.capabilities.quick_replies[0].id, CONTINUE_QUICK_REPLY_ID);
        assert!(!provider.capabilities.methods.contains(&"conversation.list".to_string()));
        assert!(!provider.capabilities.methods.contains(&"conversation.create".to_string()));
    }
}
