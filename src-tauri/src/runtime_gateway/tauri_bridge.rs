use super::gateway::Gateway;
use super::generated::{
    EventSequence, ProtocolError, ProtocolEvent, ProtocolRequest, ProtocolResponse,
};
use super::transport::{LocalTransport, Transport};
use crate::agent::codex_desktop_ipc::CodexProviderAdapter;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub const RUNTIME_GATEWAY_EVENT: &str = "runtime-gateway-event";

#[derive(Clone)]
pub struct RuntimeGatewayState {
    gateway: Arc<Gateway>,
    transport: LocalTransport,
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
        }
    }
}

impl RuntimeGatewayState {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self {
            transport: LocalTransport::new(gateway.clone()),
            gateway,
        }
    }

    pub fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }

    pub fn transport(&self) -> &LocalTransport {
        &self.transport
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
