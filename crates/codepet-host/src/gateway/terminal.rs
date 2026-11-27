use super::*;

impl ProviderGatewayService {
    pub(super) async fn terminal_request(
        &self,
        owner: &str,
        request: gateway::ProtocolRequest,
    ) -> Result<gateway::JsonRpcResponse, gateway::ProtocolRequest> {
        use gateway::ProtocolRequest;
        if !matches!(
            &request,
            ProtocolRequest::TerminalOpen { .. }
                | ProtocolRequest::TerminalRead { .. }
                | ProtocolRequest::TerminalWrite { .. }
                | ProtocolRequest::TerminalResize { .. }
                | ProtocolRequest::TerminalClose { .. }
        ) {
            return Err(request);
        }
        let abandoned_write = match &request {
            ProtocolRequest::TerminalWrite { params, .. } => Some(params.terminal_id.clone()),
            _ => None,
        };
        let cleanup = self.terminals.clone();
        let cleanup_owner = owner.to_string();
        let manager = self.terminals.clone();
        let owner = owner.to_string();
        let id = request.id().to_string();
        let worker = tokio::task::spawn_blocking(move || match request {
            ProtocolRequest::TerminalOpen {
                jsonrpc,
                id,
                params,
            } => json_rpc_response(jsonrpc, id, manager.open(&owner, params)),
            ProtocolRequest::TerminalRead {
                jsonrpc,
                id,
                params,
            } => json_rpc_response(jsonrpc, id, manager.read(&owner, params)),
            ProtocolRequest::TerminalWrite {
                jsonrpc,
                id,
                params,
            } => json_rpc_response(jsonrpc, id, manager.write(&owner, params)),
            ProtocolRequest::TerminalResize {
                jsonrpc,
                id,
                params,
            } => json_rpc_response(jsonrpc, id, manager.resize(&owner, params)),
            ProtocolRequest::TerminalClose {
                jsonrpc,
                id,
                params,
            } => json_rpc_response(jsonrpc, id, manager.close(&owner, params)),
            _ => unreachable!("terminal requests were classified before dispatch"),
        });
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker).await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                // A blocking OS write must not retain the connection's business
                // dispatcher indefinitely. Close only that PTY; never replay input.
                if let Some(terminal_id) = abandoned_write {
                    tokio::task::spawn_blocking(move || {
                        let _ = cleanup.close(
                            &cleanup_owner,
                            gateway::TerminalCloseRequest { terminal_id },
                        );
                    });
                }
                return Ok(json_rpc_response(
                    "2.0".into(),
                    id,
                    Err::<serde_json::Value, _>(crate::terminal::error(
                        "terminal_operation_timeout",
                        "终端操作超时，请重试读取或新建终端",
                        false,
                    )),
                ));
            }
        };
        Ok(result.unwrap_or_else(|_| {
            json_rpc_response(
                "2.0".into(),
                id,
                Err::<serde_json::Value, _>(crate::terminal::error(
                    "terminal_worker_failed",
                    "Terminal operation failed",
                    false,
                )),
            )
        }))
    }
}

impl Drop for ProviderGatewayService {
    fn drop(&mut self) {
        self.terminals.shutdown();
    }
}
