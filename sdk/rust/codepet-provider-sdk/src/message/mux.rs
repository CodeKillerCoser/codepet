//! Stable Provider-facing aliases and response policy over the generic transport core.
use super::codec::ProviderMessageCodec;
use crate::generated::{ProtocolError, ProviderWireMessage};
use crate::transport::mux::{Class, Connection, Driver, Incoming, Received};

pub type ProviderMux = Connection<ProviderMessageCodec>;
pub type MuxIncoming = Incoming<ProviderMessageCodec>;
pub type MuxMessage = Received<ProviderWireMessage>;
pub type MuxDriver = Driver<ProtocolError>;

pub(crate) trait ResponseFallback: Sized {
    fn respond_with_fallback(
        self,
        message: ProviderWireMessage,
        id: Option<String>,
    ) -> impl std::future::Future<Output = Result<(), ProtocolError>> + Send;
}
impl ResponseFallback for Incoming<ProviderMessageCodec> {
    async fn respond_with_fallback(
        mut self,
        message: ProviderWireMessage,
        id: Option<String>,
    ) -> Result<(), ProtocolError> {
        if let Err(e) = self.write_response(message, None).await {
            if e.code != "provider_message_encode_failed" {
                return Err(e);
            }
            let reply = crate::generated::JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                response: crate::generated::JsonRpcResponsePayload::Error {
                    error: crate::generated::RpcError {
                        code: -32000,
                        message: "Provider response exceeds negotiated message limits".into(),
                        data: Some(
                            [
                                (
                                    "code".into(),
                                    serde_json::json!("provider_response_too_large"),
                                ),
                                ("message".into(), serde_json::json!(e.message)),
                                ("retryable".into(), serde_json::json!(false)),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    },
                },
            };
            self.write_response(ProviderWireMessage::Response(reply), Some(Class::Small))
                .await?;
        }
        self.finish().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        default_transport_limits, JsonRpcInboundRequest, JsonRpcResponse, JsonRpcResponsePayload,
        ProtocolMethod, ProtocolRequest,
    };
    fn small() -> ProviderWireMessage {
        ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(
            ProtocolRequest::from_method_params(
                ProtocolMethod::ProviderDescribe,
                "test".into(),
                serde_json::json!({}),
            )
            .unwrap(),
        ))
    }
    fn reply(value: serde_json::Value) -> ProviderWireMessage {
        ProviderWireMessage::Response(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: Some("test".into()),
            response: JsonRpcResponsePayload::Ok { result: value },
        })
    }
    #[tokio::test]
    async fn asymmetric_limits_return_a_small_error_for_oversized_response() {
        let (a, b) = tokio::io::duplex(1024);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);
        let mut limits = default_transport_limits();
        limits.max_decoded_message_bytes = 1024;
        limits.max_encoded_message_bytes = 2048;
        limits.small_message_bytes = 1024;
        limits.control_message_bytes = 1024;
        let (host, server) = tokio::join!(
            ProviderMux::connect(ar, aw, true, limits),
            ProviderMux::connect(br, bw, false, default_transport_limits())
        );
        let (host, _, _hd) = host.unwrap();
        let (_, mut incoming, _sd) = server.unwrap();
        let task = tokio::spawn(async move { host.exchange(small()).await });
        let mut stream = incoming.recv().await.unwrap();
        let _message = stream.receive().await.unwrap();
        stream
            .respond_with_fallback(
                reply(serde_json::json!("x".repeat(4096))),
                Some("test".into()),
            )
            .await
            .unwrap();
        let result = task.await.unwrap().unwrap();
        assert!(matches!(
            result.message,
            ProviderWireMessage::Response(JsonRpcResponse {
                response: JsonRpcResponsePayload::Error { .. },
                ..
            })
        ));
    }
}
