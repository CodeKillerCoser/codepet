use crate::protocol::{
    activity_summary_content_id, output_content_id,
    reasoning_summary_content_id, text_content_id, user_input_content_id,
    CodexAppServerError, CodexApprovalRequest, CodexContentKind, CodexConversationSnapshot,
    CodexIncoming, CodexModel, CodexNotification, CodexPermissionLevel, CodexProject,
    CodexProjectChangeType, CodexThreadActiveFlag,
    CodexThreadItem, CodexThreadStatus, CodexTurn, CodexTurnStatus, CODEX_EXTENSION_NAMESPACE,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalRequestedEvent, ApprovalResolvedEvent, ApprovalStatus,
    ActivitySummaryContentBlock, ActivitySummaryContentBlockKind, ApprovalConversationItem,
    ApprovalConversationItemKind, ChoiceOption, ChoiceSet, CommandConversationItem,
    CommandConversationItemKind, CommandToolInput, CommandToolInputKind, ContentBlock,
    ConversationContentKind,
    ConversationCreateCapabilities, ConversationItem, ConversationItemUpsertedEvent,
    ConversationItemRole, ConversationItemStatus, ConversationStatus,
    ConversationUpsertedEvent, FlatModelCatalog, FlatModelCatalogKind, FlatModelSelection,
    HarnessDescriptor, InstanceStatus, JsonObject, ModelCatalog, ModelSelection, Project,
    ProjectChangeType, ProjectChangedEvent, ProjectRoot, ProtocolError,
    ProtocolEvent, Approval, ProviderCapabilities, ProviderCapability,
    Conversation, ProviderExtension, ProviderInstance, ProviderInstanceRoute, ProviderResourceId,
    ProviderAuthentication, TurnTask, ProviderUsage, RoutedResourceId, ToolCategory,
    FileChangeConversationItem, FileChangeConversationItemKind, MessageConversationItem,
    MessageConversationItemKind, OpaqueToolInput, OpaqueToolInputKind, OutputContentBlock,
    OutputContentBlockKind, ReasoningConversationItem, ReasoningConversationItemKind,
    StructuredJsonContentBlock, StructuredJsonContentBlockKind, StructuredToolInput,
    StructuredToolInputKind, TextContentBlock, TextContentBlockKind, ToolCommandAction,
    ToolCommandActionKind, ToolConversationItem, ToolConversationItemKind, ToolExecutionError,
    ToolFailureOutcome, ToolFailureOutcomeKind, ToolInput, ToolInvocation, ToolOrigin,
    ToolOriginKind, ToolOutcome, ToolSuccessOutcome, ToolSuccessOutcomeKind, ToolTiming,
    TurnOutputDeltaEvent, TurnSelection, TurnSendCapabilities, TurnStatus,
    TurnUpsertedEvent,
    UnknownConversationItem, UnknownConversationItemKind,
};
use serde_json::{json, Value};

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
            conversation_create: None,
            extensions: vec![extension([
                (
                    "nativeMethods",
                    json!([
                        "thread/list",
                        "thread/read",
                        "thread/turns/list",
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
        project_api_supported: bool,
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
        if project_api_supported {
            methods.extend([
                ProviderCapability::ProjectList,
                ProviderCapability::ProjectGet,
                ProviderCapability::ProjectCreate,
                ProviderCapability::ProjectUpdate,
                ProviderCapability::ProjectDelete,
            ]);
        }
        let reasoning_effort = (!reasoning_options.is_empty()).then(|| ChoiceSet {
            options: reasoning_options,
            default_id: default_reasoning,
        });
        let turn_send = TurnSendCapabilities {
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
        };
        let mut extensions = Self::unavailable_capabilities("unused".to_string()).extensions;
        if project_api_supported {
            if let Some(native_methods) = extensions
                .first_mut()
                .and_then(|extension| extension.data.get_mut("nativeMethods"))
                .and_then(Value::as_array_mut)
            {
                native_methods.extend([
                    json!("project/list"),
                    json!("project/read"),
                    json!("project/create"),
                    json!("project/update"),
                    json!("project/delete"),
                ]);
            }
        }
        Ok(ProviderCapabilities {
            revision,
            methods,
            conversation_create: Some(ConversationCreateCapabilities {
                supports_title: false,
                selection: Some(turn_send.clone()),
                workspace_mode: Some(ChoiceSet {
                    options: vec![
                        choice("main", "Main workspace"),
                        choice("worktree", "Worktree"),
                    ],
                    default_id: Some("main".to_string()),
                }),
            }),
            turn_send: Some(turn_send),
            extensions,
        })
    }

    pub fn instance(
        &self,
        plugin_id: String,
        instance_kind: String,
        display_name: String,
        harness: HarnessDescriptor,
        status: InstanceStatus,
        authentication: Option<ProviderAuthentication>,
        usage: Option<ProviderUsage>,
        capabilities: ProviderCapabilities,
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

    pub fn conversation(&self, snapshot: &CodexConversationSnapshot) -> Conversation {
        let active_turn = snapshot
            .thread
            .turns
            .iter()
            .rev()
            .find(|turn| turn.status == CodexTurnStatus::InProgress)
            .map(|turn| {
                self.turn(&snapshot.thread.id, turn)
            });
        self.conversation_with_active_turn(snapshot, active_turn)
    }

    pub(crate) fn conversation_with_active_turn(
        &self,
        snapshot: &CodexConversationSnapshot,
        active_turn: Option<TurnTask>,
    ) -> Conversation {
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
        Conversation {
            resource: self.resource(snapshot.thread.id.clone()),
            project: snapshot
                .thread
                .project_id
                .as_ref()
                .filter(|project_id| !project_id.is_empty())
                .map(|project_id| self.resource(project_id.clone())),
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
            read_state: None,
        }
    }

    pub fn project(&self, project: CodexProject) -> Result<Project, ProtocolError> {
        if project.id.trim().is_empty()
            || project.name.trim().is_empty()
            || project.roots.iter().any(|root| root.path.trim().is_empty())
        {
            return Err(protocol_error(
                "provider_protocol_error",
                "Codex App Server returned an invalid project identity, name, or root".to_string(),
                false,
            ));
        }
        let created_at = u64::try_from(project.created_at).map_err(|_| {
            protocol_error(
                "provider_protocol_error",
                "Codex App Server returned a negative project createdAt".to_string(),
                false,
            )
        })?;
        let updated_at = u64::try_from(project.updated_at).map_err(|_| {
            protocol_error(
                "provider_protocol_error",
                "Codex App Server returned a negative project updatedAt".to_string(),
                false,
            )
        })?;
        if created_at > 9_007_199_254_740_991 || updated_at > 9_007_199_254_740_991 {
            return Err(protocol_error(
                "provider_protocol_error",
                "Codex App Server returned a project timestamp outside the JSON-safe integer range"
                    .to_string(),
                false,
            ));
        }
        if !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&project.position) {
            return Err(protocol_error(
                "provider_protocol_error",
                "Codex App Server returned a project position outside the JSON-safe integer range"
                    .to_string(),
                false,
            ));
        }
        Ok(Project {
            resource: self.resource(project.id),
            name: project.name,
            roots: project
                .roots
                .into_iter()
                .map(|root| ProjectRoot { path: root.path })
                .collect(),
            metadata: project.metadata,
            position: project.position,
            created_at,
            updated_at,
        })
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
    ) -> TurnTask {
        let started_at = turn.started_at.and_then(seconds_to_ms);
        let completed_at = turn.completed_at.and_then(seconds_to_ms);
        TurnTask {
            resource: self.resource(turn.id.clone()),
            conversation: self.resource(conversation_id.to_string()),
            status: turn_status(turn.status),
            display_summary: None,
            started_at,
            updated_at: None,
            completed_at,
        }
    }

    #[cfg(test)]
    pub fn conversation_items(
        &self,
        snapshot: &CodexConversationSnapshot,
        approvals: &[(String, Approval)],
    ) -> Vec<ConversationItem> {
        let mut items = Vec::new();
        let mut emitted_approvals = vec![false; approvals.len()];
        for turn in &snapshot.thread.turns {
            self.append_conversation_turn_items(
                &snapshot.thread.id,
                turn,
                approvals,
                &mut emitted_approvals,
                &mut items,
            );
        }
        self.append_remaining_approval_items(approvals, &emitted_approvals, &mut items);
        items
    }

    pub(crate) fn append_conversation_turn_items(
        &self,
        conversation_id: &str,
        turn: &CodexTurn,
        approvals: &[(String, Approval)],
        emitted_approvals: &mut [bool],
        items: &mut Vec<ConversationItem>,
    ) {
        debug_assert_eq!(approvals.len(), emitted_approvals.len());
        let conversation = self.resource(conversation_id.to_string());
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

    #[cfg(test)]
    fn append_remaining_approval_items(
        &self,
        approvals: &[(String, Approval)],
        emitted_approvals: &[bool],
        items: &mut Vec<ConversationItem>,
    ) {
        debug_assert_eq!(approvals.len(), emitted_approvals.len());
        for (index, (related_item_id, approval)) in approvals.iter().enumerate() {
            if !emitted_approvals[index] {
                items.push(self.approval_item(related_item_id, approval));
            }
        }
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
        let mut mapped = match item {
            CodexThreadItem::UserMessage { id, text_inputs } => ConversationItem::MessageConversationItem(MessageConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: MessageConversationItemKind::Message,
                status: ConversationItemStatus::Completed,
                role: ConversationItemRole::User,
                contents: text_inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, text)| {
                        text.as_ref().map(|text| ContentBlock::TextContentBlock(TextContentBlock {
                            content_id: user_input_content_id(id, index),
                            kind: TextContentBlockKind::Text,
                            text: text.clone(),
                            truncation: None,
                        }))
                    })
                    .collect(),
            }),
            CodexThreadItem::AgentMessage { id, text } => ConversationItem::MessageConversationItem(MessageConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: MessageConversationItemKind::Message,
                status: mutable_content_status(turn.status),
                role: ConversationItemRole::Assistant,
                contents: (!mutable_text)
                    .then(|| ContentBlock::TextContentBlock(TextContentBlock {
                        content_id: text_content_id(id),
                        kind: TextContentBlockKind::Text,
                        text: text.clone(),
                        truncation: None,
                    }))
                    .into_iter()
                    .collect(),
            }),
            CodexThreadItem::Plan { id, text } => ConversationItem::MessageConversationItem(MessageConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: MessageConversationItemKind::Message,
                status: mutable_content_status(turn.status),
                role: ConversationItemRole::Assistant,
                contents: (!mutable_text)
                    .then(|| ContentBlock::TextContentBlock(TextContentBlock {
                        content_id: text_content_id(id),
                        kind: TextContentBlockKind::Text,
                        text: text.clone(),
                        truncation: None,
                    }))
                    .into_iter()
                    .collect(),
            }),
            CodexThreadItem::Reasoning { id, summary } => ConversationItem::ReasoningConversationItem(ReasoningConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ReasoningConversationItemKind::Reasoning,
                status: mutable_content_status(turn.status),
                contents: if mutable_text {
                    Vec::new()
                } else {
                    summary
                        .iter()
                        .enumerate()
                        .map(|(index, text)| ContentBlock::ReasoningSummaryContentBlock(codepet_provider_sdk::ReasoningSummaryContentBlock {
                            content_id: reasoning_summary_content_id(id, index),
                            kind: codepet_provider_sdk::ReasoningSummaryContentBlockKind::ReasoningSummary,
                            text: text.clone(),
                            truncation: None,
                        }))
                        .collect()
                },
            }),
            CodexThreadItem::CommandExecution {
                id,
                command,
                status,
                aggregated_output,
                cwd,
                duration_ms,
                exit_code,
                process_id,
                command_actions,
            } => {
                let status = activity_status(status);
                let mut result_content = Vec::new();
                if status != ConversationItemStatus::Running {
                    if let Some(output) = aggregated_output {
                        result_content.push(ContentBlock::OutputContentBlock(OutputContentBlock {
                            content_id: output_content_id(id),
                            kind: OutputContentBlockKind::Output,
                            text: output.clone(),
                            truncation: None,
                        }));
                    }
                }
                let failed = matches!(status, ConversationItemStatus::Failed | ConversationItemStatus::Interrupted)
                    || exit_code.is_some_and(|code| code != 0);
                let outcome = (status != ConversationItemStatus::Running).then(|| {
                    if failed {
                        ToolOutcome::ToolFailureOutcome(ToolFailureOutcome {
                            kind: ToolFailureOutcomeKind::Failure,
                            content: result_content,
                            error: ToolExecutionError {
                                code: Some("command_failed".to_string()),
                                message: exit_code.map_or_else(|| "Command failed".to_string(), |code| format!("Command exited with status {code}")),
                                retryable: None,
                            },
                            exit_code: *exit_code,
                            process_id: process_id.clone(),
                        })
                    } else {
                        ToolOutcome::ToolSuccessOutcome(ToolSuccessOutcome {
                            kind: ToolSuccessOutcomeKind::Success,
                            content: result_content,
                            exit_code: *exit_code,
                            process_id: process_id.clone(),
                        })
                    }
                });
                let tool = ToolInvocation {
                    call_id: id.clone(),
                    name: "shell".to_string(),
                    namespace: None,
                    category: ToolCategory::Command,
                    origin: ToolOrigin { kind: ToolOriginKind::Builtin, name: Some("codex".to_string()) },
                    input: ToolInput::CommandToolInput(CommandToolInput {
                        kind: CommandToolInputKind::Command,
                        command: command.clone(),
                        cwd: cwd.clone(),
                        shell: None,
                        truncation: None,
                        actions: (!command_actions.is_empty()).then(|| command_actions.iter().take(64).map(|action| ToolCommandAction {
                            kind: command_action_kind(&action.kind),
                            command: action.command.clone(),
                            name: action.name.clone(),
                            path: action.path.clone(),
                            query: action.query.clone(),
                        }).collect()),
                    }),
                    outcome,
                    timing: duration_ms.map(|duration_ms| ToolTiming {
                        started_at: None,
                        completed_at: None,
                        duration_ms: Some(duration_ms),
                    }),
                    annotations: None,
                };
                ConversationItem::CommandConversationItem(CommandConversationItem {
                    meta: None,
                    resource,
                    turn: turn_resource,
                    conversation: conversation.clone(),
                    kind: CommandConversationItemKind::Command,
                    status,
                    title: Some(command_title(command, command_actions)),
                    tool,
                })
            }
            CodexThreadItem::FileChange {
                id,
                status,
                change_count,
            } => ConversationItem::FileChangeConversationItem(FileChangeConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: FileChangeConversationItemKind::FileChange,
                status: activity_status(status),
                title: Some("File changes".to_string()),
                contents: vec![ContentBlock::ActivitySummaryContentBlock(ActivitySummaryContentBlock {
                    content_id: activity_summary_content_id(id),
                    kind: ActivitySummaryContentBlockKind::ActivitySummary,
                    text: format!("{change_count} file change(s)"),
                    truncation: None,
                })],
            }),
            CodexThreadItem::ToolActivity { id, title, status, details } => ConversationItem::ToolConversationItem(ToolConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: ToolConversationItemKind::Tool,
                status: status
                    .as_deref()
                    .map(activity_status)
                    .unwrap_or_else(|| item_status_from_turn(turn.status)),
                title: Some(title.clone()),
                tool: details.as_ref().map(|details| tool_invocation(id, details)).unwrap_or_else(|| opaque_tool_invocation(id, title)),
            }),
            CodexThreadItem::Unknown { .. } => ConversationItem::UnknownConversationItem(UnknownConversationItem {
                meta: None,
                resource,
                turn: turn_resource,
                conversation: conversation.clone(),
                kind: UnknownConversationItemKind::Unknown,
                status: item_status_from_turn(turn.status),
                title: Some("Unknown Codex activity".to_string()),
            }),
        };
        codepet_provider_sdk::truncate_tool_item_text(&mut mapped, codepet_provider_sdk::DEFAULT_TOOL_TEXT_BYTES);
        mapped
    }

    fn approval_item(&self, related_item_id: &str, approval: &Approval) -> ConversationItem {
        ConversationItem::ApprovalConversationItem(ApprovalConversationItem {
            meta: None,
            resource: approval.resource.clone(),
            turn: approval.turn.clone(),
            conversation: approval.conversation.clone(),
            kind: ApprovalConversationItemKind::Approval,
            status: approval_item_status(approval.status),
            title: Some(approval.title.clone()),
            related_item: Some(self.resource(related_item_id.to_string())),
            approval: approval.clone(),
        })
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
        mut approval: Approval,
        decision: ApprovalDecision,
        resolved_at_ms: u64,
    ) -> Result<(Approval, ProtocolEvent), ProtocolError> {
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
        mut approval: Approval,
        resolved_at_ms: u64,
    ) -> (Approval, ProtocolEvent) {
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
            CodexAppServerError::RejectedMessage { error, .. } => Self::error(*error),
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
            | CodexAppServerError::Io(message) => {
                protocol_error("provider_unavailable", message, true)
            }
            CodexAppServerError::Timeout(message) => protocol_error("provider_request_timeout", message, true),
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
            CodexNotification::ProjectChanged {
                project_id,
                change_type,
            } => ProtocolEvent::EventProjectChanged {
                jsonrpc: "2.0".to_string(),
                params: ProjectChangedEvent {
                    project: self.provider_resource(project_id),
                    change_type: match change_type {
                        CodexProjectChangeType::Created => ProjectChangeType::Created,
                        CodexProjectChangeType::Updated => ProjectChangeType::Updated,
                        CodexProjectChangeType::Deleted => ProjectChangeType::Deleted,
                    },
                },
            },
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
            CodexNotification::ItemUpserted {
                thread_id,
                turn_id,
                turn_status,
                item,
            } => {
                let turn = CodexTurn {
                    id: turn_id,
                    status: turn_status,
                    started_at: None,
                    completed_at: None,
                    items_view: crate::protocol::CodexTurnItemsView::Full,
                    items: Vec::new(),
                };
                ProtocolEvent::EventConversationItemUpserted {
                    jsonrpc: "2.0".to_string(),
                    params: ConversationItemUpsertedEvent {
                        item: self.conversation_item(&turn, &item, &self.resource(thread_id)),
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
                    turn: self.provider_resource(turn_id),
                    conversation: self.provider_resource(thread_id),
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
            CodexNotification::ThreadNameUpdated { .. } => return Ok(Vec::new()),
            CodexNotification::ThreadStatusChanged { .. } => return Ok(Vec::new()),
            CodexNotification::Unknown { .. } => return Ok(Vec::new()),
        };
        Ok(vec![event])
    }

    pub fn approval(&self, request: &CodexApprovalRequest) -> Approval {
        let decisions = approval_decisions(request);
        Approval {
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
        }
    }

    fn resource(&self, native_resource_id: String) -> RoutedResourceId {
        RoutedResourceId {
            provider_id: self.route.provider_instance_id.clone(),
            native_resource_id,
        }
    }

    fn provider_resource(&self, native_resource_id: String) -> ProviderResourceId {
        ProviderResourceId {
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

fn tool_invocation(id: &str, details: &crate::protocol::CodexToolDetails) -> ToolInvocation {
    let serialized_input = details.input.to_string();
    let input = match details.input.as_object() {
        Some(object) => ToolInput::StructuredToolInput(StructuredToolInput {
            kind: StructuredToolInputKind::Structured,
            value: object.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
            truncation: None,
        }),
        None => ToolInput::OpaqueToolInput(OpaqueToolInput {
            kind: OpaqueToolInputKind::Opaque,
            value: serialized_input,
            mime_type: Some("application/json".to_string()),
            truncation: None,
        }),
    };
    let outcome = details.result.as_ref().map(|value| {
        let mut content = Vec::new();
        match value {
            Value::String(text) => {
                content.push(ContentBlock::OutputContentBlock(OutputContentBlock {
                    content_id: format!("{id}:result:0"),
                    kind: OutputContentBlockKind::Output,
                    text: text.clone(),
                    truncation: None,
                }));
            }
            Value::Object(object) => {
                content.push(ContentBlock::StructuredJsonContentBlock(StructuredJsonContentBlock {
                    content_id: format!("{id}:result:0"),
                    kind: StructuredJsonContentBlockKind::StructuredJson,
                    value: object.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
                    truncation: None,
                }));
            }
            other => {
                content.push(ContentBlock::OutputContentBlock(OutputContentBlock {
                    content_id: format!("{id}:result:0"),
                    kind: OutputContentBlockKind::Output,
                    text: other.to_string(),
                    truncation: None,
                }));
            }
        }
        ToolOutcome::ToolSuccessOutcome(ToolSuccessOutcome {
            kind: ToolSuccessOutcomeKind::Success,
            content,
            exit_code: None,
            process_id: None,
        })
    });
    ToolInvocation {
        call_id: id.to_string(),
        name: details.name.clone(),
        namespace: details.namespace.clone(),
        category: tool_category(&details.name),
        origin: ToolOrigin {
            kind: match details.origin_kind.as_str() {
                "mcp" => ToolOriginKind::Mcp,
                "plugin" => ToolOriginKind::Plugin,
                "server" => ToolOriginKind::Server,
                "custom" => ToolOriginKind::Custom,
                "builtin" => ToolOriginKind::Builtin,
                _ => ToolOriginKind::Unknown,
            },
            name: details.origin_name.clone(),
        },
        input,
        outcome,
        timing: None,
        annotations: None,
    }
}

fn opaque_tool_invocation(id: &str, name: &str) -> ToolInvocation {
    ToolInvocation {
        call_id: id.to_string(),
        name: name.to_string(),
        namespace: None,
        category: tool_category(name),
        origin: ToolOrigin { kind: ToolOriginKind::Unknown, name: None },
        input: ToolInput::OpaqueToolInput(OpaqueToolInput {
            kind: OpaqueToolInputKind::Opaque,
            value: String::new(),
            mime_type: None,
            truncation: None,
        }),
        outcome: None,
        timing: None,
        annotations: None,
    }
}

fn tool_category(name: &str) -> ToolCategory {
    let normalized = name.to_ascii_lowercase();
    if normalized.contains("search") || normalized.contains("find") {
        ToolCategory::Search
    } else if normalized.contains("read") || normalized.contains("list") {
        ToolCategory::Read
    } else if normalized.contains("write") || normalized.contains("edit") || normalized.contains("patch") {
        ToolCategory::Write
    } else if normalized.contains("web") || normalized.contains("browser") {
        ToolCategory::Web
    } else if normalized.contains("agent") || normalized.contains("task") {
        ToolCategory::Agent
    } else if normalized.contains("image") || normalized.contains("audio") {
        ToolCategory::Media
    } else {
        ToolCategory::Other
    }
}

fn command_action_kind(kind: &str) -> ToolCommandActionKind {
    match kind {
        "execute" => ToolCommandActionKind::Execute,
        "read" => ToolCommandActionKind::Read,
        "list" => ToolCommandActionKind::List,
        "search" => ToolCommandActionKind::Search,
        _ => ToolCommandActionKind::Unknown,
    }
}

fn command_title(command: &str, actions: &[crate::protocol::CodexCommandAction]) -> String {
    if let Some(action) = actions.first() {
        let target = action.path.as_deref().or(action.query.as_deref()).or(action.name.as_deref());
        if let Some(target) = target {
            let verb = match action.kind.as_str() {
                "read" => "Read",
                "list" => "List",
                "search" => "Search",
                _ => "Run",
            };
            return format!("{verb} {target}");
        }
    }
    for prefix in ["/bin/zsh -lc '", "/bin/bash -lc '", "zsh -lc '", "bash -lc '"] {
        if let Some(inner) = command.strip_prefix(prefix).and_then(|value| value.strip_suffix('\'')) {
            return concise_title(inner);
        }
    }
    concise_title(command)
}

fn concise_title(command: &str) -> String {
    const MAX_CHARS: usize = 80;
    let first_line = command.lines().next().unwrap_or(command).trim();
    if first_line.chars().count() <= MAX_CHARS {
        first_line.to_string()
    } else {
        format!("{}…", first_line.chars().take(MAX_CHARS - 1).collect::<String>())
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
    use std::collections::BTreeMap;

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
            false,
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
            false,
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
    fn project_capabilities_are_advertised_only_after_upstream_probe_succeeds() {
        let unsupported = CodexProtocolMapper::capabilities(
            "revision-test".to_string(),
            vec![test_model("model-a", true, "high", &["high"])],
            false,
        )
        .unwrap();
        assert!(!unsupported
            .methods
            .contains(&ProviderCapability::ProjectList));

        let supported = CodexProtocolMapper::capabilities(
            "revision-test".to_string(),
            vec![test_model("model-a", true, "high", &["high"])],
            true,
        )
        .unwrap();
        for capability in [
            ProviderCapability::ProjectList,
            ProviderCapability::ProjectGet,
            ProviderCapability::ProjectCreate,
            ProviderCapability::ProjectUpdate,
            ProviderCapability::ProjectDelete,
        ] {
            assert!(supported.methods.contains(&capability));
        }
    }

    #[test]
    fn project_and_project_changed_notification_preserve_routed_identity() {
        let mapper = CodexProtocolMapper::new(ProviderInstanceRoute {
            device_id: "device-test".to_string(),
            provider_plugin_id: "dev.codepet.codex".to_string(),
            provider_instance_id: "codex".to_string(),
        });
        let project = mapper
            .project(CodexProject {
                id: "project-one".to_string(),
                name: "Project One".to_string(),
                roots: vec![crate::protocol::CodexProjectRoot {
                    path: "/fixture/project".to_string(),
                }],
                metadata: BTreeMap::from([("team".to_string(), "gateway".to_string())]),
                position: 7,
                created_at: 11,
                updated_at: 12,
            })
            .unwrap();
        assert_eq!(project.resource.native_resource_id, "project-one");
        assert_eq!(project.resource.provider_id, "codex");
        assert_eq!(project.position, 7);
        assert_eq!(project.metadata["team"], "gateway");

        let events = mapper
            .events(CodexIncoming::Notification(CodexNotification::ProjectChanged {
                project_id: "project-one".to_string(),
                change_type: CodexProjectChangeType::Updated,
            }))
            .unwrap();
        assert!(matches!(
            &events[0],
            ProtocolEvent::EventProjectChanged { params, .. }
                if params.project.native_resource_id == "project-one"
                    && params.project.device_id == "device-test"
                    && params.project.provider_plugin_id == "dev.codepet.codex"
                    && params.project.provider_instance_id == "codex"
                    && params.change_type == ProjectChangeType::Updated
        ));
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
                    cwd: Some("/workspace".to_string()),
                    duration_ms: Some(42),
                    exit_code: Some(0),
                    process_id: Some("123".to_string()),
                    command_actions: vec![crate::protocol::CodexCommandAction {
                        kind: "execute".to_string(),
                        command: "cargo test".to_string(),
                        name: None,
                        path: None,
                        query: None,
                    }],
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
                    details: None,
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
                .map(item_resource_id)
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
        assert_eq!(item_content_ids(&items[0]), vec!["user-one:input:0"]);
        assert_eq!(item_content_ids(&items[1]), vec!["agent-one:text"]);
        assert_eq!(item_content_ids(&items[2]), vec!["reasoning-one:summary:0"]);
        let ConversationItem::CommandConversationItem(command) = &items[3] else { panic!("command item") };
        assert_eq!(command.title.as_deref(), Some("cargo test"));
        let command_tool = &command.tool;
        assert_eq!(command_tool.name, "shell");
        let ToolInput::CommandToolInput(input) = &command_tool.input else { panic!("command input") };
        assert_eq!(input.command, "cargo test");
        assert_eq!(input.cwd.as_deref(), Some("/workspace"));
        assert!(input.truncation.is_none());
        assert_eq!(command_tool.timing.as_ref().unwrap().duration_ms, Some(42));
        let Some(ToolOutcome::ToolSuccessOutcome(outcome)) = &command_tool.outcome else { panic!("success outcome") };
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(item_content_ids(&items[3]), vec!["command-one:output"]);
        assert!(matches!(items[6], ConversationItem::UnknownConversationItem(_)));
        assert!(items.iter().all(|item| item_conversation_id(item) == "thread-history"));
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
                    cwd: None,
                    duration_ms: None,
                    exit_code: None,
                    process_id: None,
                    command_actions: Vec::new(),
                },
            ],
        );

        let items = mapper.conversation_items(&snapshot, &[]);

        assert!(item_content_ids(&items[0]).is_empty());
        assert!(item_content_ids(&items[1]).is_empty());
        let ConversationItem::CommandConversationItem(command) = &items[2] else { panic!("command item") };
        assert_eq!(command.status, ConversationItemStatus::Running);
        assert!(command.tool.outcome.is_none());
        let committed_content_ids = items
            .iter()
            .flat_map(item_content_ids)
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
                cwd: None,
                duration_ms: None,
                exit_code: None,
                process_id: None,
                command_actions: Vec::new(),
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
        let ConversationItem::ApprovalConversationItem(item) = &items[1] else { panic!("approval item") };
        assert_eq!(item.approval, approval);
        assert_eq!(
            item
                .related_item
                .as_ref()
                .unwrap()
                .native_resource_id,
            "command-one"
        );
    }

    fn item_resource_id(item: &ConversationItem) -> &str {
        match item {
            ConversationItem::MessageConversationItem(value) => &value.resource.native_resource_id,
            ConversationItem::ReasoningConversationItem(value) => &value.resource.native_resource_id,
            ConversationItem::CommandConversationItem(value) => &value.resource.native_resource_id,
            ConversationItem::FileChangeConversationItem(value) => &value.resource.native_resource_id,
            ConversationItem::ToolConversationItem(value) => &value.resource.native_resource_id,
            ConversationItem::ApprovalConversationItem(value) => &value.resource.native_resource_id,
            ConversationItem::UnknownConversationItem(value) => &value.resource.native_resource_id,
        }
    }

    fn item_conversation_id(item: &ConversationItem) -> &str {
        match item {
            ConversationItem::MessageConversationItem(value) => &value.conversation.native_resource_id,
            ConversationItem::ReasoningConversationItem(value) => &value.conversation.native_resource_id,
            ConversationItem::CommandConversationItem(value) => &value.conversation.native_resource_id,
            ConversationItem::FileChangeConversationItem(value) => &value.conversation.native_resource_id,
            ConversationItem::ToolConversationItem(value) => &value.conversation.native_resource_id,
            ConversationItem::ApprovalConversationItem(value) => &value.conversation.native_resource_id,
            ConversationItem::UnknownConversationItem(value) => &value.conversation.native_resource_id,
        }
    }

    fn item_content_ids(item: &ConversationItem) -> Vec<&str> {
        let contents = match item {
            ConversationItem::MessageConversationItem(value) => &value.contents,
            ConversationItem::ReasoningConversationItem(value) => &value.contents,
            ConversationItem::FileChangeConversationItem(value) => &value.contents,
            ConversationItem::CommandConversationItem(value) => match &value.tool.outcome {
                Some(ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
                Some(ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
                None => return Vec::new(),
            },
            ConversationItem::ToolConversationItem(value) => match &value.tool.outcome {
                Some(ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
                Some(ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
                None => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        contents.iter().map(|content| match content {
            ContentBlock::TextContentBlock(value) => value.content_id.as_str(),
            ContentBlock::ReasoningSummaryContentBlock(value) => value.content_id.as_str(),
            ContentBlock::OutputContentBlock(value) => value.content_id.as_str(),
            ContentBlock::ActivitySummaryContentBlock(value) => value.content_id.as_str(),
            ContentBlock::StructuredJsonContentBlock(value) => value.content_id.as_str(),
            ContentBlock::ImageContentBlock(value) => value.content_id.as_str(),
            ContentBlock::AudioContentBlock(value) => value.content_id.as_str(),
            ContentBlock::ResourceLinkContentBlock(value) => value.content_id.as_str(),
            ContentBlock::EmbeddedResourceContentBlock(value) => value.content_id.as_str(),
        }).collect()
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
            project_id: None,
            session_id: "session-history".to_string(),
            source: json!("appServer"),
        });
        snapshot.workspace_root = Some("/fixture".to_string());
        snapshot
    }

    #[test]
    fn conversation_uses_agent_identity_and_preserves_workspace_root() {
        let native_cwd = "/fixture/project/nested/..".to_string();
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
            project_id: None,
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
            Some(native_cwd.clone())
        );
        assert_eq!(conversation.resource.native_resource_id, "thread-stable");
        assert_eq!(
            conversation.resource.provider_id,
            "codex-stable"
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
            project_id: None,
            session_id: "session-test".to_string(),
            source: json!("appServer"),
        })
    }
}
