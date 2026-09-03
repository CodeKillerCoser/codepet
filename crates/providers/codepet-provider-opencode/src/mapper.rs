use crate::protocol::{
    OpenCodeAgent, OpenCodeMessage, OpenCodeMessageContent, OpenCodeModel,
    OpenCodePermissionAskedEventData, OpenCodeProvider, OpenCodeServerError, OpenCodeSession,
    OPENCODE_PERMISSION_LEVEL,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ChoiceOption, ChoiceSet, ConversationContent, ConversationContentKind,
    ConversationCreateCapabilities, ConversationItem, ConversationItemKind,
    ConversationItemRole, ConversationItemStatus, ConversationStatus, HarnessDescriptor,
    GroupedModelCatalog, GroupedModelCatalogKind, GroupedModelProvider, GroupedModelSelection,
    InstanceStatus, JsonObject, ModelCatalog, ModelSelection, ProtocolError, ProtocolEvent, ProviderApproval, ProviderCapabilities,
    ProviderCapability, ProviderConversation, ProviderInstance, ProviderInstanceRoute,
    ProviderTurn, RoutedResourceId, ToolCategory, ToolCommandDetails, ToolContent,
    ToolContentKind, ToolExecutionError, ToolInvocation, ToolOrigin, ToolOriginKind, ToolResult,
    ToolTiming, TurnOutputDeltaEvent, TurnSelection, TurnSendCapabilities, TurnStatus, TurnUpsertedEvent,
};
use serde_json::{json, Value};
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
            conversation_create: Some(ConversationCreateCapabilities {
                supports_title: false,
                selection: Some(TurnSendCapabilities {
                    access_mode: Some(ChoiceSet {
                        options: vec![ChoiceOption {
                            id: OPENCODE_PERMISSION_LEVEL.to_string(),
                            display_name: "OpenCode default".to_string(),
                            description: None,
                            enabled: Some(true),
                            disabled_reason: None,
                        }],
                        default_id: Some(OPENCODE_PERMISSION_LEVEL.to_string()),
                    }),
                    reasoning_effort: None,
                    model_catalog: None,
                }),
                workspace_mode: Some(ChoiceSet {
                    options: vec![ChoiceOption {
                        id: "main".to_string(),
                        display_name: "Main workspace".to_string(),
                        description: None,
                        enabled: Some(true),
                        disabled_reason: None,
                    }],
                    default_id: Some("main".to_string()),
                }),
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
            tool: None,
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
                    let mut item = self.history_item(
                        message.id.clone(),
                        turn.clone(),
                        conversation,
                        ConversationItemKind::Command,
                        terminal_message_status(message),
                        None,
                        Some("Shell".to_string()),
                        contents,
                    );
                    item.tool = Some(shell_tool_invocation(message));
                    item.title = message.command.as_deref().map(concise_tool_title).or(item.title);
                    items.push(item);
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
                let tool = opencode_tool_invocation(content);
                let mut item = self.history_item(
                    resource_id,
                    turn.clone(),
                    conversation,
                    ConversationItemKind::Tool,
                    status,
                    None,
                    content.name.clone().or_else(|| Some("Tool".to_string())),
                    Vec::new(),
                );
                item.tool = Some(tool);
                Some(item)
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
            tool: None,
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

fn opencode_tool_invocation(content: &OpenCodeMessageContent) -> ToolInvocation {
    let state = content.state.as_ref().and_then(Value::as_object);
    let input_value = state.and_then(|state| state.get("input"));
    let serialized_input = input_value.map(Value::to_string);
    let input_fits = serialized_input.as_ref().is_none_or(|value| value.len() <= 256 * 1024);
    let input: JsonObject = input_fits
        .then(|| input_value.and_then(Value::as_object))
        .flatten()
        .map(|object| object.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
        .unwrap_or_default();
    let raw_input = serialized_input
        .filter(|_| !input_fits || input_value.is_some_and(|value| !value.is_object()))
        .map(|value| bounded_tool_result_text(&value).0);
    let name = content.name.clone().unwrap_or_else(|| "tool".to_string());
    let result = state.and_then(opencode_tool_result);
    let command = input_value
        .and_then(Value::as_object)
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .map(|command| ToolCommandDetails {
            command: bounded_tool_result_text(command).0,
            cwd: input_value.and_then(Value::as_object).and_then(|input| input.get("cwd")).and_then(Value::as_str).map(str::to_string),
            exit_code: state
                .and_then(|state| state.get("exitCode"))
                .and_then(Value::as_i64),
            process_id: None,
            actions: None,
        });
    ToolInvocation {
        call_id: content.call_id.clone().unwrap_or_else(|| content.id.clone()),
        name: name.clone(),
        namespace: None,
        category: opencode_tool_category(&name, command.is_some()),
        origin: ToolOrigin { kind: ToolOriginKind::Server, name: Some("opencode".to_string()) },
        input,
        raw_input,
        result,
        timing: content.time.as_ref().map(|time| ToolTiming {
            started_at: Some(time.created),
            completed_at: time.completed,
            duration_ms: time.completed.and_then(|completed| completed.checked_sub(time.created)),
        }),
        command,
        annotations: None,
        extension: None,
    }
}

fn opencode_tool_result(state: &serde_json::Map<String, Value>) -> Option<ToolResult> {
    let status = state.get("status").and_then(Value::as_str);
    let mut content = Vec::new();
    if let Some(output) = state.get("output") {
        append_tool_content(&mut content, "output", output);
    }
    if let Some(parts) = state.get("content").and_then(Value::as_array) {
        for (index, part) in parts.iter().take(128).enumerate() {
            append_tool_content(&mut content, &format!("content:{index}"), part);
        }
    }
    let structured_value = state.get("structured");
    let structured_content = structured_value
        .filter(|value| value.to_string().len() <= 256 * 1024)
        .and_then(Value::as_object)
        .map(|object| object.iter().map(|(key, value)| (key.clone(), value.clone())).collect());
    if structured_content.is_none() {
        if let Some(structured) = structured_value {
            append_tool_content(&mut content, "structured", structured);
        }
    }
    let error = state.get("error").map(|error| ToolExecutionError {
        code: None,
        message: error.as_str().map(str::to_string).unwrap_or_else(|| error.to_string()),
        retryable: None,
        details: None,
    });
    (matches!(status, Some("completed" | "error")) || !content.is_empty() || structured_content.is_some() || error.is_some())
        .then_some(ToolResult { content, structured_content, error })
}

fn append_tool_content(content: &mut Vec<ToolContent>, id: &str, value: &Value) {
    const MAX_RESULT_BYTES: usize = 256 * 1024;
    let used = content.iter().filter_map(|content| content.text.as_ref()).map(String::len).sum::<usize>();
    if used >= MAX_RESULT_BYTES {
        return;
    }
    let text = value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.get("text").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| value.to_string());
    let (text, truncated, total_bytes) = bounded_tool_result_text_with_limit(&text, MAX_RESULT_BYTES - used);
    content.push(ToolContent {
        content_id: id.to_string(),
        kind: ToolContentKind::Text,
        text: Some(text),
        uri: None,
        mime_type: Some(if value.is_string() { "text/plain" } else { "application/json" }.to_string()),
        name: None,
        truncated: truncated.then_some(true),
        total_bytes: truncated.then_some(total_bytes),
    });
}

fn shell_tool_invocation(message: &OpenCodeMessage) -> ToolInvocation {
    let command = message.command.clone().unwrap_or_default();
    let result = (message.output.is_some() || message.error.is_some()).then(|| {
        let content = message.output.as_ref().map(|output| {
            let (text, truncated, total_bytes) = bounded_tool_result_text(output);
            ToolContent {
                content_id: format!("{}:output", message.id),
                kind: ToolContentKind::Text,
                text: Some(text),
                uri: None,
                mime_type: Some("text/plain".to_string()),
                name: Some("Command output".to_string()),
                truncated: truncated.then_some(true),
                total_bytes: truncated.then_some(total_bytes),
            }
        }).into_iter().collect();
        ToolResult {
            content,
            structured_content: None,
            error: message.error.as_ref().map(|error| ToolExecutionError {
                code: None,
                message: error.to_string(),
                retryable: None,
                details: None,
            }),
        }
    });
    ToolInvocation {
        call_id: message.id.clone(),
        name: "shell".to_string(),
        namespace: None,
        category: ToolCategory::Command,
        origin: ToolOrigin { kind: ToolOriginKind::Server, name: Some("opencode".to_string()) },
        input: [("command".to_string(), json!(command))].into_iter().collect(),
        raw_input: None,
        result,
        timing: Some(ToolTiming {
            started_at: Some(message.time.created),
            completed_at: message.time.completed,
            duration_ms: message.time.completed.and_then(|completed| completed.checked_sub(message.time.created)),
        }),
        command: Some(ToolCommandDetails {
            command,
            cwd: None,
            exit_code: None,
            process_id: None,
            actions: None,
        }),
        annotations: None,
        extension: None,
    }
}

fn opencode_tool_category(name: &str, command: bool) -> ToolCategory {
    if command {
        return ToolCategory::Command;
    }
    let name = name.to_ascii_lowercase();
    if name.contains("read") || name.contains("list") {
        ToolCategory::Read
    } else if name.contains("write") || name.contains("edit") || name.contains("patch") {
        ToolCategory::Write
    } else if name.contains("search") || name.contains("find") {
        ToolCategory::Search
    } else if name.contains("web") || name.contains("browser") {
        ToolCategory::Web
    } else if name.contains("agent") || name.contains("task") {
        ToolCategory::Agent
    } else {
        ToolCategory::Other
    }
}

fn concise_tool_title(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or(command).trim();
    if first_line.chars().count() <= 80 {
        first_line.to_string()
    } else {
        format!("{}…", first_line.chars().take(79).collect::<String>())
    }
}

fn bounded_tool_result_text(text: &str) -> (String, bool, u64) {
    bounded_tool_result_text_with_limit(text, 256 * 1024)
}

fn bounded_tool_result_text_with_limit(text: &str, max_bytes: usize) -> (String, bool, u64) {
    let total_bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if text.len() <= max_bytes {
        return (text.to_string(), false, total_bytes);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}\n[tool output truncated]", &text[..end]), true, total_bytes)
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
