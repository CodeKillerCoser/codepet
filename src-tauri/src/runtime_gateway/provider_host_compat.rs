use codepet_desktop_sdk as compat;
use codepet_gateway_sdk::{self as gateway, ProtocolServer as GatewayProtocolServer};
use codepet_host::{GatewayEventSubscription, ProviderGatewayService};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

const ROUTE_EXTENSION_NAMESPACE: &str = "codepet.gateway.route";
const TURN_SEND_CALLER_SCOPE: &str = "tauri-desktop-runtime-v0";
const COMPAT_DATA_UNREPRESENTABLE: &str = "compat_data_unrepresentable";

#[derive(Clone)]
pub struct CompatProviderGateway {
    gateway: Option<Arc<ProviderGatewayService>>,
}

impl CompatProviderGateway {
    pub fn new(gateway: Option<Arc<ProviderGatewayService>>) -> Self {
        Self { gateway }
    }

    pub async fn request(&self, request: compat::ProtocolRequest) -> compat::ProtocolResponse {
        compat::dispatch(self, request).await
    }

    pub fn replay(
        &self,
        after_event_sequence: Option<compat::EventSequence>,
    ) -> Result<Vec<compat::ProtocolEvent>, compat::ProtocolError> {
        let gateway = self.gateway()?;
        gateway
            .replay_events(after_event_sequence.map(event_cursor).as_deref())
            .map_err(map_error)?
            .into_iter()
            .filter_map(|event| self.map_event(event).transpose())
            .collect()
    }

    pub fn subscribe_current(&self) -> Result<CompatEventSubscription, compat::ProtocolError> {
        let gateway = self.gateway()?;
        let cursor = gateway.current_event_cursor();
        Ok(CompatEventSubscription {
            subscription: gateway
                .subscribe_events(Some(&cursor))
                .map_err(map_error)?,
            mapper: self.clone(),
        })
    }

    fn gateway(&self) -> Result<&Arc<ProviderGatewayService>, compat::ProtocolError> {
        self.gateway.as_ref().ok_or_else(|| {
            compat_error(
                "provider_host_unavailable",
                "Provider Host Gateway is unavailable".to_string(),
                true,
            )
        })
    }

    async fn provider_route(
        &self,
        provider_id: &str,
    ) -> Result<gateway::GatewayProviderRoute, compat::ProtocolError> {
        if provider_id.trim().is_empty() {
            return Err(compat_error(
                "unknown_provider",
                "Provider id must not be empty".to_string(),
                false,
            ));
        }
        self.gateway()?
            .resolve_provider_route(provider_id)
            .await
            .map_err(map_error)
    }

    fn map_event(
        &self,
        event: gateway::ProtocolEvent,
    ) -> Result<Option<compat::ProtocolEvent>, compat::ProtocolError> {
        let mapped = match event {
            gateway::ProtocolEvent::ProjectChanged { .. }
            | gateway::ProtocolEvent::ProviderChanged { .. }
            | gateway::ProtocolEvent::ConversationActivityChanged { .. }
            | gateway::ProtocolEvent::ConversationItemUpserted { .. } => return Ok(None),
            gateway::ProtocolEvent::ConversationUpserted { params, .. } => compat::ProtocolEvent::ConversationUpserted {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&params.event_cursor)?,
                payload: compat::ConversationUpsertedEvent {
                    conversation: map_conversation(params.payload.conversation)?,
                },
            },
            gateway::ProtocolEvent::TurnUpserted { params, .. } => compat::ProtocolEvent::TurnUpserted {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&params.event_cursor)?,
                payload: compat::TurnUpsertedEvent {
                    turn: map_turn(params.payload.turn)?,
                },
            },
            gateway::ProtocolEvent::TurnOutputDelta { params, .. } => {
                let event_cursor = params.event_cursor;
                let payload = params.payload;
                let route = route_extension(
                    &payload.turn.device_id,
                    &payload.turn.provider_plugin_id,
                    &payload.turn.provider_instance_id,
                    Some(&payload.turn.native_resource_id),
                );
                compat::ProtocolEvent::TurnOutputDelta {
                    protocol_version: compat::PROTOCOL_VERSION,
                    event_sequence: event_sequence(&event_cursor)?,
                    payload: compat::TurnOutputDeltaEvent {
                        provider_id: payload.turn.provider_instance_id.clone(),
                        conversation_id: payload.conversation.native_resource_id,
                        turn_id: payload.turn.native_resource_id,
                        output_id: payload.content_id,
                        kind: map_content_kind(payload.kind).to_string(),
                        delta: payload.delta,
                        extension: Some(route),
                    },
                }
            }
            gateway::ProtocolEvent::ApprovalRequested { params, .. } => compat::ProtocolEvent::ApprovalRequested {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&params.event_cursor)?,
                payload: compat::ApprovalRequestedEvent {
                    approval: map_approval(params.payload.approval),
                },
            },
            gateway::ProtocolEvent::ApprovalResolved { params, .. } => compat::ProtocolEvent::ApprovalResolved {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&params.event_cursor)?,
                payload: compat::ApprovalResolvedEvent {
                    approval: map_approval(params.payload.approval),
                },
            },
        };
        Ok(Some(mapped))
    }
}

