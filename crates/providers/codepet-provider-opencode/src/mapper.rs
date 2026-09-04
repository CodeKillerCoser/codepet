use crate::protocol::{
    OpenCodeAgent, OpenCodeMessage, OpenCodeMessageContent, OpenCodeModel,
    OpenCodePermissionAskedEventData, OpenCodeProvider, OpenCodeServerError, OpenCodeSession,
    OPENCODE_PERMISSION_LEVEL,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ActivitySummaryContentBlock, ActivitySummaryContentBlockKind, ChoiceOption, ChoiceSet,
    CommandConversationItem, CommandConversationItemKind, CommandToolInput, CommandToolInputKind,
    ContentBlock, ConversationContentKind, ConversationCreateCapabilities, ConversationItem,
    ConversationItemRole, ConversationItemStatus, ConversationStatus, HarnessDescriptor,
    GroupedModelCatalog, GroupedModelCatalogKind, GroupedModelProvider, GroupedModelSelection,
    InstanceStatus, ModelCatalog, ModelSelection, ProtocolError, ProtocolEvent, ProviderApproval, ProviderCapabilities,
    ProviderCapability, ProviderConversation, ProviderInstance, ProviderInstanceRoute,
    MessageConversationItem, MessageConversationItemKind, OpaqueToolInput, OpaqueToolInputKind,
    OutputContentBlock, OutputContentBlockKind, ProviderTurn, ReasoningConversationItem,
    ReasoningConversationItemKind, RoutedResourceId, StructuredJsonContentBlock,
    StructuredJsonContentBlockKind, StructuredToolInput, StructuredToolInputKind,
    TextContentBlock, TextContentBlockKind, ToolCategory, ToolConversationItem,
    ToolConversationItemKind, ToolExecutionError, ToolFailureOutcome, ToolFailureOutcomeKind,
    ToolInput, ToolInvocation, ToolOrigin, ToolOriginKind, ToolOutcome, ToolSuccessOutcome,
    ToolSuccessOutcomeKind,
    ToolTiming, TurnOutputDeltaEvent, TurnSelection, TurnSendCapabilities, TurnStatus, TurnUpsertedEvent,
    UnknownConversationItem, UnknownConversationItemKind,
};
use serde_json::Value;
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
        authentication: Option<codepet_provider_sdk::ProviderAuthentication>,
        usage: Option<codepet_provider_sdk::ProviderUsage>,
    ) -> ProviderInstance {
        ProviderInstance {
            route: self.route.clone(),
            plugin_id,
            instance_kind,
            display_name,
            harness,
            status,
            authentication,
            usage,
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
            project: None,
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
        ConversationItem::MessageConversationItem(MessageConversationItem {
            resource: self.resource(item_id.clone()),
            turn: turn.resource.clone(),
            conversation: conversation.clone(),
            kind: MessageConversationItemKind::Message,
            status: ConversationItemStatus::Completed,
            role: ConversationItemRole::User,
            contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                content_id: format!("{item_id}:text"),
                kind: TextContentBlockKind::Text,
                text,
                truncation: None,
            })],
        })
    }

    pub fn conversation_items(
        &self,
        conversation: &RoutedResourceId,
        messages: &[OpenCodeMessage],
    ) -> Vec<ConversationItem> {
        let mut items = Vec::new();
        let mut turn = self.resource("history-turn:initial".to_string());
        for message in messages {
            if message.kind == "user" {
                turn = self.resource(format!("history-turn:{}", message.id));
            }
            match message.kind.as_str() {
                "user" => items.push(self.message_item(
                    message.id.clone(), turn.clone(), conversation,
                    ConversationItemStatus::Completed, ConversationItemRole::User,
                    message.text.as_deref().map(|text| ContentBlock::TextContentBlock(TextContentBlock {
                        content_id: format!("{}:text", message.id),
                        kind: TextContentBlockKind::Text, text: text.to_string(), truncation: None,
                    })).into_iter().collect(),
                )),
                "assistant" => {
                    let status = assistant_status(message);
                    for content in message.content.as_deref().unwrap_or_default() {
                        if let Some(item) = self.assistant_content_item(
                            &message.id,
                            content,
                            &turn,
                            conversation,
                            status,
                        ) {
                            items.push(item);
                        }
                    }
                    if message.content.as_ref().is_none_or(Vec::is_empty) {
                        let contents = message.error.as_ref().map(|error| ContentBlock::ActivitySummaryContentBlock(ActivitySummaryContentBlock {
                            content_id: format!("{}:error", message.id),
                            kind: ActivitySummaryContentBlockKind::ActivitySummary,
                            text: error.to_string(), truncation: None,
                        })).into_iter().collect();
                        items.push(self.message_item(
                            message.id.clone(), turn.clone(), conversation, status,
                            ConversationItemRole::Assistant, contents,
                        ));
                    }
                }
                "shell" => {
                    items.push(ConversationItem::CommandConversationItem(CommandConversationItem {
                        resource: self.resource(message.id.clone()), turn: turn.clone(),
                        conversation: conversation.clone(), kind: CommandConversationItemKind::Command,
                        status: assistant_status(message),
                        title: message.command.as_deref().map(concise_tool_title).or_else(|| Some("Shell".to_string())),
                        tool: shell_tool_invocation(message),
                    }));
                }
                "system" | "synthetic" => items.push(self.message_item(
                    message.id.clone(), turn.clone(), conversation,
                    ConversationItemStatus::Completed, ConversationItemRole::Assistant,
                    message.text.as_deref().map(|text| ContentBlock::TextContentBlock(TextContentBlock {
                        content_id: format!("{}:text", message.id),
                        kind: TextContentBlockKind::Text, text: text.to_string(), truncation: None,
                    })).into_iter().collect(),
                )),
                "compaction" => items.push(self.unknown_item(message.id.clone(), turn.clone(), conversation,
                    ConversationItemStatus::Completed, Some("Compaction".to_string()))),
                _ => items.push(self.unknown_item(message.id.clone(), turn.clone(), conversation,
                    terminal_message_status(message), Some(format!("OpenCode {}", message.kind)))),
            }
        }
        items
    }

    fn assistant_content_item(
        &self,
        message_id: &str,
        content: &OpenCodeMessageContent,
        turn: &RoutedResourceId,
        conversation: &RoutedResourceId,
        message_status: ConversationItemStatus,
    ) -> Option<ConversationItem> {
        let resource_id = format!("{message_id}:{}", content.id);
        match content.kind.as_str() {
            "text" => Some(self.message_item(resource_id, turn.clone(), conversation,
                message_status, ConversationItemRole::Assistant,
                content.text.as_deref().map(|text| ContentBlock::TextContentBlock(TextContentBlock {
                    content_id: format!("{message_id}:{}:text", content.id),
                    kind: TextContentBlockKind::Text, text: text.to_string(), truncation: None,
                })).into_iter().collect(),
            )),
            "reasoning" => Some(ConversationItem::ReasoningConversationItem(ReasoningConversationItem {
                resource: self.resource(resource_id), turn: turn.clone(), conversation: conversation.clone(),
                kind: ReasoningConversationItemKind::Reasoning, status: message_status,
                contents: content.text.as_deref().map(|text| ContentBlock::ReasoningSummaryContentBlock(codepet_provider_sdk::ReasoningSummaryContentBlock {
                    content_id: format!("{message_id}:{}:summary:0", content.id),
                    kind: codepet_provider_sdk::ReasoningSummaryContentBlockKind::ReasoningSummary,
                    text: text.to_string(), truncation: None,
                })).into_iter().collect(),
            })),
            "tool" => {
                let status = tool_status(content.state.as_ref());
                let tool = opencode_tool_invocation(content);
                Some(ConversationItem::ToolConversationItem(ToolConversationItem {
                    resource: self.resource(resource_id), turn: turn.clone(), conversation: conversation.clone(),
                    kind: ToolConversationItemKind::Tool, status,
                    title: content.name.clone().or_else(|| Some("Tool".to_string())), tool,
                }))
            }
            _ => Some(self.unknown_item(resource_id, turn.clone(), conversation, message_status,
                Some(format!("OpenCode {}", content.kind)))),
        }
    }

    fn message_item(&self, resource_id: String, turn: RoutedResourceId,
        conversation: &RoutedResourceId, status: ConversationItemStatus,
        role: ConversationItemRole, contents: Vec<ContentBlock>) -> ConversationItem {
        ConversationItem::MessageConversationItem(MessageConversationItem {
            resource: self.resource(resource_id), turn, conversation: conversation.clone(),
            kind: MessageConversationItemKind::Message, status, role, contents,
        })
    }

    fn unknown_item(&self, resource_id: String, turn: RoutedResourceId,
        conversation: &RoutedResourceId, status: ConversationItemStatus,
        title: Option<String>) -> ConversationItem {
        ConversationItem::UnknownConversationItem(UnknownConversationItem {
            resource: self.resource(resource_id), turn, conversation: conversation.clone(),
            kind: UnknownConversationItemKind::Unknown, status, title,
        })
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
    let name = content.name.clone().unwrap_or_else(|| "tool".to_string());
    let command = input_value
        .and_then(Value::as_object)
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str);
    let input = if let Some(command) = command {
        ToolInput::CommandToolInput(CommandToolInput {
            kind: CommandToolInputKind::Command,
            command: command.to_string(),
            cwd: input_value.and_then(Value::as_object).and_then(|input| input.get("cwd")).and_then(Value::as_str).map(str::to_string),
            shell: None,
            truncation: None,
            actions: None,
        })
    } else if let Some(object) = input_value.and_then(Value::as_object) {
        ToolInput::StructuredToolInput(StructuredToolInput {
            kind: StructuredToolInputKind::Structured,
            value: object.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
            truncation: None,
        })
    } else {
        ToolInput::OpaqueToolInput(OpaqueToolInput {
            kind: OpaqueToolInputKind::Opaque,
            value: input_value.map(Value::to_string).unwrap_or_default(),
            mime_type: Some("application/json".to_string()),
            truncation: None,
        })
    };
    ToolInvocation {
        call_id: content.call_id.clone().unwrap_or_else(|| content.id.clone()),
        name: name.clone(),
        namespace: None,
        category: opencode_tool_category(&name, command.is_some()),
        origin: ToolOrigin { kind: ToolOriginKind::Server, name: Some("opencode".to_string()) },
        input,
        outcome: state.and_then(opencode_tool_outcome),
        timing: content.time.as_ref().map(|time| ToolTiming {
            started_at: Some(time.created),
            completed_at: time.completed,
            duration_ms: time.completed.and_then(|completed| completed.checked_sub(time.created)),
        }),
        annotations: None,
        extension: None,
    }
}

