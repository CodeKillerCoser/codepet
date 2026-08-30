use crate::protocol::{
    CodexAppServerError, CodexApprovalRequest, CodexConversationSnapshot, CodexIncoming,
    CodexNotification, CodexPermissionLevel, CodexThreadStatus, CodexTurn, CodexTurnStatus,
    CODEX_EXTENSION_NAMESPACE,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ConversationStatus, ConversationUpsertedEvent, InstanceStatus, JsonObject, ProtocolError,
    ProtocolEvent, ProviderApproval, ProviderCapabilities, ProviderCapability,
    ProviderConversation, ProviderExtension, ProviderInstance, ProviderInstanceRoute,
    ProviderTurn, RoutedResourceId, TurnOutputDeltaEvent, TurnStatus, TurnUpsertedEvent,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub struct CodexProtocolMapper {
    route: ProviderInstanceRoute,
    pending_approvals: HashMap<String, ProviderApproval>,
}

impl CodexProtocolMapper {
    pub fn new(route: ProviderInstanceRoute) -> Self {
        Self {
            route,
            pending_approvals: HashMap::new(),
        }
    }

    pub fn reset_session_state(&mut self) {
        self.pending_approvals.clear();
    }

    pub fn capabilities(
        models: Vec<String>,
        reasoning_efforts: Vec<String>,
    ) -> ProviderCapabilities {
        ProviderCapabilities {
            methods: vec![
                ProviderCapability::ConversationList,
                ProviderCapability::ConversationGet,
                ProviderCapability::ConversationCreate,
                ProviderCapability::TurnStart,
                ProviderCapability::TurnSteer,
                ProviderCapability::TurnInterrupt,
                ProviderCapability::ApprovalResolve,
            ],
            permission_levels: vec![
                "read-only".to_string(),
                "workspace-write".to_string(),
                "full-access".to_string(),
            ],
            models,
            reasoning_efforts,
            extensions: vec![extension([
                (
                    "nativeMethods",
                    json!([
                        "thread/list",
                        "thread/read",
                        "thread/resume",
                        "thread/start",
                        "turn/start",
                        "turn/steer",
                        "turn/interrupt",
                        "item/commandExecution/requestApproval",
                        "item/fileChange/requestApproval"
                    ]),
                ),
                (
                    "unsupportedApprovalKinds",
                    json!(["permissions", "tool-user-input", "mcp-elicitation"]),
                ),
            ])],
        }
    }

    pub fn instance(
        &self,
        plugin_id: String,
        instance_kind: String,
        display_name: String,
        status: InstanceStatus,
        capabilities: ProviderCapabilities,
    ) -> ProviderInstance {
        ProviderInstance {
            route: self.route.clone(),
            plugin_id,
            instance_kind,
            display_name,
            status,
            capabilities,
        }
    }

    pub fn conversation(&self, snapshot: &CodexConversationSnapshot) -> ProviderConversation {
        let permission_level = snapshot
            .permission_level
            .unwrap_or(CodexPermissionLevel::WorkspaceWrite);
        let active_turn = snapshot
            .thread
            .turns
            .iter()
            .rev()
            .find(|turn| turn.status == CodexTurnStatus::InProgress)
            .map(|turn| {
                self.turn(
                    &snapshot.thread.id,
                    turn,
                    seconds_to_ms(snapshot.thread.updated_at),
                )
            });
        let status = if active_turn.is_some() {
            ConversationStatus::Running
        } else {
            match snapshot.thread.status {
                CodexThreadStatus::Active { .. } => ConversationStatus::Running,
                CodexThreadStatus::SystemError | CodexThreadStatus::Unknown => {
                    ConversationStatus::Error
                }
                CodexThreadStatus::NotLoaded | CodexThreadStatus::Idle => ConversationStatus::Idle,
            }
        };
        let mut extension_data = BTreeMap::new();
        extension_data.insert(
            "nativeThreadStatus".to_string(),
            json!(thread_status_name(&snapshot.thread.status)),
        );
        extension_data.insert(
            "permissionSource".to_string(),
            json!(if snapshot.permission_level.is_some() {
                "app-server"
            } else {
                "provider-default"
            }),
        );
        if let Some(cwd) = &snapshot.thread.cwd {
            extension_data.insert("nativeCwd".to_string(), json!(cwd));
        }
        ProviderConversation {
            resource: self.resource(snapshot.thread.id.clone()),
            title: snapshot
                .thread
                .name
                .clone()
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    (!snapshot.thread.preview.is_empty()).then(|| snapshot.thread.preview.clone())
                })
                .unwrap_or_else(|| "Codex conversation".to_string()),
            preview: (!snapshot.thread.preview.is_empty()).then(|| snapshot.thread.preview.clone()),
            status,
            permission_level: permission_level_name(permission_level).to_string(),
            model: snapshot.model.clone(),
            reasoning_effort: snapshot.reasoning_effort.clone(),
            workspace_root: snapshot.workspace_root.clone(),
            created_at: seconds_to_ms(snapshot.thread.created_at),
            updated_at: seconds_to_ms(snapshot.thread.updated_at),
            active_turn,
            extension: Some(ProviderExtension {
                namespace: CODEX_EXTENSION_NAMESPACE.to_string(),
                data: extension_data,
            }),
        }
    }

    pub fn turn(
        &self,
        conversation_id: &str,
        turn: &CodexTurn,
        observed_at_ms: u64,
    ) -> ProviderTurn {
        let started_at = turn.started_at.map(seconds_to_ms);
        let completed_at = turn.completed_at.map(seconds_to_ms);
        ProviderTurn {
            resource: self.resource(turn.id.clone()),
            conversation: self.resource(conversation_id.to_string()),
            status: turn_status(turn.status),
            display_summary: None,
            started_at,
            updated_at: completed_at.or(started_at).unwrap_or(observed_at_ms),
            completed_at,
            extension: Some(extension([(
                "nativeStatus",
                json!(turn_status_name(turn.status)),
            )])),
        }
    }

    pub fn events(
        &mut self,
        incoming: CodexIncoming,
        observed_at_ms: u64,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        match incoming {
            CodexIncoming::Notification(notification) => {
                self.notification_events(notification, observed_at_ms)
            }
            CodexIncoming::ApprovalRequested(request) => {
                let approval = self.approval(&request);
                self.pending_approvals.insert(
                    approval.resource.native_resource_id.clone(),
                    approval.clone(),
                );
                Ok(vec![ProtocolEvent::EventApprovalRequested {
                    jsonrpc: "2.0".to_string(),
                    params: ApprovalRequestedEvent { approval },
                }])
            }
            CodexIncoming::UnsupportedServerRequest { method, .. } => {
                eprintln!("Unsupported Codex App Server request rejected: {method}");
                Ok(Vec::new())
            }
        }
    }

    pub fn approval_resolved(
        &mut self,
        approval_id: &str,
        decision: ApprovalDecision,
        resolved_at_ms: u64,
    ) -> Result<(ProviderApproval, ProtocolEvent), ProtocolError> {
        let mut approval = self
            .pending_approvals
            .remove(approval_id)
            .ok_or_else(|| protocol_error(
                "approval_not_found",
                format!("approval {approval_id} is not pending"),
                false,
            ))?;
        approval.status = match decision {
            ApprovalDecision::Approve => ApprovalStatus::Approved,
            ApprovalDecision::Deny => ApprovalStatus::Denied,
        };
        approval.decision = Some(decision);
        approval.resolved_at = Some(resolved_at_ms);
        let event = ProtocolEvent::EventApprovalResolved {
            jsonrpc: "2.0".to_string(),
            params: ApprovalResolvedEvent {
                approval: approval.clone(),
            },
        };
        Ok((approval, event))
    }

    pub fn error(error: CodexAppServerError) -> ProtocolError {
        match error {
            CodexAppServerError::Rpc { code, message } => ProtocolError {
                code: "provider_error".to_string(),
                message,
                retryable: code == -32001,
                details: Some(object([("nativeCode", json!(code))])),
            },
            CodexAppServerError::Protocol(message) => {
                protocol_error("provider_protocol_error", message, false)
            }
            CodexAppServerError::Spawn(message)
            | CodexAppServerError::Io(message)
            | CodexAppServerError::Timeout(message) => {
                protocol_error("provider_unavailable", message, true)
            }
            CodexAppServerError::ProcessExited => protocol_error(
                "provider_unavailable",
                "Codex App Server exited".to_string(),
                true,
            ),
            CodexAppServerError::Shutdown => protocol_error(
                "provider_unavailable",
                "Codex App Server session is shut down".to_string(),
                true,
            ),
        }
    }

    fn notification_events(
        &mut self,
        notification: CodexNotification,
        observed_at_ms: u64,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        let event = match notification {
            CodexNotification::ThreadStarted { thread } => {
                ProtocolEvent::EventConversationUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: ConversationUpsertedEvent {
                        conversation: self
                            .conversation(&CodexConversationSnapshot::from_thread(thread)),
                    },
                }
            }
            CodexNotification::TurnStarted { thread_id, turn }
            | CodexNotification::TurnCompleted { thread_id, turn } => {
                ProtocolEvent::EventTurnUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: TurnUpsertedEvent {
                        turn: self.turn(&thread_id, &turn, observed_at_ms),
                    },
                }
            }
            CodexNotification::OutputDelta {
                native_method,
                thread_id: _,
                turn_id,
                item_id,
                kind,
                delta,
            } => ProtocolEvent::EventTurnOutputDelta {
                jsonrpc: "2.0".to_string(),
                params: TurnOutputDeltaEvent {
                    turn: self.resource(turn_id),
                    output_id: item_id,
                    kind,
                    delta,
                    extension: Some(extension([("nativeMethod", json!(native_method))])),
                },
            },
            CodexNotification::ServerRequestResolved {
                request_id,
                thread_id: _,
            } => {
                let approval_id = request_id.approval_id();
                let Some(mut approval) = self.pending_approvals.remove(&approval_id) else {
                    return Ok(Vec::new());
                };
                approval.status = ApprovalStatus::Expired;
                approval.resolved_at = Some(observed_at_ms);
                ProtocolEvent::EventApprovalResolved {
                    jsonrpc: "2.0".to_string(),
                    params: ApprovalResolvedEvent { approval },
                }
            }
            CodexNotification::Unknown { .. } => return Ok(Vec::new()),
        };
        Ok(vec![event])
    }

    fn approval(&self, request: &CodexApprovalRequest) -> ProviderApproval {
        let decisions = approval_decisions(request);
        let mut decision_mapping = BTreeMap::new();
        if decisions.contains(&ApprovalDecision::Approve) {
            decision_mapping.insert("approve", "accept");
        }
        if decisions.contains(&ApprovalDecision::Deny) {
            decision_mapping.insert("deny", "decline");
        }
        ProviderApproval {
            resource: self.resource(request.approval_id()),
            conversation: self.resource(request.thread_id.clone()),
            turn: self.resource(request.turn_id.clone()),
            kind: request.kind.as_str().to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            status: ApprovalStatus::Pending,
            decisions,
            requested_at: request.requested_at_ms,
            resolved_at: None,
            decision: None,
            extension: Some(extension([
                ("nativeMethod", json!(request.native_method())),
                ("nativeItemId", json!(request.item_id)),
                (
                    "nativeAvailableDecisions",
                    json!(request.available_decisions),
                ),
                (
                    "decisionMapping",
                    json!(decision_mapping),
                ),
            ])),
        }
    }

    fn resource(&self, native_resource_id: String) -> RoutedResourceId {
        RoutedResourceId {
            device_id: self.route.device_id.clone(),
            provider_instance_id: self.route.provider_instance_id.clone(),
            native_resource_id,
        }
    }
}

