use super::*;

pub(super) fn gateway_plugin_visible(plugin: &PluginRuntimeSnapshot) -> bool {
    // Built-ins describe installed runtimes, not merely shipped plugin binaries.
    // Keep custom providers and installed-but-idle runtimes visible.
    !matches!(
        plugin.catalog.plugin_id.as_str(),
        "dev.codepet.codex" | "dev.codepet.claude" | "dev.codepet.opencode"
    ) || plugin
        .runtime_inventory
        .as_ref()
        .is_some_and(|inventory| !inventory.harness_list.is_empty() || inventory.selected.is_some())
}

pub(super) fn gateway_provider(
    plugin: &PluginRuntimeSnapshot,
    runtime: &ProviderInstanceRuntimeSnapshot,
) -> GatewayProviderRuntime {
    let capabilities = runtime
        .instance
        .as_ref()
        .map(|instance| map_capabilities(&instance.capabilities))
        .unwrap_or_else(|| {
            empty_gateway_capabilities(format!("provider-unavailable-{}", plugin.generation))
        });
    let harness_version = runtime
        .instance
        .as_ref()
        .and_then(|instance| instance.harness.version.clone());
    let executable_path = runtime
        .instance
        .as_ref()
        .and_then(|instance| instance.harness.executable_path.clone())
        .or_else(|| configured_executable_path(&runtime.record.settings));
    GatewayProviderRuntime {
        provider_ordered_ids: runtime
            .instance
            .as_ref()
            .and_then(|instance| instance.capabilities.conversation_list_query.as_ref())
            .is_some_and(|query| query.ordered_ids == Some(true)),
        provider_mark_read: runtime.instance.as_ref().is_some_and(|instance| {
            instance
                .capabilities
                .methods
                .contains(&provider::ProviderCapability::ConversationMarkRead)
        }),
        route: provider::ProviderInstanceRoute {
            device_id: runtime.record.device_id.clone(),
            provider_plugin_id: plugin.catalog.plugin_id.clone(),
            provider_instance_id: runtime.record.instance_id.clone(),
        },
        summary: gateway::ProviderSummary {
            plugin_id: Some(plugin.catalog.plugin_id.clone()),
            features: plugin.reported.as_ref().and_then(|p| p.features.clone()),
            id: runtime.record.instance_id.clone(),
            identity: gateway::ProviderIdentity {
                display_name: runtime.record.display_name.clone(),
                icon: Some(plugin.catalog.icon.clone()),
                default_workspace_root: plugin
                    .reported
                    .as_ref()
                    .and_then(|descriptor| descriptor.default_workspace_root.clone()),
            },
            runtime: gateway::ProviderRuntime {
                connection_status: Some(plugin.connection_status.clone()),
                generation: Some(plugin.generation),
                status: if plugin.catalog.enabled && runtime.record.enabled {
                    provider_runtime_status(plugin.state, runtime.instance.as_ref())
                } else {
                    gateway::ProviderStatus::Unavailable
                },
                version: harness_version,
                executable_path,
                authentication: runtime
                    .instance
                    .as_ref()
                    .and_then(|instance| instance.authentication.as_ref())
                    .cloned(),
                usage: runtime
                    .instance
                    .as_ref()
                    .and_then(|instance| instance.usage.as_ref())
                    .map(map_usage),
            },
            capabilities: gateway::ProviderCapabilitiesSummary {
                revision: capabilities.revision.clone(),
            },
        },
        capabilities,
    }
}

pub(super) fn configured_executable_path(settings: &provider::JsonObject) -> Option<String> {
    [
        "appServerExecutable",
        "claudeExecutable",
        "serverExecutable",
    ]
    .into_iter()
    .find_map(|key| settings.get(key).and_then(|value| value.as_str()))
    .filter(|path| !path.trim().is_empty())
    .map(ToOwned::to_owned)
}

