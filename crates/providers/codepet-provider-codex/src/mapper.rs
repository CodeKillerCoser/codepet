use crate::protocol::{
    activity_summary_content_id, command_content_id, output_content_id,
    reasoning_summary_content_id, text_content_id, user_input_content_id,
    CodexAppServerError, CodexApprovalRequest, CodexContentKind, CodexConversationSnapshot,
    CodexIncoming, CodexModel, CodexNotification, CodexPermissionLevel, CodexThreadActiveFlag,
    CodexThreadItem, CodexThreadStatus, CodexTurn, CodexTurnStatus, CODEX_EXTENSION_NAMESPACE,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ChoiceOption, ChoiceSet, ConversationContent, ConversationContentKind, ConversationItem,
    ConversationItemKind, ConversationItemRole, ConversationItemStatus, ConversationStatus,
    ConversationUpsertedEvent, FlatModelCatalog, FlatModelCatalogKind, FlatModelSelection,
    HarnessDescriptor, InstanceStatus, JsonObject, ModelCatalog, ModelSelection, ProtocolError,
    ProtocolEvent, ProviderApproval, ProviderCapabilities, ProviderCapability,
    ProviderConversation, ProviderExtension, ProviderInstance, ProviderInstanceRoute,
    ProviderTurn, RoutedResourceId, TurnOutputDeltaEvent, TurnSelection,
    TurnSendCapabilities, TurnStatus, TurnUpsertedEvent,
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

    pub fn unavailable_capabilities(revision: String) -> ProviderCapabilities {
        ProviderCapabilities {
            revision,
            methods: vec![
                ProviderCapability::ConversationList,
                ProviderCapability::ConversationSearch,
                ProviderCapability::ConversationGet,
                ProviderCapability::ConversationCreate,
                ProviderCapability::TurnSteer,
                ProviderCapability::TurnInterrupt,
                ProviderCapability::ApprovalResolve,
            ],
            turn_send: None,
            extensions: vec![extension([
                (
                    "nativeMethods",
                    json!([
                        "thread/list",
                        "thread/read",
                        "thread/resume",
                        "thread/start",
                        "model/list",
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

    pub fn capabilities(
        revision: String,
        models: Vec<CodexModel>,
    ) -> Result<ProviderCapabilities, ProtocolError> {
        let visible_models = models
            .into_iter()
            .filter(|model| !model.hidden)
            .collect::<Vec<_>>();
        if visible_models.is_empty() {
            return Err(protocol_error(
                "capability_discovery_failed",
                "Codex model/list returned no visible models".to_string(),
                true,
            ));
        }
        let mut model_options = Vec::with_capacity(visible_models.len());
        let mut reasoning_options: Option<Vec<ChoiceOption>> = None;
        let mut default_model = None;
        let mut default_reasoning_candidate = None;
        for model in visible_models {
            if model.model.trim().is_empty() || model.display_name.trim().is_empty() {
                return Err(protocol_error(
                    "capability_discovery_failed",
                    "Codex model/list returned an empty model id or display name".to_string(),
                    false,
                ));
            }
            let model_id = model.model.clone();
            if model_options
                .iter()
                .any(|option: &ChoiceOption| option.id == model.model)
            {
                return Err(protocol_error(
                    "capability_discovery_failed",
                    format!("Codex model/list returned duplicate model {}", model.model),
                    false,
                ));
            }
            if model.is_default {
                if default_model.is_some() {
                    return Err(protocol_error(
                        "capability_discovery_failed",
                        "Codex model/list returned multiple default models".to_string(),
                        false,
                    ));
                }
                default_model = Some(FlatModelSelection {
                    kind: FlatModelCatalogKind::Flat,
                    model_id: model.model.clone(),
                });
                default_reasoning_candidate = Some(model.default_reasoning_effort.clone());
            }
            model_options.push(ChoiceOption {
                id: model.model,
                display_name: model.display_name,
                description: (!model.description.trim().is_empty()).then_some(model.description),
                enabled: Some(true),
                disabled_reason: None,
            });
            let mut model_reasoning_options = Vec::new();
            for effort in model.supported_reasoning_efforts {
                if effort.reasoning_effort.trim().is_empty() {
                    return Err(protocol_error(
                        "capability_discovery_failed",
                        "Codex model/list returned an empty reasoning effort".to_string(),
                        false,
                    ));
                }
                if model_reasoning_options
                    .iter()
                    .any(|option: &ChoiceOption| option.id == effort.reasoning_effort)
                {
                    return Err(protocol_error(
                        "capability_discovery_failed",
                        format!(
                            "Codex model/list returned duplicate reasoning effort {} for model {}",
                            effort.reasoning_effort, model_id
                        ),
                        false,
                    ));
                }
                model_reasoning_options.push(ChoiceOption {
                    display_name: choice_display_name(&effort.reasoning_effort),
                    id: effort.reasoning_effort,
                    description: (!effort.description.trim().is_empty())
                        .then_some(effort.description),
                    enabled: Some(true),
                    disabled_reason: None,
                });
            }
            match reasoning_options.as_mut() {
                None => reasoning_options = Some(model_reasoning_options),
                Some(common) => common.retain(|option| {
                    model_reasoning_options
                        .iter()
                        .any(|candidate| candidate.id == option.id)
                }),
            }
        }
        let reasoning_options = reasoning_options.unwrap_or_default();
        let default_reasoning = default_reasoning_candidate.filter(|candidate| {
            reasoning_options
                .iter()
                .any(|option| option.id == *candidate)
        });
        let mut methods = Self::unavailable_capabilities(revision.clone()).methods;
        methods.push(ProviderCapability::TurnStart);
        let reasoning_effort = (!reasoning_options.is_empty()).then(|| ChoiceSet {
            options: reasoning_options,
            default_id: default_reasoning,
        });
        Ok(ProviderCapabilities {
            revision,
            methods,
            turn_send: Some(TurnSendCapabilities {
                access_mode: Some(ChoiceSet {
                    options: vec![
                        choice("read-only", "Read only"),
                        choice("workspace-write", "Workspace write"),
                        choice("full-access", "Full access"),
                    ],
                    default_id: None,
                }),
                reasoning_effort,
                model_catalog: Some(ModelCatalog::FlatModelCatalog(FlatModelCatalog {
                    kind: FlatModelCatalogKind::Flat,
                    models: model_options,
                    default_selection: default_model,
                })),
            }),
            extensions: Self::unavailable_capabilities("unused".to_string()).extensions,
        })
    }

    pub fn instance(
        &self,
        plugin_id: String,
        instance_kind: String,
        display_name: String,
        harness: HarnessDescriptor,
        status: InstanceStatus,
        capabilities: ProviderCapabilities,
    ) -> ProviderInstance {
        ProviderInstance {
            route: self.route.clone(),
            plugin_id,
            instance_kind,
            display_name,
            harness,
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
            selection: snapshot_selection(snapshot),
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

    pub fn turn_user_item(
        &self,
        conversation_id: &str,
        turn: &CodexTurn,
    ) -> Option<ConversationItem> {
        let conversation = self.resource(conversation_id.to_string());
        turn.items
            .iter()
            .find(|item| matches!(item, CodexThreadItem::UserMessage { .. }))
            .map(|item| self.conversation_item(turn, item, &conversation))
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

    pub fn conversation_items(
        &self,
        snapshot: &CodexConversationSnapshot,
        approvals: &[(String, ProviderApproval)],
    ) -> Vec<ConversationItem> {
        let mut items = Vec::new();
        let conversation = self.resource(snapshot.thread.id.clone());
        let mut emitted_approvals = vec![false; approvals.len()];
        for turn in &snapshot.thread.turns {
            for item in &turn.items {
                items.push(self.conversation_item(turn, item, &conversation));
                for (index, (related_item_id, approval)) in approvals.iter().enumerate() {
                    if !emitted_approvals[index]
                        && approval.turn.native_resource_id == turn.id
                        && related_item_id == item.id()
                    {
                        items.push(self.approval_item(related_item_id, approval));
                        emitted_approvals[index] = true;
                    }
                }
            }
            for (index, (related_item_id, approval)) in approvals.iter().enumerate() {
                if !emitted_approvals[index] && approval.turn.native_resource_id == turn.id {
                    items.push(self.approval_item(related_item_id, approval));
                    emitted_approvals[index] = true;
                }
            }
        }
        for (index, (related_item_id, approval)) in approvals.iter().enumerate() {
            if !emitted_approvals[index] {
                items.push(self.approval_item(related_item_id, approval));
            }
        }
        items
    }

    fn conversation_item(
        &self,
        turn: &CodexTurn,
        item: &CodexThreadItem,
        conversation: &RoutedResourceId,
    ) -> ConversationItem {
        let resource = self.resource(item.id().to_string());
        let turn_resource = self.resource(turn.id.clone());
        let mutable_text = turn.status == CodexTurnStatus::InProgress;
        match item {
            CodexThreadItem::UserMessage { id, text_inputs } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::Message,
                status: ConversationItemStatus::Completed,
                role: Some(ConversationItemRole::User),
                title: None,
                contents: text_inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, text)| {
                        text.as_ref().map(|text| ConversationContent {
                            content_id: user_input_content_id(id, index),
                            kind: ConversationContentKind::Text,
                            text: text.clone(),
                        })
                    })
                    .collect(),
                related_item: None,
                approval: None,
            },
            CodexThreadItem::AgentMessage { id, text } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::Message,
                status: mutable_content_status(turn.status),
                role: Some(ConversationItemRole::Assistant),
                title: None,
                contents: (!mutable_text)
                    .then(|| ConversationContent {
                        content_id: text_content_id(id),
                        kind: ConversationContentKind::Text,
                        text: text.clone(),
                    })
                    .into_iter()
                    .collect(),
                related_item: None,
                approval: None,
            },
            CodexThreadItem::Plan { id, text } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::Message,
                status: mutable_content_status(turn.status),
                role: Some(ConversationItemRole::Assistant),
                title: Some("Plan".to_string()),
                contents: (!mutable_text)
                    .then(|| ConversationContent {
                        content_id: text_content_id(id),
                        kind: ConversationContentKind::Text,
                        text: text.clone(),
                    })
                    .into_iter()
                    .collect(),
                related_item: None,
                approval: None,
            },
            CodexThreadItem::Reasoning { id, summary } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::Reasoning,
                status: mutable_content_status(turn.status),
                role: Some(ConversationItemRole::Assistant),
                title: None,
                contents: if mutable_text {
                    Vec::new()
                } else {
                    summary
                        .iter()
                        .enumerate()
                        .map(|(index, text)| ConversationContent {
                            content_id: reasoning_summary_content_id(id, index),
                            kind: ConversationContentKind::ReasoningSummary,
                            text: text.clone(),
                        })
                        .collect()
                },
                related_item: None,
                approval: None,
            },
            CodexThreadItem::CommandExecution {
                id,
                command,
                status,
                aggregated_output,
            } => {
                let status = activity_status(status);
                let mut contents = vec![ConversationContent {
                    content_id: command_content_id(id),
                    kind: ConversationContentKind::Command,
                    text: command.clone(),
                }];
                if status != ConversationItemStatus::Running {
                    if let Some(output) = aggregated_output {
                        contents.push(ConversationContent {
                            content_id: output_content_id(id),
                            kind: ConversationContentKind::Output,
                            text: output.clone(),
                        });
                    }
                }
                ConversationItem {
                    resource,
                    turn: turn_resource,
                    conversation: conversation.clone(),
                    kind: ConversationItemKind::Command,
                    status,
                    role: None,
                    title: Some("Command".to_string()),
                    contents,
                    related_item: None,
                    approval: None,
                }
            }
            CodexThreadItem::FileChange {
                id,
                status,
                change_count,
            } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::FileChange,
                status: activity_status(status),
                role: None,
                title: Some("File changes".to_string()),
                contents: vec![ConversationContent {
                    content_id: activity_summary_content_id(id),
                    kind: ConversationContentKind::ActivitySummary,
                    text: format!("{change_count} file change(s)"),
                }],
                related_item: None,
                approval: None,
            },
            CodexThreadItem::ToolActivity { title, status, .. } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::Tool,
                status: status
                    .as_deref()
                    .map(activity_status)
                    .unwrap_or_else(|| item_status_from_turn(turn.status)),
                role: None,
                title: Some(title.clone()),
                contents: Vec::new(),
                related_item: None,
                approval: None,
            },
            CodexThreadItem::Unknown { .. } => ConversationItem {
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ConversationItemKind::Unknown,
                status: item_status_from_turn(turn.status),
                role: None,
                title: Some("Unknown Codex activity".to_string()),
                contents: Vec::new(),
                related_item: None,
                approval: None,
            },
        }
    }

    fn approval_item(&self, related_item_id: &str, approval: &ProviderApproval) -> ConversationItem {
        ConversationItem {
            resource: approval.resource.clone(),
            turn: approval.turn.clone(),
            conversation: approval.conversation.clone(),
            kind: ConversationItemKind::Approval,
            status: approval_item_status(approval.status),
            role: None,
            title: Some(approval.title.clone()),
            contents: Vec::new(),
            related_item: Some(self.resource(related_item_id.to_string())),
            approval: Some(approval.clone()),
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
            CodexAppServerError::Rpc { code, message, .. } => ProtocolError {
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
            CodexNotification::ThreadStarted { snapshot } => {
                ProtocolEvent::EventConversationUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: ConversationUpsertedEvent {
                        conversation: self.conversation(&snapshot),
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
                content_id,
                kind,
                delta,
            } => ProtocolEvent::EventTurnOutputDelta {
                jsonrpc: "2.0".to_string(),
                params: TurnOutputDeltaEvent {
                    turn: self.resource(turn_id),
                    conversation: self.resource(thread_id),
                    item_id,
                    content_id,
                    kind: content_kind(kind),
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
            requested_at: Some(request.requested_at_ms),
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

fn content_kind(kind: CodexContentKind) -> ConversationContentKind {
    match kind {
        CodexContentKind::Text => ConversationContentKind::Text,
        CodexContentKind::ReasoningSummary => ConversationContentKind::ReasoningSummary,
        CodexContentKind::Output => ConversationContentKind::Output,
    }
}

fn mutable_content_status(status: CodexTurnStatus) -> ConversationItemStatus {
    match status {
        CodexTurnStatus::InProgress => ConversationItemStatus::Running,
        _ => ConversationItemStatus::Completed,
    }
}

fn item_status_from_turn(status: CodexTurnStatus) -> ConversationItemStatus {
    match status {
        CodexTurnStatus::InProgress => ConversationItemStatus::Running,
        CodexTurnStatus::Completed => ConversationItemStatus::Completed,
        CodexTurnStatus::Failed => ConversationItemStatus::Failed,
        CodexTurnStatus::Interrupted => ConversationItemStatus::Interrupted,
    }
}

fn activity_status(status: &str) -> ConversationItemStatus {
    match status {
        "pending" => ConversationItemStatus::Pending,
        "inProgress" | "running" => ConversationItemStatus::Running,
        "completed" => ConversationItemStatus::Completed,
        "failed" => ConversationItemStatus::Failed,
        "interrupted" | "cancelled" | "canceled" => ConversationItemStatus::Interrupted,
        "declined" => ConversationItemStatus::Declined,
        _ => ConversationItemStatus::Unknown,
    }
}

fn approval_item_status(status: ApprovalStatus) -> ConversationItemStatus {
    match status {
        ApprovalStatus::Pending => ConversationItemStatus::Pending,
        ApprovalStatus::Approved => ConversationItemStatus::Approved,
        ApprovalStatus::Denied => ConversationItemStatus::Denied,
        ApprovalStatus::Expired => ConversationItemStatus::Expired,
    }
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

fn snapshot_selection(snapshot: &CodexConversationSnapshot) -> Option<TurnSelection> {
    let selection = TurnSelection {
        access_mode_id: snapshot
            .permission_level
            .map(permission_level_name)
            .map(str::to_string),
        reasoning_effort_id: snapshot.reasoning_effort.clone(),
        model: snapshot.model.clone().map(|model_id| {
            ModelSelection::FlatModelSelection(FlatModelSelection {
                kind: FlatModelCatalogKind::Flat,
                model_id,
            })
        }),
    };
    (selection.access_mode_id.is_some()
        || selection.reasoning_effort_id.is_some()
        || selection.model.is_some())
        .then_some(selection)
}

fn choice(id: &str, display_name: &str) -> ChoiceOption {
    ChoiceOption {
        id: id.to_string(),
        display_name: display_name.to_string(),
        description: None,
        enabled: Some(true),
        disabled_reason: None,
    }
}

fn choice_display_name(id: &str) -> String {
    let mut words = id
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(first) = words.first_mut() {
        if let Some(initial) = first.get_mut(0..1) {
            initial.make_ascii_uppercase();
        }
    }
    words.join(" ")
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
    use std::fs;

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
            items_view: crate::protocol::CodexTurnItemsView::Full,
            items: Vec::new(),
        };

        let mapped = mapper.turn("thread-test", &turn);

        assert_eq!(mapped.started_at, Some(1_000));
        assert_eq!(mapped.completed_at, Some(2_000));
        assert_eq!(mapped.updated_at, None);
    }

    #[test]
    fn reasoning_control_is_the_intersection_of_visible_model_efforts() {
        let capabilities = CodexProtocolMapper::capabilities(
            "revision-test".to_string(),
            vec![
                test_model("model-a", true, "high", &["low", "high"]),
                test_model("model-b", false, "medium", &["high", "medium"]),
            ],
        )
        .unwrap();
        let reasoning = capabilities
            .turn_send
            .as_ref()
            .and_then(|turn_send| turn_send.reasoning_effort.as_ref())
            .unwrap();

        assert_eq!(
            reasoning
                .options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            vec!["high"]
        );
        assert_eq!(reasoning.default_id.as_deref(), Some("high"));
    }

    #[test]
    fn reasoning_control_is_omitted_when_models_have_no_common_effort() {
        let capabilities = CodexProtocolMapper::capabilities(
            "revision-test".to_string(),
            vec![
                test_model("model-a", true, "low", &["low"]),
                test_model("model-b", false, "high", &["high"]),
            ],
        )
        .unwrap();

        assert!(capabilities
            .turn_send
            .as_ref()
            .unwrap()
            .reasoning_effort
            .is_none());
    }

    #[test]
    fn timestamp_overflow_is_unknown_instead_of_saturating() {
        assert_eq!(seconds_to_ms(i64::MAX), None);
        assert_eq!(seconds_to_ms(-1), None);
        assert_eq!(seconds_to_ms(42), Some(42_000));
    }

    fn test_model(
        model: &str,
        is_default: bool,
        default_reasoning_effort: &str,
        reasoning_efforts: &[&str],
    ) -> CodexModel {
        CodexModel {
            id: format!("record-{model}"),
            model: model.to_string(),
            display_name: model.to_string(),
            description: String::new(),
            hidden: false,
            is_default,
            default_reasoning_effort: default_reasoning_effort.to_string(),
            supported_reasoning_efforts: reasoning_efforts
                .iter()
                .map(|effort| crate::protocol::CodexReasoningEffortOption {
                    reasoning_effort: (*effort).to_string(),
                    description: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn completed_history_preserves_item_order_and_stable_content_ids() {
        let mapper = test_mapper();
        let snapshot = history_snapshot(
            CodexTurnStatus::Completed,
            vec![
                CodexThreadItem::UserMessage {
                    id: "user-one".to_string(),
                    text_inputs: vec![Some("hello".to_string()), None],
                },
                CodexThreadItem::AgentMessage {
                    id: "agent-one".to_string(),
                    text: "answer".to_string(),
                },
                CodexThreadItem::Reasoning {
                    id: "reasoning-one".to_string(),
                    summary: vec!["summary".to_string()],
                },
                CodexThreadItem::CommandExecution {
                    id: "command-one".to_string(),
                    command: "cargo test".to_string(),
                    status: "completed".to_string(),
                    aggregated_output: Some("ok".to_string()),
                },
                CodexThreadItem::FileChange {
                    id: "file-one".to_string(),
                    status: "completed".to_string(),
                    change_count: 2,
                },
                CodexThreadItem::ToolActivity {
                    id: "tool-one".to_string(),
                    title: "server/tool".to_string(),
                    status: Some("completed".to_string()),
                },
                CodexThreadItem::Unknown {
                    id: "unknown-one".to_string(),
                },
            ],
        );

        let items = mapper.conversation_items(&snapshot, &[]);

        assert_eq!(
            items
                .iter()
                .map(|item| item.resource.native_resource_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "user-one",
                "agent-one",
                "reasoning-one",
                "command-one",
                "file-one",
                "tool-one",
                "unknown-one"
            ]
        );
        assert_eq!(items[0].contents[0].content_id, "user-one:input:0");
        assert_eq!(items[1].contents[0].content_id, "agent-one:text");
        assert_eq!(
            items[2].contents[0].content_id,
            "reasoning-one:summary:0"
        );
        assert_eq!(items[3].contents[0].content_id, "command-one:command");
        assert_eq!(items[3].contents[1].content_id, "command-one:output");
        assert_eq!(items[6].kind, ConversationItemKind::Unknown);
        assert!(items[6].contents.is_empty());
        assert!(items.iter().all(|item| {
            item.conversation.native_resource_id == "thread-history"
        }));
    }

    #[test]
    fn in_progress_snapshot_omits_mutable_assistant_reasoning_and_output_bodies() {
        let mapper = test_mapper();
        let snapshot = history_snapshot(
            CodexTurnStatus::InProgress,
            vec![
                CodexThreadItem::AgentMessage {
                    id: "agent-live".to_string(),
                    text: "partial answer".to_string(),
                },
                CodexThreadItem::Reasoning {
                    id: "reasoning-live".to_string(),
                    summary: vec!["partial summary".to_string()],
                },
                CodexThreadItem::CommandExecution {
                    id: "command-live".to_string(),
                    command: "cargo test".to_string(),
                    status: "inProgress".to_string(),
                    aggregated_output: Some("partial output".to_string()),
                },
            ],
        );

        let items = mapper.conversation_items(&snapshot, &[]);

        assert!(items[0].contents.is_empty());
        assert!(items[1].contents.is_empty());
        assert_eq!(items[2].contents.len(), 1);
        assert_eq!(items[2].contents[0].kind, ConversationContentKind::Command);
        assert_eq!(items[2].status, ConversationItemStatus::Running);
        let committed_content_ids = items
            .iter()
            .flat_map(|item| item.contents.iter())
            .map(|content| content.content_id.as_str())
            .collect::<Vec<_>>();
        assert!(!committed_content_ids.contains(&"agent-live:text"));
        assert!(!committed_content_ids.contains(&"reasoning-live:summary:0"));
        assert!(!committed_content_ids.contains(&"command-live:output"));
    }

    #[test]
    fn observed_approval_is_inserted_after_its_related_command() {
        let mapper = test_mapper();
        let snapshot = history_snapshot(
            CodexTurnStatus::Completed,
            vec![CodexThreadItem::CommandExecution {
                id: "command-one".to_string(),
                command: "cargo test".to_string(),
                status: "completed".to_string(),
                aggregated_output: None,
            }],
        );
        let request = CodexApprovalRequest {
            request_id: crate::protocol::JsonRpcId::String("approval-one".to_string()),
            session_generation: "generation-one".to_string(),
            kind: crate::protocol::CodexApprovalKind::CommandExecution,
            thread_id: "thread-history".to_string(),
            turn_id: "turn-history".to_string(),
            item_id: "command-one".to_string(),
            title: "Run command".to_string(),
            description: None,
            requested_at_ms: 10,
            available_decisions: vec!["accept".to_string(), "decline".to_string()],
        };
        let approval = mapper.approval(&request);

        let items = mapper.conversation_items(
            &snapshot,
            &[("command-one".to_string(), approval.clone())],
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[1].kind, ConversationItemKind::Approval);
        assert_eq!(items[1].approval.as_ref(), Some(&approval));
        assert_eq!(
            items[1]
                .related_item
                .as_ref()
                .unwrap()
                .native_resource_id,
            "command-one"
        );
    }

    fn test_mapper() -> CodexProtocolMapper {
        CodexProtocolMapper::new(ProviderInstanceRoute {
            device_id: "device-test".to_string(),
            provider_plugin_id: "dev.codepet.codex".to_string(),
            provider_instance_id: "codex".to_string(),
        })
    }

    fn history_snapshot(
        status: CodexTurnStatus,
        items: Vec<CodexThreadItem>,
    ) -> CodexConversationSnapshot {
        let mut snapshot = CodexConversationSnapshot::from_thread(CodexThread {
            id: "thread-history".to_string(),
            name: None,
            preview: "history".to_string(),
            cwd: "/fixture".to_string(),
            created_at: 1,
            updated_at: 2,
            status: if status == CodexTurnStatus::InProgress {
                CodexThreadStatus::Active {
                    active_flags: Vec::new(),
                }
            } else {
                CodexThreadStatus::Idle
            },
            turns: vec![CodexTurn {
                id: "turn-history".to_string(),
                status,
                started_at: Some(1),
                completed_at: (status != CodexTurnStatus::InProgress).then_some(2),
                items_view: crate::protocol::CodexTurnItemsView::Full,
                items,
            }],
            cli_version: "0.151.0".to_string(),
            ephemeral: false,
            model_provider: "openai".to_string(),
            project_id: Value::Null,
            session_id: "session-history".to_string(),
            source: json!("appServer"),
        });
        snapshot.workspace_root = Some("/fixture".to_string());
        snapshot
    }

    #[test]
    fn workspace_projection_preserves_resource_identity_and_native_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("nested")).unwrap();
        let native_cwd = project.join("nested").join("..");
        let native_cwd = native_cwd.to_string_lossy().into_owned();
        let snapshot = CodexConversationSnapshot::from_thread(CodexThread {
            id: "thread-stable".to_string(),
            name: None,
            preview: "fixture".to_string(),
            cwd: native_cwd.clone(),
            created_at: 1,
            updated_at: 2,
            status: CodexThreadStatus::Idle,
            turns: Vec::new(),
            cli_version: "0.151.0".to_string(),
            ephemeral: false,
            model_provider: "openai".to_string(),
            project_id: Value::Null,
            session_id: "session-stable".to_string(),
            source: json!("appServer"),
        });
        let mapper = CodexProtocolMapper::new(ProviderInstanceRoute {
            device_id: "device-stable".to_string(),
            provider_plugin_id: "dev.codepet.codex".to_string(),
            provider_instance_id: "codex-stable".to_string(),
        });

        let conversation = mapper.conversation(&snapshot);

        assert_eq!(
            conversation.workspace_root,
            Some(project.to_string_lossy().into_owned())
        );
        assert_eq!(conversation.resource.native_resource_id, "thread-stable");
        assert_eq!(conversation.resource.device_id, "device-stable");
        assert_eq!(
            conversation.resource.provider_plugin_id,
            "dev.codepet.codex"
        );
        assert_eq!(
            conversation.resource.provider_instance_id,
            "codex-stable"
        );
        assert_eq!(
            conversation.extension.unwrap().data["nativeCwd"],
            json!(native_cwd)
        );
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
