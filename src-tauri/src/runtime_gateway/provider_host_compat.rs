use codepet_gateway_sdk::compat_v0 as compat;
use codepet_gateway_sdk::{self as gateway, ProtocolServer as GatewayProtocolServer};
use codepet_host::{GatewayEventSubscription, ProviderGatewayService};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

const CODEX_PLUGIN_ID: &str = "dev.codepet.codex";
const ROUTE_EXTENSION_NAMESPACE: &str = "codepet.gateway.route";

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
    ) -> Result<gateway::ProviderInstance, compat::ProtocolError> {
        if provider_id.trim().is_empty() {
            return Err(compat_error(
                "unknown_provider",
                "Provider id must not be empty".to_string(),
                false,
            ));
        }
        let providers = GatewayProtocolServer::provider_list(
            self.gateway()?.as_ref(),
            gateway::ProviderListRequest { device_id: None },
        )
        .await
        .map_err(map_error)?
        .providers;
        let matches = providers
            .into_iter()
            .filter(|provider| provider.route.provider_instance_id == provider_id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [provider] => Ok(provider.clone()),
            [] => Err(compat_error(
                "unknown_provider",
                format!("Provider instance is not registered: {provider_id}"),
                false,
            )),
            _ => Err(compat_error(
                "ambiguous_provider",
                format!("Provider instance id is not unique across devices: {provider_id}"),
                false,
            )),
        }
    }

    fn map_event(
        &self,
        event: gateway::ProtocolEvent,
    ) -> Result<Option<compat::ProtocolEvent>, compat::ProtocolError> {
        let mapped = match event {
            gateway::ProtocolEvent::DeviceStatusChanged { .. } => return Ok(None),
            gateway::ProtocolEvent::ProviderStatusChanged {
                event_cursor,
                payload,
                ..
            } => compat::ProtocolEvent::ProviderStatusChanged {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&event_cursor)?,
                payload: compat::ProviderStatusChangedEvent {
                    provider: map_provider(payload.provider),
                    previous_status: payload.previous_status.map(map_provider_status),
                },
            },
            gateway::ProtocolEvent::ConversationUpserted {
                event_cursor,
                payload,
                ..
            } => compat::ProtocolEvent::ConversationUpserted {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&event_cursor)?,
                payload: compat::ConversationUpsertedEvent {
                    conversation: map_conversation(payload.conversation)?,
                },
            },
            gateway::ProtocolEvent::TurnUpserted {
                event_cursor,
                payload,
                ..
            } => compat::ProtocolEvent::TurnUpserted {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&event_cursor)?,
                payload: compat::TurnUpsertedEvent {
                    turn: map_turn(payload.turn)?,
                },
            },
            gateway::ProtocolEvent::TurnOutputDelta {
                event_cursor,
                payload,
                ..
            } => {
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
            gateway::ProtocolEvent::ApprovalRequested {
                event_cursor,
                payload,
                ..
            } => compat::ProtocolEvent::ApprovalRequested {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&event_cursor)?,
                payload: compat::ApprovalRequestedEvent {
                    approval: map_approval(payload.approval),
                },
            },
            gateway::ProtocolEvent::ApprovalResolved {
                event_cursor,
                payload,
                ..
            } => compat::ProtocolEvent::ApprovalResolved {
                protocol_version: compat::PROTOCOL_VERSION,
                event_sequence: event_sequence(&event_cursor)?,
                payload: compat::ApprovalResolvedEvent {
                    approval: map_approval(payload.approval),
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
            if let Some(event) = self.mapper.map_event(event)? {
                return Ok(event);
            }
        }
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
                gateway::ProviderListRequest { device_id: None },
            )
            .await
            .map_err(map_error)?
            .providers;
            Ok(compat::HandshakeResponse {
                protocol_version: compat::PROTOCOL_VERSION,
                server_name: gateway.server_name().to_string(),
                server_version: gateway.server_version().to_string(),
                providers: providers.into_iter().map(map_provider).collect(),
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
                gateway::ProviderListRequest { device_id: None },
            )
            .await
            .map_err(map_error)?;
            Ok(compat::ProviderListResponse {
                providers: response.providers.into_iter().map(map_provider).collect(),
            })
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: compat::ConversationListRequest,
    ) -> compat::ProtocolFuture<'a, compat::ConversationListResponse> {
        Box::pin(async move {
            let route = match request.provider_id.as_deref() {
                Some(provider_id) => Some(self.provider_route(provider_id).await?.route),
                None => None,
            };
            let response = GatewayProtocolServer::conversation_list(
                self.gateway()?.as_ref(),
                gateway::ConversationListRequest {
                    route,
                    cursor: request.cursor,
                    limit: request.limit,
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
                    conversation: routed_resource(provider.route, request.conversation_id),
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
                    route: provider.route,
                    title: request.title,
                    permission_level: permission_level_name(request.permission_level).to_string(),
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                    workspace_root: request.workspace_root,
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
                    "Codex Provider does not advertise quick replies".to_string(),
                    false,
                ));
            }
            let provider = self.provider_route(&request.provider_id).await?;
            let route = provider.route;
            let conversation = routed_resource(route.clone(), request.conversation_id);
            let response = GatewayProtocolServer::turn_send(
                self.gateway()?.as_ref(),
                gateway::TurnSendRequest {
                    conversation,
                    client_message_id: request.client_message_id,
                    message: request.message,
                    steer_turn: request
                        .steer_turn_id
                        .map(|turn_id| routed_resource(route, turn_id)),
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
            let route = provider.route;
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
                    approval: routed_resource(provider.route, request.approval_id),
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

fn map_provider(provider: gateway::ProviderInstance) -> compat::Provider {
    let provider_id = provider.route.provider_instance_id.clone();
    let methods = provider
        .capabilities
        .methods
        .iter()
        .filter_map(|method| match method {
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
    let can_interrupt = provider
        .capabilities
        .methods
        .contains(&gateway::GatewayCapability::TurnInterrupt);
    let can_steer = provider.plugin_id == CODEX_PLUGIN_ID
        && provider
            .capabilities
            .methods
            .contains(&gateway::GatewayCapability::TurnSend);
    compat::Provider {
        id: provider_id,
        provider_type: provider.plugin_id.clone(),
        display_name: provider.display_name,
        version: provider.version,
        status: map_provider_status(provider.status),
        capabilities: compat::ProviderCapabilities {
            methods,
            permission_levels: provider
                .capabilities
                .permission_levels
                .iter()
                .filter_map(|level| map_permission_level(level))
                .collect(),
            models: provider.capabilities.models,
            reasoning_efforts: provider.capabilities.reasoning_efforts,
            quick_replies: Vec::new(),
            can_steer,
            can_interrupt,
            extension: None,
        },
        extension: Some(route_extension(
            &provider.route.device_id,
            &provider.route.provider_plugin_id,
            &provider.route.provider_instance_id,
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
                "compat_data_unrepresentable",
                "compat Runtime Gateway requires a known conversation permission level"
                    .to_string(),
                false,
            )
        })?;
    let created_at = conversation.created_at.ok_or_else(|| {
        compat_error(
            "compat_data_unrepresentable",
            "compat Runtime Gateway requires a confirmed conversation createdAt".to_string(),
            false,
        )
    })?;
    let updated_at = conversation.updated_at.ok_or_else(|| {
        compat_error(
            "compat_data_unrepresentable",
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
                "compat_data_unrepresentable",
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
            "compat_data_unrepresentable",
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
        gateway::ProviderStatus::Disconnected => compat::ProviderStatus::Disconnected,
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