pub(super) fn map_usage(usage: &provider::ProviderUsage) -> gateway::ProviderUsage {
    let mut total_bytes = 0usize;
    let details = usage.details.as_ref().map(|details| {
        details
            .iter()
            .take(MAX_USAGE_DETAILS)
            .filter_map(|detail| {
                let data = redact_usage_data(&detail.data);
                let serialized_bytes = serde_json::to_vec(&data).ok()?.len();
                if serialized_bytes > MAX_USAGE_DETAIL_BYTES
                    || total_bytes.saturating_add(serialized_bytes) > MAX_USAGE_DETAILS_BYTES
                {
                    return None;
                }
                total_bytes = total_bytes.saturating_add(serialized_bytes);
                Some(gateway::ProviderUsageDetail {
                    namespace: detail.namespace.clone(),
                    schema_version: detail.schema_version.clone(),
                    data,
                })
            })
            .collect()
    });
    gateway::ProviderUsage {
        display_text: usage.display_text.clone(),
        observed_at: usage.observed_at,
        details,
    }
}

pub(super) fn redact_usage_data(data: &provider::JsonObject) -> gateway::JsonObject {
    data.iter()
        .filter(|(key, _)| !usage_key_is_sensitive(key))
        .map(|(key, value)| (key.clone(), redact_usage_value(value)))
        .collect()
}

pub(super) fn redact_usage_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .filter(|(key, _)| !usage_key_is_sensitive(key))
                .map(|(key, value)| (key.clone(), redact_usage_value(value)))
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(redact_usage_value).collect())
        }
        _ => value.clone(),
    }
}

pub(super) fn usage_key_is_sensitive(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "authorization",
        "accesstoken",
        "refreshtoken",
        "password",
        "secret",
        "cookie",
        "email",
        "accountid",
    ]
    .iter()
    .any(|sensitive| normalized.contains(sensitive))
}

pub(super) fn provider_runtime_status(
    state: PluginRuntimeState,
    instance: Option<&provider::ProviderInstance>,
) -> gateway::ProviderStatus {
    match state {
        PluginRuntimeState::Stopped => gateway::ProviderStatus::Stopped,
        PluginRuntimeState::Starting => gateway::ProviderStatus::Connecting,
        PluginRuntimeState::Ready => instance
            .map(|instance| instance_status_to_gateway(instance.status.clone()))
            .unwrap_or(gateway::ProviderStatus::Unavailable),
        PluginRuntimeState::Crashed => gateway::ProviderStatus::Error,
    }
}

pub(super) fn instance_status_to_gateway(
    status: provider::InstanceStatus,
) -> gateway::ProviderStatus {
    match status {
        provider::InstanceStatus::Created => gateway::ProviderStatus::Unavailable,
        provider::InstanceStatus::Stopped => gateway::ProviderStatus::Stopped,
        provider::InstanceStatus::Starting | provider::InstanceStatus::Stopping => {
            gateway::ProviderStatus::Connecting
        }
        provider::InstanceStatus::Ready => gateway::ProviderStatus::Ready,
        provider::InstanceStatus::Error => gateway::ProviderStatus::Error,
        provider::InstanceStatus::Unrecognized(raw) => gateway::ProviderStatus::Unrecognized(raw),
    }
}

