use super::*;

impl Provider for CodexProvider {
    fn notice_observe<'a>(
        &'a self,
        request: codepet_provider_sdk::NoticeObserveRequest,
    ) -> codepet_provider_sdk::ProtocolFuture<'a, codepet_provider_sdk::NoticeObserveResponse> {
        Box::pin(async move { self.observation.observe(request.enabled).await })
    }

    fn provider_initialize<'a>(
        &'a self,
        request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async move {
            let protocol = ProtocolVersionOffer {
                version: PROTOCOL_VERSION,
                min_version: codepet_provider_sdk::MIN_PROTOCOL_VERSION,
            }
            .negotiate(&request.protocol)?;
            if request.host_client_id.trim().is_empty()
                || request.host_device_id.trim().is_empty()
                || request.host_version.trim().is_empty()
            {
                return Err(protocol_error(
                    "invalid_host_identity",
                    "Host client, device, and version must not be empty".to_string(),
                    false,
                ));
            }
            let mut state = lock(&self.state);
            if let Some(device_id) = state.host_device_id.as_ref() {
                if device_id != &request.host_device_id
                    || state.initialized_client_id.as_ref() != Some(&request.host_client_id)
                {
                    return Err(protocol_error(
                        "provider_already_initialized",
                        "Provider process is already bound to another Host identity".to_string(),
                        false,
                    ));
                }
            } else {
                state.host_device_id = Some(request.host_device_id);
                state.initialized_client_id = Some(request.host_client_id);
            }
            drop(state);
            self.data.initialize(request.directories.as_ref())?;
            self.scanner.set_selection_storage(self.data.clone());
            eprintln!(
                "provider.initialize completed; business storage configured={}",
                self.data.configured()
            );
            self.scanner.start_cancellable(
                "codex",
                "@openai/codex",
                discover_codex_candidates,
                |candidate, timeout, control| {
                    inspect_runtime_candidate(candidate, "codex", timeout, control)
                },
            );
            Ok(ProviderInitializeResponse {
                protocol,
                plugin: Self::descriptor(),
            })
        })
    }

    fn usage_query<'a>(
        &'a self,
        request: codepet_provider_sdk::UsageQueryRequest,
    ) -> ProtocolFuture<'a, codepet_provider_sdk::UsageQueryResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let data = self.data.clone();
            let result = tokio::task::spawn_blocking(move || {
                let instance = request.route.provider_instance_id;
                if request
                    .query
                    .page
                    .as_ref()
                    .and_then(|p| p.cursor.as_ref())
                    .is_none()
                {
                    let response = runtime
                        .ready_server()?
                        .account_usage_read()
                        .map_err(|e| codepet_provider_data::error("usage_unavailable", e))?;
                    data.import_codex_daily(&instance, &response)?;
                }
                data.query(&instance, request.query, true)
            })
            .await
            .map_err(|e| codepet_provider_data::error("usage_query_failed", e))??;
            Ok(codepet_provider_sdk::UsageQueryResponse { result })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        _request: ProviderDescribeRequest,
    ) -> ProtocolFuture<'a, ProviderDescribeResponse> {
        Box::pin(async move {
            Ok(ProviderDescribeResponse {
                plugin: Self::descriptor(),
            })
        })
    }

    fn runtime_get_installed<'a>(
        &'a self,
        request: RuntimeGetInstalledRequest,
    ) -> ProtocolFuture<'a, RuntimeGetInstalledResponse> {
        Box::pin(async move {
            if request.refresh == Some(true) && self.scanner.snapshot().scanning != Some(true) {
                self.scanner.stop();
                self.scanner.start_cancellable(
                    "codex",
                    "@openai/codex",
                    discover_codex_candidates,
                    |candidate, timeout, control| {
                        inspect_runtime_candidate(candidate, "codex", timeout, control)
                    },
                );
            }
            if request.refresh == Some(true) {
                let instances = lock(&self.state)
                    .instances
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                for instance in instances {
                    instance.refresh_metadata();
                }
            }
            Ok(self.scanner.snapshot())
        })
    }
    fn runtime_select<'a>(
        &'a self,
        request: RuntimeSelectRequest,
    ) -> ProtocolFuture<'a, RuntimeSelectResponse> {
        Box::pin(async move {
            let mut candidate = request.candidate;
            candidate.executable_path = local_runtime::resolve_executable(
                std::path::Path::new(&candidate.executable_path),
                "codex",
                "@openai/codex",
            )
            .map_err(|error| protocol_error("invalid_runtime_selection", error, false))?
            .to_string_lossy()
            .into_owned();
            let selected = self.scanner.select(&candidate)?;
            Ok(RuntimeSelectResponse {
                pending: selected.version.is_empty().then(|| RuntimeCandidate {
                    executable_path: selected.executable_path.clone(),
                    source: selected.source.clone(),
                }),
                selected: (!selected.version.is_empty()).then_some(selected),
            })
        })
    }

    fn instance_create<'a>(
        &'a self,
        mut request: InstanceCreateRequest,
    ) -> ProtocolFuture<'a, InstanceCreateResponse> {
        Box::pin(async move {
            Self::descriptor().validate_instance_kind(&request.instance_kind)?;
            validate_route(&request.route)?;
            if request.display_name.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_provider_instance",
                    "Provider instance display name must not be empty".to_string(),
                    false,
                ));
            }
            if std::env::var("CODEPET_RUNTIME_MIN_VERSION").is_ok_and(|value| !value.is_empty()) {
                if let Some(path) = request
                    .settings
                    .get("appServerExecutable")
                    .and_then(Value::as_str)
                {
                    let runtime = self
                        .scanner
                        .inspect(RuntimeCandidate {
                            executable_path: path.to_string(),
                            source: codepet_provider_sdk::RuntimeCandidateSource::Configured,
                        })
                        .await?;
                    if runtime.version.is_empty() {
                        return Err(protocol_error(
                            "runtime_scanning",
                            "Runtime version validation is still in progress".into(),
                            true,
                        ));
                    }
                    local_runtime::require_compatible_runtime(
                        &local_runtime::apply_runtime_requirement(runtime),
                    )?;
                }
            }
            let selected = if request.settings.contains_key("appServerExecutable") {
                None
            } else {
                let inventory = self.scanner.snapshot();
                if inventory.scanning == Some(true) {
                    return Err(protocol_error(
                        "runtime_scanning",
                        "Runtime discovery is still in progress".into(),
                        true,
                    ));
                }
                let installation = inventory.selected;
                Some(installation.ok_or_else(|| {
                    protocol_error(
                        "provider_unavailable",
                        "Codex Provider did not find a local runtime".to_string(),
                        true,
                    )
                })?)
            };
            if let Some(selected) = selected.as_ref() {
                if selected.version.is_empty() {
                    return Err(protocol_error(
                        "runtime_scanning",
                        "Runtime version validation is still in progress".into(),
                        true,
                    ));
                }
                local_runtime::require_compatible_runtime(
                    &local_runtime::apply_runtime_requirement(selected.clone()),
                )?;
                request.settings.insert(
                    "appServerExecutable".to_string(),
                    json!(selected.executable_path.clone()),
                );
            }
            let settings = decode_settings(request.settings.clone())?;
            let mut state = lock(&self.state);
            let host_device = state.host_device_id.as_deref().ok_or_else(|| {
                protocol_error(
                    "provider_not_initialized",
                    "Provider must be initialized before creating instances".to_string(),
                    false,
                )
            })?;
            if request.route.device_id != host_device {
                return Err(protocol_error(
                    "wrong_device_route",
                    "Provider instance targets a different Host device".to_string(),
                    false,
                ));
            }
            if let Some(existing) = state.instances.get(&request.route.provider_instance_id) {
                if existing.route != request.route
                    || existing.instance_kind != request.instance_kind
                    || existing.display_name != request.display_name
                    || existing.settings != settings
                {
                    return Err(protocol_error(
                        "provider_instance_conflict",
                        "Provider instance id was reused with different configuration".to_string(),
                        false,
                    ));
                }
                return Ok(InstanceCreateResponse {
                    instance: existing.snapshot(),
                });
            }
            let runtime = Arc::new(CodexInstanceRuntime::new(
                request,
                settings,
                self.events.clone(),
                self.lifecycle_hook.clone(),
            ));
            let instance = runtime.snapshot();
            state
                .instances
                .insert(runtime.route.provider_instance_id.clone(), runtime);
            Ok(InstanceCreateResponse { instance })
        })
    }

    fn instance_start<'a>(
        &'a self,
        request: InstanceStartRequest,
    ) -> ProtocolFuture<'a, InstanceStartResponse> {
        Box::pin(async move {
            self.start_runtime(self.instance(&request.route)?, false)
                .await
        })
    }

    fn instance_release_control<'a>(
        &'a self,
        request: codepet_provider_sdk::InstanceReleaseControlRequest,
    ) -> ProtocolFuture<'a, codepet_provider_sdk::InstanceReleaseControlResponse> {
        Box::pin(async move {
            use codepet_provider_sdk::{
                InstanceReleaseControlOutcome as Outcome, InstanceReleaseControlResponse,
            };
            let runtime = self.instance(&request.route)?;
            // The SDK holds exclusive request admission and verifies the full idle window.
            // This adapter check also protects native activity which has not yet emitted events.
            let _release = runtime.release_transition.lock().await;
            {
                let state = lock(&runtime.mutable);
                if self.shutdown.load(Ordering::SeqCst)
                    || state.destroyed
                    || state.status.clone() != InstanceStatus::Ready
                {
                    return Ok(InstanceReleaseControlResponse {
                        outcome: Outcome::Unavailable,
                    });
                }
                if !state.pending_approvals.is_empty()
                    || state
                        .executions
                        .values()
                        .any(|slot| match &*lock(&slot.state) {
                            ExecutionSlotState::Creating(_) => true,
                            ExecutionSlotState::Ready(execution) => {
                                execution.active_turn_id.is_some()
                            }
                            _ => false,
                        })
                {
                    return Ok(InstanceReleaseControlResponse {
                        outcome: Outcome::Busy,
                    });
                }
            }
            runtime.stop().await?;
            // No conversation acquire/resume: the replacement only restores service availability.
            self.start_runtime(runtime.clone(), false)
                .await
                .map_err(|error| {
                    protocol_error(
                        "release_control_restart_failed",
                        format!(
                            "Control released, but clean runtime restart failed: {}",
                            error.message
                        ),
                        true,
                    )
                })?;
            Ok(InstanceReleaseControlResponse {
                outcome: Outcome::Released,
            })
        })
    }

    fn instance_stop<'a>(
        &'a self,
        request: InstanceStopRequest,
    ) -> ProtocolFuture<'a, InstanceStopResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            Ok(InstanceStopResponse {
                instance: runtime.stop().await?,
            })
        })
    }

    fn instance_destroy<'a>(
        &'a self,
        request: InstanceDestroyRequest,
    ) -> ProtocolFuture<'a, InstanceDestroyResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            {
                let _transition = lock(&runtime.lifecycle_transition);
                let mut mutable = lock(&runtime.mutable);
                if mutable.destroyed {
                    return Ok(InstanceDestroyResponse { destroyed: true });
                }
                if mutable.cleanup_in_progress
                    || matches!(
                        mutable.status.clone(),
                        InstanceStatus::Ready | InstanceStatus::Starting | InstanceStatus::Stopping
                    )
                {
                    return Err(protocol_error(
                        "provider_instance_running",
                        "Stop the Codex Provider instance before destroying it".to_string(),
                        false,
                    ));
                }
                mutable.destroyed = true;
                CodexInstanceRuntime::cancel_metadata(&mut mutable);
                mutable.lifecycle_generation = mutable.lifecycle_generation.wrapping_add(1);
                mutable.server_session_id = None;
                mutable.server_generation = None;
                runtime.lifecycle_changed.notify_all();
            }
            let mut state = lock(&self.state);
            if state
                .instances
                .get(&request.route.provider_instance_id)
                .is_some_and(|current| Arc::ptr_eq(current, &runtime))
            {
                state.instances.remove(&request.route.provider_instance_id);
            }
            Ok(InstanceDestroyResponse { destroyed: true })
        })
    }

    fn instance_capabilities<'a>(
        &'a self,
        request: InstanceCapabilitiesRequest,
    ) -> ProtocolFuture<'a, InstanceCapabilitiesResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let capabilities = runtime.snapshot().capabilities;
            Ok(InstanceCapabilitiesResponse { capabilities })
        })
    }

    fn project_list<'a>(
        &'a self,
        request: ProjectListRequest,
    ) -> ProtocolFuture<'a, ProjectListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            require_capability(&runtime, ProviderCapability::ProjectList)?;
            let limit = request.limit.map(u32::try_from).transpose().map_err(|_| {
                protocol_error(
                    "invalid_request",
                    "project list limit exceeds the Codex App Server range".to_string(),
                    false,
                )
            })?;
            let session = runtime.ready_server()?;
            let page =
                tokio::task::spawn_blocking(move || session.project_list(request.cursor, limit))
                    .await
                    .map_err(provider_task_error)?
                    .map_err(CodexProtocolMapper::error)?;
            let projects = {
                let mapper = lock(&runtime.mapper);
                page.data
                    .into_iter()
                    .map(|project| mapper.project(project))
                    .collect::<Result<Vec<_>, _>>()?
            };
            Ok(ProjectListResponse {
                projects,
                page_info: PageInfo {
                    next_cursor: page.next_cursor,
                },
            })
        })
    }

    fn project_get<'a>(
        &'a self,
        request: ProjectGetRequest,
    ) -> ProtocolFuture<'a, ProjectGetResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.project)?;
            require_capability(&runtime, ProviderCapability::ProjectGet)?;
            let project_id = request.project.native_resource_id;
            let session = runtime.ready_server()?;
            let project = tokio::task::spawn_blocking(move || session.project_read(&project_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
            let project = lock(&runtime.mapper).project(project)?;
            Ok(ProjectGetResponse { project })
        })
    }

    fn project_create<'a>(
        &'a self,
        request: ProjectCreateRequest,
    ) -> ProtocolFuture<'a, ProjectCreateResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            require_capability(&runtime, ProviderCapability::ProjectCreate)?;
            validate_project_fields(&request.name, &request.roots)?;
            let session = runtime.ready_server()?;
            let native_request = CodexProjectCreateRequest {
                idempotency_key: request.idempotency_key,
                name: request.name,
                roots: request
                    .roots
                    .into_iter()
                    .map(|root| CodexProjectRoot { path: root.path })
                    .collect(),
                metadata: request.metadata,
            };
            let project =
                tokio::task::spawn_blocking(move || session.project_create(native_request))
                    .await
                    .map_err(provider_task_error)?
                    .map_err(CodexProtocolMapper::error)?;
            let project = lock(&runtime.mapper).project(project)?;
            Ok(ProjectCreateResponse { project })
        })
    }

    fn project_update<'a>(
        &'a self,
        request: ProjectUpdateRequest,
    ) -> ProtocolFuture<'a, ProjectUpdateResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.project)?;
            require_capability(&runtime, ProviderCapability::ProjectUpdate)?;
            if let Some(name) = request.name.as_deref() {
                if name.trim().is_empty() {
                    return Err(protocol_error(
                        "invalid_request",
                        "project name must not be empty".to_string(),
                        false,
                    ));
                }
            }
            if request.roots.as_ref().is_some_and(|roots| {
                roots.is_empty() || roots.iter().any(|root| root.path.trim().is_empty())
            }) {
                return Err(protocol_error(
                    "invalid_request",
                    "project roots must contain non-empty paths".to_string(),
                    false,
                ));
            }
            let session = runtime.ready_server()?;
            let native_request = CodexProjectUpdateRequest {
                project_id: request.project.native_resource_id,
                name: request.name,
                roots: request.roots.map(|roots| {
                    roots
                        .into_iter()
                        .map(|root| CodexProjectRoot { path: root.path })
                        .collect()
                }),
                metadata: request.metadata,
            };
            let project =
                tokio::task::spawn_blocking(move || session.project_update(native_request))
                    .await
                    .map_err(provider_task_error)?
                    .map_err(CodexProtocolMapper::error)?;
            let project = lock(&runtime.mapper).project(project)?;
            Ok(ProjectUpdateResponse { project })
        })
    }

    fn project_delete<'a>(
        &'a self,
        request: ProjectDeleteRequest,
    ) -> ProtocolFuture<'a, ProjectDeleteResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.project)?;
            require_capability(&runtime, ProviderCapability::ProjectDelete)?;
            let project_id = request.project.native_resource_id;
            let session = runtime.ready_server()?;
            tokio::task::spawn_blocking(move || session.project_delete(&project_id))
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)?;
            Ok(ProjectDeleteResponse {})
        })
    }

    fn conversation_active_list<'a>(
        &'a self,
        request: codepet_provider_sdk::ConversationActiveListRequest,
    ) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationActiveListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if !runtime.hook_ready.load(Ordering::SeqCst) {
                return Err(protocol_error(
                    "unsupported",
                    "Hook activity observation is not ready".into(),
                    true,
                ));
            }
            let generation = runtime.query_generation()?;
            if let Some(page) = runtime.atoms.active_cached(&generation, &request)? {
                return Ok(page);
            }
            let epoch = runtime.atoms.event_epoch();
            let rows = lock(&runtime.hook_activity).active_rows(&runtime.route);
            if generation != runtime.query_generation()? || epoch != runtime.atoms.event_epoch() {
                return Err(conversation_atoms::generation_changed());
            }
            runtime.atoms.active(&generation, &request, rows)
        })
    }

    fn conversation_unread_list<'a>(
        &'a self,
        request: codepet_provider_sdk::ConversationUnreadListRequest,
    ) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationUnreadListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            runtime.atoms.unread(&runtime.query_generation()?, &request)
        })
    }

    fn conversation_mark_read<'a>(
        &'a self,
        request: codepet_provider_sdk::ConversationMarkReadRequest,
    ) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationMarkReadResponse> {
        Box::pin(async move {
            let runtime =
                self.instance(&conversation_atoms::resource_route(&request.conversation))?;
            runtime.query_generation()?;
            runtime.atoms.mark_read(&request, runtime.events.as_ref())
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            let generation = runtime.query_generation()?;
            conversation_atoms::validate_list(&request)?;
            if !matches!(
                request.project_filter,
                ConversationProjectFilter::ConversationProjectFilterAll(_)
            ) {
                require_capability(&runtime, ProviderCapability::ProjectList)?;
            }
            runtime.list_directory_page(&generation, &request).await
        })
    }

    fn conversation_search<'a>(
        &'a self,
        request: ConversationSearchRequest,
    ) -> ProtocolFuture<'a, ConversationSearchResponse> {
        Box::pin(async move {
            let runtime = self.instance(&request.route)?;
            if request.search_term.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_request",
                    "conversation.search requires a non-empty searchTerm".to_string(),
                    false,
                ));
            }
            let limit = request.limit.map(u32::try_from).transpose().map_err(|_| {
                protocol_error(
                    "invalid_request",
                    "conversation search limit exceeds the Codex App Server range".to_string(),
                    false,
                )
            })?;
            let session = runtime.ready_server()?;
            let page = tokio::task::spawn_blocking(move || {
                session.thread_list(CodexThreadListRequest {
                    cursor: request.cursor,
                    limit,
                    project_id: None,
                    workspace_root: None,
                    search_term: Some(request.search_term),
                })
            })
            .await
            .map_err(provider_task_error)?
            .map_err(CodexProtocolMapper::error)?;
            let conversations = {
                let mapper = lock(&runtime.mapper);
                page.data
                    .iter()
                    .map(|snapshot| {
                        let mut row = mapper.conversation(snapshot);
                        runtime.project_conversation(&mut row);
                        row
                    })
                    .collect::<Vec<_>>()
            };
            Ok(ConversationSearchResponse {
                conversations,
                page_info: PageInfo {
                    next_cursor: page.next_cursor,
                },
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProtocolFuture<'a, ConversationGetResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            let requested_limit = request.limit.unwrap_or(DEFAULT_CONVERSATION_GET_TURN_LIMIT);
            if !(1..=MAX_CONVERSATION_GET_TURN_LIMIT).contains(&requested_limit) {
                return Err(protocol_error(
                    "invalid_request",
                    format!(
                        "conversation.get limit must be between 1 and {MAX_CONVERSATION_GET_TURN_LIMIT}"
                    ),
                    false,
                ));
            }
            let initial_cursor = request.cursor;
            let session = runtime.ready_server()?;
            tokio::task::spawn_blocking(move || {
                let (snapshot, used_pending_snapshot) =
                    match session.thread_read_metadata(&conversation_id) {
                        Ok(snapshot) => (snapshot, false),
                        Err(error)
                            if error.is_thread_not_loaded(&conversation_id)
                                || is_created_conversation_not_ready(&error, &conversation_id) =>
                        {
                            let snapshot = lock(&runtime.mutable)
                                .pending_materialization
                                .get(&conversation_id)
                                .cloned();
                            match snapshot {
                                Some(snapshot) => (snapshot, true),
                                None => return Err(CodexProtocolMapper::error(error)),
                            }
                        }
                        Err(error) => return Err(CodexProtocolMapper::error(error)),
                    };
                let approvals = {
                    let mutable = lock(&runtime.mutable);
                    mutable
                        .approval_history
                        .iter()
                        .filter(|observed| {
                            observed.approval.conversation.native_resource_id == snapshot.thread.id
                        })
                        .map(|observed| (observed.item_id.clone(), observed.approval.clone()))
                        .collect::<Vec<_>>()
                };
                let mut emitted_approvals = vec![false; approvals.len()];
                let mut items = Vec::new();
                let mut active_turn = None;
                let loaded = load_conversation_turns(
                    &session,
                    &conversation_id,
                    initial_cursor,
                    requested_limit,
                    used_pending_snapshot,
                )?;
                let turns = loaded.turns;
                let response_next_cursor = loaded.next_cursor;
                let history_materialized = loaded.materialized;
                let mapper = lock(&runtime.mapper);
                for turn in turns.into_iter().rev() {
                    if turn.status == CodexTurnStatus::InProgress {
                        active_turn = Some(mapper.turn(&snapshot.thread.id, &turn));
                    }
                    mapper.append_conversation_turn_items(
                        &snapshot.thread.id,
                        &turn,
                        &approvals,
                        &mut emitted_approvals,
                        &mut items,
                    );
                }
                let mut conversation = mapper.conversation_with_active_turn(&snapshot, active_turn);
                drop(mapper);
                runtime.project_conversation(&mut conversation);
                if history_materialized {
                    lock(&runtime.mutable)
                        .pending_materialization
                        .remove(&conversation_id);
                }
                Ok(ConversationGetResponse {
                    conversation,
                    items,
                    page_info: Some(PageInfo {
                        next_cursor: response_next_cursor,
                    }),
                })
            })
            .await
            .map_err(provider_task_error)?
        })
    }

    fn conversation_release_interaction<'a>(
        &'a self,
        request: codepet_provider_sdk::ConversationReleaseInteractionRequest,
    ) -> ProtocolFuture<'a, codepet_provider_sdk::ConversationReleaseInteractionResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let observation = self.observation.clone();
            let shutdown = self.shutdown.clone();
            // Cleanup continues even if the requesting client disconnects.
            tokio::spawn(async move {
                let _release = runtime.release_transition.lock().await;
                // Retrying a completed release must not recycle the clean server again.
                if {
                    let state = lock(&runtime.mutable);
                    state.interaction_released
                        && matches!(
                            state.status.clone(),
                            InstanceStatus::Ready | InstanceStatus::Starting
                        )
                        && state.executions.is_empty()
                        && state.failed_shutdown_sessions.is_empty()
                } {
                    return Ok(
                        codepet_provider_sdk::ConversationReleaseInteractionResponse {
                            released: true,
                            scope: "providerInstance".into(),
                        },
                    );
                }
                {
                    let _lifecycle = lock(&runtime.lifecycle_transition);
                    lock(&runtime.mutable).interaction_released = true;
                }
                let release_error = |error: ProtocolError| {
                    protocol_error("interaction_release_failed", error.message, true)
                };
                runtime.stop().await.map_err(release_error)?;
                let cleanup_runtime = runtime.clone();
                tokio::task::spawn_blocking(move || {
                    cleanup_runtime.shutdown_owned_sessions(Vec::new())
                })
                .await
                .map_err(provider_task_error)?
                .map_err(CodexProtocolMapper::error)
                .map_err(release_error)?;
                // Keep interactions paused, but restore a fresh, usable runtime.
                // Starting a server never resumes any of the released conversations.
                Self::start_runtime_owned(runtime.clone(), false, observation, shutdown)
                    .await
                    .map_err(|error| {
                        protocol_error(
                            "interaction_restart_failed",
                            format!(
                                "Interaction released, but the clean harness could not restart: {}",
                                error.message
                            ),
                            true,
                        )
                    })?;
                Ok(
                    codepet_provider_sdk::ConversationReleaseInteractionResponse {
                        released: true,
                        scope: "providerInstance".into(),
                    },
                )
            })
            .await
            .map_err(provider_task_error)?
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: ConversationAcquireInteractionRequest,
    ) -> ProtocolFuture<'a, ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let admitted_generation = lock(&runtime.mutable).lifecycle_generation;
            let _transition = runtime.interaction_transition.lock().await;
            if lock(&runtime.mutable).lifecycle_generation != admitted_generation {
                return Err(protocol_error(
                    "interaction_released",
                    "Instance lifecycle changed while acquire was queued".into(),
                    false,
                ));
            }
            let needs_start = {
                let mutable = lock(&runtime.mutable);
                mutable.interaction_released || mutable.status == InstanceStatus::Stopped
            };
            if needs_start {
                self.start_runtime(runtime.clone(), true).await?;
                tokio::time::timeout(Duration::from_secs(30), async {
                    loop {
                        let status = lock(&runtime.mutable).status.clone();
                        match status {
                            InstanceStatus::Ready => return Ok(()),
                            InstanceStatus::Starting => {
                                tokio::time::sleep(Duration::from_millis(10)).await
                            }
                            _ => {
                                return Err(protocol_error(
                                    "provider_unavailable",
                                    "Released harness could not restart".into(),
                                    true,
                                ))
                            }
                        }
                    }
                })
                .await
                .map_err(|_| {
                    protocol_error(
                        "provider_unavailable",
                        "Timed out restarting released harness".into(),
                        true,
                    )
                })??;
            }
            let expected_generation = lock(&runtime.mutable).lifecycle_generation;
            drop(_transition);
            let conversation_id = request.conversation.native_resource_id;
            let runtime = runtime.clone();
            tokio::task::spawn_blocking(move || {
                let force = request.force.unwrap_or(false);
                let _takeover_gate = force.then(|| DESKTOP_TAKEOVER_GATE.write().unwrap_or_else(|e| e.into_inner()));
                let _normal_gate = (!force).then(|| DESKTOP_TAKEOVER_GATE.read().unwrap_or_else(|e| e.into_inner()));
                if force { runtime.prepare_forced_takeover(&conversation_id, expected_generation)?; }
                runtime.with_current_execution_inner(
                    &conversation_id,
                    "conversation.acquireInteraction",
                    Some(expected_generation),
                    |handle| {
                        let configuration = handle
                            .session
                            .thread_configuration(&conversation_id)
                            .ok_or_else(|| {
                                protocol_error(
                                    "provider_protocol_error",
                                    format!(
                                        "Codex thread/resume did not return configuration for conversation {conversation_id}"
                                    ),
                                    false,
                                )
                            })?;
                        Ok(ExecutionStep::Complete(
                            ConversationAcquireInteractionResponse {
                                selection: TurnSelection {
                                    access_mode_id: configuration
                                        .permission_level
                                        .map(permission_level_id)
                                        .map(str::to_string),
                                    reasoning_effort_id: configuration.reasoning_effort.clone(),
                                    model: configuration.model.clone().map(|model_id| {
                                        ModelSelection::FlatModelSelection(FlatModelSelection {
                                            kind: FlatModelCatalogKind::Flat,
                                            model_id,
                                        })
                                    }),
                                },
                                lease_expires_at: None,
                            },
                        ))
                    },
                )
            })
            .await
            .map_err(provider_task_error)?
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
            if request.title.is_some() {
                return Err(protocol_error(
                    "capability_unsupported",
                    "Codex App Server thread/start does not support setting a title".to_string(),
                    false,
                ));
            }
            if request.extension.is_some() {
                return Err(protocol_error(
                    "capability_unsupported",
                    "Codex Provider does not define conversation.create extensions".to_string(),
                    false,
                ));
            }
            let runtime = self.instance(&request.route)?;
            let project_id = if let Some(project) = request.project.as_ref() {
                validate_resource_route(project, &request.route)?;
                require_capability(&runtime, ProviderCapability::ProjectGet)?;
                Some(project.native_resource_id.clone())
            } else {
                None
            };
            let permission_level = parse_permission_level(&request.permission_level)?;
            let workspace_root = prepare_conversation_workspace(
                request.workspace_root.as_deref(),
                request.workspace_mode.as_deref(),
            )?;
            let session = runtime.ready_server()?;
            let operation_runtime = runtime.clone();
            let snapshot = tokio::task::spawn_blocking(move || {
                let server_slot = {
                    let mutable = lock(&operation_runtime.mutable);
                    mutable
                        .server_session_id
                        .and_then(|id| mutable.sessions.get(&id))
                        .cloned()
                        .ok_or_else(|| instance_session_cancelled_error("server"))?
                };
                let outcome = session.thread_start_outcome_with_sender(
                    CodexThreadStartRequest {
                        workspace_root,
                        project_id,
                        permission_level,
                        model: request.model,
                        reasoning_effort: request.reasoning_effort,
                    },
                    |message| {
                        let _send_gate = lock(&server_slot.send_gate);
                        if !operation_runtime.session_is_current(&server_slot)
                            || server_slot.cancelled.load(Ordering::SeqCst)
                        {
                            return CodexRequestOutcome::NotSent(CodexAppServerError::Shutdown);
                        }
                        session.write_prepared_request(message)
                    },
                );
                match outcome {
                    CodexRequestOutcome::Success(snapshot) => {
                        let mut mutable = lock(&operation_runtime.mutable);
                        if mutable.server_generation.as_deref() != Some(session.generation())
                            || mutable.status.clone() != InstanceStatus::Ready
                        {
                            return Err(instance_session_cancelled_error("server"));
                        }
                        let slot = Arc::new(ExecutionSlot::new());
                        slot.set_starting_session(session.clone());
                        slot.set_ready(ExecutionSession {
                            session: session.clone(),
                            generation: session.generation().to_string(),
                            active_turn_id: None,
                        });
                        mutable.executions.insert(snapshot.thread.id.clone(), slot);
                        mutable.interaction_released = false;
                        mutable
                            .pending_materialization
                            .insert(snapshot.thread.id.clone(), snapshot.clone());
                        Ok(snapshot)
                    }
                    outcome => Err(execution_outcome_error("thread/start", None, outcome)),
                }
            })
            .await
            .map_err(provider_task_error)??;
            let mut conversation = lock(&runtime.mapper).conversation(&snapshot);
            runtime.project_conversation(&mut conversation);
            Ok(ConversationCreateResponse { conversation })
        })
    }

    fn turn_start<'a>(
        &'a self,
        request: TurnStartRequest,
    ) -> ProtocolFuture<'a, TurnStartResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.conversation)?;
            let conversation_id = request.conversation.native_resource_id;
            let capabilities = lock(&runtime.mutable).capabilities.clone();
            if request.capability_revision != capabilities.revision
                && request.capability_revision != runtime.snapshot().capabilities.revision
            {
                return Err(protocol_error(
                    "stale_capability_revision",
                    "turn.start capabilityRevision no longer matches the Provider instance"
                        .to_string(),
                    true,
                ));
            }
            if request.input.text.trim().is_empty() {
                return Err(protocol_error(
                    "invalid_turn_input",
                    "turn.start text input must not be empty".to_string(),
                    false,
                ));
            }
            let title_input: String = request.input.text.chars().take(6000).collect();
            let input_text = request.input.text;
            let client_request_id = request.client_request_id;
            let requested_selection = request.selection;
            let pending_snapshot = lock(&runtime.mutable)
                .pending_materialization
                .get(&conversation_id)
                .cloned();
            let operation_runtime = runtime.clone();
            let operation_conversation_id = conversation_id.clone();
            let (turn, effective_selection, needs_title) = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.start",
                    move |handle| {
                        let snapshot = match pending_snapshot.clone() {
                            Some(snapshot) => snapshot,
                            None => match handle.session.thread_read(&execution_conversation_id) {
                                Ok(snapshot) => snapshot,
                                Err(error) => {
                                    return Err(CodexProtocolMapper::error(error));
                                }
                            },
                        };
                        if let Some(active_turn) = snapshot
                            .thread
                            .turns
                            .iter()
                            .find(|turn| turn.status == CodexTurnStatus::InProgress)
                        {
                            handle
                                .slot
                                .mark_active_turn(&handle.generation, &active_turn.id);
                            return Err(protocol_error(
                                "turn_already_active",
                                "Codex conversation already has an active turn".to_string(),
                                false,
                            ));
                        }
                        if handle.slot.has_active_turn(&handle.generation) {
                            execution_runtime.finish_turn_execution(
                                &execution_conversation_id,
                                handle,
                                None,
                            );
                            if !handle.slot.matches_generation(&handle.generation) {
                                return Ok(ExecutionStep::Retry);
                            }
                        }
                        let needs_title = snapshot
                            .thread
                            .name
                            .as_deref()
                            .is_none_or(|name| name.trim().is_empty());
                        let effective_selection = match resolve_turn_selection(
                            &capabilities,
                            requested_selection.clone(),
                            TurnSelection {
                                access_mode_id: snapshot
                                    .permission_level
                                    .map(permission_level_id)
                                    .map(str::to_string),
                                reasoning_effort_id: snapshot.reasoning_effort,
                                model: snapshot.model.map(|model_id| {
                                    ModelSelection::FlatModelSelection(FlatModelSelection {
                                        kind: FlatModelCatalogKind::Flat,
                                        model_id,
                                    })
                                }),
                            },
                        ) {
                            Ok(selection) => selection,
                            Err(error) => {
                                return Err(error);
                            }
                        };
                        let permission_level = match effective_selection
                            .access_mode_id
                            .as_deref()
                            .map(parse_permission_level)
                            .transpose()
                        {
                            Ok(permission_level) => permission_level,
                            Err(error) => {
                                return Err(error);
                            }
                        };
                        let model = match effective_selection.model.as_ref() {
                            Some(ModelSelection::Unrecognized(_)) => {
                                return Err(protocol_error(
                                    "unsupported_model_selection",
                                    "Unknown model selection".into(),
                                    false,
                                ))
                            }
                            Some(ModelSelection::FlatModelSelection(selection)) => {
                                Some(selection.model_id.clone())
                            }
                            Some(ModelSelection::GroupedModelSelection(_)) => {
                                return Err(protocol_error(
                                    "invalid_model_selection",
                                    "Codex Provider requires a flat model selection".to_string(),
                                    false,
                                ));
                            }
                            None => None,
                        };
                        let outcome = handle.session.turn_start_outcome(CodexTurnStartRequest {
                            thread_id: execution_conversation_id.clone(),
                            message: input_text.clone(),
                            client_message_id: Some(client_request_id.clone()),
                            permission_level,
                            model,
                            reasoning_effort: effective_selection.reasoning_effort_id.clone(),
                        });
                        match outcome {
                            CodexRequestOutcome::Success(turn) => {
                                if turn.status == CodexTurnStatus::InProgress {
                                    handle.slot.mark_active_turn(&handle.generation, &turn.id);
                                } else {
                                    execution_runtime.finish_turn_execution(
                                        &execution_conversation_id,
                                        handle,
                                        Some(&turn.id),
                                    );
                                }
                                Ok(ExecutionStep::Complete((
                                    turn,
                                    effective_selection,
                                    needs_title,
                                )))
                            }
                            outcome => Err(execution_outcome_error(
                                "turn/start",
                                Some(&execution_conversation_id),
                                outcome,
                            )),
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            let schedule_title = needs_title
                && lock(&runtime.mutable)
                    .auto_title_attempted
                    .insert(conversation_id.clone());
            if let (true, Ok(title_session)) = (schedule_title, runtime.ready_server()) {
                let title_runtime = runtime.clone();
                let title_conversation = conversation_id.clone();
                let title_model = match &effective_selection.model {
                    Some(ModelSelection::FlatModelSelection(model)) => Some(model.model_id.clone()),
                    _ => None,
                };
                tokio::spawn(async move {
                    let Ok(_permit) = title_runtime.title_slots.clone().acquire_owned().await
                    else {
                        return;
                    };
                    if lock(&title_runtime.mutable).server_generation.as_deref()
                        != Some(title_session.generation())
                    {
                        return;
                    }
                    let diagnostic_id = title_conversation.clone();
                    let outcome = tokio::task::spawn_blocking(move || {
                        title_session.generate_thread_title(
                            &title_conversation,
                            &title_input,
                            title_model.as_deref(),
                        )
                    })
                    .await;
                    match outcome {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => eprintln!("Codex automatic title generation failed conversation_id={diagnostic_id}: {error}"),
                        Err(error) => eprintln!("Codex automatic title task failed conversation_id={diagnostic_id}: {error}"),
                    }
                });
            }
            let mapper = lock(&runtime.mapper);
            let user_item = mapper.turn_user_item(&conversation_id, &turn);
            let mapped_turn = mapper.turn(&conversation_id, &turn);
            Ok(TurnStartResponse {
                accepted: true,
                turn: mapped_turn,
                user_item,
                effective_selection,
            })
        })
    }

    fn turn_steer<'a>(
        &'a self,
        request: TurnSteerRequest,
    ) -> ProtocolFuture<'a, TurnSteerResponse> {
        Box::pin(async move {
            validate_same_resource_route(&request.conversation, &request.turn)?;
            let runtime = self.resource_instance(&request.conversation)?;
            let turn_id = request.turn.native_resource_id;
            let conversation_id = request.conversation.native_resource_id;
            let native_conversation_id = conversation_id.clone();
            let operation_conversation_id = conversation_id.clone();
            let expected_turn_id = turn_id.clone();
            let message = request.message;
            let client_message_id = request.client_message_id;
            let operation_runtime = runtime.clone();
            let turn = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.steer",
                    move |handle| {
                        let outcome = handle.session.turn_steer_outcome(CodexTurnSteerRequest {
                            thread_id: native_conversation_id.clone(),
                            expected_turn_id: expected_turn_id.clone(),
                            message: message.clone(),
                            client_message_id: Some(client_message_id.clone()),
                        });
                        match outcome {
                            CodexRequestOutcome::Success(turn) => {
                                if turn.status == CodexTurnStatus::InProgress {
                                    handle.slot.mark_active_turn(&handle.generation, &turn.id);
                                } else {
                                    execution_runtime.finish_turn_execution(
                                        &execution_conversation_id,
                                        handle,
                                        Some(&turn.id),
                                    );
                                }
                                Ok(ExecutionStep::Complete(turn))
                            }
                            outcome => Err(execution_outcome_error(
                                "turn/steer",
                                Some(&execution_conversation_id),
                                outcome,
                            )),
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn);
            Ok(TurnSteerResponse { turn: mapped_turn })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProtocolFuture<'a, TurnInterruptResponse> {
        Box::pin(async move {
            validate_same_resource_route(&request.conversation, &request.turn)?;
            let runtime = self.resource_instance(&request.conversation)?;
            let turn_id = request.turn.native_resource_id;
            let conversation_id = request.conversation.native_resource_id;
            let native_conversation_id = conversation_id.clone();
            let operation_conversation_id = conversation_id.clone();
            let native_turn_id = turn_id.clone();
            let operation_runtime = runtime.clone();
            let turn = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "turn.interrupt",
                    move |handle| {
                        let outcome = handle
                            .session
                            .turn_interrupt_outcome(&native_conversation_id, &native_turn_id);
                        match outcome {
                            CodexRequestOutcome::Success(turn) => {
                                if turn.status == CodexTurnStatus::InProgress {
                                    handle.slot.mark_active_turn(&handle.generation, &turn.id);
                                } else {
                                    execution_runtime.finish_turn_execution(
                                        &execution_conversation_id,
                                        handle,
                                        Some(&turn.id),
                                    );
                                }
                                Ok(ExecutionStep::Complete(turn))
                            }
                            outcome => Err(execution_outcome_error(
                                "turn/interrupt",
                                Some(&execution_conversation_id),
                                outcome,
                            )),
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            let mapped_turn = lock(&runtime.mapper).turn(&conversation_id, &turn);
            Ok(TurnInterruptResponse { turn: mapped_turn })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async move {
            let runtime = self.resource_instance(&request.approval)?;
            let approval_id = request.approval.native_resource_id.clone();
            let resource_generation = approval_generation(&approval_id).ok_or_else(|| {
                protocol_error(
                    "invalid_approval_resource",
                    "approval nativeResourceId does not contain a session generation".to_string(),
                    false,
                )
            })?;
            if !runtime.has_execution_generation(resource_generation) {
                return Err(protocol_error(
                    "stale_approval_session",
                    "approval belongs to a closed Codex execution session".to_string(),
                    false,
                ));
            }
            let pending = {
                let mutable = lock(&runtime.mutable);
                mutable.pending_approvals.get(&approval_id).cloned()
            }
            .ok_or_else(|| {
                protocol_error(
                    "approval_not_found",
                    format!("approval {approval_id} is not pending in this Provider instance"),
                    false,
                )
            })?;
            if pending.request.session_generation != resource_generation
                || pending.approval.resource.native_resource_id
                    != request.approval.native_resource_id
            {
                return Err(protocol_error(
                    "stale_approval_session",
                    "approval route does not match the owning Codex App Server session".to_string(),
                    false,
                ));
            }
            let conversation_id = pending.request.thread_id.clone();
            let operation_runtime = runtime.clone();
            let operation_conversation_id = conversation_id.clone();
            let pending_request = pending.request.clone();
            let pending_approval = pending.approval;
            let operation_approval_id = approval_id.clone();
            let operation_generation = resource_generation.to_string();
            let decision = request.decision;
            let approval = tokio::task::spawn_blocking(move || {
                let execution_runtime = operation_runtime.clone();
                let execution_conversation_id = operation_conversation_id.clone();
                operation_runtime.with_current_execution(
                    &operation_conversation_id,
                    "approval.resolve",
                    move |handle| {
                        if handle.generation != operation_generation {
                            return Err(protocol_error(
                                "stale_approval_session",
                                "approval belongs to a closed Codex execution session".to_string(),
                                false,
                            ));
                        }
                        match handle
                            .session
                            .respond_to_approval_outcome(&pending_request, decision.clone())
                        {
                            CodexRequestOutcome::Success(()) => {
                                lock(&execution_runtime.mutable)
                                    .pending_approvals
                                    .remove(&operation_approval_id);
                                let (approval, event) = lock(&execution_runtime.mapper)
                                    .approval_resolved(
                                        pending_approval.clone(),
                                        decision.clone(),
                                        now_ms(),
                                    )?;
                                {
                                    let mut mutable = lock(&execution_runtime.mutable);
                                    update_recorded_approval(&mut mutable, &approval);
                                }
                                if let Err(error) = execution_runtime.events.publish(event) {
                                    return Err(error);
                                }
                                Ok(ExecutionStep::Complete(approval))
                            }
                            outcome => Err(execution_outcome_error(
                                "approval/resolve",
                                Some(&execution_conversation_id),
                                outcome,
                            )),
                        }
                    },
                )
            })
            .await
            .map_err(provider_task_error)??;
            Ok(ApprovalResolveResponse { approval })
        })
    }

    fn provider_shutdown<'a>(
        &'a self,
        _request: ProviderShutdownRequest,
    ) -> ProtocolFuture<'a, ProviderShutdownResponse> {
        Box::pin(async move {
            self.scanner.stop();
            if self.shutdown.swap(true, Ordering::SeqCst) {
                loop {
                    let notified = self.shutdown_changed.notified();
                    if self.shutdown_complete.load(Ordering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
                return Ok(ProviderShutdownResponse { accepted: true });
            }
            self.observation.shutdown().await;
            let instances = lock(&self.state)
                .instances
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let mut first_error = None;
            for runtime in instances {
                if let Err(error) = runtime.stop().await {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            self.shutdown_complete.store(true, Ordering::SeqCst);
            self.shutdown_changed.notify_waiters();
            if let Some(error) = first_error {
                return Err(error);
            }
            Ok(ProviderShutdownResponse { accepted: true })
        })
    }
}
