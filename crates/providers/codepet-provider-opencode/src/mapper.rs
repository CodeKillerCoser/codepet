use crate::protocol::{
    OpenCodeAgent, OpenCodeMessage, OpenCodeMessageContent, OpenCodeModel,
    OpenCodePermissionAskedEventData, OpenCodeProvider, OpenCodeServerError, OpenCodeSession,
    OPENCODE_PERMISSION_LEVEL,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ChoiceOption, ChoiceSet, ConversationContent, ConversationContentKind, ConversationItem, ConversationItemKind,
    ConversationItemRole, ConversationItemStatus, ConversationStatus, HarnessDescriptor,
    GroupedModelCatalog, GroupedModelCatalogKind, GroupedModelProvider, GroupedModelSelection,
    InstanceStatus, ModelCatalog, ModelSelection, ProtocolError, ProtocolEvent, ProviderApproval, ProviderCapabilities,
    ProviderCapability, ProviderConversation, ProviderInstance, ProviderInstanceRoute,
    ProviderTurn, RoutedResourceId, TurnOutputDeltaEvent, TurnSelection, TurnSendCapabilities, TurnStatus,
    TurnUpsertedEvent,
};
use std::collections::{BTreeMap, HashMap};

pub struct OpenCodeProtocolMapper {
    route: ProviderInstanceRoute,
}

impl OpenCodeProtocolMapper {
    pub fn new(route: ProviderInstanceRoute) -> Self {
        Self { route }
    }

    pub fn base_capabilities() -> ProviderCapabilities {
        ProviderCapabilities {
            revision: "opencode-server-1.18.25-controls-v1".to_string(),
            methods: vec![
                ProviderCapability::ConversationList,
                ProviderCapability::ConversationGet,
                ProviderCapability::ConversationCreate,
                ProviderCapability::TurnStart,
                ProviderCapability::TurnSteer,
                ProviderCapability::TurnInterrupt,
                ProviderCapability::ApprovalResolve,
            ],
            turn_send: Some(TurnSendCapabilities {
                access_mode: None,
                reasoning_effort: None,
                model_catalog: None,
            }),
            extensions: Vec::new(),
        }
    }