fn approval_decisions(request: &CodexApprovalRequest) -> Vec<ApprovalDecision> {
    let accepts_legacy_defaults = request.available_decisions.is_empty();
    let mut decisions = Vec::new();
    if accepts_legacy_defaults
        || request
            .available_decisions
            .iter()
            .any(|decision| decision == "accept")
    {
        decisions.push(ApprovalDecision::Approve);
    }
    if accepts_legacy_defaults
        || request
            .available_decisions
            .iter()
            .any(|decision| decision == "decline")
    {
        decisions.push(ApprovalDecision::Deny);
    }
    decisions
}

fn turn_status(status: CodexTurnStatus) -> TurnStatus {
    match status {
        CodexTurnStatus::InProgress => TurnStatus::Running,
        CodexTurnStatus::Completed => TurnStatus::Completed,
        CodexTurnStatus::Failed => TurnStatus::Failed,
        CodexTurnStatus::Interrupted => TurnStatus::Interrupted,
    }
}

fn turn_status_name(status: CodexTurnStatus) -> &'static str {
    match status {
        CodexTurnStatus::InProgress => "inProgress",
        CodexTurnStatus::Completed => "completed",
        CodexTurnStatus::Failed => "failed",
        CodexTurnStatus::Interrupted => "interrupted",
    }
}

