use super::protocol::{
    CodexAppServerError, CodexApprovalRequest, CodexConversationSnapshot, CodexIncoming,
    CodexNotification, CodexThreadStatus, CodexTurn, CodexTurnStatus,
    CODEX_EXTENSION_NAMESPACE, CODEX_PROVIDER_ID,
};
use crate::runtime_gateway::generated::{
    Approval, ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    Conversation, ConversationStatus, ConversationUpsertedEvent, EventSequence, JsonObject,
    PermissionLevel, ProtocolError, ProtocolEvent, Provider, ProviderCapabilities,
    ProviderExtension, ProviderStatus, TurnOutputDeltaEvent, TurnTask, TurnTaskStatus,
    TurnUpsertedEvent, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub struct CodexProtocolMapper {
    provider_id: String,
    pending_approvals: HashMap<String, Approval>,
}

impl Default for CodexProtocolMapper {
    fn default() -> Self {
        Self::new(CODEX_PROVIDER_ID)
    }
}

impl CodexProtocolMapper {
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            pending_approvals: HashMap::new(),
        }
    }

    pub fn provider(
        &self,
        version: Option<String>,
        status: ProviderStatus,
        models: Vec<String>,
        reasoning_efforts: Vec<String>,
    ) -> Provider {
        Provider {
            id: self.provider_id.clone(),
            provider_type: "codex".to_string(),
            display_name: "Codex".to_string(),
            version,
            status,
            capabilities: ProviderCapabilities {
                methods: vec![
                    "conversation.list".to_string(),
                    "conversation.get".to_string(),
                    "conversation.create".to_string(),
                    "turn.send".to_string(),
                    "turn.interrupt".to_string(),
                    "approval.resolve".to_string(),
                ],
                permission_levels: vec![
                    PermissionLevel::ReadOnly,
                    PermissionLevel::WorkspaceWrite,
                    PermissionLevel::FullAccess,
                ],
                models,
                reasoning_efforts,
                quick_replies: Vec::new(),
                can_steer: true,
                can_interrupt: true,
                extension: Some(extension([
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
                ])),
            },
            extension: Some(extension([
                ("protocol", json!("v2")),
                ("managedConversationEvents", json!(true)),
                ("externalConversationEvents", json!(false)),
            ])),
        }
    }

    pub fn conversation(&self, snapshot: &CodexConversationSnapshot) -> Conversation {
        let permission_level = snapshot
            .permission_level
            .unwrap_or(PermissionLevel::WorkspaceWrite);
        let active_turn = snapshot
            .thread
            .turns
            .iter()
            .rev()
            .find(|turn| turn.status == CodexTurnStatus::InProgress)
            .map(|turn| self.turn(&snapshot.thread.id, turn, seconds_to_ms(snapshot.thread.updated_at)));
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
            "workspaceKind".to_string(),
            json!(if snapshot.workspace_root.is_some() {
                "project"
            } else {
                "normal"
            }),
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
        Conversation {
            id: snapshot.thread.id.clone(),
            provider_id: self.provider_id.clone(),
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
            permission_level,
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

    pub fn turn(&self, conversation_id: &str, turn: &CodexTurn, observed_at_ms: u64) -> TurnTask {
        let started_at = turn.started_at.map(seconds_to_ms);
        let completed_at = turn.completed_at.map(seconds_to_ms);
        TurnTask {
            id: turn.id.clone(),
            provider_id: self.provider_id.clone(),
            conversation_id: conversation_id.to_string(),
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
        event_sequence: EventSequence,
        observed_at_ms: u64,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        match incoming {
            CodexIncoming::Notification(notification) => {
                self.notification_events(notification, event_sequence, observed_at_ms)
            }
            CodexIncoming::ApprovalRequested(request) => {
                let approval = self.approval(&request);
                self.pending_approvals
                    .insert(approval.id.clone(), approval.clone());
                Ok(vec![ProtocolEvent::ApprovalRequested {
                    protocol_version: PROTOCOL_VERSION,
                    event_sequence,
                    payload: ApprovalRequestedEvent { approval },
                }])
            }
            CodexIncoming::UnsupportedServerRequest { method, .. } => Err(ProtocolError {
                code: "capability_unsupported".to_string(),
                message: format!("Codex server request {method} is not supported by protocol v0"),
                retryable: false,
                details: Some(object([("nativeMethod", json!(method))])),
            }),
        }
    }

    pub fn approval_resolved_event(
        &mut self,
        approval_id: &str,
        decision: ApprovalDecision,
        event_sequence: EventSequence,
        resolved_at_ms: u64,
    ) -> Result<ProtocolEvent, ProtocolError> {
        let mut approval = self
            .pending_approvals
            .remove(approval_id)
            .ok_or_else(|| ProtocolError {
                code: "approval_not_found".to_string(),
                message: format!("approval {approval_id} is not pending"),
                retryable: false,
                details: None,
            })?;
        approval.status = match decision {
            ApprovalDecision::Approve => ApprovalStatus::Approved,
            ApprovalDecision::Deny => ApprovalStatus::Denied,
        };
        approval.decision = Some(decision);
        approval.resolved_at = Some(resolved_at_ms);
        Ok(ProtocolEvent::ApprovalResolved {
            protocol_version: PROTOCOL_VERSION,
            event_sequence,
            payload: ApprovalResolvedEvent { approval },
        })
    }

    pub fn error(&self, error: CodexAppServerError) -> ProtocolError {
        match error {
            CodexAppServerError::UnsupportedCapability {
                capability,
                message,
            } => ProtocolError {
                code: "capability_unsupported".to_string(),
                message,
                retryable: false,
                details: Some(object([("capability", json!(capability))])),
            },
            CodexAppServerError::Rpc { code, message } => ProtocolError {
                code: "provider_error".to_string(),
                message,
                retryable: code == -32001,
                details: Some(object([("nativeCode", json!(code))])),
            },
            CodexAppServerError::Protocol(message) => ProtocolError {
                code: "provider_protocol_error".to_string(),
                message,
                retryable: false,
                details: None,
            },
            CodexAppServerError::Spawn(message) | CodexAppServerError::Io(message) => {
                ProtocolError {
                    code: "provider_unavailable".to_string(),
                    message,
                    retryable: true,
                    details: None,
                }
            }
            CodexAppServerError::ProcessExited => ProtocolError {
                code: "provider_unavailable".to_string(),
                message: "Codex App Server exited".to_string(),
                retryable: true,
                details: None,
            },
            CodexAppServerError::Shutdown => ProtocolError {
                code: "provider_unavailable".to_string(),
                message: "Codex App Server session is shut down".to_string(),
                retryable: true,
                details: None,
            },
        }
    }

    fn notification_events(
        &mut self,
        notification: CodexNotification,
        event_sequence: EventSequence,
        observed_at_ms: u64,
    ) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        let event = match notification {
            CodexNotification::ThreadStarted { thread } => {
                let conversation = self.conversation(&CodexConversationSnapshot::from_thread(thread));
                ProtocolEvent::ConversationUpserted {
                    protocol_version: PROTOCOL_VERSION,
                    event_sequence,
                    payload: ConversationUpsertedEvent { conversation },
                }
            }
            CodexNotification::TurnStarted { thread_id, turn }
            | CodexNotification::TurnCompleted { thread_id, turn } => {
                ProtocolEvent::TurnUpserted {
                    protocol_version: PROTOCOL_VERSION,
                    event_sequence,
                    payload: TurnUpsertedEvent {
                        turn: self.turn(&thread_id, &turn, observed_at_ms),
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
            } => ProtocolEvent::TurnOutputDelta {
                protocol_version: PROTOCOL_VERSION,
                event_sequence,
                payload: TurnOutputDeltaEvent {
                    provider_id: self.provider_id.clone(),
                    conversation_id: thread_id,
                    turn_id,
                    output_id: item_id.clone(),
                    kind,
                    delta,
                    extension: Some(extension([
                        ("nativeMethod", json!(native_method)),
                        ("nativeItemId", json!(item_id)),
                    ])),
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
                ProtocolEvent::ApprovalResolved {
                    protocol_version: PROTOCOL_VERSION,
                    event_sequence,
                    payload: ApprovalResolvedEvent { approval },
                }
            }
            CodexNotification::Unknown { .. } => return Ok(Vec::new()),
        };
        Ok(vec![event])
    }

    fn approval(&self, request: &CodexApprovalRequest) -> Approval {
        Approval {
            id: request.approval_id(),
            provider_id: self.provider_id.clone(),
            conversation_id: request.thread_id.clone(),
            turn_id: request.turn_id.clone(),
            kind: request.kind.as_str().to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            status: ApprovalStatus::Pending,
            decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
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
                    json!({ "approve": "accept", "deny": "decline" }),
                ),
            ])),
        }
    }
}

