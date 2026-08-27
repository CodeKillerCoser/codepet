use super::gateway::Gateway;
use super::generated::{
    EventSequence, ProtocolError, ProtocolEvent, ProtocolRequest, ProtocolResponse,
    ProviderStatusChangedEvent, PROTOCOL_VERSION,
};
use super::transport::{LocalTransport, Transport};
use crate::agent::codex_app_server::CodexProviderAdapter;
use crate::runtime_gateway::ProviderAdapter;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub const RUNTIME_GATEWAY_EVENT: &str = "runtime-gateway-event";

#[derive(Clone)]
pub struct RuntimeGatewayState {
    gateway: Arc<Gateway>,
    transport: LocalTransport,
    codex: Arc<Mutex<Option<Arc<CodexProviderAdapter>>>>,
}

impl Default for RuntimeGatewayState {
    fn default() -> Self {
        let gateway = Arc::new(Gateway::default());
        let codex = Arc::new(CodexProviderAdapter::spawn(gateway.event_sink()));
        if let Err(error) = gateway.registry().register(codex.clone()) {
            crate::app_log::error(
                "runtime_gateway",
                &format!("failed to register Codex provider error={error:?}"),
            );
        }
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex: Arc::new(Mutex::new(Some(codex))),
        }
    }
}

impl RuntimeGatewayState {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
            codex: Arc::new(Mutex::new(None)),
        }
    }

    pub fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }

    pub fn transport(&self) -> &LocalTransport {
        &self.transport
    }

    pub fn refresh_codex_provider(&self) -> Result<(), ProtocolError> {
        let mut slot = self.codex.lock().map_err(|_| ProtocolError {
            code: "gateway_state_error".to_string(),
            message: "Codex provider refresh lock is unavailable".to_string(),
            retryable: true,
            details: None,
        })?;
        let previous_provider = slot.as_ref().map(|adapter| adapter.provider());
        let replacement = Arc::new(CodexProviderAdapter::spawn(self.gateway.event_sink()));
        let replacement_provider = replacement.provider();
        self.gateway.registry().register(replacement.clone())?;
        if let Some(previous) = slot.replace(replacement) {
            previous.retire();
        }
        let event = ProtocolEvent::ProviderStatusChanged {
            protocol_version: PROTOCOL_VERSION,
            event_sequence: 0,
            payload: ProviderStatusChangedEvent {
                provider: replacement_provider,
                previous_status: previous_provider.map(|provider| provider.status),
            },
        };
        if let Err(error) = self.gateway.event_sink().publish(event) {
            crate::app_log::error(
                "runtime_gateway",
                &format!("failed to publish refreshed Codex provider error={error:?}"),
            );
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn runtime_gateway_request(
    state: tauri::State<'_, RuntimeGatewayState>,
    request: ProtocolRequest,
) -> Result<ProtocolResponse, ProtocolError> {
    let transport = state.transport().clone();
    Ok(transport.request(request).await)
}

#[tauri::command]
pub fn runtime_gateway_replay(
    state: tauri::State<'_, RuntimeGatewayState>,
    after_event_sequence: Option<EventSequence>,
) -> Result<Vec<ProtocolEvent>, ProtocolError> {
    state.transport().replay(after_event_sequence)
}

pub fn start_event_bridge(
    app: AppHandle,
    state: &RuntimeGatewayState,
) -> Result<(), ProtocolError> {
    let current_sequence = state.gateway().current_event_sequence();
    let mut subscription = state.transport().subscribe(Some(current_sequence))?;
    tauri::async_runtime::spawn(async move {
        loop {
            match subscription.next_event().await {
                Ok(event) => {
                    let _ = app.emit(RUNTIME_GATEWAY_EVENT, event);
                }
                Err(error) => {
                    crate::app_log::error(
                        "runtime_gateway",
                        &format!("local event bridge stopped error={error:?}"),
                    );
                    break;
                }
            }
        }
    });
    Ok(())
}