fn thread_status_name(status: &CodexThreadStatus) -> &'static str {
    match status {
        CodexThreadStatus::NotLoaded => "notLoaded",
        CodexThreadStatus::Idle => "idle",
        CodexThreadStatus::SystemError => "systemError",
        CodexThreadStatus::Active { .. } => "active",
        CodexThreadStatus::Unknown => "unknown",
    }
}

fn permission_level_name(permission: CodexPermissionLevel) -> &'static str {
    match permission {
        CodexPermissionLevel::ReadOnly => "read-only",
        CodexPermissionLevel::WorkspaceWrite => "workspace-write",
        CodexPermissionLevel::FullAccess => "full-access",
    }
}

pub fn parse_permission_level(value: &str) -> Result<CodexPermissionLevel, ProtocolError> {
    match value {
        "read-only" => Ok(CodexPermissionLevel::ReadOnly),
        "workspace-write" => Ok(CodexPermissionLevel::WorkspaceWrite),
        "full-access" => Ok(CodexPermissionLevel::FullAccess),
        _ => Err(protocol_error(
            "invalid_permission_level",
            format!("unsupported Codex permission level: {value}"),
            false,
        )),
    }
}

fn seconds_to_ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0).saturating_mul(1_000)
}

fn protocol_error(code: &str, message: String, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.to_string(),
        message,
        retryable,
        details: None,
    }
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn extension<const N: usize>(entries: [(&str, Value); N]) -> ProviderExtension {
    ProviderExtension {
        namespace: CODEX_EXTENSION_NAMESPACE.to_string(),
        data: object(entries),
    }
}
