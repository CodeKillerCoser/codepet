use super::super::session::GatewayFrame;

pub(super) const FRAME_BYTES: usize = 16 * 1024;
pub(super) const HEADER_BYTES: usize = 12;
pub(super) const REQUEST_BYTES: usize = 256 * 1024;
pub(super) const RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAGIC: &[u8] = b"CPG1";

pub(super) fn encode_fragment(total: usize, offset: usize, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(total as u32).to_be_bytes());
    frame.extend_from_slice(&(offset as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub(super) struct Decoder {
    maximum: usize,
    total: usize,
    partial: Vec<u8>,
}

impl Decoder {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            total: 0,
            partial: Vec::new(),
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
        if bytes.len() <= HEADER_BYTES || bytes.len() > FRAME_BYTES || &bytes[..4] != MAGIC {
            return Err(());
        }
        let total = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| ())?) as usize;
        let offset = u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| ())?) as usize;
        let payload = &bytes[HEADER_BYTES..];
        if total == 0 {
            if offset != 0 || payload.len() < 2 || payload.len() > 125 {
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
        }
        if self.total != total {
            return Err(());
        }
        self.partial.extend_from_slice(payload);
        if self.partial.len() == total {
            self.total = 0;
            Ok(Some(GatewayFrame::Text(std::mem::take(&mut self.partial))))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
