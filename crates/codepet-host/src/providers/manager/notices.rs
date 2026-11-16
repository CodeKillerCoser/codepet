//! Notice RPC uses the current Provider connection and its authenticated plugin identity.
use super::*;
use codepet_provider_sdk::common::NoticeSource;
use codepet_provider_sdk::{
    JsonRpcResponse, JsonRpcResponsePayload, NoticeObserveRequest, RpcError,
};
#[derive(Clone)]
pub(super) struct SourceState {
    generation: u64,
    connected: bool,
    configured: bool,
    message: String,
    gate: Arc<Mutex<()>>,
}
impl Default for SourceState {
    fn default() -> Self {
        Self {
            generation: 0,
            connected: false,
            configured: false,
            message: "正在连接观察来源".into(),
            gate: Arc::new(Mutex::new(())),
        }
    }
}
impl PluginManager {
    pub async fn notice_action(
        &self,
        plugin: &str,
        request: codepet_provider_sdk::NoticeActionExecuteRequest,
    ) -> Result<codepet_provider_sdk::NoticeActionResult, codepet_provider_sdk::ProtocolError> {
        let process = self
            .inner
            .plugins
            .read()
            .await
            .get(plugin)
            .and_then(|p| p.process.clone())
            .ok_or_else(|| notice_error("Notice source is unavailable"))?;
        process.client().notice_action_execute(request).await
    }
    pub fn notice_store(&self) -> Arc<crate::notices::NoticeStore> {
        self.inner.notices.clone()
    }
    pub async fn notice_sources(&self) -> HostResult<Vec<NoticeSource>> {
        let mut sources = vec![];
        for snapshot in self.snapshots().await {
            if !snapshot
                .reported
                .as_ref()
                .and_then(|p| p.features.as_ref())
                .is_some_and(|f| f.contains(&codepet_provider_sdk::ProviderFeature::Observation))
            {
                continue;
            }
            let enabled = self
                .inner
                .notices
                .enabled(&snapshot.catalog.plugin_id)
                .map_err(HostError::from)?;
            let state = self
                .inner
                .notice_sources
                .lock()
                .unwrap()
                .get(&snapshot.catalog.plugin_id)
                .cloned()
                .unwrap_or_default();
            sources.push(NoticeSource {
                plugin_id: snapshot.catalog.plugin_id,
                display_name: snapshot.catalog.display_name,
                icon: Some(snapshot.catalog.icon),
                enabled,
                connected: enabled
                    && state.connected
                    && state.generation == snapshot.generation
                    && snapshot.state == PluginRuntimeState::Ready,
                message: state.message,
            });
        }
        Ok(sources)
    }
    pub async fn set_notice_source_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> HostResult<NoticeSource> {
        self.configure_notice_source(id, Some(enabled)).await
    }
    async fn configure_notice_source(
        &self,
        id: &str,
        requested: Option<bool>,
    ) -> HostResult<NoticeSource> {
        let gate = self
            .inner
            .notice_sources
            .lock()
            .unwrap()
            .entry(id.into())
            .or_default()
            .gate
            .clone();
        let _guard = gate.lock().await;
        let (process, generation) = {
            let plugins = self.inner.plugins.read().await;
            let p = plugins
                .get(id)
                .ok_or_else(|| HostError::new("unknown_notice_source", id))?;
            if p.state != PluginRuntimeState::Ready
                || !p
                    .reported
                    .as_ref()
                    .and_then(|p| p.features.as_ref())
                    .is_some_and(|f| {
                        f.contains(&codepet_provider_sdk::ProviderFeature::Observation)
                    })
            {
                return Err(HostError::new("provider_not_ready", id));
            }
            (
                p.process
                    .clone()
                    .ok_or_else(|| HostError::new("provider_not_ready", id))?,
                p.generation,
            )
        };
        let enabled = requested.unwrap_or(self.inner.notices.enabled(id).map_err(HostError::from)?);
        let response = process
            .client()
            .notice_observe(NoticeObserveRequest { enabled })
            .await;
        if !self
            .is_current_process(id, generation, Arc::as_ptr(&process) as usize)
            .await
        {
            return Err(HostError::new("provider_replaced", id));
        }
        let mut state = self
            .inner
            .notice_sources
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_default();
        state.generation = generation;
        match response {
            Ok(response) if response.enabled == enabled => {
                self.inner
                    .notices
                    .set_enabled(id, enabled)
                    .map_err(HostError::from)?;
                state.connected = enabled;
                state.configured = true;
                state.message = response.message;
            }
            result => {
                state.connected = false;
                state.configured = false;
                state.message = match result {
                    Err(e) => e.message,
                    Ok(_) => "Provider returned a different observation state".into(),
                };
                self.inner
                    .notice_sources
                    .lock()
                    .unwrap()
                    .insert(id.into(), state.clone());
                return Err(HostError::new("notice_observation_failed", state.message));
            }
        }
        self.inner
            .notice_sources
            .lock()
            .unwrap()
            .insert(id.into(), state);
        self.notice_sources()
            .await?
            .into_iter()
            .find(|s| s.plugin_id == id)
            .ok_or_else(|| HostError::new("unknown_notice_source", id))
    }
    pub fn start_notice_observation(&self) {
        if self
            .inner
            .notice_observation_started
            .swap(true, Ordering::SeqCst)
        {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut attempts: BTreeMap<String, (u64, std::time::Instant, Arc<AtomicBool>)> =
                BTreeMap::new();
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if inner.shutting_down.load(Ordering::SeqCst) {
                    return;
                }
                let manager = PluginManager { inner };
                let snapshots = manager.snapshots().await;
                attempts.retain(|id, _| snapshots.iter().any(|s| &s.catalog.plugin_id == id));
                for snapshot in snapshots {
                    let id = snapshot.catalog.plugin_id;
                    if snapshot.state != PluginRuntimeState::Ready
                        || !snapshot
                            .reported
                            .as_ref()
                            .and_then(|p| p.features.as_ref())
                            .is_some_and(|f| {
                                f.contains(&codepet_provider_sdk::ProviderFeature::Observation)
                            })
                    {
                        continue;
                    }
                    let configured = manager
                        .inner
                        .notice_sources
                        .lock()
                        .unwrap()
                        .get(&id)
                        .is_some_and(|s| s.generation == snapshot.generation && s.configured);
                    if configured {
                        continue;
                    }
                    if attempts.get(&id).is_some_and(|(generation, at, busy)| {
                        busy.load(Ordering::SeqCst)
                            || (*generation == snapshot.generation
                                && at.elapsed() < Duration::from_secs(5))
                    }) {
                        continue;
                    }
                    let busy = Arc::new(AtomicBool::new(true));
                    attempts.insert(
                        id.clone(),
                        (snapshot.generation, std::time::Instant::now(), busy.clone()),
                    );
                    let manager = manager.clone();
                    tokio::spawn(async move {
                        let _ = manager.configure_notice_source(&id, None).await;
                        busy.store(false, Ordering::SeqCst);
                    });
                }
            }
        });
    }
    pub(super) async fn notice_request(
        &self,
        plugin: &str,
        generation: u64,
        identity: usize,
        request: codepet_provider_sdk::ProtocolRequest,
    ) -> JsonRpcResponse {
        let id = Some(request.id().clone());
        if request.method().direction() != "pluginToHost" {
            return rpc_error(id, -32601, "Provider cannot call a hostToPlugin method");
        }
        let operation = self.plugin_operation(plugin).ok();
        let _operation = if let Some(operation) = operation.as_ref() {
            Some(operation.lock().await)
        } else {
            None
        };
        let current = self.is_current_process(plugin, generation, identity).await;
        let valid = {
            let plugins = self.inner.plugins.read().await;
            plugins.get(plugin).is_some_and(|p| {
                p.generation == generation
                    && p.state == PluginRuntimeState::Ready
                    && p.reported
                        .as_ref()
                        .and_then(|p| p.features.as_ref())
                        .is_some_and(|f| {
                            f.contains(&codepet_provider_sdk::ProviderFeature::Observation)
                        })
            })
        };
        let result = if !valid || !current {
            Err(codepet_provider_sdk::ProtocolError {
                code: "provider_not_ready".into(),
                message: "Notice publication requires the current initialized observation plugin"
                    .into(),
                retryable: true,
                details: None,
            })
        } else {
            let store = self.inner.notices.clone();
            let plugin = plugin.to_owned();
            tokio::task::spawn_blocking(move || {
                use codepet_provider_sdk::ProtocolRequest::*;
                if !store.enabled(&plugin)? {
                    return Err(notice_error("Notice source is disabled"));
                }
                match request {
                    NoticeCreate { params, .. } => store
                        .create(&plugin, params)
                        .and_then(|r| serde_json::to_value(r).map_err(notice_error)),
                    NoticeUpdate { params, .. } => store
                        .update(&plugin, params)
                        .and_then(|r| serde_json::to_value(r).map_err(notice_error)),
                    NoticeFinish { params, .. } => store
                        .finish(&plugin, params)
                        .and_then(|r| serde_json::to_value(r).map_err(notice_error)),
                    _ => Err(notice_error("Provider cannot call a hostToPlugin method")),
                }
            })
            .await
            .unwrap_or_else(|e| Err(notice_error(e)))
        };
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            response: match result {
                Ok(result) => JsonRpcResponsePayload::Ok { result },
                Err(error) => JsonRpcResponsePayload::Error {
                    error: RpcError {
                        code: -32000,
                        message: error.message.clone(),
                        data: serde_json::to_value(error).ok().and_then(|v| {
                            v.as_object()
                                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        }),
                    },
                },
            },
        }
    }
}
pub(super) fn rpc_error(
    id: Option<codepet_provider_sdk::RequestId>,
    code: i64,
    message: &str,
) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        response: JsonRpcResponsePayload::Error {
            error: RpcError {
                code,
                message: message.into(),
                data: None,
            },
        },
    }
}
fn notice_error(e: impl ToString) -> codepet_provider_sdk::ProtocolError {
    codepet_provider_sdk::ProtocolError {
        code: "notice_request_failed".into(),
        message: e.to_string(),
        retryable: false,
        details: None,
    }
}
