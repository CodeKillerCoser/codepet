//! Provider JSON semantics and bounded mux encoding, injected into the transport core.
use crate::generated::{decode_wire_message, JsonRpcInboundRequest, ProtocolDispatchLane, ProtocolError, ProviderWireMessage};
use crate::transport::{error::TransportError, frame::{encode_header, ProviderFrameHeader, ProviderFrameEncoding, MAX_PROVIDER_FRAME_BYTES}, mux::{Class, MessageCodec}};
use std::io::{Read, Write};
use super::error;

#[derive(Clone)]
pub struct ProviderMessageCodec;

impl MessageCodec for ProviderMessageCodec {
    type Message = ProviderWireMessage;
    type Error = ProtocolError;
    const MIN_ENCODED_BYTES: usize = 10;

    fn classify(message: &ProviderWireMessage, small_message_bytes: usize) -> Result<Class, ProtocolError> {
        classify(message, small_message_bytes)
    }

    fn encode(message: &ProviderWireMessage, encoded_cap: usize, decoded_cap: usize) -> Result<(Vec<u8>, usize), ProtocolError> {
        encode(message, encoded_cap, decoded_cap).map_err(|mut e| { e.code = "provider_message_encode_failed".into(); e })
    }

    fn decode(frame: &[u8], decoded: usize, class: Class) -> Result<ProviderWireMessage, ProtocolError> {
        let message = decode(frame, decoded)?;
        if class == Class::Control && !matches!(&message,
            ProviderWireMessage::Response(_) | ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(_))) {
            return Err(error("event used control reserve"));
        }
        if let ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(ref request)) = message {
            if (class == Class::Control) != (request.method().dispatch_lane() == ProtocolDispatchLane::Control) {
                return Err(error("request used the wrong dispatch reserve"));
            }
        }
        Ok(message)
    }

    fn response_class(message: &ProviderWireMessage) -> Option<Class> {
        if matches!(message, ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(r)) if r.method().dispatch_lane() == ProtocolDispatchLane::Control) {
            Some(Class::Control)
        } else { None }
    }
}

impl From<TransportError> for ProtocolError {
    fn from(value: TransportError) -> Self { error(value.message) }
}

struct LimitedBuffer { bytes: Vec<u8>, cap: usize }
impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.cap.saturating_sub(self.bytes.len()) { return Err(std::io::Error::other("message exceeds byte limit")); }
        if self.bytes.capacity() < self.bytes.len() + bytes.len() { let target = (self.bytes.len() + bytes.len()).next_power_of_two().min(self.cap); self.bytes.reserve_exact(target - self.bytes.len()); }
        self.bytes.extend_from_slice(bytes); Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