fn map_content_kind(kind: gateway::ConversationContentKind) -> &'static str {
    match kind {
        gateway::ConversationContentKind::Text => "text",
        gateway::ConversationContentKind::ReasoningSummary => "reasoning-summary",
        gateway::ConversationContentKind::Command => "command",
        gateway::ConversationContentKind::Output => "output",
        gateway::ConversationContentKind::ActivitySummary => "activity-summary",
    }
}

pub struct CompatEventSubscription {
    subscription: GatewayEventSubscription,
    mapper: CompatProviderGateway,
}

impl CompatEventSubscription {
    pub async fn next_event(&mut self) -> Result<compat::ProtocolEvent, compat::ProtocolError> {
        loop {
            let event = self.subscription.next_event().await.map_err(map_error)?;
            if let Some(event) = map_subscription_event(&self.mapper, event)? {
                return Ok(event);
            }
        }
    }
}

fn map_subscription_event(
    mapper: &CompatProviderGateway,
    event: gateway::ProtocolEvent,
) -> Result<Option<compat::ProtocolEvent>, compat::ProtocolError> {
    match mapper.map_event(event) {
        Err(error) if error.code == COMPAT_DATA_UNREPRESENTABLE => Ok(None),
        result => result,
    }
}

impl compat::ProtocolServer for CompatProviderGateway {
    fn protocol_handshake<'a>(
        &'a self,
        request: compat::HandshakeRequest,
    ) -> compat::ProtocolFuture<'a, compat::HandshakeResponse> {
        Box::pin(async move {
            if request.min_protocol_version > compat::PROTOCOL_VERSION
                || request.max_protocol_version < compat::PROTOCOL_VERSION
            {
                return Err(compat_error(
                    "unsupported_protocol_version",
                    "compat Runtime Gateway protocol v0 is outside the client range".to_string(),
                    false,
                ));
            }
            if request.client_name.trim().is_empty() || request.client_version.trim().is_empty() {
                return Err(compat_error(
                    "invalid_gateway_client",
                    "Gateway client identity fields must not be empty".to_string(),
                    false,
                ));
            }
            let gateway = self.gateway()?;
            if let Some(sequence) = request.last_event_sequence {
                gateway
                    .replay_events(Some(&event_cursor(sequence)))
                    .map_err(map_error)?;
            }
            let providers = GatewayProtocolServer::provider_list(
                gateway.as_ref(),
                gateway::ProviderListRequest {},
            )
            .await
            .map_err(map_error)?
            .providers;
            let mut mapped_providers = Vec::new();
            for provider in providers {
                let route = gateway.resolve_provider_route(&provider.id).await.map_err(map_error)?;
                let description = GatewayProtocolServer::provider_describe(
                    gateway.as_ref(),
                    gateway::ProviderDescribeRequest { id: provider.id.clone() },
                ).await.map_err(map_error)?;
                mapped_providers.push(map_provider(provider, route, description.capabilities));
            }
            Ok(compat::HandshakeResponse {
                protocol_version: compat::PROTOCOL_VERSION,
                server_name: gateway.server_name().to_string(),
                server_version: gateway.server_version().to_string(),
                providers: mapped_providers,
                event_sequence: event_sequence(&gateway.current_event_cursor())?,
            })
        })
    }

    fn provider_list<'a>(
        &'a self,
        _request: compat::ProviderListRequest,
    ) -> compat::ProtocolFuture<'a, compat::ProviderListResponse> {
        Box::pin(async move {
            let response = GatewayProtocolServer::provider_list(
                self.gateway()?.as_ref(),
                gateway::ProviderListRequest {},
            )
            .await
            .map_err(map_error)?;
            let gateway = self.gateway()?;
            let mut providers = Vec::new();
            for provider in response.providers {
                let route = gateway.resolve_provider_route(&provider.id).await.map_err(map_error)?;
                let description = GatewayProtocolServer::provider_describe(
                    gateway.as_ref(),
                    gateway::ProviderDescribeRequest { id: provider.id.clone() },
                ).await.map_err(map_error)?;
                providers.push(map_provider(provider, route, description.capabilities));
            }
            Ok(compat::ProviderListResponse { providers })
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: compat::ConversationListRequest,
    ) -> compat::ProtocolFuture<'a, compat::ConversationListResponse> {
        Box::pin(async move {
            let route = match request.provider_id.as_deref() {
                Some(provider_id) => Some(self.provider_route(provider_id).await?),
                None => None,
            };
            let response = GatewayProtocolServer::conversation_list(
                self.gateway()?.as_ref(),
                gateway::ConversationListRequest {
                    route,
                    cursor: request.cursor,
                    limit: request.limit,
                    project_filter: gateway::ConversationProjectFilter::ConversationProjectFilterAll(
                        gateway::ConversationProjectFilterAll {
                            kind: gateway::ConversationProjectFilterAllKind::All,
                        },
                    ),
                },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::ConversationListResponse {
                conversations: response
                    .conversations
                    .into_iter()
                    .map(map_conversation)
                    .collect::<Result<Vec<_>, compat::ProtocolError>>()?,
                next_cursor: response.page_info.next_cursor,
                event_sequence: event_sequence(&response.snapshot_cursor)?,
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: compat::ConversationGetRequest,
    ) -> compat::ProtocolFuture<'a, compat::ConversationGetResponse> {
        Box::pin(async move {
            let provider = self.provider_route(&request.provider_id).await?;
            let response = GatewayProtocolServer::conversation_get(
                self.gateway()?.as_ref(),
                gateway::ConversationGetRequest {
                    conversation: routed_resource(provider, request.conversation_id),
                    cursor: None,
                    limit: None,
                },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::ConversationGetResponse {
                conversation: map_conversation(response.conversation)?,
            })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: compat::ConversationCreateRequest,
    ) -> compat::ProtocolFuture<'a, compat::ConversationCreateResponse> {
        Box::pin(async move {
            let provider = self.provider_route(&request.provider_id).await?;
            let response = GatewayProtocolServer::conversation_create(
                self.gateway()?.as_ref(),
                gateway::ConversationCreateRequest {
                    route: provider,
                    project: None,
                    title: request.title,
                    permission_level: permission_level_name(request.permission_level).to_string(),
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                    workspace_root: request.workspace_root,
                    workspace_mode: None,
                },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::ConversationCreateResponse {
                conversation: map_conversation(response.conversation)?,
            })
        })
    }

    fn turn_send<'a>(
        &'a self,
        request: compat::TurnSendRequest,
    ) -> compat::ProtocolFuture<'a, compat::TurnSendResponse> {
        Box::pin(async move {
            if request.quick_reply_id.is_some() {
                return Err(compat_error(
                    "capability_unsupported",
                    "Provider Gateway does not advertise quick replies".to_string(),
                    false,
                ));
            }
            if request.steer_turn_id.is_some() {
                return Err(compat_error(
                    "capability_unsupported",
                    "Gateway v1 turn.send starts a new turn and does not steer an active turn"
                        .to_string(),
                    false,
                ));
            }
            let provider = self.provider_route(&request.provider_id).await?;
            let description = GatewayProtocolServer::provider_describe(
                self.gateway()?.as_ref(),
                gateway::ProviderDescribeRequest { id: request.provider_id.clone() },
            )
            .await
            .map_err(map_error)?;
            let conversation = routed_resource(provider, request.conversation_id);
            let response = self.gateway()?.turn_send_for_caller_scope(
                TURN_SEND_CALLER_SCOPE,
                gateway::TurnSendRequest {
                    conversation,
                    client_request_id: request.client_message_id,
                    capability_revision: description.capabilities.revision,
                    input: gateway::TurnInput {
                        kind: gateway::TurnInputKind::Text,
                        text: request.message,
                    },
                    selection: gateway::TurnSelection {
                        access_mode_id: None,
                        reasoning_effort_id: None,
                        model: None,
                    },
                },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::TurnSendResponse {
                turn: map_turn(response.turn)?,
            })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: compat::TurnInterruptRequest,
    ) -> compat::ProtocolFuture<'a, compat::TurnInterruptResponse> {
        Box::pin(async move {
            let provider = self.provider_route(&request.provider_id).await?;
            let route = provider;
            let response = GatewayProtocolServer::turn_interrupt(
                self.gateway()?.as_ref(),
                gateway::TurnInterruptRequest {
                    conversation: routed_resource(
                        route.clone(),
                        request.conversation_id,
                    ),
                    turn: routed_resource(route, request.turn_id),
                },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::TurnInterruptResponse {
                turn: map_turn(response.turn)?,
            })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: compat::ApprovalResolveRequest,
    ) -> compat::ProtocolFuture<'a, compat::ApprovalResolveResponse> {
        Box::pin(async move {
            let provider = self.provider_route(&request.provider_id).await?;
            let response = GatewayProtocolServer::approval_resolve(
                self.gateway()?.as_ref(),
                gateway::ApprovalResolveRequest {
                    approval: routed_resource(provider, request.approval_id),
                    decision: match request.decision {
                        compat::ApprovalDecision::Approve => gateway::ApprovalDecision::Approve,
                        compat::ApprovalDecision::Deny => gateway::ApprovalDecision::Deny,
                    },
                },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::ApprovalResolveResponse {
                approval: map_approval(response.approval),
            })
        })
    }
}

fn map_provider(
    provider: gateway::ProviderSummary,
    route: gateway::GatewayProviderRoute,
    capabilities: gateway::GatewayCapabilities,
) -> compat::Provider {
    let provider_id = provider.id.clone();
    let methods = capabilities
        .methods
        .iter()
        .filter_map(|method| match method {
            gateway::GatewayCapability::ProjectList
            | gateway::GatewayCapability::ProjectGet
            | gateway::GatewayCapability::ProjectCreate
            | gateway::GatewayCapability::ProjectUpdate
            | gateway::GatewayCapability::ProjectDelete => None,
            gateway::GatewayCapability::ConversationList => Some("conversation.list"),
            gateway::GatewayCapability::ConversationSearch => None,
            gateway::GatewayCapability::ConversationGet => Some("conversation.get"),
            gateway::GatewayCapability::ConversationCreate => Some("conversation.create"),
            gateway::GatewayCapability::TurnSend => Some("turn.send"),
            gateway::GatewayCapability::TurnInterrupt => Some("turn.interrupt"),
            gateway::GatewayCapability::ApprovalResolve => Some("approval.resolve"),
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    let can_interrupt = capabilities
        .methods
        .contains(&gateway::GatewayCapability::TurnInterrupt);
    let turn_send = capabilities.turn_send.as_ref();
    let permission_levels = turn_send
        .and_then(|capabilities| capabilities.access_mode.as_ref())
        .map(|choices| {
            choices
                .options
                .iter()
                .filter(|option| option.enabled != Some(false))
                .filter_map(|option| map_permission_level(&option.id))
                .collect()
        })
        .unwrap_or_default();
    let models = turn_send
        .and_then(|capabilities| capabilities.model_catalog.as_ref())
        .and_then(|catalog| match catalog {
            gateway::ModelCatalog::FlatModelCatalog(catalog) => Some(
                catalog
                    .models
                    .iter()
                    .filter(|option| option.enabled != Some(false))
                    .map(|option| option.id.clone())
                    .collect(),
            ),
            gateway::ModelCatalog::GroupedModelCatalog(_) => None,
        })
        .unwrap_or_default();
    let reasoning_efforts = turn_send
        .and_then(|capabilities| capabilities.reasoning_effort.as_ref())
        .map(|choices| {
            choices
                .options
                .iter()
                .filter(|option| option.enabled != Some(false))
                .map(|option| option.id.clone())
                .collect()
        })
        .unwrap_or_default();
    compat::Provider {
        id: provider_id,
        provider_type: route.provider_plugin_id.clone(),
        display_name: provider.identity.display_name,
        version: provider.runtime.version,
        status: map_provider_status(provider.runtime.status),
        capabilities: compat::ProviderCapabilities {
            methods,
            permission_levels,
            models,
            reasoning_efforts,
            quick_replies: Vec::new(),
            can_steer: false,
            can_interrupt,
            extension: None,
        },
        extension: Some(route_extension(
            &route.device_id,
            &route.provider_plugin_id,
            &route.provider_instance_id,
            None,
        )),
    }
}

fn map_conversation(
    conversation: gateway::Conversation,
) -> Result<compat::Conversation, compat::ProtocolError> {
    let provider_id = conversation.resource.provider_instance_id.clone();
    let route = route_extension(
        &conversation.resource.device_id,
        &conversation.resource.provider_plugin_id,
        &conversation.resource.provider_instance_id,
        Some(&conversation.resource.native_resource_id),
    );
    let permission_level = conversation
        .permission_level
        .as_deref()
        .and_then(map_permission_level)
        .ok_or_else(|| {
            compat_error(
                COMPAT_DATA_UNREPRESENTABLE,
                "compat Runtime Gateway requires a known conversation permission level"
                    .to_string(),
                false,
            )
        })?;
    let created_at = conversation.created_at.ok_or_else(|| {
        compat_error(
            COMPAT_DATA_UNREPRESENTABLE,
            "compat Runtime Gateway requires a confirmed conversation createdAt".to_string(),
            false,
        )
    })?;
    let updated_at = conversation.updated_at.ok_or_else(|| {
        compat_error(
            COMPAT_DATA_UNREPRESENTABLE,
            "compat Runtime Gateway requires a confirmed conversation updatedAt".to_string(),
            false,
        )
    })?;
    let active_turn = conversation
        .active_turn
        .map(map_turn)
        .transpose()?;
    let status = match conversation.status {
        gateway::ConversationStatus::Idle => compat::ConversationStatus::Idle,
        gateway::ConversationStatus::Running => compat::ConversationStatus::Running,
        gateway::ConversationStatus::WaitingApproval => {
            compat::ConversationStatus::WaitingApproval
        }
        gateway::ConversationStatus::WaitingUserInput => {
            return Err(compat_error(
                COMPAT_DATA_UNREPRESENTABLE,
                "compat Runtime Gateway cannot represent waiting-user-input".to_string(),
                false,
            ));
        }
        gateway::ConversationStatus::Error => compat::ConversationStatus::Error,
        gateway::ConversationStatus::Archived => compat::ConversationStatus::Archived,
    };
    Ok(compat::Conversation {
        id: conversation.resource.native_resource_id,
        provider_id,
        title: conversation.title,
        preview: conversation.preview,
        status,
        permission_level,
        model: conversation.model,
        reasoning_effort: conversation.reasoning_effort,
        workspace_root: conversation.workspace_root,
        created_at,
        updated_at,
        active_turn,
        extension: Some(route),
    })
}

fn map_turn(turn: gateway::TurnTask) -> Result<compat::TurnTask, compat::ProtocolError> {
    let route = route_extension(
        &turn.resource.device_id,
        &turn.resource.provider_plugin_id,
        &turn.resource.provider_instance_id,
        Some(&turn.resource.native_resource_id),
    );
    let updated_at = turn.updated_at.ok_or_else(|| {
        compat_error(
            COMPAT_DATA_UNREPRESENTABLE,
            "compat Runtime Gateway requires a confirmed turn updatedAt".to_string(),
            false,
        )
    })?;
    Ok(compat::TurnTask {
        id: turn.resource.native_resource_id,
        provider_id: turn.resource.provider_instance_id.clone(),
        conversation_id: turn.conversation.native_resource_id,
        status: match turn.status {
            gateway::TurnStatus::Queued => compat::TurnTaskStatus::Queued,
            gateway::TurnStatus::Running => compat::TurnTaskStatus::Running,
            gateway::TurnStatus::WaitingApproval => compat::TurnTaskStatus::WaitingApproval,
            gateway::TurnStatus::Completed => compat::TurnTaskStatus::Completed,
            gateway::TurnStatus::Failed => compat::TurnTaskStatus::Failed,
            gateway::TurnStatus::Interrupted => compat::TurnTaskStatus::Interrupted,
        },
        display_summary: turn.display_summary,
        started_at: turn.started_at,
        updated_at,
        completed_at: turn.completed_at,
        extension: Some(route),
    })
}

fn map_approval(approval: gateway::Approval) -> compat::Approval {
    let route = route_extension(
        &approval.resource.device_id,
        &approval.resource.provider_plugin_id,
        &approval.resource.provider_instance_id,
        Some(&approval.resource.native_resource_id),
    );
    compat::Approval {
        id: approval.resource.native_resource_id,
        provider_id: approval.resource.provider_instance_id.clone(),
        conversation_id: approval.conversation.native_resource_id,
        turn_id: approval.turn.native_resource_id,
        kind: approval.kind,
        title: approval.title,
        description: approval.description,
        status: match approval.status {
            gateway::ApprovalStatus::Pending => compat::ApprovalStatus::Pending,
            gateway::ApprovalStatus::Approved => compat::ApprovalStatus::Approved,
            gateway::ApprovalStatus::Denied => compat::ApprovalStatus::Denied,
            gateway::ApprovalStatus::Expired => compat::ApprovalStatus::Expired,
        },
        decisions: approval
            .decisions
            .into_iter()
            .map(|decision| match decision {
                gateway::ApprovalDecision::Approve => compat::ApprovalDecision::Approve,
                gateway::ApprovalDecision::Deny => compat::ApprovalDecision::Deny,
            })
            .collect(),
        requested_at: approval.requested_at,
        resolved_at: approval.resolved_at,
        decision: approval.decision.map(|decision| match decision {
            gateway::ApprovalDecision::Approve => compat::ApprovalDecision::Approve,
            gateway::ApprovalDecision::Deny => compat::ApprovalDecision::Deny,
        }),
        extension: Some(route),
    }
}

fn map_provider_status(status: gateway::ProviderStatus) -> compat::ProviderStatus {
    match status {
        gateway::ProviderStatus::Stopped => compat::ProviderStatus::Disconnected,
        gateway::ProviderStatus::Connecting => compat::ProviderStatus::Connecting,
        gateway::ProviderStatus::Ready => compat::ProviderStatus::Ready,
        gateway::ProviderStatus::Unavailable => compat::ProviderStatus::Unavailable,
        gateway::ProviderStatus::Error => compat::ProviderStatus::Error,
    }
}

fn map_permission_level(level: &str) -> Option<compat::PermissionLevel> {
    match level {
        "read-only" => Some(compat::PermissionLevel::ReadOnly),
        "workspace-write" => Some(compat::PermissionLevel::WorkspaceWrite),
        "full-access" => Some(compat::PermissionLevel::FullAccess),
        _ => None,
    }
}

fn permission_level_name(level: compat::PermissionLevel) -> &'static str {
    match level {
        compat::PermissionLevel::ReadOnly => "read-only",
        compat::PermissionLevel::WorkspaceWrite => "workspace-write",
        compat::PermissionLevel::FullAccess => "full-access",
    }
}

fn routed_resource(
    route: gateway::GatewayProviderRoute,
    native_resource_id: String,
) -> gateway::RoutedResourceId {
    gateway::RoutedResourceId {
        device_id: route.device_id,
        provider_plugin_id: route.provider_plugin_id,
        provider_instance_id: route.provider_instance_id,
        native_resource_id,
    }
}

fn route_extension(
    device_id: &str,
    plugin_id: &str,
    provider_instance_id: &str,
    native_resource_id: Option<&str>,
) -> compat::ProviderExtension {
    let mut data = BTreeMap::new();
    data.insert("deviceId".to_string(), json!(device_id));
    data.insert(
        "providerInstanceId".to_string(),
        json!(provider_instance_id),
    );
    data.insert("providerPluginId".to_string(), json!(plugin_id));
    if let Some(native_resource_id) = native_resource_id {
        data.insert("nativeResourceId".to_string(), json!(native_resource_id));
    }
    compat::ProviderExtension {
        namespace: ROUTE_EXTENSION_NAMESPACE.to_string(),
        data,
    }
}

fn event_cursor(sequence: u64) -> gateway::EventCursor {
    format!("event-{sequence:020}")
}

fn event_sequence(cursor: &str) -> Result<u64, compat::ProtocolError> {
    let sequence = cursor
        .strip_prefix("event-")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            compat_error(
                "invalid_event_cursor",
                format!("invalid Provider Gateway event cursor: {cursor}"),
                false,
            )
        })?;
    if event_cursor(sequence) != cursor {
        return Err(compat_error(
            "invalid_event_cursor",
            format!("non-canonical Provider Gateway event cursor: {cursor}"),
            false,
        ));
    }
    Ok(sequence)
}

fn map_error(error: gateway::ProtocolError) -> compat::ProtocolError {
    compat::ProtocolError {
        code: error.code,
        message: error.message,
        retryable: error.retryable,
        details: error.details,
    }
}

fn compat_error(code: &str, message: String, retryable: bool) -> compat::ProtocolError {
    compat::ProtocolError {
        code: code.to_string(),
        message,
        retryable,
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(native_resource_id: &str) -> gateway::RoutedResourceId {
        gateway::RoutedResourceId {
            device_id: "device-test".to_string(),
            provider_plugin_id: "dev.codepet.test".to_string(),
            provider_instance_id: "instance-test".to_string(),
            native_resource_id: native_resource_id.to_string(),
        }
    }

    fn turn_event(event_cursor: &str, updated_at: Option<u64>) -> gateway::ProtocolEvent {
        gateway::ProtocolEvent::TurnUpserted {
            jsonrpc: "2.0".to_string(),
            params: gateway::ProtocolEventParams {
                event_cursor: event_cursor.to_string(),
                payload: gateway::TurnUpsertedEvent {
                    turn: gateway::TurnTask {
                        resource: resource("turn-test"),
                        conversation: resource("conversation-test"),
                        status: gateway::TurnStatus::Running,
                        display_summary: None,
                        started_at: Some(10),
                        updated_at,
                        completed_at: None,
                    },
                },
            },
        }
    }

    #[test]
    fn subscription_skips_unrepresentable_turn_and_maps_the_following_event() {
        let mapper = CompatProviderGateway::new(None);
        let missing_timestamp = turn_event("event-00000000000000000001", None);

        let mapping_error = mapper.map_event(missing_timestamp.clone()).unwrap_err();
        assert_eq!(mapping_error.code, COMPAT_DATA_UNREPRESENTABLE);
        assert!(map_subscription_event(&mapper, missing_timestamp)
            .unwrap()
            .is_none());

        let mapped = map_subscription_event(
            &mapper,
            turn_event("event-00000000000000000002", Some(20)),
        )
        .unwrap()
        .expect("the next representable event must still be emitted");
        let compat::ProtocolEvent::TurnUpserted {
            event_sequence,
            payload,
            ..
        } = mapped
        else {
            panic!("expected a compat turn.upserted event");
        };
        assert_eq!(event_sequence, 2);
        assert_eq!(payload.turn.updated_at, 20);
    }

    #[test]
    fn subscription_keeps_non_representation_mapping_errors_fatal() {
        let mapper = CompatProviderGateway::new(None);

        let error = map_subscription_event(
            &mapper,
            turn_event("event-2", Some(20)),
        )
        .unwrap_err();

        assert_eq!(error.code, "invalid_event_cursor");
    }
}