fn opencode_tool_outcome(state: &serde_json::Map<String, Value>) -> Option<ToolOutcome> {
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
    if let Some(structured) = state.get("structured") {
        if let Some(object) = structured.as_object() {
            content.push(ContentBlock::StructuredJsonContentBlock(StructuredJsonContentBlock {
                content_id: "structured".to_string(),
                kind: StructuredJsonContentBlockKind::StructuredJson,
                value: object.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
                truncation: None,
            }));
        } else {
            append_tool_content(&mut content, "structured", structured);
        }
    }
    let error = state.get("error").map(|error| ToolExecutionError {
        code: None,
        message: concise_error_message(error),
        retryable: None,
    });
    if !matches!(status, Some("completed" | "error")) && content.is_empty() && error.is_none() {
        return None;
    }
    let exit_code = state.get("exitCode").and_then(Value::as_i64);
    Some(match error {
        Some(error) => ToolOutcome::ToolFailureOutcome(ToolFailureOutcome {
            kind: ToolFailureOutcomeKind::Failure, content, error, exit_code, process_id: None,
        }),
        None => ToolOutcome::ToolSuccessOutcome(ToolSuccessOutcome {
            kind: ToolSuccessOutcomeKind::Success, content, exit_code, process_id: None,
        }),
    })
}