fn serialize<W: Write>(message: &ProviderWireMessage, writer: W) -> Result<(), ProtocolError> {
    let version = match message {
        ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(v)) => v.jsonrpc_version(),
        ProviderWireMessage::Response(v) => &v.jsonrpc,
        ProviderWireMessage::Event(v) => v.jsonrpc_version(),
        _ => return Err(error("unsupported mux message kind")),
    };
    if version != "2.0" { return Err(error("jsonrpc must be exactly 2.0")); }
    match message {
        ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(v)) => serde_json::to_writer(writer, v),
        ProviderWireMessage::Response(v) => serde_json::to_writer(writer, v),
        ProviderWireMessage::Event(v) => serde_json::to_writer(writer, v),
        _ => return Err(error("unsupported mux message kind")),
    }.map_err(error)
}
fn classify(message: &ProviderWireMessage, small_message_bytes: usize) -> Result<Class, ProtocolError> {
    if let ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) = message {
        if request.method().dispatch_lane() == ProtocolDispatchLane::Control { return Ok(Class::Control); }
    }
    let mut prefix = LimitedBuffer { bytes: Vec::new(), cap: small_message_bytes - 10 };
    Ok(if serialize(message, &mut prefix).is_ok() { Class::Small } else { Class::Normal })
}
pub(super) fn encode(message: &ProviderWireMessage, encoded_cap: usize, decoded_cap: usize) -> Result<(Vec<u8>, usize), ProtocolError> {
    let mut json = LimitedBuffer { bytes: Vec::new(), cap: decoded_cap };
    serialize(message, &mut json)?;
    let decoded = json.bytes.len();
    let (encoding, payload) = if decoded >= 32*1024 {
        let target = LimitedBuffer { bytes: Vec::new(), cap: encoded_cap - 10 };
        let mut compressor = zstd::stream::write::Encoder::new(target, 1).map_err(error)?;
        compressor.write_all(&json.bytes).map_err(error)?;
        (ProviderFrameEncoding::ZstdJson, compressor.finish().map_err(error)?.bytes)
    } else { (ProviderFrameEncoding::RawJson, std::mem::take(&mut json.bytes)) };
    drop(json);
    if payload.len() + 10 > encoded_cap { return Err(error("encoded message exceeds byte limit")); }
    let payload_len = payload.len();
    let mut frame = payload;
    frame.reserve_exact(10); frame.resize(payload_len + 10, 0);
    frame.copy_within(..payload_len, 10);
    frame[..10].copy_from_slice(&encode_header(payload_len, encoding));
    Ok((frame, decoded))
}
pub(super) fn decode(frame: &[u8], decoded: usize) -> Result<ProviderWireMessage, ProtocolError> {
    let header: &[u8; 10] = frame.get(..10).ok_or_else(|| error("truncated frame"))?.try_into().unwrap();
    let parsed = ProviderFrameHeader::decode(header, MAX_PROVIDER_FRAME_BYTES).map_err(error)?;
    if parsed.payload_length + 10 != frame.len() { return Err(error("frame length mismatch")); }
    let json = if frame[5] == 1 {
        let mut decompressor = zstd::stream::read::Decoder::new(&frame[10..]).map_err(error)?.take(decoded as u64 + 1);
        let mut target = LimitedBuffer { bytes: Vec::new(), cap: decoded };
        std::io::copy(&mut decompressor, &mut target).map_err(error)?;
        target.bytes
    } else { frame[10..].to_vec() };
    if json.len() != decoded { return Err(error("decoded length differs from reservation")); }
    decode_wire_message(&json).map_err(|e| error(e.error.message))
}

pub(crate) fn serialized_size(message: &ProviderWireMessage, cap: usize) -> Result<usize, ProtocolError> {
    struct Counter { count: usize, cap: usize }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.cap.saturating_sub(self.count) { return Err(std::io::Error::other("message exceeds decoded limit")); }
            self.count += bytes.len(); Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut counter = Counter { count: 0, cap }; serialize(message, &mut counter)?; Ok(counter.count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::{JsonRpcResponse, JsonRpcResponsePayload, ProtocolMethod, ProtocolRequest};
    fn reply(value: serde_json::Value) -> ProviderWireMessage {
        ProviderWireMessage::Response(JsonRpcResponse { jsonrpc: "2.0".into(), id: Some("test".into()), response: JsonRpcResponsePayload::Ok { result: value } })
    }
    #[test]
    fn compressed_payload_cannot_exceed_declared_decoded_bytes() {
        let (frame, length) = encode(&reply(serde_json::json!("x".repeat(1024*1024))), 65536, 2*1024*1024).unwrap();
        assert!(decode(&frame, 64).is_err());
        assert!(decode(&frame, length + 1).is_err());
        assert!(decode(&frame, length).is_ok());
        assert!(encode(&reply(serde_json::json!("x".repeat(10000))), 10000, 1000).is_err());
    }

    #[test]
    fn provider_codec_rejects_requests_using_the_wrong_reserved_lane() {
        for (method, params, expected) in [
            (ProtocolMethod::ProviderDescribe, serde_json::json!({}), Class::Small),
            (ProtocolMethod::ProviderPing, serde_json::json!({
                "sequence": 1, "hostSessionId": "test",
                "clients": {"revision": 1, "connections": []}, "instances": [],
            }), Class::Control),
        ] {
            let message = ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(
                ProtocolRequest::from_method_params(method, "lane-test".into(), params).unwrap(),
            ));
            assert_eq!(ProviderMessageCodec::classify(&message, 65536).unwrap(), expected);
            let (frame, decoded) = ProviderMessageCodec::encode(&message, 65536, 65536).unwrap();
            assert!(ProviderMessageCodec::decode(&frame, decoded, expected).is_ok());
            let wrong = if expected == Class::Control { Class::Small } else { Class::Control };
            assert_eq!(ProviderMessageCodec::decode(&frame, decoded, wrong).unwrap_err().message,
                "request used the wrong dispatch reserve");
        }
    }

}
