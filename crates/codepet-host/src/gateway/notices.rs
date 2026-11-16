//! Local Notice queries/actions over the existing Gateway RPC and event bus.
use super::*;
use codepet_provider_sdk::{
    self as p,
    common::{NoticeActionResultStatus, NoticeActivationTarget},
};
pub trait NoticeActivator: Send + Sync {
    fn activate<'a>(
        &'a self,
        target: NoticeActivationTarget,
    ) -> p::ProtocolFuture<'a, NoticeActionResultStatus>;
}
impl ProviderGatewayService {
    pub fn set_notice_activator(&self, activator: Arc<dyn NoticeActivator>) {
        *self.notice_activator.lock().unwrap() = Some(activator);
    }
    pub(super) async fn local_notice_request(
        &self,
        request: &gateway::ProtocolRequest,
    ) -> Option<gateway::JsonRpcResponse> {
        use gateway::ProtocolRequest::*;
        let (id, result) = match request {
            NoticeList { id, params, .. } => {
                let limit = params.limit.unwrap_or(100).clamp(1, 200);
                let offset = params.offset.unwrap_or(0);
                let store = self.manager.notice_store();
                let result = tokio::task::spawn_blocking(move || store.list(offset, limit + 1))
                    .await
                    .map_err(|e| notice_error(e))
                    .and_then(|r| r);
                let result = match result {
                    Ok(mut notices) => {
                        let has_more = notices.len() > limit as usize;
                        notices.truncate(limit as usize);
                        match self.manager.notice_sources().await {
                            Ok(sources) => serde_json::to_value(gateway::NoticeListResponse {
                                notices,
                                sources,
                                has_more,
                            })
                            .map_err(notice_error),
                            Err(e) => Err(gateway_error(e)),
                        }
                    }
                    Err(e) => Err(e),
                };
                (id, result)
            }
            NoticeGet { id, params, .. } => {
                let result =
                    self.manager
                        .notice_store()
                        .get(&params.notice_id)
                        .and_then(|notice| {
                            serde_json::to_value(gateway::NoticeGetResponse { notice })
                                .map_err(notice_error)
                        });
                (id, result)
            }
            NoticeSourceSetEnabled { id, params, .. } => {
                let result = self
                    .manager
                    .set_notice_source_enabled(&params.plugin_id, params.enabled)
                    .await
                    .map_err(gateway_error)
                    .and_then(|source| {
                        serde_json::to_value(gateway::NoticeSourceSetEnabledResponse { source })
                            .map_err(notice_error)
                    });
                (id, result)
            }
            NoticeActionExecute { id, params, .. } => {
                let request = p::NoticeActionExecuteRequest {
                    notice_id: params.notice_id.clone(),
                    action_id: params.action_id.clone(),
                    expected_revision: params.expected_revision,
                    execution_id: params.execution_id.clone(),
                    input: params.input.clone(),
                };
                (
                    id,
                    self.execute_notice_action(request)
                        .await
                        .and_then(|r| serde_json::to_value(r).map_err(notice_error)),
                )
            }
            _ => return None,
        };
        Some(json_rpc_response("2.0".into(), id.clone(), result))
    }
    async fn execute_notice_action(
        &self,
        request: p::NoticeActionExecuteRequest,
    ) -> Result<p::NoticeActionResult, p::ProtocolError> {
        let store = self.manager.notice_store();
        let notice = store.get(&request.notice_id)?;
        let snapshot = self
            .manager
            .snapshot(&notice.plugin_id)
            .await
            .map_err(gateway_error)?;
        if snapshot.state != PluginRuntimeState::Ready || !store.enabled(&notice.plugin_id)? {
            return Err(notice_error("Notice source is unavailable"));
        }
        let claim = {
            let store = store.clone();
            let request = request.clone();
            tokio::task::spawn_blocking(move || store.claim_action(&request))
                .await
                .map_err(notice_error)??
        };
        match claim {
            crate::notices::ActionClaim::Existing(result) => Ok(result),
            crate::notices::ActionClaim::Execute { notice, action } => {
                let result =
                    if let p::NoticeAction::NoticeActivateAction(action) = action {
                        let activator = self.notice_activator.lock().unwrap().clone();
                        match (activator, action.target) {
                            (Some(activator), Some(target)) => activator
                                .activate(target)
                                .await
                                .map(|status| p::NoticeActionResult {
                                    execution_id: request.execution_id.clone(),
                                    status,
                                    message: None,
                                }),
                            _ => Err(notice_error("Application activation is unavailable")),
                        }
                    } else {
                        match self
                            .manager
                            .notice_action(&notice.plugin_id, request.clone())
                            .await
                        {
                            Ok(result) => Ok(result),
                            Err(error) => Err(error),
                        }
                    };
                let result = result.unwrap_or_else(|e| p::NoticeActionResult {
                    execution_id: request.execution_id.clone(),
                    status: NoticeActionResultStatus::Unknown,
                    message: Some(e.message),
                });
                if result.execution_id != request.execution_id {
                    return Err(notice_error(
                        "Provider returned a different action execution ID",
                    ));
                }
                store.complete_action(&result)?;
                Ok(result)
            }
        }
    }
    pub(super) fn start_notice_forwarding(self: &Arc<Self>) {
        let mut changed = self.manager.notice_store().subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                match changed.recv().await {
                    Ok(notice) => {
                        let Some(service) = weak.upgrade() else {
                            return;
                        };
                        let _ = service
                            .events
                            .publish(gateway::ProtocolEvent::NoticeChanged {
                                jsonrpc: "2.0".into(),
                                params: gateway::ProtocolEventParams {
                                    event_cursor: String::new(),
                                    payload: gateway::NoticeChangedEvent { notice },
                                },
                            });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let Some(service) = weak.upgrade() else {
                            return;
                        };
                        if let Ok(notices) = service.manager.notice_store().list(0, 200) {
                            for notice in notices {
                                let _ =
                                    service
                                        .events
                                        .publish(gateway::ProtocolEvent::NoticeChanged {
                                            jsonrpc: "2.0".into(),
                                            params: gateway::ProtocolEventParams {
                                                event_cursor: String::new(),
                                                payload: gateway::NoticeChangedEvent { notice },
                                            },
                                        });
                            }
                        }
                    }
                    Err(_) => return,
                }
            }
        });
    }
}
fn notice_error(error: impl ToString) -> p::ProtocolError {
    p::ProtocolError {
        code: "notice_action_failed".into(),
        message: error.to_string(),
        retryable: false,
        details: None,
    }
}