fn append_tool_content(content: &mut Vec<ContentBlock>, id: &str, value: &Value) {
    let text = value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.get("text").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| value.to_string());
    content.push(ContentBlock::OutputContentBlock(OutputContentBlock {
        content_id: id.to_string(),
        kind: OutputContentBlockKind::Output,
        text,
        truncation: None,
    }));
}

fn shell_tool_invocation(message: &OpenCodeMessage) -> ToolInvocation {
    let command = message.command.clone().unwrap_or_default();
    let outcome = (message.output.is_some() || message.error.is_some()).then(|| {
        let content = message.output.as_ref().map(|output| {
            ContentBlock::OutputContentBlock(OutputContentBlock {
                content_id: format!("{}:output", message.id),
                kind: OutputContentBlockKind::Output,
                text: output.clone(),
                truncation: None,
            })
        }).into_iter().collect();
        match message.error.as_ref() {
            Some(error) => ToolOutcome::ToolFailureOutcome(ToolFailureOutcome {
                kind: ToolFailureOutcomeKind::Failure, content,
                error: ToolExecutionError { code: None, message: concise_error_message(error), retryable: None },
                exit_code: None, process_id: None,
            }),
            None => ToolOutcome::ToolSuccessOutcome(ToolSuccessOutcome {
                kind: ToolSuccessOutcomeKind::Success, content, exit_code: None, process_id: None,
            }),
        }
    });
    ToolInvocation {
        call_id: message.id.clone(),
        name: "shell".to_string(),
        namespace: None,
        category: ToolCategory::Command,
        origin: ToolOrigin { kind: ToolOriginKind::Server, name: Some("opencode".to_string()) },
        input: ToolInput::CommandToolInput(CommandToolInput {
            kind: CommandToolInputKind::Command, command, cwd: None, shell: None, truncation: None, actions: None,
        }),
        outcome,
        timing: Some(ToolTiming {
            started_at: Some(message.time.created),
            completed_at: message.time.completed,
            duration_ms: message.time.completed.and_then(|completed| completed.checked_sub(message.time.created)),
        }),
        annotations: None,
        extension: None,
    }
}