    pub fn capabilities(
        agents: &[OpenCodeAgent],
        models: &[OpenCodeModel],
        providers: &[OpenCodeProvider],
    ) -> Result<ProviderCapabilities, ProtocolError> {
        let mut access_options = agents
            .iter()
            .filter(|agent| agent.mode == "primary" && !agent.hidden)
            .map(|agent| ChoiceOption {
                id: agent.id.clone(),
                display_name: choice_display_name(&agent.id),
                description: agent.description.clone(),
                enabled: Some(true),
                disabled_reason: None,
            })
            .collect::<Vec<_>>();
        if access_options.is_empty() {
            access_options = vec![
                ChoiceOption {
                    id: "build".to_string(),
                    display_name: "Build".to_string(),
                    description: Some("OpenCode's default tool-enabled agent.".to_string()),
                    enabled: Some(true),
                    disabled_reason: None,
                },
                ChoiceOption {
                    id: "plan".to_string(),
                    display_name: "Plan".to_string(),
                    description: Some("OpenCode's read-only planning agent.".to_string()),
                    enabled: Some(true),
                    disabled_reason: None,
                },
            ];
        }
        let mut active_provider_ids = providers
            .iter()
            .filter(|provider| provider.disabled != Some(true))
            .map(|provider| provider.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        if active_provider_ids.is_empty()
            && models.iter().any(|model| model.provider_id == "opencode")
        {
            active_provider_ids.insert("opencode");
        }
        let visible_models = models
            .iter()
            .filter(|model| {
                model.enabled
                    && model.status != "deprecated"
                    && active_provider_ids.contains(model.provider_id.as_str())
            })
            .collect::<Vec<_>>();
        if visible_models.is_empty() {
            return Err(protocol_error(
                "capability_discovery_failed",
                format!(
                    "OpenCode returned no enabled models ({} models, {} providers, active providers: {})",
                    models.len(),
                    providers.len(),
                    active_provider_ids.iter().copied().collect::<Vec<_>>().join(",")
                ),
                true,
            ));
        }
        let provider_names = providers
            .iter()
            .filter(|provider| provider.disabled != Some(true))
            .map(|provider| (provider.id.as_str(), provider.name.as_str()))
            .collect::<HashMap<_, _>>();
        let mut grouped = BTreeMap::<String, Vec<ChoiceOption>>::new();
        let mut common_variants: Option<Vec<String>> = None;
        for model in &visible_models {
            grouped
                .entry(model.provider_id.clone())
                .or_default()
                .push(ChoiceOption {
                    id: model.id.clone(),
                    display_name: model.name.clone(),
                    description: None,
                    enabled: Some(true),
                    disabled_reason: None,
                });
            let mut variants = vec!["default".to_string()];
            for variant in &model.variants {
                if !variant.id.trim().is_empty() && !variants.contains(&variant.id) {
                    variants.push(variant.id.clone());
                }
            }
            match common_variants.as_mut() {
                None => common_variants = Some(variants),
                Some(common) => common.retain(|variant| variants.contains(variant)),
            }
        }
        let default_model = visible_models[0];
        let model_providers = grouped
            .into_iter()
            .map(|(provider_id, models)| GroupedModelProvider {
                display_name: provider_names
                    .get(provider_id.as_str())
                    .copied()
                    .unwrap_or(provider_id.as_str())
                    .to_string(),
                id: provider_id,
                description: None,
                models,
            })
            .collect();
        let variants = common_variants.unwrap_or_else(|| vec!["default".to_string()]);
        let mut capabilities = Self::base_capabilities();
        capabilities.turn_send = Some(TurnSendCapabilities {
            access_mode: Some(ChoiceSet {
                default_id: access_options
                    .iter()
                    .any(|option| option.id == "build")
                    .then(|| "build".to_string())
                    .or_else(|| access_options.first().map(|option| option.id.clone())),
                options: access_options,
            }),
            reasoning_effort: Some(ChoiceSet {
                options: variants
                    .into_iter()
                    .map(|variant| ChoiceOption {
                        display_name: choice_display_name(&variant),
                        id: variant,
                        description: None,
                        enabled: Some(true),
                        disabled_reason: None,
                    })
                    .collect(),
                default_id: Some("default".to_string()),
            }),
            model_catalog: Some(ModelCatalog::GroupedModelCatalog(GroupedModelCatalog {
                kind: GroupedModelCatalogKind::Grouped,
                providers: model_providers,
                default_selection: Some(GroupedModelSelection {
                    kind: GroupedModelCatalogKind::Grouped,
                    provider_id: default_model.provider_id.clone(),
                    model_id: default_model.id.clone(),
                }),
            })),
        });
        Ok(capabilities)
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
            permission_level: Some(session.agent.clone().unwrap_or_else(|| OPENCODE_PERMISSION_LEVEL.to_string())),
            model: session.model.as_ref().map(|model| format!("{}/{}", model.provider_id, model.id)),
            reasoning_effort: session.model.as_ref().and_then(|model| model.variant.clone()),
            selection: session_selection(session),
            workspace_root: Some(session.workspace_root()),
            created_at: Some(session.time.created),
            updated_at: Some(session.time.updated),
            active_turn,
            extension: None,
        }
    }

    pub fn user_message_item(
        &self,
        conversation: &RoutedResourceId,
        turn: &ProviderTurn,
        item_id: String,
        text: String,
    ) -> ConversationItem {
        ConversationItem {
            resource: self.resource(item_id.clone()),
            turn: turn.resource.clone(),
            conversation: conversation.clone(),
            kind: ConversationItemKind::Message,
            status: ConversationItemStatus::Completed,
            role: Some(ConversationItemRole::User),
            title: None,
            contents: vec![ConversationContent {
                content_id: format!("{item_id}:text"),
                kind: ConversationContentKind::Text,
                text,
            }],
            related_item: None,
            approval: None,
        }
    }

