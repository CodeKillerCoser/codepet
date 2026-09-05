use crate::{
    decode_wire_message, encode_event, encode_request, encode_response, JsonRpcInboundError,
    JsonRpcInboundRequest, ProtocolError, ProviderWireMessage, RpcError,
    JSON_RPC_INTERNAL_ERROR, JSON_RPC_INVALID_REQUEST,
};
use std::io::{ErrorKind, Read, Write};
use std::time::Instant;

pub const PROVIDER_FRAME_MAGIC: [u8; 4] = *b"CPRF";
pub const PROVIDER_FRAME_VERSION: u8 = 1;
pub const PROVIDER_FRAME_HEADER_BYTES: usize = 10;
pub const MAX_PROVIDER_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES: usize = 32 * 1024;

const MIN_COMPRESSION_SAVINGS_BYTES: usize = 1024;
const ZSTD_COMPRESSION_LEVEL: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProviderFrameEncoding {
    RawJson = 0,
    ZstdJson = 1,
}

impl ProviderFrameEncoding {
    fn from_byte(value: u8) -> Result<Self, JsonRpcInboundError> {
        match value {
            0 => Ok(Self::RawJson),
            1 => Ok(Self::ZstdJson),
            _ => Err(inbound_frame_error(format!(
                "unsupported Provider frame encoding: {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFrameCodec {
    max_frame_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFrameHeader {
    pub payload_length: usize,
    encoding: ProviderFrameEncoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFrameEncodeMetrics {
    pub encoding: ProviderFrameEncoding,
    pub json_bytes: usize,
    pub encoded_payload_bytes: usize,
    pub frame_bytes: usize,
    pub json_encode_us: u128,
    pub compression_us: u128,
}

#[derive(Debug)]
pub struct EncodedProviderFrame {
    pub frame: Vec<u8>,
    pub metrics: ProviderFrameEncodeMetrics,
}

impl ProviderFrameEncoding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawJson => "raw-json",
            Self::ZstdJson => "zstd-json",
        }
    }
}

impl ProviderFrameCodec {
    pub fn new(max_frame_bytes: usize) -> Result<Self, ProtocolError> {
        if !(PROVIDER_FRAME_HEADER_BYTES..=MAX_PROVIDER_FRAME_BYTES)
            .contains(&max_frame_bytes)
        {
            return Err(ProtocolError {
                code: "invalid_frame_limit".to_string(),
                message: format!(
                    "Provider frame limit must be between {PROVIDER_FRAME_HEADER_BYTES} and {MAX_PROVIDER_FRAME_BYTES} bytes"
                ),
                retryable: false,
                details: None,
            });
        }
        Ok(Self { max_frame_bytes })
    }

    pub const fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    pub fn decode_header(
        &self,
        header: &[u8; PROVIDER_FRAME_HEADER_BYTES],
    ) -> Result<ProviderFrameHeader, JsonRpcInboundError> {
        if header[..4] != PROVIDER_FRAME_MAGIC {
            return Err(inbound_frame_error("invalid Provider frame magic"));
        }
        if header[4] != PROVIDER_FRAME_VERSION {
            return Err(inbound_frame_error(format!(
                "unsupported Provider frame version: {}",
                header[4]
            )));
        }
        let encoding = ProviderFrameEncoding::from_byte(header[5])?;
        let payload_length =
            u32::from_be_bytes(header[6..10].try_into().expect("fixed header")) as usize;
        let frame_length = PROVIDER_FRAME_HEADER_BYTES
            .checked_add(payload_length)
            .ok_or_else(|| inbound_frame_error("Provider frame length overflow"))?;
        if frame_length > self.max_frame_bytes {
            return Err(inbound_frame_error(format!(
                "Provider frame exceeds {} bytes",
                self.max_frame_bytes
            )));
        }
        Ok(ProviderFrameHeader {
            payload_length,
            encoding,
        })
    }

    pub fn encode_message(
        &self,
        message: &ProviderWireMessage,
    ) -> Result<Vec<u8>, ProtocolError> {
        self.encode_message_with_metrics(message)
            .map(|encoded| encoded.frame)
    }

    pub fn encode_message_with_metrics(
        &self,
        message: &ProviderWireMessage,
    ) -> Result<EncodedProviderFrame, ProtocolError> {
        let json_stopwatch = Instant::now();
        let payload = match message {
            ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => {
                encode_request(request)?
            }
            ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(_)) => {
                return Err(ProtocolError {
                    code: "cannot_encode_rejected_request".to_string(),
                    message: "a rejected inbound request is not a wire request".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            ProviderWireMessage::Response(response) => encode_response(response)?,
            ProviderWireMessage::Notification(notification) => {
                if notification.jsonrpc != "2.0" {
                    return Err(ProtocolError {
                        code: "protocol_codec_error".to_string(),
                        message: "notification jsonrpc must be exactly 2.0".to_string(),
                        retryable: false,
                        details: None,
                    });
                }
                serde_json::to_vec(notification)
                    .map_err(|error| encode_error("encode notification", error))?
            }
            ProviderWireMessage::Event(event) => encode_event(event)?,
        };
        let json_encode_us = json_stopwatch.elapsed().as_micros();
        self.frame_payload(payload, json_encode_us)
    }

    pub fn decode_frame(
        &self,
        frame: &[u8],
    ) -> Result<ProviderWireMessage, JsonRpcInboundError> {
        if frame.len() < PROVIDER_FRAME_HEADER_BYTES {
            return Err(inbound_frame_error("Provider frame header is truncated"));
        }
        let header: &[u8; PROVIDER_FRAME_HEADER_BYTES] = frame[..PROVIDER_FRAME_HEADER_BYTES]
            .try_into()
            .expect("checked Provider frame header");
        let decoded_header = self.decode_header(header)?;
        if decoded_header.payload_length != frame.len() - PROVIDER_FRAME_HEADER_BYTES {
            return Err(inbound_frame_error(
                "Provider frame payloadLength does not match the frame body",
            ));
        }
        self.decode_payload(
            decoded_header.encoding,
            &frame[PROVIDER_FRAME_HEADER_BYTES..],
        )
    }

    pub fn read_message<R: Read>(
        &self,
        reader: &mut R,
    ) -> Result<Option<ProviderWireMessage>, JsonRpcInboundError> {
        let mut header = [0_u8; PROVIDER_FRAME_HEADER_BYTES];
        let first = reader
            .read(&mut header[..1])
            .map_err(|error| read_error("read Provider frame header", error))?;
        if first == 0 {
            return Ok(None);
        }
        reader
            .read_exact(&mut header[1..])
            .map_err(|error| truncated_read_error("read Provider frame header", error))?;
        let decoded_header = self.decode_header(&header)?;
        let mut payload = vec![0_u8; decoded_header.payload_length];
        reader
            .read_exact(&mut payload)
            .map_err(|error| truncated_read_error("read Provider frame payload", error))?;
        self.decode_payload(decoded_header.encoding, &payload).map(Some)
    }

    pub fn write_message<W: Write>(
        &self,
        writer: &mut W,
        message: &ProviderWireMessage,
    ) -> Result<(), ProtocolError> {
        let frame = self.encode_message(message)?;
        writer.write_all(&frame).map_err(|error| ProtocolError {
            code: "provider_frame_write_failed".to_string(),
            message: format!("write Provider frame: {error}"),
            retryable: true,
            details: None,
        })
    }

    fn frame_payload(
        &self,
        payload: Vec<u8>,
        json_encode_us: u128,
    ) -> Result<EncodedProviderFrame, ProtocolError> {
        let uncompressed_length = payload.len();
        let compression_stopwatch = Instant::now();
        let (encoding, encoded_payload) = if uncompressed_length
            >= PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES
        {
            let compressed = zstd::bulk::compress(&payload, ZSTD_COMPRESSION_LEVEL).map_err(
                |error| ProtocolError {
                    code: "provider_frame_compression_failed".to_string(),
                    message: format!("compress Provider JSON payload: {error}"),
                    retryable: false,
                    details: None,
                },
            )?;
            let savings = uncompressed_length.saturating_sub(compressed.len());
            if savings >= MIN_COMPRESSION_SAVINGS_BYTES
                && savings.saturating_mul(10) >= uncompressed_length
            {
                (ProviderFrameEncoding::ZstdJson, compressed)
            } else {
                (ProviderFrameEncoding::RawJson, payload)
            }
        } else {
            (ProviderFrameEncoding::RawJson, payload)
        };
        let compression_us = compression_stopwatch.elapsed().as_micros();
        let frame_length = PROVIDER_FRAME_HEADER_BYTES
            .checked_add(encoded_payload.len())
            .ok_or_else(|| {
                frame_too_large(
                    self.max_frame_bytes,
                    uncompressed_length,
                    encoded_payload.len(),
                    encoding,
                    json_encode_us,
                    compression_us,
                )
            })?;
        if frame_length > self.max_frame_bytes || encoded_payload.len() > u32::MAX as usize {
            return Err(frame_too_large(
                self.max_frame_bytes,
                uncompressed_length,
                encoded_payload.len(),
                encoding,
                json_encode_us,
                compression_us,
            ));
        }
        let mut frame = Vec::with_capacity(frame_length);
        frame.extend_from_slice(&PROVIDER_FRAME_MAGIC);
        frame.push(PROVIDER_FRAME_VERSION);
        frame.push(encoding as u8);
        frame.extend_from_slice(&(encoded_payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&encoded_payload);
        Ok(EncodedProviderFrame {
            metrics: ProviderFrameEncodeMetrics {
                encoding,
                json_bytes: uncompressed_length,
                encoded_payload_bytes: encoded_payload.len(),
                frame_bytes: frame_length,
                json_encode_us,
                compression_us,
            },
            frame,
        })
    }

    fn decode_payload(
        &self,
        encoding: ProviderFrameEncoding,
        payload: &[u8],
    ) -> Result<ProviderWireMessage, JsonRpcInboundError> {
        let decoded = match encoding {
            ProviderFrameEncoding::RawJson => payload.to_vec(),
            ProviderFrameEncoding::ZstdJson => zstd::stream::decode_all(payload).map_err(|error| {
                inbound_frame_error(format!("decompress Provider JSON payload: {error}"))
            })?,
        };
        decode_wire_message(&decoded)
    }
}

impl Default for ProviderFrameCodec {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_PROVIDER_FRAME_BYTES,
        }
    }
}

fn frame_too_large(
    max_frame_bytes: usize,
    json_bytes: usize,
    encoded_payload_bytes: usize,
    encoding: ProviderFrameEncoding,
    json_encode_us: u128,
    compression_us: u128,
) -> ProtocolError {
    ProtocolError {
        code: "provider_frame_too_large".to_string(),
        message: format!("Provider frame exceeds {max_frame_bytes} bytes"),
        retryable: false,
        details: Some(
            [
                ("maxFrameBytes".to_string(), serde_json::json!(max_frame_bytes)),
                ("jsonBytes".to_string(), serde_json::json!(json_bytes)),
                (
                    "encodedPayloadBytes".to_string(),
                    serde_json::json!(encoded_payload_bytes),
                ),
                ("encoding".to_string(), serde_json::json!(encoding.as_str())),
                (
                    "jsonEncodeUs".to_string(),
                    serde_json::json!(json_encode_us.to_string()),
                ),
                (
                    "compressionUs".to_string(),
                    serde_json::json!(compression_us.to_string()),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    }
}

fn encode_error(context: &str, error: serde_json::Error) -> ProtocolError {
    ProtocolError {
        code: "protocol_codec_error".to_string(),
        message: format!("{context}: {error}"),
        retryable: false,
        details: None,
    }
}

fn inbound_frame_error(message: impl Into<String>) -> JsonRpcInboundError {
    JsonRpcInboundError {
        id: None,
        error: RpcError {
            code: JSON_RPC_INVALID_REQUEST,
            message: message.into(),
            data: None,
        },
    }
}

fn read_error(context: &str, error: std::io::Error) -> JsonRpcInboundError {
    JsonRpcInboundError {
        id: None,
        error: RpcError {
            code: JSON_RPC_INTERNAL_ERROR,
            message: format!("{context}: {error}"),
            data: None,
        },
    }
}

fn truncated_read_error(context: &str, error: std::io::Error) -> JsonRpcInboundError {
    if error.kind() == ErrorKind::UnexpectedEof {
        inbound_frame_error(format!("{context}: truncated frame"))
    } else {
        read_error(context, error)
    }
}