fn concise_error_message(value: &Value) -> String {
    let message = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
    message.chars().take(512).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn history_message(
        id: &str,
        kind: &str,
        text: Option<&str>,
        content: Option<Vec<OpenCodeMessageContent>>,
    ) -> OpenCodeMessage {
        OpenCodeMessage {
            id: id.to_string(),
            kind: kind.to_string(),
            time: crate::protocol::OpenCodeMessageTime {
                created: 1,
                completed: Some(2),
            },
            text: text.map(str::to_string),
            content,
            command: None,
            output: None,
            summary: None,
            recent: None,
            finish: Some("stop".to_string()),
            error: None,
        }
    }

    fn text_content(id: &str, text: &str) -> OpenCodeMessageContent {
        OpenCodeMessageContent {
            id: id.to_string(),
            kind: "text".to_string(),
            text: Some(text.to_string()),
            name: None,
            call_id: None,
            state: None,
            time: None,
        }
    }

    #[test]
    fn assistant_content_identity_is_scoped_by_its_message() {
        let mapper = OpenCodeProtocolMapper::new(ProviderInstanceRoute {
            device_id: "device".to_string(),
            provider_plugin_id: "opencode".to_string(),
            provider_instance_id: "default".to_string(),
        });
        let conversation = mapper.resource("session".to_string());
        let messages = vec![
            history_message("user-one", "user", Some("one"), None),
            history_message(
                "assistant-one",
                "assistant",
                None,
                Some(vec![text_content("text-0", "first")]),
            ),
            history_message("user-two", "user", Some("two"), None),
            history_message(
                "assistant-two",
                "assistant",
                None,
                Some(vec![text_content("text-0", "second")]),
            ),
        ];

        let items = mapper.conversation_items(&conversation, &messages);

        let ConversationItem::MessageConversationItem(first) = &items[1] else { panic!("message") };
        let ConversationItem::MessageConversationItem(second) = &items[3] else { panic!("message") };
        assert_eq!(first.resource.native_resource_id, "assistant-one:text-0");
        let ContentBlock::TextContentBlock(first_text) = &first.contents[0] else { panic!("text") };
        assert_eq!(first_text.content_id, "assistant-one:text-0:text");
        assert_eq!(second.resource.native_resource_id, "assistant-two:text-0");
        let ContentBlock::TextContentBlock(second_text) = &second.contents[0] else { panic!("text") };
        assert_eq!(second_text.content_id, "assistant-two:text-0:text");
        assert_ne!(first_text.content_id, second_text.content_id);
    }

    #[test]
    fn shell_command_input_starts_complete_for_the_shared_budget() {
        let mut message = history_message("shell-one", "shell", None, None);
        message.command = Some("cargo test".to_string());
        let tool = shell_tool_invocation(&message);
        let ToolInput::CommandToolInput(input) = tool.input else { panic!("command input") };
        assert_eq!(input.command, "cargo test");
        assert!(input.truncation.is_none());
    }

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