pub(super) fn map_capabilities(
    capabilities: &provider::ProviderCapabilities,
) -> gateway::GatewayCapabilities {
    let mut methods = Vec::new();
    for method in &capabilities.methods {
        let mapped = match method {
            provider::ProviderCapability::Unrecognized(_) => continue,
            provider::ProviderCapability::ProjectList => {
                Some(gateway::GatewayCapability::ProjectList)
            }
            provider::ProviderCapability::ProjectGet => {
                Some(gateway::GatewayCapability::ProjectGet)
            }
            provider::ProviderCapability::ProjectCreate => {
                Some(gateway::GatewayCapability::ProjectCreate)
            }
            provider::ProviderCapability::ProjectUpdate => {
                Some(gateway::GatewayCapability::ProjectUpdate)
            }
            provider::ProviderCapability::ProjectDelete => {
                Some(gateway::GatewayCapability::ProjectDelete)
            }
            provider::ProviderCapability::ConversationList => {
                Some(gateway::GatewayCapability::ConversationList)
            }
            provider::ProviderCapability::ConversationSearch => {
                Some(gateway::GatewayCapability::ConversationSearch)
            }
            provider::ProviderCapability::ConversationReleaseInteraction => {
                Some(gateway::GatewayCapability::ConversationReleaseInteraction)
            }
            provider::ProviderCapability::ConversationGet => {
                Some(gateway::GatewayCapability::ConversationGet)
            }
            provider::ProviderCapability::ConversationCreate => {
                Some(gateway::GatewayCapability::ConversationCreate)
            }
            provider::ProviderCapability::TurnStart => Some(gateway::GatewayCapability::TurnSend),
            provider::ProviderCapability::UsageQuery => {
                Some(gateway::GatewayCapability::CodepetUsageQuery)
            }
            provider::ProviderCapability::TurnSteer => Some(gateway::GatewayCapability::TurnSteer),
            provider::ProviderCapability::InstanceReleaseControl => None,
            provider::ProviderCapability::ConversationActiveList
            | provider::ProviderCapability::ConversationUnreadList
            | provider::ProviderCapability::ConversationMarkRead => None,
            provider::ProviderCapability::TurnInterrupt => {
                Some(gateway::GatewayCapability::TurnInterrupt)
            }
            provider::ProviderCapability::ApprovalResolve => {
                Some(gateway::GatewayCapability::ApprovalResolve)
            }
        };
        if let Some(mapped) = mapped {
            if !methods.contains(&mapped) {
                methods.push(mapped);
            }
        }
    }
    if capabilities
        .conversation_list_query
        .as_ref()
        .is_some_and(|query| query.updated_after && query.ids)
        && [
            provider::ProviderCapability::ConversationList,
            provider::ProviderCapability::ConversationActiveList,
            provider::ProviderCapability::ConversationUnreadList,
            provider::ProviderCapability::ConversationMarkRead,
        ]
        .iter()
        .all(|required| capabilities.methods.contains(required))
    {
        methods.push(gateway::GatewayCapability::ConversationRecent);
    }
    gateway::GatewayCapabilities {
        usage_datasets: capabilities.usage_datasets.clone(),
        revision: capabilities.revision.clone(),
        methods,
        turn_send: capabilities.turn_send.clone(),
        conversation_create: capabilities.conversation_create.clone(),
    }
}

pub(super) fn empty_gateway_capabilities(revision: String) -> gateway::GatewayCapabilities {
    gateway::GatewayCapabilities {
        usage_datasets: None,
        revision,
        methods: Vec::new(),
        turn_send: None,
        conversation_create: None,
    }
}

pub(super) fn sanitize_provider_conversation(
    mut conversation: gateway::Conversation,
) -> gateway::Conversation {
    conversation.read_state = None;
    conversation
}

pub(super) fn gateway_resource(
    resource: provider::ProviderResourceId,
) -> gateway::RoutedResourceId {
    gateway::RoutedResourceId {
        provider_id: resource.provider_instance_id,
        native_resource_id: resource.native_resource_id,
    }
}

pub(super) fn turn_send_key(
    caller_scope: &str,
    request: &gateway::TurnSendRequest,
) -> Result<TurnSendKey, gateway::ProtocolError> {
    if caller_scope.trim().is_empty() {
        return Err(gateway::ProtocolError {
            code: "invalid_caller_scope".to_string(),
            message: "turn.send internal caller scope must not be empty".to_string(),
            retryable: false,
            details: None,
        });
    }
    validate_gateway_resource(&request.conversation)?;
    if request.client_request_id.trim().is_empty() {
        return Err(gateway::ProtocolError {
            code: "invalid_client_request_id".to_string(),
            message: "turn.send clientRequestId must not be empty".to_string(),
            retryable: false,
            details: None,
        });
    }
    Ok(TurnSendKey {
        caller_scope: caller_scope.to_string(),
        provider_id: request.conversation.provider_id.clone(),
        client_request_id: request.client_request_id.clone(),
    })
}

pub(super) fn json_rpc_response<T: serde::Serialize>(
    jsonrpc: String,
    id: gateway::RequestId,
    result: Result<T, gateway::ProtocolError>,
) -> gateway::JsonRpcResponse {
    let response = match result {
        Ok(result) => match serde_json::to_value(result) {
            Ok(result) => gateway::JsonRpcResponsePayload::Ok { result },
            Err(error) => gateway::JsonRpcResponsePayload::Error {
                error: gateway::RpcError {
                    code: gateway::JSON_RPC_INTERNAL_ERROR,
                    message: format!("encode Gateway response: {error}"),
                    data: None,
                },
            },
        },
        Err(error) => {
            let message = error.message.clone();
            let data = serde_json::to_value(error).ok().and_then(|value| {
                value
                    .as_object()
                    .cloned()
                    .map(|entries| entries.into_iter().collect())
            });
            gateway::JsonRpcResponsePayload::Error {
                error: gateway::RpcError {
                    code: -32000,
                    message,
                    data,
                },
            }
        }
    };
    gateway::JsonRpcResponse {
        jsonrpc,
        id: Some(id),
        response,
    }
}