    pub fn conversation_items(
        &self,
        conversation: &RoutedResourceId,
        messages: &[OpenCodeMessage],
    ) -> Vec<ConversationItem> {
        let mut items = Vec::new();
        let mut turn = self.resource("history-turn:initial".to_string());
        let mut text_budget = 12 * 1024 * 1024;
        for message in messages {
            if message.kind == "user" {
                turn = self.resource(format!("history-turn:{}", message.id));
            }
            match message.kind.as_str() {
                "user" => items.push(self.history_item(
                    message.id.clone(),
                    turn.clone(),
                    conversation,
                    ConversationItemKind::Message,
                    ConversationItemStatus::Completed,
                    Some(ConversationItemRole::User),
                    None,
                    message.text.as_deref().map(|text| ConversationContent {
                        content_id: format!("{}:text", message.id),
                        kind: ConversationContentKind::Text,
                        text: bounded_history_text(text, &mut text_budget),
                    }).into_iter().collect(),
                )),
                "assistant" => {
                    let status = assistant_status(message);
                    for content in message.content.as_deref().unwrap_or_default() {
                        if let Some(item) = self.assistant_content_item(
                            content,
                            &turn,
                            conversation,
                            status,
                            &mut text_budget,
                        ) {
                            items.push(item);
                        }
                    }
                    if message.content.as_ref().is_none_or(Vec::is_empty) {
                        let contents = message.error.as_ref().map(|error| ConversationContent {
                            content_id: format!("{}:error", message.id),
                            kind: ConversationContentKind::ActivitySummary,
                            text: bounded_history_text(&error.to_string(), &mut text_budget),
                        }).into_iter().collect();
                        items.push(self.history_item(
                            message.id.clone(),
                            turn.clone(),
                            conversation,
                            ConversationItemKind::Message,
                            status,
                            Some(ConversationItemRole::Assistant),
                            None,
                            contents,
                        ));
                    }
                }
                "shell" => {
                    let mut contents = Vec::new();
                    if let Some(command) = message.command.as_deref() {
                        contents.push(ConversationContent {
                            content_id: format!("{}:command", message.id),
                            kind: ConversationContentKind::Command,
                            text: bounded_history_text(command, &mut text_budget),
                        });
                    }
                    if let Some(output) = message.output.as_deref() {
                        contents.push(ConversationContent {
                            content_id: format!("{}:output", message.id),
                            kind: ConversationContentKind::Output,
                            text: bounded_history_text(output, &mut text_budget),
                        });
                    }
                    items.push(self.history_item(
                        message.id.clone(),
                        turn.clone(),
                        conversation,
                        ConversationItemKind::Command,
                        terminal_message_status(message),
                        None,
                        Some("Shell".to_string()),
                        contents,
                    ));
                }
                "system" | "synthetic" => items.push(self.history_item(
                    message.id.clone(),
                    turn.clone(),
                    conversation,
                    ConversationItemKind::Message,
                    ConversationItemStatus::Completed,
                    None,
                    Some(if message.kind == "system" { "System" } else { "Synthetic" }.to_string()),
                    message.text.as_deref().map(|text| ConversationContent {
                        content_id: format!("{}:text", message.id),
                        kind: ConversationContentKind::Text,
                        text: bounded_history_text(text, &mut text_budget),
                    }).into_iter().collect(),
                )),
                "compaction" => {
                    let contents = [
                        ("summary", message.summary.as_deref()),
                        ("recent", message.recent.as_deref()),
                    ]
                    .into_iter()
                    .filter_map(|(name, text)| text.map(|text| ConversationContent {
                        content_id: format!("{}:{name}", message.id),
                        kind: ConversationContentKind::ActivitySummary,
                        text: bounded_history_text(text, &mut text_budget),
                    }))
                    .collect();
                    items.push(self.history_item(
                        message.id.clone(),
                        turn.clone(),
                        conversation,
                        ConversationItemKind::Unknown,
                        ConversationItemStatus::Completed,
                        None,
                        Some("Compaction".to_string()),
                        contents,
                    ));
                }
                _ => items.push(self.history_item(
                    message.id.clone(),
                    turn.clone(),
                    conversation,
                    ConversationItemKind::Unknown,
                    terminal_message_status(message),
                    None,
                    Some(format!("OpenCode {}", message.kind)),
                    Vec::new(),
                )),
            }
        }
        items
    }

    fn assistant_content_item(
        &self,
        content: &OpenCodeMessageContent,
        turn: &RoutedResourceId,
        conversation: &RoutedResourceId,
        message_status: ConversationItemStatus,
        text_budget: &mut usize,
    ) -> Option<ConversationItem> {
        let resource_id = format!("{}:{}", turn.native_resource_id, content.id);
        match content.kind.as_str() {
            "text" => Some(self.history_item(
                resource_id,
                turn.clone(),
                conversation,
                ConversationItemKind::Message,
                message_status,
                Some(ConversationItemRole::Assistant),
                None,
                content.text.as_deref().map(|text| ConversationContent {
                    content_id: format!("{}:text", content.id),
                    kind: ConversationContentKind::Text,
                    text: bounded_history_text(text, text_budget),
                }).into_iter().collect(),
            )),
            "reasoning" => Some(self.history_item(
                resource_id,
                turn.clone(),
                conversation,
                ConversationItemKind::Reasoning,
                message_status,
                Some(ConversationItemRole::Assistant),
                None,
                content.text.as_deref().map(|text| ConversationContent {
                    content_id: format!("{}:summary:0", content.id),
                    kind: ConversationContentKind::ReasoningSummary,
                    text: bounded_history_text(text, text_budget),
                }).into_iter().collect(),
            )),
            "tool" => {
                let status = tool_status(content.state.as_ref());
                let summary = content.state.as_ref().map(|state| {
                    bounded_history_text(&state.to_string(), text_budget)
                });
                Some(self.history_item(
                    resource_id,
                    turn.clone(),
                    conversation,
                    ConversationItemKind::Tool,
                    status,
                    None,
                    content.name.clone().or_else(|| Some("Tool".to_string())),
                    summary.map(|text| ConversationContent {
                        content_id: format!("{}:activity", content.id),
                        kind: ConversationContentKind::ActivitySummary,
                        text,
                    }).into_iter().collect(),
                ))
            }
            _ => Some(self.history_item(
                resource_id,
                turn.clone(),
                conversation,
                ConversationItemKind::Unknown,
                message_status,
                None,
                Some(format!("OpenCode {}", content.kind)),
                Vec::new(),
            )),
        }
    }

