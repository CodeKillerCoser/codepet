//! Stable Provider-facing aliases and response policy over the generic transport core.
use crate::generated::{ProtocolError, ProviderWireMessage};
use crate::transport::mux::{Class, Connection, Incoming, Received, Driver};
use super::codec::ProviderMessageCodec;

pub type ProviderMux = Connection<ProviderMessageCodec>;
pub type MuxIncoming = Incoming<ProviderMessageCodec>;
pub type MuxMessage = Received<ProviderWireMessage>;
pub type MuxDriver = Driver<ProtocolError>;

impl Incoming<ProviderMessageCodec> {
    pub(crate) async fn respond_with_fallback(mut self, message: ProviderWireMessage, id: Option<String>) -> Result<(), ProtocolError> {
        if let Err(e) = self.write_response(message, None).await {
            if e.code != "provider_message_encode_failed" { return Err(e); }
            let reply = crate::generated::JsonRpcResponse { jsonrpc: "2.0".into(), id, response: crate::generated::JsonRpcResponsePayload::Error {
                error: crate::generated::RpcError { code: -32000, message: "Provider response exceeds negotiated message limits".into(),
                    data: Some([("code".into(), serde_json::json!("provider_response_too_large")), ("message".into(), serde_json::json!(e.message)), ("retryable".into(), serde_json::json!(false))].into_iter().collect()) }
            } };
            self.write_response(ProviderWireMessage::Response(reply), Some(Class::Small)).await?;
        }
        self.finish().await
    }
}
