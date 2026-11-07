use super::super::session::GatewayFrame;
use std::io::{Read, Write};

pub(super) const COMPRESSION_SDP: &str = "a=x-codepet-compression:gzip";
pub(super) fn supports_compression(sdp: &str) -> bool {
    sdp.lines().any(|line| line.trim() == COMPRESSION_SDP)
}
pub(super) fn compress_message(bytes: &[u8], enabled: bool) -> (Vec<u8>, bool) {
    if enabled && bytes.len() >= FRAME_BYTES {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        if encoder.write_all(bytes).is_ok() {
            if let Ok(encoded) = encoder.finish() {
                if encoded.len() * 10 <= bytes.len() * 9 {
                    return (encoded, true);
                }
            }
        }
    }
    (bytes.to_vec(), false)
}

pub(super) const FRAME_BYTES: usize = 16 * 1024;
pub(super) const HEADER_BYTES: usize = 12;
pub(super) const REQUEST_BYTES: usize = 256 * 1024;
pub(super) const RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAGIC: &[u8] = b"CPG1";

pub(super) fn encode_fragment(total: usize, offset: usize, payload: &[u8]) -> Vec<u8> {
    encode_fragment_compressed(total, offset, payload, false)
}
pub(super) fn encode_fragment_compressed(
    total: usize,
    offset: usize,
    payload: &[u8],
    compressed: bool,
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(if compressed { b"CPGZ" } else { MAGIC });
    frame.extend_from_slice(&(total as u32).to_be_bytes());
    frame.extend_from_slice(&(offset as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub(super) struct Decoder {
    maximum: usize,
    total: usize,
    partial: Vec<u8>,
    pub(super) allow_compression: bool,
    compressed: bool,
}

impl Decoder {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            total: 0,
            partial: Vec::new(),
            allow_compression: false,
            compressed: false,
        }
    }

    pub(super) fn received_bytes(&self) -> usize {
        self.partial.len()
    }
    pub(super) fn total_bytes(&self) -> usize {
        self.total
    }

    #[cfg(test)]
    pub(super) fn pending(&self) -> bool {
        self.total != 0
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Option<GatewayFrame>, ()> {
        if bytes.len() <= HEADER_BYTES || bytes.len() > FRAME_BYTES {
            return Err(());
        }
        let compressed = &bytes[..4] == b"CPGZ";
        if &bytes[..4] != MAGIC && !(compressed && self.allow_compression) {
            return Err(());
        }
        let total = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| ())?) as usize;
        let offset = u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| ())?) as usize;
        let payload = &bytes[HEADER_BYTES..];
        if total == 0 {
            if compressed || offset != 0 || payload.len() < 2 || payload.len() > 125 {
                return Err(());
            }
            let code = u16::from_be_bytes([payload[0], payload[1]]);
            let reason = std::str::from_utf8(&payload[2..])
                .map_err(|_| ())?
                .to_owned();
            self.partial.clear();
            self.total = 0;
            return Ok(Some(GatewayFrame::Close { code, reason }));
        }
        if total > self.maximum || offset != self.partial.len() || offset + payload.len() > total {
            return Err(());
        }
        if self.total == 0 {
            self.total = total;
            self.compressed = compressed;
        }
        if self.total != total || self.compressed != compressed {
            return Err(());
        }
        self.partial.extend_from_slice(payload);
        if self.partial.len() == total {
            self.total = 0;
            let bytes = std::mem::take(&mut self.partial);
            let bytes = if compressed {
                let mut decoded = Vec::new();
                flate2::read::GzDecoder::new(bytes.as_slice())
                    .take(self.maximum as u64 + 1)
                    .read_to_end(&mut decoded)
                    .map_err(|_| ())?;
                if decoded.len() > self.maximum {
                    return Err(());
                }
                decoded
            } else {
                bytes
            };
            Ok(Some(GatewayFrame::Text(bytes)))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negotiated_compression_roundtrips_and_enforces_inflated_limit() {
        let original = "中文🙂 conversation output ".repeat(20000).into_bytes();
        let (encoded, compressed) = compress_message(&original, true);
        assert!(compressed && encoded.len() < original.len() / 10);
        assert!(!compress_message(&original, false).1);
        let frame = encode_fragment_compressed(encoded.len(), 0, &encoded, true);
        assert!(Decoder::new(RESPONSE_BYTES).push(&frame).is_err());
        let mut small = Decoder::new(4096);
        small.allow_compression = true;
        assert!(small.push(&frame).is_err());
        let mut decoder = Decoder::new(RESPONSE_BYTES);
        decoder.allow_compression = true;
        assert!(
            matches!(decoder.push(&frame).unwrap(), Some(GatewayFrame::Text(bytes)) if bytes == original)
        );
        let mut invalid = encoded;
        invalid[0] = 0;
        assert!(decoder
            .push(&encode_fragment_compressed(
                invalid.len(),
                0,
                &invalid,
                true
            ))
            .is_err());
    }

    #[test]
    fn large_utf8_message_roundtrips_across_fragments() {
        let message = "中文🙂".repeat(12000).into_bytes();
        let mut decoder = Decoder::new(REQUEST_BYTES);
        let mut result = None;
        for (index, part) in message.chunks(FRAME_BYTES - HEADER_BYTES).enumerate() {
            result = decoder
                .push(&encode_fragment(
                    message.len(),
                    index * (FRAME_BYTES - HEADER_BYTES),
                    part,
                ))
                .unwrap();
        }
        assert!(matches!(result, Some(GatewayFrame::Text(value)) if value == message));
        assert!(!decoder.pending());
    }

    #[test]
    fn bounds_offsets_and_interleaving_are_rejected() {
        assert!(Decoder::new(10)
            .push(&encode_fragment(11, 0, b"x"))
            .is_err());
        assert!(Decoder::new(10).push(&encode_fragment(2, 1, b"x")).is_err());
        let mut decoder = Decoder::new(10);
        assert!(decoder
            .push(&encode_fragment(3, 0, b"x"))
            .unwrap()
            .is_none());
        assert!(decoder.push(&encode_fragment(4, 1, b"x")).is_err());
        assert!(Decoder::new(10)
            .push(&encode_fragment(1, 0, b"xx"))
            .is_err());
        assert!(Decoder::new(10).push(b"CPG1").is_err());
    }

    #[test]
    fn close_can_interrupt_partial_message_but_is_bounded() {
        let mut decoder = Decoder::new(10);
        decoder.push(&encode_fragment(10, 0, b"x")).unwrap();
        let mut payload = 1008_u16.to_be_bytes().to_vec();
        payload.extend_from_slice(b"revoked");
        assert!(
            matches!(decoder.push(&encode_fragment(0, 0, &payload)).unwrap(),
            Some(GatewayFrame::Close { code: 1008, reason }) if reason == "revoked")
        );
        assert!(!decoder.pending());
        assert!(decoder.push(&encode_fragment(0, 0, &[0; 126])).is_err());
    }
}
