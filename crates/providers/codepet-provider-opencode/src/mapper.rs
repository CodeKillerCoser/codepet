use crate::protocol::{
    OpenCodePermissionAskedEventData, OpenCodeServerError, OpenCodeSession,
    OPENCODE_PERMISSION_LEVEL,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ConversationStatus, ConversationUpsertedEvent, InstanceStatus, ProtocolError,
    ProtocolEvent, ProviderApproval, ProviderCapabilities, ProviderCapability,
    ProviderConversation, ProviderInstance, ProviderInstanceRoute, ProviderTurn,
    RoutedResourceId, TurnOutputDeltaEvent, TurnStatus, TurnUpsertedEvent,
};

pub struct OpenCodeProtocolMapper {
    route: ProviderInstanceRoute,
}

impl OpenCodeProtocolMapper {
    pub fn new(route: ProviderInstanceRoute) -> Self {
        Self { route }
    }

    pub fn capabilities() -> ProviderCapabilities {
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
            permission_levels: vec![OPENCODE_PERMISSION_LEVEL.to_string()],
            models: Vec::new(),
            reasoning_efforts: Vec::new(),
            extensions: Vec::new(),
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

    pub fn conversation(
        &self,
        session: &OpenCodeSession,
        server_active: bool,
        active_turn: Option<ProviderTurn>,
        waiting_approval: bool,
    ) -> ProviderConversation {
        let status = if session.time.archived.is_some() {
            ConversationStatus::Archived
        } else if waiting_approval {
            ConversationStatus::WaitingApproval
        } else if active_turn.is_some() || server_active {
            ConversationStatus::Running
        } else {
            ConversationStatus::Idle
        };
        ProviderConversation {
            resource: self.resource(session.id.clone()),
            title: if session.title.trim().is_empty() {
                session.id.clone()
            } else {
                session.title.clone()
            },
            preview: None,
            status,
            permission_level: Some(OPENCODE_PERMISSION_LEVEL.to_string()),
            model: None,
            reasoning_effort: None,
            workspace_root: session.workspace_root(),
            created_at: Some(session.time.created),
            updated_at: Some(session.time.updated),
            active_turn,
            extension: None,
        }
    }

    pub fn turn(
        &self,
        conversation_id: &str,
        turn_id: &str,
        status: TurnStatus,
        started_at: Option<u64>,
        updated_at: Option<u64>,
        completed_at: Option<u64>,
    ) -> ProviderTurn {
        ProviderTurn {
            resource: self.resource(turn_id.to_string()),
            conversation: self.resource(conversation_id.to_string()),
            status,
            display_summary: None,
            started_at,
            updated_at,
            completed_at,
            extension: None,
        }
    }

    pub fn approval(
        &self,
        request: &OpenCodePermissionAskedEventData,
        turn: &ProviderTurn,
        approval_resource_id: String,
        requested_at: u64,
    ) -> ProviderApproval {
        ProviderApproval {
            resource: self.resource(approval_resource_id),
            conversation: self.resource(request.session_id.clone()),
            turn: turn.resource.clone(),
            kind: request.action.clone(),
            title: format!("Allow {}", request.action),
            description: (!request.resources.is_empty()).then(|| request.resources.join("\n")),
            status: ApprovalStatus::Pending,
            decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
            requested_at,
            resolved_at: None,
            decision: None,
            extension: None,
        }
    }

    pub fn conversation_event(&self, conversation: ProviderConversation) -> ProtocolEvent {
        ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".to_string(),
            params: ConversationUpsertedEvent { conversation },
        }
    }

    pub fn turn_event(&self, turn: ProviderTurn) -> ProtocolEvent {
        ProtocolEvent::EventTurnUpserted {
            jsonrpc: "2.0".to_string(),
            params: TurnUpsertedEvent { turn },
        }
    }

    pub fn delta_event(
        &self,
        turn: &ProviderTurn,
        output_id: String,
        kind: &str,
        delta: String,
    ) -> ProtocolEvent {
        ProtocolEvent::EventTurnOutputDelta {
            jsonrpc: "2.0".to_string(),
            params: TurnOutputDeltaEvent {
                turn: turn.resource.clone(),
                conversation: turn.conversation.clone(),
                output_id,
                kind: kind.to_string(),
                delta,
                extension: None,
            },
        }
    }

    pub fn approval_requested_event(&self, approval: ProviderApproval) -> ProtocolEvent {
        ProtocolEvent::EventApprovalRequested {
            jsonrpc: "2.0".to_string(),
            params: ApprovalRequestedEvent { approval },
        }
    }

    pub fn resolve_approval(
        &self,
        mut approval: ProviderApproval,
        decision: ApprovalDecision,
        resolved_at: Option<u64>,
    ) -> (ProviderApproval, ProtocolEvent) {
        approval.status = match decision {
            ApprovalDecision::Approve => ApprovalStatus::Approved,
            ApprovalDecision::Deny => ApprovalStatus::Denied,
        };
        approval.decision = Some(decision);
        approval.resolved_at = resolved_at;
        let event = ProtocolEvent::EventApprovalResolved {
            jsonrpc: "2.0".to_string(),
            params: ApprovalResolvedEvent {
                approval: approval.clone(),
            },
        };
        (approval, event)
    }

    pub fn expire_approval(
        &self,
        mut approval: ProviderApproval,
        resolved_at: Option<u64>,
    ) -> (ProviderApproval, ProtocolEvent) {
        approval.status = ApprovalStatus::Expired;
        approval.resolved_at = resolved_at;
        let event = ProtocolEvent::EventApprovalResolved {
            jsonrpc: "2.0".to_string(),
            params: ApprovalResolvedEvent {
                approval: approval.clone(),
            },
        };
        (approval, event)
    }

    pub fn resource(&self, native_resource_id: String) -> RoutedResourceId {
        RoutedResourceId {
            device_id: self.route.device_id.clone(),
            provider_plugin_id: self.route.provider_plugin_id.clone(),
            provider_instance_id: self.route.provider_instance_id.clone(),
            native_resource_id,
        }
    }

    pub fn error(error: OpenCodeServerError) -> ProtocolError {
        match error {
            OpenCodeServerError::Http { status: 400, message } => {
                protocol_error("opencode_invalid_request", message, false)
            }
            OpenCodeServerError::Http { status: 401 | 403, message } => {
                protocol_error("opencode_unauthorized", message, false)
            }
            OpenCodeServerError::Http { status: 404, message } => {
                protocol_error("opencode_resource_not_found", message, false)
            }
            OpenCodeServerError::Http { status: 409, message } => {
                protocol_error("opencode_conflict", message, false)
            }
            OpenCodeServerError::Http { status, message } => protocol_error(
                "opencode_http_error",
                format!("HTTP {status}: {message}"),
                status >= 500,
            ),
            OpenCodeServerError::Spawn(message) => {
                protocol_error("opencode_spawn_failed", message, true)
            }
            OpenCodeServerError::Timeout(message) => {
                protocol_error("opencode_timeout", message, true)
            }
            OpenCodeServerError::Io(message) => {
                protocol_error("opencode_io_error", message, true)
            }
            OpenCodeServerError::ProcessExited(message) => {
                protocol_error("opencode_process_exited", message, true)
            }
            OpenCodeServerError::Protocol(message) => {
                protocol_error("opencode_protocol_error", message, false)
            }
            OpenCodeServerError::Shutdown => {
                protocol_error("provider_unavailable", "OpenCode Server is shut down".to_string(), true)
            }
        }
    }
}

pub fn protocol_error(code: &str, message: String, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.to_string(),
        message,
        retryable,
        details: None,
    }
}
