use super::*;

impl ProtocolServer for ProviderGatewayService {
    fn turn_steer<'a>(
        &'a self,
        request: gateway::TurnSteerRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnSteerResponse> {
        Box::pin(self.turn_steer_for_caller_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn conversation_input_queue_get<'a>(
        &'a self,
        request: gateway::InputQueueGetRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::InputQueueSnapshot> {
        Box::pin(self.input_queue_get(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn conversation_input_queue_enqueue<'a>(
        &'a self,
        request: gateway::InputQueueEnqueueRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::InputQueueMutationResponse> {
        Box::pin(self.input_queue_enqueue(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn conversation_input_queue_withdraw<'a>(
        &'a self,
        request: gateway::InputQueueWithdrawRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::InputQueueMutationResponse> {
        Box::pin(self.input_queue_withdraw(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn conversation_input_queue_steer<'a>(
        &'a self,
        request: gateway::InputQueueSteerRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::InputQueueMutationResponse> {
        Box::pin(self.input_queue_steer(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn conversation_input_queue_resume<'a>(
        &'a self,
        request: gateway::InputQueueResumeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::InputQueueSnapshot> {
        Box::pin(self.input_queue_resume(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn conversation_input_queue_acknowledge_edit<'a>(
        &'a self,
        request: gateway::InputQueueAcknowledgeEditRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::InputQueueSnapshot> {
        Box::pin(self.input_queue_acknowledge_edit(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }

    fn conversation_recent<'a>(
        &'a self,
        request: gateway::ConversationRecentRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationRecentResponse> {
        Box::pin(self.conversation_recent_for_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }
    fn protocol_describe<'a>(
        &'a self,
        _request: gateway::ProtocolDescribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProtocolDescribeResponse> {
        Box::pin(async move {
            Ok(gateway::ProtocolDescribeResponse {
                features: vec![
                    gateway::ProtocolFeature::TraceContextV1,
                    gateway::ProtocolFeature::TerminalPtyV1,
                ],
            })
        })
    }

    fn protocol_ping<'a>(
        &'a self,
        request: gateway::PingRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::PingResponse> {
        Box::pin(async move {
            let (epoch, revision, providers) = self.provider_inventory_snapshot(None).await?;
            Ok(gateway::PingResponse {
                sequence: request.sequence,
                inventory_epoch: Some(epoch),
                inventory_revision: Some(revision),
                providers: providers.into_iter().map(|p| p.summary).collect(),
            })
        })
    }

    fn protocol_handshake<'a>(
        &'a self,
        request: gateway::HandshakeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::HandshakeResponse> {
        Box::pin(async move {
            let remote_host_identity =
                self.remote_host_identity
                    .clone()
                    .ok_or_else(|| gateway::ProtocolError {
                        code: "remote_host_identity_unavailable".to_string(),
                        message:
                            "Remote Gateway handshake requires a transport-injected Host identity"
                                .to_string(),
                        retryable: false,
                        details: None,
                    })?;
            let protocol = gateway_protocol_offer().negotiate(&request.protocol)?;
            if request.client_id.trim().is_empty()
                || request.client_version.trim().is_empty()
                || !device_descriptor_is_valid(&request.device)
            {
                return Err(gateway::ProtocolError {
                    code: "invalid_gateway_client".to_string(),
                    message:
                        "Gateway client identity and device descriptor fields must not be empty"
                            .to_string(),
                    retryable: false,
                    details: None,
                });
            }
            if let Some(cursor) = request.last_event_cursor.as_deref() {
                self.events.replay(Some(cursor))?;
            }
            // Capture the replay boundary before reading Provider snapshots. Any concurrent
            // update is therefore either reflected in the snapshot or replayable after it.
            let event_cursor = self.current_event_cursor();
            let (epoch, revision, providers) = self.provider_inventory_snapshot(None).await?;
            Ok(gateway::HandshakeResponse {
                inventory_epoch: Some(epoch),
                inventory_revision: Some(revision),
                protocol,
                device: gateway::GatewayDevice {
                    name: remote_host_identity.descriptor.device_name,
                    operating_system: remote_host_identity.descriptor.operating_system,
                    system_version: remote_host_identity.descriptor.system_version,
                },
                providers: providers.into_iter().map(|p| p.summary).collect(),
                event_cursor,
            })
        })
    }

    fn event_subscribe<'a>(
        &'a self,
        request: gateway::EventSubscribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::EventSubscribeResponse> {
        Box::pin(async move {
            self.events.replay(Some(&request.after_cursor))?;
            Ok(gateway::EventSubscribeResponse {
                subscribed_after_cursor: request.after_cursor,
            })
        })
    }

    fn provider_list<'a>(
        &'a self,
        _request: gateway::ProviderListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProviderListResponse> {
        Box::pin(async move {
            let (epoch, revision, providers) = self.provider_inventory_snapshot(None).await?;
            Ok(gateway::ProviderListResponse {
                inventory_epoch: Some(epoch),
                inventory_revision: Some(revision),
                providers: providers.into_iter().map(|p| p.summary).collect(),
            })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        request: gateway::ProviderDescribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProviderDescribeResponse> {
        Box::pin(async move {
            let provider = self
                .gateway_providers(None)
                .await?
                .into_iter()
                .find(|provider| provider.summary.id == request.provider_id)
                .ok_or_else(|| gateway::ProtocolError {
                    code: "unknown_provider".to_string(),
                    message: "provider.describe id does not identify a configured Provider"
                        .to_string(),
                    retryable: false,
                    details: None,
                })?;
            Ok(gateway::ProviderDescribeResponse {
                provider: provider.summary,
                capabilities: provider.capabilities,
            })
        })
    }

    fn project_list<'a>(
        &'a self,
        request: gateway::ProjectListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectListResponse> {
        Box::pin(async move {
            let snapshot_cursor = self.current_event_cursor();
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let response = self
                .manager
                .project_list(provider::ProjectListRequest {
                    route,
                    cursor: request.cursor,
                    limit: request.limit,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectListResponse {
                projects: response.projects,
                page_info: gateway::PageInfo {
                    next_cursor: response.page_info.next_cursor,
                },
                snapshot_cursor,
            })
        })
    }

    fn project_get<'a>(
        &'a self,
        request: gateway::ProjectGetRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectGetResponse> {
        Box::pin(async move {
            let project = self.resolve_resource(request.project).await?;
            let response = self
                .manager
                .project_get(provider::ProjectGetRequest { project })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectGetResponse {
                project: response.project,
            })
        })
    }

    fn project_create<'a>(
        &'a self,
        request: gateway::ProjectCreateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectCreateResponse> {
        Box::pin(async move {
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let response = self
                .manager
                .project_create(provider::ProjectCreateRequest {
                    route,
                    idempotency_key: request.idempotency_key,
                    name: request.name,
                    roots: request.roots.into_iter().collect(),
                    metadata: request.metadata,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectCreateResponse {
                project: response.project,
            })
        })
    }

    fn project_update<'a>(
        &'a self,
        request: gateway::ProjectUpdateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectUpdateResponse> {
        Box::pin(async move {
            let project = self.resolve_resource(request.project).await?;
            let response = self
                .manager
                .project_update(provider::ProjectUpdateRequest {
                    project,
                    name: request.name,
                    roots: request.roots.map(|roots| roots.into_iter().collect()),
                    metadata: request.metadata,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectUpdateResponse {
                project: response.project,
            })
        })
    }

    fn project_delete<'a>(
        &'a self,
        request: gateway::ProjectDeleteRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ProjectDeleteResponse> {
        Box::pin(async move {
            let project = self.resolve_resource(request.project).await?;
            self.manager
                .project_delete(provider::ProjectDeleteRequest { project })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ProjectDeleteResponse {})
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: gateway::ConversationListRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationListResponse> {
        Box::pin(async move {
            let project_provider_id = match &request.project_filter {
                gateway::ConversationProjectFilter::ConversationProjectFilterProject(filter) => {
                    validate_gateway_resource(&filter.project)?;
                    Some(filter.project.provider_id.as_str())
                }
                _ => None,
            };
            if project_provider_id.is_some_and(|id| id != request.provider_id) {
                return Err(gateway::ProtocolError {
                    code: "mismatched_provider_route".to_string(),
                    message: "conversation project filter must target the requested Provider"
                        .to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let project_filter = self.resolve_project_filter(request.project_filter).await?;
            let snapshot_cursor = self.current_event_cursor();
            let response = self
                .manager
                .conversation_list(provider::ConversationListRequest {
                    route,
                    cursor: request.cursor,
                    limit: request.limit,
                    project_filter,
                    query: None,
                    reader_scope: None,
                })
                .await
                .map_err(gateway_error)?;
            let mut conversations = Vec::with_capacity(response.conversations.len());
            for conversation in response.conversations {
                let mut conversation = sanitize_provider_conversation(conversation);
                self.observe_conversation_summary(&conversation)?;
                self.conversation_state
                    .decorate(DEFAULT_TURN_SEND_CALLER_SCOPE, &mut conversation)
                    .map_err(gateway_error)?;
                conversations.push(conversation);
            }
            Ok(gateway::ConversationListResponse {
                conversations,
                page_info: gateway::PageInfo {
                    next_cursor: response.page_info.next_cursor,
                },
                snapshot_cursor,
            })
        })
    }

    fn conversation_search<'a>(
        &'a self,
        request: gateway::ConversationSearchRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationSearchResponse> {
        Box::pin(async move {
            if request.search_term.trim().is_empty() {
                return Err(gateway::ProtocolError {
                    code: "invalid_request".to_string(),
                    message: "conversation.search requires a non-empty searchTerm".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let snapshot_cursor = self.current_event_cursor();
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let response = self
                .manager
                .conversation_search(provider::ConversationSearchRequest {
                    route,
                    search_term: request.search_term,
                    cursor: request.cursor,
                    limit: request.limit,
                })
                .await
                .map_err(gateway_error)?;
            let mut conversations = Vec::with_capacity(response.conversations.len());
            for conversation in response.conversations {
                let mut conversation = sanitize_provider_conversation(conversation);
                self.observe_conversation_summary(&conversation)?;
                self.conversation_state
                    .decorate(DEFAULT_TURN_SEND_CALLER_SCOPE, &mut conversation)
                    .map_err(gateway_error)?;
                conversations.push(conversation);
            }
            Ok(gateway::ConversationSearchResponse {
                conversations,
                page_info: gateway::PageInfo {
                    next_cursor: response.page_info.next_cursor,
                },
                snapshot_cursor,
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: gateway::ConversationGetRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationGetResponse> {
        Box::pin(async move {
            let initial_page = request.cursor.is_none();
            let snapshot_cursor = self.current_event_cursor();
            let conversation = self.resolve_resource(request.conversation).await?;
            let response = self
                .manager
                .conversation_get(provider::ConversationGetRequest {
                    conversation,
                    cursor: request.cursor,
                    limit: request.limit,
                })
                .await
                .map_err(gateway_error)?;
            let mut conversation = sanitize_provider_conversation(response.conversation);
            let items = response.items;
            self.observe_conversation_summary(&conversation)?;
            if self
                .conversation_state
                .observe_detail(&conversation.resource, &items)
                .map_err(gateway_error)?
                .is_some()
            {
                self.invalidate_recent(&conversation.resource.provider_id)?;
            }
            self.conversation_state
                .decorate(DEFAULT_TURN_SEND_CALLER_SCOPE, &mut conversation)
                .map_err(gateway_error)?;
            let turns = if initial_page {
                Some(
                    self.conversation_state
                        .recent_turn_failures(&conversation.resource)
                        .map_err(gateway_error)?,
                )
            } else {
                None
            };
            Ok(gateway::ConversationGetResponse {
                turns,
                conversation,
                items,
                page_info: response.page_info.map(|page_info| gateway::PageInfo {
                    next_cursor: page_info.next_cursor,
                }),
                snapshot_cursor,
            })
        })
    }

    fn conversation_release_interaction<'a>(
        &'a self,
        request: gateway::ConversationReleaseInteractionRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationReleaseInteractionResponse> {
        Box::pin(async move {
            if let Some(store) = &self.input_queue {
                for route in store.pause_provider(&request.conversation.provider_id)? {
                    self.publish_queue_changed(&route)?;
                }
            }
            let conversation = self.resolve_resource(request.conversation).await?;
            let response = self
                .manager
                .conversation_release_interaction(provider::ConversationReleaseInteractionRequest {
                    conversation,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationReleaseInteractionResponse {
                released: response.released,
                scope: "providerInstance".into(),
            })
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: gateway::ConversationAcquireInteractionRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            let conversation = self.resolve_resource(request.conversation).await?;
            let response = self
                .manager
                .conversation_acquire_interaction(provider::ConversationAcquireInteractionRequest {
                    conversation,
                    force: None,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationAcquireInteractionResponse {
                selection: response.selection,
                lease_expires_at: response.lease_expires_at,
            })
        })
    }

    fn conversation_resume<'a>(
        &'a self,
        request: gateway::ConversationResumeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationResumeResponse> {
        Box::pin(async move {
            let limit = request.limit.unwrap_or(20);
            if !(1..=100).contains(&limit) {
                return Err(gateway::ProtocolError {
                    code: "invalid_request".to_string(),
                    message: "conversation.resume limit must be between 1 and 100".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            let interaction_result = if request.force.unwrap_or(false) {
                async {
                    let conversation = self.resolve_resource(request.conversation.clone()).await?;
                    self.manager
                        .conversation_acquire_interaction(
                            provider::ConversationAcquireInteractionRequest {
                                conversation,
                                force: Some(true),
                            },
                        )
                        .await
                        .map(|response| gateway::ConversationAcquireInteractionResponse {
                            selection: response.selection,
                            lease_expires_at: response.lease_expires_at,
                        })
                        .map_err(gateway_error)
                }
                .await
            } else {
                self.conversation_acquire_interaction(
                    gateway::ConversationAcquireInteractionRequest {
                        conversation: request.conversation.clone(),
                    },
                )
                .await
            };
            let interaction = match interaction_result {
                Ok(interaction) => interaction,
                Err(error) => {
                    return Ok(gateway::ConversationResumeResponse {
                        interaction_acquired: false,
                        interaction: None,
                        interaction_error: Some(error),
                        history: None,
                        history_error: None,
                    });
                }
            };
            let history = self
                .conversation_get(gateway::ConversationGetRequest {
                    conversation: request.conversation,
                    cursor: None,
                    limit: Some(limit),
                })
                .await;
            let (history, history_error) = match history {
                Ok(history) => (Some(history), None),
                Err(error) => (None, Some(error)),
            };
            Ok(gateway::ConversationResumeResponse {
                interaction_acquired: true,
                interaction: Some(interaction),
                interaction_error: None,
                history,
                history_error,
            })
        })
    }

    fn codepet_usage_query<'a>(
        &'a self,
        request: gateway::UsageQueryRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::UsageQueryResponse> {
        Box::pin(async move {
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let grouped = !request.query.aggregation.group_by.is_empty();
            let peak_requested = request
                .query
                .summaries
                .as_ref()
                .is_some_and(|v| v.contains(&gateway::UsageQuerySummariesItems::PeakDaily));
            let mut response = self
                .manager
                .usage_query(provider::UsageQueryRequest {
                    route,
                    query: request.query,
                })
                .await
                .map_err(gateway_error)?;
            // The generated Rust nested Option collapses explicit null on Provider decoding.
            // Restore requested nullable fields before forwarding the Gateway wire response.
            if grouped {
                for row in &mut response.result.rows {
                    row.model_id.get_or_insert(None);
                }
            }
            if peak_requested {
                if let Some(summary) = &mut response.result.summaries {
                    summary.peak_daily.get_or_insert(None);
                }
            }
            Ok(gateway::UsageQueryResponse {
                result: response.result,
            })
        })
    }

    fn conversation_mark_read<'a>(
        &'a self,
        request: gateway::ConversationMarkReadRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationMarkReadResponse> {
        Box::pin(self.mark_read_for_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request))
    }

    fn conversation_create<'a>(
        &'a self,
        request: gateway::ConversationCreateRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ConversationCreateResponse> {
        Box::pin(async move {
            let route = self.resolve_provider_route(&request.provider_id).await?;
            let project = match request.project {
                Some(project) => Some(self.resolve_resource(project).await?),
                None => None,
            };
            let response = self
                .manager
                .conversation_create(provider::ConversationCreateRequest {
                    route,
                    project,
                    title: request.title,
                    permission_level: request.permission_level,
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                    workspace_root: request.workspace_root,
                    workspace_mode: request.workspace_mode,
                    extension: None,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ConversationCreateResponse {
                conversation: sanitize_provider_conversation(response.conversation),
            })
        })
    }

    fn turn_send<'a>(
        &'a self,
        request: gateway::TurnSendRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnSendResponse> {
        Box::pin(async move {
            self.turn_send_for_caller_scope(DEFAULT_TURN_SEND_CALLER_SCOPE, request)
                .await
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: gateway::TurnInterruptRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::TurnInterruptResponse> {
        Box::pin(async move {
            ensure_same_gateway_route(&request.conversation, &request.turn)?;
            let lock = self.input_lock(&request.conversation)?;
            let _guard = lock.lock().await;
            if let Some(store) = &self.input_queue {
                store.pause(&request.conversation)?;
                self.publish_queue_changed(&request.conversation)?;
            }
            let conversation = self.resolve_resource(request.conversation).await?;
            let turn = self.resolve_resource(request.turn).await?;
            let response = self
                .manager
                .turn_interrupt(provider::TurnInterruptRequest { conversation, turn })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::TurnInterruptResponse {
                turn: response.turn,
            })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: gateway::ApprovalResolveRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::ApprovalResolveResponse> {
        Box::pin(async move {
            let approval = self.resolve_resource(request.approval).await?;
            let response = self
                .manager
                .approval_resolve(provider::ApprovalResolveRequest {
                    approval,
                    decision: request.decision,
                })
                .await
                .map_err(gateway_error)?;
            Ok(gateway::ApprovalResolveResponse {
                approval: response.approval,
            })
        })
    }
}
