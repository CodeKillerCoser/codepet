use crate::protocol::{
    CodexAppServerError, CodexApprovalRequest, CodexConversationSnapshot, CodexIncoming,
    CodexNotification, CodexPermissionLevel, CodexThreadActiveFlag, CodexThreadStatus,
    CodexTurn, CodexTurnStatus, CODEX_EXTENSION_NAMESPACE,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ConversationStatus, ConversationUpsertedEvent, InstanceStatus, JsonObject, ProtocolError,
    ProtocolEvent, ProviderApproval, ProviderCapabilities, ProviderCapability,
    ProviderConversation, ProviderExtension, ProviderInstance, ProviderInstanceRoute,
    ProviderTurn, RoutedResourceId, TurnOutputDeltaEvent, TurnStatus, TurnUpsertedEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct CodexProtocolMapper {
    route: ProviderInstanceRoute,
}

impl CodexProtocolMapper {
    pub fn new(route: ProviderInstanceRoute) -> Self {
        Self { route }
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
        let active_turn = snapshot
            .thread
            .turns
            .iter()
            .rev()
            .find(|turn| turn.status == CodexTurnStatus::InProgress)
            .map(|turn| {
                self.turn(&snapshot.thread.id, turn)
            });
        let status = match snapshot.thread.status {
            CodexThreadStatus::Active { ref active_flags }
                if active_flags.contains(&CodexThreadActiveFlag::WaitingOnApproval) => {
                    ConversationStatus::WaitingApproval
                }
            CodexThreadStatus::Active { ref active_flags }
                if active_flags.contains(&CodexThreadActiveFlag::WaitingOnUserInput) => {
                    ConversationStatus::WaitingUserInput
                }
            CodexThreadStatus::Active { .. } => ConversationStatus::Running,
            CodexThreadStatus::SystemError => ConversationStatus::Error,
            CodexThreadStatus::NotLoaded | CodexThreadStatus::Idle => ConversationStatus::Idle,
        };
        let mut extension_data = BTreeMap::new();
        extension_data.insert(
            "nativeThreadStatus".to_string(),
            json!(thread_status_name(&snapshot.thread.status)),
        );
        extension_data.insert("nativeCwd".to_string(), json!(snapshot.thread.cwd));
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
                .unwrap_or_else(|| snapshot.thread.id.clone()),
            preview: (!snapshot.thread.preview.is_empty()).then(|| snapshot.thread.preview.clone()),
            status,
            permission_level: snapshot
                .permission_level
                .map(permission_level_name)
                .map(str::to_string),
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
    ) -> ProviderTurn {
        let started_at = turn.started_at.and_then(seconds_to_ms);
        let completed_at = turn.completed_at.and_then(seconds_to_ms);
        ProviderTurn {
            resource: self.resource(turn.id.clone()),
            conversation: self.resource(conversation_id.to_string()),
            status: turn_status(turn.status),
            display_summary: None,
            started_at,
            updated_at: None,
            completed_at,
            extension: Some(extension([(
                "nativeStatus",
                json!(turn_status_name(turn.status)),
            )])),
        }
    }

    pub fn events(
        &self,
        incoming: CodexIncoming,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        match incoming {
            CodexIncoming::Notification(notification) => self.notification_events(notification),
            CodexIncoming::ApprovalRequested(request) => {
                let approval = self.approval(&request);
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
        &self,
        mut approval: ProviderApproval,
        decision: ApprovalDecision,
        resolved_at_ms: u64,
    ) -> Result<(ProviderApproval, ProtocolEvent), ProtocolError> {
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

    pub fn approval_expired(
        &self,
        mut approval: ProviderApproval,
        resolved_at_ms: u64,
    ) -> (ProviderApproval, ProtocolEvent) {
        approval.status = ApprovalStatus::Expired;
        approval.resolved_at = Some(resolved_at_ms);
        let event = ProtocolEvent::EventApprovalResolved {
            jsonrpc: "2.0".to_string(),
            params: ApprovalResolvedEvent {
                approval: approval.clone(),
            },
        };
        (approval, event)
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
        &self,
        notification: CodexNotification,
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
                        turn: self.turn(&thread_id, &turn),
                    },
                }
            }
            CodexNotification::OutputDelta {
                native_method,
                thread_id,
                turn_id,
                item_id,
                kind,
                delta,
            } => ProtocolEvent::EventTurnOutputDelta {
                jsonrpc: "2.0".to_string(),
                params: TurnOutputDeltaEvent {
                    turn: self.resource(turn_id),
                    conversation: self.resource(thread_id),
                    output_id: item_id,
                    kind,
                    delta,
                    extension: Some(extension([("nativeMethod", json!(native_method))])),
                },
            },
            CodexNotification::ServerRequestResolved {
                ..
            } => return Ok(Vec::new()),
            CodexNotification::Unknown { .. } => return Ok(Vec::new()),
        };
        Ok(vec![event])
    }

    pub fn approval(&self, request: &CodexApprovalRequest) -> ProviderApproval {
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
            provider_plugin_id: self.route.provider_plugin_id.clone(),
            provider_instance_id: self.route.provider_instance_id.clone(),
            native_resource_id,
        }
    }
}

