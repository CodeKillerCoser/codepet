//! Reusable binary message framing and bounded raw/zstd payload coding.
use crate::{transport::frame::*, protocol::*, TransportError};
use std::io::{Read, Write};

/// Sender policy does not change the decoder or wire profile.
#[derive(Clone, Copy, Debug)]
pub enum EncodingPolicy {
    Raw,
    Zstd(i32),
    Adaptive,
}

pub fn encode(
    json: Vec<u8>,
    encoded_cap: usize,
    decoded_cap: usize,
    force_zstd: bool,
) -> Result<(Vec<u8>, usize), TransportError> {
    encode_with_policy(
        json,
        encoded_cap,
        decoded_cap,
        if force_zstd {
            EncodingPolicy::Zstd(COMPRESSION_NORMAL_LEVEL)
        } else {
            EncodingPolicy::Adaptive
        },
    )
}
pub fn encode_with_policy(
    json: Vec<u8>,
    encoded_cap: usize,
    decoded_cap: usize,
    policy: EncodingPolicy,
) -> Result<(Vec<u8>, usize), TransportError> {
    if json.is_empty() || json.len() > decoded_cap {
        return Err(TransportError::new("decoded message exceeds byte limit"));
    }
    let decoded = json.len();
    let compress = match policy {
        EncodingPolicy::Raw => false,
        EncodingPolicy::Zstd(_) => true,
        EncodingPolicy::Adaptive => decoded >= PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES,
    };
    let (encoding, payload) = if compress {
        let level = match policy {
            EncodingPolicy::Zstd(level) => level,
            _ => {
                if decoded >= COMPRESSION_LARGE_MESSAGE_BYTES {
                    COMPRESSION_LARGE_LEVEL
                } else {
                    COMPRESSION_NORMAL_LEVEL
                }
            }
        };
        let mut compressor = zstd::stream::write::Encoder::new(
            BoundedBuffer::new(encoded_cap.saturating_sub(FRAME_HEADER_BYTES)),
            level,
        )
        .map_err(err)?;
        let compressed = compressor
            .write_all(&json)
            .and_then(|_| compressor.finish().map(|v| v.bytes));
        match compressed {
            Ok(bytes)
                if !matches!(policy, EncodingPolicy::Adaptive)
                    || (bytes.len() + COMPRESSION_MINIMUM_SAVED_BYTES <= decoded && bytes.len() <= decoded - decoded * COMPRESSION_MINIMUM_SAVED_PERCENT / 100) =>
            {
                (ProviderFrameEncoding::ZstdJson, bytes)
            }
            Ok(_) if matches!(policy, EncodingPolicy::Adaptive) => {
                (ProviderFrameEncoding::RawJson, json)
            }
            Err(_)
                if matches!(policy, EncodingPolicy::Adaptive)
                    && decoded.saturating_add(FRAME_HEADER_BYTES) <= encoded_cap =>
            {
                (ProviderFrameEncoding::RawJson, json)
            }
            Err(e) => return Err(err(e)),
            Ok(bytes) => (ProviderFrameEncoding::ZstdJson, bytes),
        }
    } else {
        (ProviderFrameEncoding::RawJson, json)
    };
    if payload.len().saturating_add(FRAME_HEADER_BYTES) > encoded_cap {
        return Err(TransportError::new("encoded message exceeds byte limit"));
    }
    let mut frame = Vec::with_capacity(payload.len() + FRAME_HEADER_BYTES);
    frame.extend_from_slice(&encode_header(payload.len(), encoding));
    frame.extend_from_slice(&payload);
    Ok((frame, decoded))
}

pub fn decode(
    frame: &[u8],
    encoded_cap: usize,
    decoded_cap: usize,
    expected_decoded: Option<usize>,
    require_zstd: bool,
) -> Result<Vec<u8>, TransportError> {
    let header = frame
        .get(..FRAME_HEADER_BYTES)
        .ok_or_else(|| TransportError::new("truncated frame"))?;
    let parsed = ProviderFrameHeader::decode(header.try_into().unwrap(), encoded_cap)?;
    if parsed.payload_length + FRAME_HEADER_BYTES != frame.len() {
        return Err(TransportError::new("frame length mismatch"));
    }
    if require_zstd && parsed.encoding != ProviderFrameEncoding::ZstdJson {
        return Err(TransportError::new("profile requires zstd-json"));
    }
    let limit = expected_decoded.unwrap_or(decoded_cap);
    if limit > decoded_cap || limit == 0 {
        return Err(TransportError::new("decoded length exceeds limit"));
    }
    let json = match parsed.encoding {
        ProviderFrameEncoding::RawJson => {
            if frame.len() - FRAME_HEADER_BYTES > limit {
                return Err(TransportError::new("decoded message exceeds byte limit"));
            }
            frame[FRAME_HEADER_BYTES..].to_vec()
        }
        ProviderFrameEncoding::ZstdJson => {
            // Bound both zstd's working window and its decoded output.
            let mut decoder = zstd::stream::read::Decoder::new(&frame[FRAME_HEADER_BYTES..]).map_err(err)?;
            let window_log = FRAME_ZSTD_WINDOW_LOG_MAX as u32; // zstd streaming encoders may declare a larger window than this message.
            decoder.window_log_max(window_log).map_err(err)?;
            let mut decoder = decoder.take(limit as u64 + 1);
            let mut target = BoundedBuffer::new(limit);
            std::io::copy(&mut decoder, &mut target).map_err(err)?;
            target.bytes
        }
    };
    if json.is_empty() || expected_decoded.is_some_and(|n| n != json.len()) {
        return Err(TransportError::new(
            "decoded length differs from reservation",
        ));
    }
    Ok(json)
}
fn err(error: impl ToString) -> TransportError {
    TransportError::new(error)
}

pub struct BoundedBuffer {
    pub bytes: Vec<u8>,
    cap: usize,
}
impl BoundedBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
        }
    }
}
impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.cap.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("message exceeds byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
