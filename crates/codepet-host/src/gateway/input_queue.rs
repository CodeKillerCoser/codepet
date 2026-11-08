use super::*;
use crate::input_queue::{error, InputQueueStore};

impl ProviderGatewayService {
    pub(super) fn queue_store(&self) -> Result<&InputQueueStore, gateway::ProtocolError> {
        self.input_queue.as_ref().ok_or_else(|| {
            error(
                "input_queue_unavailable",
                "Host has not configured persistent input queues",
            )
        })
    }
    pub(super) fn input_lock(
        &self,
        route: &gateway::RoutedResourceId,
    ) -> Result<Arc<AsyncMutex<()>>, gateway::ProtocolError> {
        let mut locks = self
            .queue_locks
            .lock()
            .map_err(|_| error("input_queue_unavailable", "Queue lock is unavailable"))?;
        locks.retain(|_, v| v.strong_count() > 0);
        let entry = locks
            .entry((route.provider_id.clone(), route.native_resource_id.clone()))
            .or_default();
        if let Some(lock) = entry.upgrade() {
            return Ok(lock);
        }
        let lock = Arc::new(AsyncMutex::new(()));
        *entry = Arc::downgrade(&lock);
        Ok(lock)
    }
    pub(super) fn input_request_lock(
        &self,
        caller: &str,
        method: &str,
        provider: &str,
        id: &str,
    ) -> Result<Arc<AsyncMutex<()>>, gateway::ProtocolError> {
        let mut locks = self
            .input_request_locks
            .lock()
            .map_err(|_| error("input_queue_unavailable", "Request lock is unavailable"))?;
        locks.retain(|_, v| v.strong_count() > 0);
        let entry = locks
            .entry((caller.into(), method.into(), provider.into(), id.into()))
            .or_default();
        if let Some(lock) = entry.upgrade() {
            return Ok(lock);
        }
        let lock = Arc::new(AsyncMutex::new(()));
        *entry = Arc::downgrade(&lock);
        Ok(lock)
    }
    pub(super) fn add_queue_capabilities(&self, capabilities: &mut gateway::GatewayCapabilities) {
        if self.input_queue.is_none()
            || !capabilities
                .methods
                .contains(&gateway::GatewayCapability::TurnSend)
            || !capabilities
                .methods
                .contains(&gateway::GatewayCapability::ConversationGet)
        {
            return;
        }
        for cap in [
            gateway::GatewayCapability::ConversationInputQueueGet,
            gateway::GatewayCapability::ConversationInputQueueEnqueue,
            gateway::GatewayCapability::ConversationInputQueueWithdraw,
            gateway::GatewayCapability::ConversationInputQueueResume,
            gateway::GatewayCapability::ConversationInputQueueAcknowledgeEdit,
        ] {
            if !capabilities.methods.contains(&cap) {
                capabilities.methods.push(cap);
            }
        }
        if capabilities
            .methods
            .contains(&gateway::GatewayCapability::TurnSteer)
        {
            capabilities
                .methods
                .push(gateway::GatewayCapability::ConversationInputQueueSteer);
        }
    }
    pub(super) fn publish_queue_changed(
        &self,
        route: &gateway::RoutedResourceId,
    ) -> Result<(), gateway::ProtocolError> {
        let Some(store) = &self.input_queue else {
            return Ok(());
        };
        // Notifications are hints. A publication failure must not turn a committed
        // mutation into a failed acknowledgement; clients also refresh snapshots.
        let publication = (|| {
            let Some(revision) = store.revision(route)? else {
                return Ok(());
            };
            self.events
                .publish(gateway::ProtocolEvent::ConversationInputQueueChanged {
                    jsonrpc: "2.0".into(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::InputQueueChangedEvent {
                            conversation: route.clone(),
                            revision,
                        },
                    },
                })?;
            Ok::<(), gateway::ProtocolError>(())
        })();
        if let Err(e) = publication {
            eprintln!("Input queue notification failed: {}", e.code);
        }
        Ok(())
    }
    pub(super) async fn input_capabilities(
        &self,
        route: &gateway::RoutedResourceId,
        revision: &str,
        capability: gateway::GatewayCapability,
    ) -> Result<gateway::GatewayCapabilities, gateway::ProtocolError> {
        let provider = self
            .gateway_providers(None)
            .await?
            .into_iter()
            .find(|p| p.summary.id == route.provider_id)
            .ok_or_else(|| {
                error(
                    "unknown_provider_instance",
                    "Provider instance was not found",
                )
            })?;
        if provider.summary.runtime.status != gateway::ProviderStatus::Ready {
            return Err(error(
                "provider_instance_unavailable",
                "Provider is not ready",
            ));
        }
        if !provider.capabilities.methods.contains(&capability) {
            return Err(error(
                "provider_capability_unsupported",
                "Provider does not support this input operation",
            ));
        }
        if provider.capabilities.revision != revision {
            return Err(error(
                "stale_capability_revision",
                "Refresh Provider capabilities before submitting this input",
            ));
        }
        Ok(provider.capabilities)
    }
    pub(super) async fn input_conversation(
        &self,
        route: &gateway::RoutedResourceId,
    ) -> Result<gateway::Conversation, gateway::ProtocolError> {
        let conversation = self.resolve_resource(route.clone()).await?;
        Ok(self
            .manager
            .conversation_get(provider::ConversationGetRequest {
                conversation,
                cursor: None,
                limit: Some(1),
            })
            .await
            .map_err(gateway_error)?
            .conversation)
    }
    pub(super) async fn validate_steer(
        &self,
        request: &gateway::TurnSteerRequest,
    ) -> Result<(), gateway::ProtocolError> {
        validate_input(&request.client_request_id, &request.input)?;
        ensure_same_gateway_route(&request.conversation, &request.turn)?;
        self.input_capabilities(
            &request.conversation,
            &request.capability_revision,
            gateway::GatewayCapability::TurnSteer,
        )
        .await?;
        let snapshot = self.input_conversation(&request.conversation).await?;
        if snapshot.active_turn.as_ref().map(|turn| &turn.resource) != Some(&request.turn) {
            return Err(error(
                "stale_active_turn",
                "The active turn changed; refresh the conversation before steering",
            ));
        }
        if snapshot.status != gateway::ConversationStatus::Running {
            return Err(error(
                "turn_not_steerable",
                "Steering is unavailable while awaiting approval or user input",
            ));
        }
        Ok(())
    }
    pub(super) async fn perform_steer(
        &self,
        request: gateway::TurnSteerRequest,
    ) -> Result<gateway::TurnSteerResponse, gateway::ProtocolError> {
        self.validate_steer(&request).await?;
        let response = self
            .manager
            .turn_steer(provider::TurnSteerRequest {
                conversation: self.resolve_resource(request.conversation).await?,
                turn: self.resolve_resource(request.turn).await?,
                client_message_id: request.client_request_id,
                message: request.input.text,
            })
            .await
            .map_err(gateway_error)?;
        Ok(gateway::TurnSteerResponse {
            turn: response.turn,
        })
    }
    pub async fn turn_steer_for_caller_scope(
        &self,
        caller: &str,
        request: gateway::TurnSteerRequest,
    ) -> Result<gateway::TurnSteerResponse, gateway::ProtocolError> {
        validate_input(&request.client_request_id, &request.input)?;
        let request_lock = self.input_request_lock(
            caller,
            "turn.steer",
            &request.conversation.provider_id,
            &request.client_request_id,
        )?;
        let _request_guard = request_lock.lock().await;
        let key = (
            caller.to_owned(),
            request.conversation.provider_id.clone(),
            request.client_request_id.clone(),
        );
        let lock = self.input_lock(&request.conversation)?;
        let _guard = lock.lock().await;
        if let Some((old, result)) = self.turn_steers.lock().await.get(&key) {
            if old != &request {
                return Err(error(
                    "client_request_conflict",
                    "clientRequestId was already used for a different steer",
                ));
            }
            return result.clone();
        }
        let result = self.perform_steer(request.clone()).await;
        let mut cache = self.turn_steers.lock().await;
        if cache.len() >= TURN_SEND_CACHE_CAPACITY {
            cache.pop_first();
        }
        cache.insert(key, (request, result.clone()));
        result
    }
    pub(super) async fn input_queue_get(
        &self,
        caller: &str,
        request: gateway::InputQueueGetRequest,
    ) -> Result<gateway::InputQueueSnapshot, gateway::ProtocolError> {
        self.resolve_resource(request.conversation.clone()).await?;
        self.queue_store()?
            .snapshot(&request.conversation, caller, self.current_event_cursor())
    }
    pub(super) async fn input_queue_enqueue(
        &self,
        caller: &str,
        request: gateway::InputQueueEnqueueRequest,
    ) -> Result<gateway::InputQueueMutationResponse, gateway::ProtocolError> {
        validate_input(&request.client_request_id, &request.input)?;
        let store = self.queue_store()?;
        let lock = self.input_lock(&request.conversation)?;
        let _guard = lock.lock().await;
        if let Some(result) = store.existing_enqueue(caller, &request)? {
            return Ok(result);
        }
        let capabilities = self
            .input_capabilities(
                &request.conversation,
                &request.capability_revision,
                gateway::GatewayCapability::TurnSend,
            )
            .await?;
        validate_gateway_turn_selection(&capabilities, &request.selection)?;
        let conversation = self.input_conversation(&request.conversation).await?;
        if conversation.status == gateway::ConversationStatus::Archived {
            return Err(error(
                "conversation_archived",
                "Archived conversations cannot accept queued input",
            ));
        }
        let result = store.enqueue(caller, &request, &conversation)?;
        self.publish_queue_changed(&request.conversation)?;
        self.queue_wake.notify_one();
        Ok(result)
    }
    pub(super) async fn input_queue_withdraw(
        &self,
        caller: &str,
        request: gateway::InputQueueWithdrawRequest,
    ) -> Result<gateway::InputQueueMutationResponse, gateway::ProtocolError> {
        validate_request_id(&request.client_request_id)?;
        let lock = self.input_lock(&request.conversation)?;
        let _guard = lock.lock().await;
        let result = self.queue_store()?.withdraw(caller, &request)?;
        self.publish_queue_changed(&request.conversation)?;
        self.queue_wake.notify_one();
        Ok(result)
    }
    pub(super) async fn input_queue_acknowledge_edit(
        &self,
        caller: &str,
        request: gateway::InputQueueAcknowledgeEditRequest,
    ) -> Result<gateway::InputQueueSnapshot, gateway::ProtocolError> {
        self.queue_store()?.acknowledge_edit(caller, &request)?;
        self.publish_queue_changed(&request.conversation)?;
        self.input_queue_get(
            caller,
            gateway::InputQueueGetRequest {
                conversation: request.conversation,
            },
        )
        .await
    }
    pub(super) async fn input_queue_resume(
        &self,
        caller: &str,
        request: gateway::InputQueueResumeRequest,
    ) -> Result<gateway::InputQueueSnapshot, gateway::ProtocolError> {
        let lock = self.input_lock(&request.conversation)?;
        let _guard = lock.lock().await;
        let conversation = self.input_conversation(&request.conversation).await?;
        if matches!(conversation.status, gateway::ConversationStatus::Archived) {
            return Err(error(
                "conversation_unavailable",
                "Resolve the conversation state before resuming queued messages",
            ));
        }
        self.manager
            .conversation_acquire_interaction(provider::ConversationAcquireInteractionRequest {
                conversation: self.resolve_resource(request.conversation.clone()).await?,
                force: None,
            })
            .await
            .map_err(gateway_error)?;
        self.queue_store()?.resume(
            &request,
            conversation.active_turn.as_ref().map(|turn| &turn.resource),
        )?;
        self.publish_queue_changed(&request.conversation)?;
        self.queue_wake.notify_one();
        self.input_queue_get(
            caller,
            gateway::InputQueueGetRequest {
                conversation: request.conversation,
            },
        )
        .await
    }
    pub(super) async fn input_queue_steer(
        &self,
        caller: &str,
        request: gateway::InputQueueSteerRequest,
    ) -> Result<gateway::InputQueueMutationResponse, gateway::ProtocolError> {
        validate_request_id(&request.client_request_id)?;
        let lock = self.input_lock(&request.conversation)?;
        let _guard = lock.lock().await;
        let store = self.queue_store()?;
        if let Some(result) = store.steer_operation(caller, &request)? {
            return Ok(result);
        }
        let snapshot =
            store.snapshot(&request.conversation, caller, self.current_event_cursor())?;
        let item = snapshot
            .items
            .iter()
            .find(|i| i.id == request.item_id)
            .ok_or_else(|| error("input_queue_item_not_found", "Queue item was not found"))?;
        let steer = gateway::TurnSteerRequest {
            conversation: request.conversation.clone(),
            turn: request.turn.clone(),
            client_request_id: format!("queue:{}", item.id),
            capability_revision: request.capability_revision.clone(),
            input: item.input.clone(),
        };
        if let Err(e) = self.validate_steer(&steer).await {
            // A stale steer must not quietly become a fresh turn via auto-drain.
            store.pause(&request.conversation)?;
            self.publish_queue_changed(&request.conversation)?;
            return Err(e);
        }
        store.claim_steer(caller, &request)?;
        let dispatch = store.dispatch_guard(&request.conversation, &request.item_id);
        self.publish_queue_changed(&request.conversation)?;
        let result = match self.perform_steer(steer).await {
            Ok(response) => store.finish(
                &request.conversation,
                &request.item_id,
                Some(&response.turn),
                None,
            ),
            Err(e) => store.finish(
                &request.conversation,
                &request.item_id,
                None,
                Some((&e, uncertain(&e))),
            ),
        }?;
        dispatch.complete();
        self.publish_queue_changed(&request.conversation)?;
        self.queue_wake.notify_one();
        Ok(result)
    }
    pub(super) fn start_input_queue_worker(self: &Arc<Self>) {
        if self.input_queue.is_none() {
            return;
        }
        let weak = Arc::downgrade(self);
        let wake = self.queue_wake.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                tokio::select! {_ = wake.notified()=>{},_ = tick.tick()=>{}}
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let routes = match service.queue_store().and_then(|s| s.routes(None)) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("Input queue scan failed: {}", e.code);
                        continue;
                    }
                };
                for route in routes {
                    let service = Arc::downgrade(&service);
                    tokio::spawn(async move {
                        if let Some(service) = service.upgrade() {
                            if let Err(e) = service.drive_input_queue(route).await {
                                eprintln!("Input queue dispatch failed: {}", e.code);
                            }
                        }
                    });
                }
            }
        });
    }
    pub(super) async fn drive_input_queue(
        &self,
        route: gateway::RoutedResourceId,
    ) -> Result<(), gateway::ProtocolError> {
        let lock = self.input_lock(&route)?;
        let Ok(_guard) = lock.try_lock() else {
            return Ok(());
        };
        let store = self.queue_store()?;
        let Some(pending) = store.head(&route)? else {
            return Ok(());
        };
        let snapshot = match self.input_conversation(&route).await {
            Ok(s) => s,
            Err(e) => {
                store.finish(&route, &pending.item.id, None, Some((&e, false)))?;
                return self.publish_queue_changed(&route);
            }
        };
        if let Some(turn) = snapshot.active_turn {
            store.remember_active(&route, &turn.resource)?;
            return Ok(());
        }
        if matches!(
            snapshot.status,
            gateway::ConversationStatus::Running
                | gateway::ConversationStatus::WaitingApproval
                | gateway::ConversationStatus::WaitingUserInput
        ) {
            return Ok(());
        }
        if !matches!(
            snapshot.status,
            gateway::ConversationStatus::Idle | gateway::ConversationStatus::Error
        ) {
            store.finish(
                &route,
                &pending.item.id,
                None,
                Some((
                    &error(
                        "conversation_unavailable",
                        "Conversation is not idle; review it before continuing the queue",
                    ),
                    false,
                )),
            )?;
            return self.publish_queue_changed(&route);
        }
        if let Err(e) = self
            .manager
            .conversation_acquire_interaction(provider::ConversationAcquireInteractionRequest {
                conversation: self.resolve_resource(route.clone()).await?,
                force: None,
            })
            .await
        {
            store.finish(
                &route,
                &pending.item.id,
                None,
                Some((&gateway_error(e), false)),
            )?;
            return self.publish_queue_changed(&route);
        }
        let validation = self
            .input_capabilities(
                &route,
                &pending.item.capability_revision,
                gateway::GatewayCapability::TurnSend,
            )
            .await
            .and_then(|caps| validate_gateway_turn_selection(&caps, &pending.item.selection));
        if let Err(e) = validation {
            store.finish(&route, &pending.item.id, None, Some((&e, false)))?;
            return self.publish_queue_changed(&route);
        }
        store.claim_start(&route, &pending.item.id)?;
        let dispatch = store.dispatch_guard(&route, &pending.item.id);
        self.publish_queue_changed(&route)?;
        let request = gateway::TurnSendRequest {
            conversation: route.clone(),
            client_request_id: format!("queue:{}", pending.item.id),
            capability_revision: pending.item.capability_revision,
            input: pending.item.input,
            selection: pending.item.selection,
        };
        match self.perform_turn_send(request).await {
            Ok(response) => {
                store.finish(&route, &pending.item.id, Some(&response.turn), None)?;
            }
            Err(e) => {
                store.finish(&route, &pending.item.id, None, Some((&e, uncertain(&e))))?;
            }
        }
        dispatch.complete();
        self.publish_queue_changed(&route)?;
        self.queue_wake.notify_one();
        Ok(())
    }
}
fn validate_request_id(id: &str) -> Result<(), gateway::ProtocolError> {
    if id.trim().is_empty() || id.len() > 512 {
        return Err(error(
            "invalid_client_request_id",
            "A nonempty request ID of at most 512 bytes is required",
        ));
    }
    Ok(())
}
fn validate_input(id: &str, input: &gateway::TurnInput) -> Result<(), gateway::ProtocolError> {
    validate_request_id(id)?;
    if input.text.trim().is_empty() || input.text.len() > 128 * 1024 {
        return Err(error(
            "invalid_turn_input",
            "Input must contain text and be at most 128 KiB",
        ));
    }
    Ok(())
}
fn uncertain(e: &gateway::ProtocolError) -> bool {
    if e.details
        .as_ref()
        .and_then(|d| d.get("outcome"))
        .and_then(|v| v.as_str())
        == Some("unknown")
    {
        return true;
    }
    !matches!(
        e.code.as_str(),
        "turn_already_active"
            | "stale_active_turn"
            | "turn_not_steerable"
            | "invalid_turn_input"
            | "stale_capability_revision"
            | "provider_capability_unsupported"
            | "unsupported"
            | "conversation_write_conflict"
            | "provider_instance_unavailable"
            | "invalid_turn_selection"
            | "unsupported_model"
            | "unsupported_access_mode"
            | "unsupported_reasoning_effort"
    )
}