    fn history_item(
        &self,
        resource_id: String,
        turn: RoutedResourceId,
        conversation: &RoutedResourceId,
        kind: ConversationItemKind,
        status: ConversationItemStatus,
        role: Option<ConversationItemRole>,
        title: Option<String>,
        contents: Vec<ConversationContent>,
    ) -> ConversationItem {
        ConversationItem {
            resource: self.resource(resource_id),
            turn,
            conversation: conversation.clone(),
            kind,
            status,
            role,
            title,
            contents,
            related_item: None,
            approval: None,
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
            requested_at: None,
            resolved_at: None,
            decision: None,
            extension: None,
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
        item_id: String,
        content_id: String,
        kind: ConversationContentKind,
        delta: String,
    ) -> ProtocolEvent {
        ProtocolEvent::EventTurnOutputDelta {
            jsonrpc: "2.0".to_string(),
            params: TurnOutputDeltaEvent {
                turn: turn.resource.clone(),
                conversation: turn.conversation.clone(),
                item_id,
                content_id,
                kind,
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

fn session_selection(session: &OpenCodeSession) -> Option<TurnSelection> {
    let model = session.model.as_ref()?;
    Some(TurnSelection {
        access_mode_id: session.agent.clone(),
        reasoning_effort_id: Some(
            model
                .variant
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ),
        model: Some(ModelSelection::GroupedModelSelection(
            GroupedModelSelection {
                kind: GroupedModelCatalogKind::Grouped,
                provider_id: model.provider_id.clone(),
                model_id: model.id.clone(),
            },
        )),
    })
}

fn choice_display_name(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn assistant_status(message: &OpenCodeMessage) -> ConversationItemStatus {
    if message.error.is_some() {
        ConversationItemStatus::Failed
    } else {
        terminal_message_status(message)
    }
}

fn terminal_message_status(message: &OpenCodeMessage) -> ConversationItemStatus {
    if message.time.completed.is_some() || message.finish.is_some() {
        ConversationItemStatus::Completed
    } else {
        ConversationItemStatus::Running
    }
}

fn tool_status(state: Option<&serde_json::Value>) -> ConversationItemStatus {
    match state
        .and_then(|state| state.get("status"))
        .and_then(serde_json::Value::as_str)
    {
        Some("pending") => ConversationItemStatus::Pending,
        Some("running") => ConversationItemStatus::Running,
        Some("completed") => ConversationItemStatus::Completed,
        Some("error") => ConversationItemStatus::Failed,
        _ => ConversationItemStatus::Unknown,
    }
}

fn bounded_history_text(text: &str, budget: &mut usize) -> String {
    const MAX_CONTENT_BYTES: usize = 256 * 1024;
    const OMITTED: &str = "\n[OpenCode history content truncated]";
    if *budget == 0 {
        return "[OpenCode history content omitted]".to_string();
    }
    let limit = (*budget).min(MAX_CONTENT_BYTES);
    if text.len() <= limit {
        *budget -= text.len();
        return text.to_string();
    }
    let content_limit = limit.saturating_sub(OMITTED.len());
    let mut end = content_limit.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    *budget = budget.saturating_sub(end + OMITTED.len());
    format!("{}{}", &text[..end], OMITTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_do_not_advertise_unimplemented_conversation_search() {
        assert!(!OpenCodeProtocolMapper::base_capabilities()
            .methods
            .contains(&ProviderCapability::ConversationSearch));
    }

    #[test]
    fn capabilities_fall_back_to_native_build_and_plan_outside_a_project() {
        let capabilities = OpenCodeProtocolMapper::capabilities(
            &[],
            &[OpenCodeModel {
                id: "model".to_string(),
                provider_id: "provider".to_string(),
                name: "Model".to_string(),
                status: "active".to_string(),
                enabled: true,
                variants: Vec::new(),
            }],
            &[OpenCodeProvider {
                id: "provider".to_string(),
                name: "Provider".to_string(),
                disabled: None,
            }],
        )
        .unwrap();
        let controls = capabilities.turn_send.unwrap();
        let access = controls.access_mode.unwrap();
        assert_eq!(access.default_id.as_deref(), Some("build"));
        assert_eq!(access.options.iter().map(|option| option.id.as_str()).collect::<Vec<_>>(), vec!["build", "plan"]);
        assert_eq!(controls.reasoning_effort.unwrap().default_id.as_deref(), Some("default"));
        assert!(controls.model_catalog.is_some());
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