pub(super) fn validate_gateway_turn_selection(
    capabilities: &gateway::GatewayCapabilities,
    selection: &gateway::TurnSelection,
) -> Result<(), gateway::ProtocolError> {
    let turn_send = capabilities.turn_send.as_ref();
    validate_choice_selection(
        "access mode",
        selection.access_mode_id.as_deref(),
        turn_send.and_then(|capabilities| capabilities.access_mode.as_ref()),
    )?;
    validate_choice_selection(
        "reasoning effort",
        selection.reasoning_effort_id.as_deref(),
        turn_send.and_then(|capabilities| capabilities.reasoning_effort.as_ref()),
    )?;
    let catalog = turn_send.and_then(|capabilities| capabilities.model_catalog.as_ref());
    match (&selection.model, catalog) {
        (None, _) => Ok(()),
        (Some(_), None) => Err(gateway::ProtocolError {
            code: "unsupported_turn_control".to_string(),
            message: "Provider does not advertise a model selector for turn.send".to_string(),
            retryable: false,
            details: None,
        }),
        (
            Some(gateway::ModelSelection::FlatModelSelection(selection)),
            Some(gateway::ModelCatalog::FlatModelCatalog(catalog)),
        ) => validate_choice_option("model", &selection.model_id, &catalog.models),
        (
            Some(gateway::ModelSelection::GroupedModelSelection(selection)),
            Some(gateway::ModelCatalog::GroupedModelCatalog(catalog)),
        ) => {
            let group = catalog
                .providers
                .iter()
                .find(|group| group.id == selection.provider_id)
                .ok_or_else(|| gateway::ProtocolError {
                    code: "unknown_turn_selection".to_string(),
                    message: format!(
                        "Provider does not advertise model provider {}",
                        selection.provider_id
                    ),
                    retryable: false,
                    details: None,
                })?;
            validate_choice_option("model", &selection.model_id, &group.models)
        }
        _ => Err(gateway::ProtocolError {
            code: "turn_model_shape_mismatch".to_string(),
            message: "turn.send model selection kind does not match the advertised model catalog"
                .to_string(),
            retryable: false,
            details: None,
        }),
    }
}

pub(super) fn validate_choice_selection(
    label: &str,
    selected_id: Option<&str>,
    choices: Option<&gateway::ChoiceSet>,
) -> Result<(), gateway::ProtocolError> {
    let Some(selected_id) = selected_id else {
        return Ok(());
    };
    let Some(choices) = choices else {
        return Err(gateway::ProtocolError {
            code: "unsupported_turn_control".to_string(),
            message: format!("Provider does not advertise a {label} selector for turn.send"),
            retryable: false,
            details: None,
        });
    };
    validate_choice_option(label, selected_id, &choices.options)
}

pub(super) fn validate_choice_option(
    label: &str,
    selected_id: &str,
    options: &[gateway::ChoiceOption],
) -> Result<(), gateway::ProtocolError> {
    let option = options
        .iter()
        .find(|option| option.id == selected_id)
        .ok_or_else(|| gateway::ProtocolError {
            code: "unknown_turn_selection".to_string(),
            message: format!("Provider does not advertise {label} {selected_id}"),
            retryable: false,
            details: None,
        })?;
    if option.enabled == Some(false) {
        return Err(gateway::ProtocolError {
            code: "disabled_turn_selection".to_string(),
            message: option
                .disabled_reason
                .clone()
                .unwrap_or_else(|| format!("Provider disabled {label} {selected_id}")),
            retryable: false,
            details: None,
        });
    }
    Ok(())
}

pub(super) fn ensure_same_gateway_route(
    left: &gateway::RoutedResourceId,
    right: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    validate_gateway_resource(left)?;
    validate_gateway_resource(right)?;
    if left.provider_id == right.provider_id {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "gateway_route_mismatch".to_string(),
        message: "conversation and steer turn target different Provider routes".to_string(),
        retryable: false,
        details: None,
    })
}

pub(super) fn ensure_same_resource_identity(
    actual: &gateway::RoutedResourceId,
    expected: &provider::ProviderResourceId,
) -> Result<(), gateway::ProtocolError> {
    if actual.provider_id == expected.provider_instance_id
        && actual.native_resource_id == expected.native_resource_id
    {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "provider_resource_identity_mismatch".to_string(),
        message: "Provider returned a turn for a different conversation".to_string(),
        retryable: false,
        details: None,
    })
}