fn turn_status(status: CodexTurnStatus) -> TurnTaskStatus {
    match status {
        CodexTurnStatus::InProgress => TurnTaskStatus::Running,
        CodexTurnStatus::Completed => TurnTaskStatus::Completed,
        CodexTurnStatus::Failed => TurnTaskStatus::Failed,
        CodexTurnStatus::Interrupted => TurnTaskStatus::Interrupted,
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

fn seconds_to_ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0).saturating_mul(1_000)
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
    use crate::agent::codex_app_server::protocol::{
        CodexApprovalKind, CodexThread, JsonRpcId,
    };

    fn thread(id: &str, turns: Vec<CodexTurn>) -> CodexThread {
        CodexThread {
            id: id.to_string(),
            name: Some("Fixture thread".to_string()),
            preview: "hello".to_string(),
            cwd: Some("/tmp/native-cwd".to_string()),
            created_at: 10,
            updated_at: 20,
            status: if turns.is_empty() {
                CodexThreadStatus::Idle
            } else {
                CodexThreadStatus::Active {
                    active_flags: Vec::new(),
                }
            },
            turns,
        }
    }

    #[test]
    fn maps_normal_and_project_conversations_without_inferring_workspace_root() {
        let mapper = CodexProtocolMapper::default();
        let normal = CodexConversationSnapshot::from_thread(thread("normal", Vec::new()));
        let project = CodexConversationSnapshot {
            thread: thread("project", Vec::new()),
            workspace_root: Some("/work/project".to_string()),
            permission_level: Some(PermissionLevel::FullAccess),
            model: Some("gpt-fixture".to_string()),
            reasoning_effort: Some("high".to_string()),
        };

        let normal = mapper.conversation(&normal);
        let project = mapper.conversation(&project);

        assert_eq!(normal.workspace_root, None);
        assert_eq!(normal.permission_level, PermissionLevel::WorkspaceWrite);
        assert_eq!(project.workspace_root.as_deref(), Some("/work/project"));
        assert_eq!(project.permission_level, PermissionLevel::FullAccess);
        assert_eq!(project.model.as_deref(), Some("gpt-fixture"));
        assert_eq!(project.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn maps_all_native_turn_terminal_states() {
        let mapper = CodexProtocolMapper::default();
        let cases = [
            (CodexTurnStatus::InProgress, TurnTaskStatus::Running),
            (CodexTurnStatus::Completed, TurnTaskStatus::Completed),
            (CodexTurnStatus::Failed, TurnTaskStatus::Failed),
            (CodexTurnStatus::Interrupted, TurnTaskStatus::Interrupted),
        ];
        for (native, standard) in cases {
            let mapped = mapper.turn(
                "thread-one",
                &CodexTurn {
                    id: "turn-one".to_string(),
                    status: native,
                    started_at: Some(10),
                    completed_at: Some(20),
                },
                30_000,
            );
            assert_eq!(mapped.status, standard);
        }
    }

    #[test]
    fn maps_output_delta_without_copying_unknown_payload() {
        let mut mapper = CodexProtocolMapper::default();
        let events = mapper
            .events(
                CodexIncoming::Notification(CodexNotification::OutputDelta {
                    native_method: "item/agentMessage/delta".to_string(),
                    thread_id: "thread-one".to_string(),
                    turn_id: "turn-one".to_string(),
                    item_id: "item-one".to_string(),
                    kind: "assistant-message".to_string(),
                    delta: "hello".to_string(),
                }),
                7,
                1_000,
            )
            .unwrap();
        let ProtocolEvent::TurnOutputDelta { payload, .. } = &events[0] else {
            panic!("expected output delta");
        };
        assert_eq!(payload.delta, "hello");
        let extension = payload.extension.as_ref().unwrap();
        assert_eq!(extension.data.len(), 2);
        assert!(!extension.data.contains_key("raw"));
    }

    #[test]
    fn maps_binary_approval_and_records_lossy_native_decisions() {
        let mut mapper = CodexProtocolMapper::default();
        let request = CodexApprovalRequest {
            request_id: JsonRpcId::String("approval-one".to_string()),
            kind: CodexApprovalKind::CommandExecution,
            thread_id: "thread-one".to_string(),
            turn_id: "turn-one".to_string(),
            item_id: "item-one".to_string(),
            title: "Run command".to_string(),
            description: Some("cargo test".to_string()),
            requested_at_ms: 55,
            available_decisions: vec![
                "accept".to_string(),
                "acceptForSession".to_string(),
                "decline".to_string(),
                "cancel".to_string(),
            ],
        };
        let approval_id = request.approval_id();
        let events = mapper
            .events(CodexIncoming::ApprovalRequested(request), 8, 60)
            .unwrap();
        let ProtocolEvent::ApprovalRequested { payload, .. } = &events[0] else {
            panic!("expected approval request");
        };
        assert_eq!(payload.approval.decisions.len(), 2);
        assert_eq!(
            payload.approval.extension.as_ref().unwrap().data["decisionMapping"],
            json!({ "approve": "accept", "deny": "decline" })
        );
        let resolved = mapper
            .approval_resolved_event(&approval_id, ApprovalDecision::Approve, 9, 70)
            .unwrap();
        let ProtocolEvent::ApprovalResolved { payload, .. } = resolved else {
            panic!("expected approval resolved");
        };
        assert_eq!(payload.approval.status, ApprovalStatus::Approved);
    }

    #[test]
    fn unsupported_server_request_becomes_capability_protocol_error() {
        let mut mapper = CodexProtocolMapper::default();
        let error = mapper
            .events(
                CodexIncoming::UnsupportedServerRequest {
                    request_id: JsonRpcId::Number(77),
                    method: "item/tool/requestUserInput".to_string(),
                },
                10,
                80,
            )
            .unwrap_err();
        assert_eq!(error.code, "capability_unsupported");
        assert!(!error.retryable);
        assert_eq!(
            error.details.unwrap()["nativeMethod"],
            "item/tool/requestUserInput"
        );
    }
}