fn approval_decisions(request: &CodexApprovalRequest) -> Vec<ApprovalDecision> {
    let mut decisions = Vec::new();
    if request
        .available_decisions
        .iter()
        .any(|decision| decision == "accept")
    {
        decisions.push(ApprovalDecision::Approve);
    }
    if request
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

fn seconds_to_ms(value: i64) -> Option<u64> {
    u64::try_from(value).ok()?.checked_mul(1_000)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CodexThread;

    #[test]
    fn active_thread_flags_keep_approval_and_user_input_states_distinct() {
        let mapper = CodexProtocolMapper::new(ProviderInstanceRoute {
            device_id: "device-test".to_string(),
            provider_plugin_id: "dev.codepet.codex".to_string(),
            provider_instance_id: "codex".to_string(),
        });
        let waiting_approval = mapper.conversation(&snapshot(
            CodexThreadActiveFlag::WaitingOnApproval,
        ));
        let waiting_user = mapper.conversation(&snapshot(
            CodexThreadActiveFlag::WaitingOnUserInput,
        ));

        assert_eq!(waiting_approval.status, ConversationStatus::WaitingApproval);
        assert_eq!(waiting_user.status, ConversationStatus::WaitingUserInput);
    }

    #[test]
    fn turn_does_not_invent_an_updated_timestamp() {
        let mapper = CodexProtocolMapper::new(ProviderInstanceRoute {
            device_id: "device-test".to_string(),
            provider_plugin_id: "dev.codepet.codex".to_string(),
            provider_instance_id: "codex".to_string(),
        });
        let turn = CodexTurn {
            id: "turn-test".to_string(),
            status: CodexTurnStatus::Completed,
            started_at: Some(1),
            completed_at: Some(2),
            items: Vec::new(),
        };

        let mapped = mapper.turn("thread-test", &turn);

        assert_eq!(mapped.started_at, Some(1_000));
        assert_eq!(mapped.completed_at, Some(2_000));
        assert_eq!(mapped.updated_at, None);
    }

    #[test]
    fn timestamp_overflow_is_unknown_instead_of_saturating() {
        assert_eq!(seconds_to_ms(i64::MAX), None);
        assert_eq!(seconds_to_ms(-1), None);
        assert_eq!(seconds_to_ms(42), Some(42_000));
    }

    fn snapshot(flag: CodexThreadActiveFlag) -> CodexConversationSnapshot {
        CodexConversationSnapshot::from_thread(CodexThread {
            id: "thread-test".to_string(),
            name: None,
            preview: "fixture".to_string(),
            cwd: "/fixture".to_string(),
            created_at: 1,
            updated_at: 2,
            status: CodexThreadStatus::Active {
                active_flags: vec![flag],
            },
            turns: Vec::new(),
            cli_version: "0.151.0".to_string(),
            ephemeral: false,
            model_provider: "openai".to_string(),
            project_id: Value::Null,
            session_id: "session-test".to_string(),
            source: json!("appServer"),
        })
    }
}