pub(super) fn ensure_same_agent_resource_identity(
    actual: &gateway::RoutedResourceId,
    expected: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    if actual == expected {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "provider_resource_identity_mismatch".to_string(),
        message: "Provider returned a resource with a different identity".to_string(),
        retryable: false,
        details: None,
    })
}

pub(super) fn ensure_same_provider_route(
    left: &gateway::RoutedResourceId,
    right: &provider::ProviderResourceId,
) -> Result<(), gateway::ProtocolError> {
    if left.provider_id == right.provider_instance_id {
        return Ok(());
    }
    Err(gateway::ProtocolError {
        code: "gateway_route_mismatch".to_string(),
        message: "Provider returned resources owned by different routes".to_string(),
        retryable: false,
        details: None,
    })
}

pub(super) fn validate_remote_host_identity(identity: &RemoteHostIdentity) -> HostResult<()> {
    if identity.device_id.trim().is_empty() || !device_descriptor_is_valid(&identity.descriptor) {
        return Err(HostError::new(
            "invalid_remote_host_identity",
            "Gateway Host identity requires a device ID and complete descriptor",
        ));
    }
    Ok(())
}

pub(super) fn device_descriptor_is_valid(descriptor: &gateway::DeviceDescriptor) -> bool {
    !descriptor.device_name.trim().is_empty()
        && !descriptor.operating_system.trim().is_empty()
        && !descriptor.system_version.trim().is_empty()
}

pub(super) fn validate_gateway_resource(
    resource: &gateway::RoutedResourceId,
) -> Result<(), gateway::ProtocolError> {
    if resource.provider_id.trim().is_empty() || resource.native_resource_id.trim().is_empty() {
        return Err(gateway::ProtocolError {
            code: "invalid_gateway_resource".to_string(),
            message: "providerId and nativeResourceId must not be empty".to_string(),
            retryable: false,
            details: None,
        });
    }
    Ok(())
}

pub(super) fn gateway_protocol_offer() -> gateway::ProtocolVersionOffer {
    gateway::ProtocolVersionOffer {
        version: gateway::PROTOCOL_VERSION,
        min_version: gateway::MIN_PROTOCOL_VERSION,
    }
}

pub(super) fn gateway_error(error: HostError) -> gateway::ProtocolError {
    error.into_protocol_error()
}

pub(super) fn event_cursor(sequence: u64) -> gateway::EventCursor {
    format!("{EVENT_CURSOR_PREFIX}{sequence:020}")
}

pub(super) fn event_cursor_sequence(cursor: &str) -> Result<u64, gateway::ProtocolError> {
    let sequence = cursor
        .strip_prefix(EVENT_CURSOR_PREFIX)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| gateway::ProtocolError {
            code: "invalid_event_cursor".to_string(),
            message: format!("invalid Gateway event cursor: {cursor}"),
            retryable: false,
            details: None,
        })?;
    if event_cursor(sequence) != cursor {
        return Err(gateway::ProtocolError {
            code: "invalid_event_cursor".to_string(),
            message: format!("non-canonical Gateway event cursor: {cursor}"),
            retryable: false,
            details: None,
        });
    }
    Ok(sequence)
}

pub(super) fn protocol_event_sequence(
    event: &gateway::ProtocolEvent,
) -> Result<u64, gateway::ProtocolError> {
    event_cursor_sequence(event.event_cursor())
}

pub(super) fn set_event_cursor(event: &mut gateway::ProtocolEvent, cursor: gateway::EventCursor) {
    event.set_event_cursor(cursor);
}

pub(super) fn gateway_state_error() -> gateway::ProtocolError {
    gateway::ProtocolError {
        code: "gateway_state_error".to_string(),
        message: "Gateway event state lock is unavailable".to_string(),
        retryable: true,
        details: None,
    }
}

pub(super) fn cursor_error(
    code: &str,
    message: &str,
    requested: u64,
    current: u64,
) -> gateway::ProtocolError {
    let mut details = BTreeMap::new();
    details.insert("requestedSequence".to_string(), requested.into());
    details.insert("currentSequence".to_string(), current.into());
    gateway::ProtocolError {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        details: Some(details),
    }
}
