//! CPRF message-body byte layout; STDIO carries these bodies inside negotiated mux streams.
use super::error::TransportError;

pub const PROVIDER_FRAME_MAGIC: [u8; 4] = *b"CPRF";
pub const PROVIDER_FRAME_VERSION: u8 = 1;
pub const PROVIDER_FRAME_HEADER_BYTES: usize = 10;
pub const MAX_PROVIDER_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProviderFrameEncoding {
    RawJson = 0,
    ZstdJson = 1,
}

impl ProviderFrameEncoding {
    pub(crate) fn from_byte(value: u8) -> Result<Self, TransportError> {
        match value {
            0 => Ok(Self::RawJson),
            1 => Ok(Self::ZstdJson),
            _ => Err(TransportError::new(format!(
                "unsupported Provider frame encoding: {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFrameHeader {
    pub payload_length: usize,
    pub(crate) encoding: ProviderFrameEncoding,
}

impl ProviderFrameEncoding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawJson => "raw-json",
            Self::ZstdJson => "zstd-json",
        }
    }
}

impl ProviderFrameHeader {
    pub(crate) fn decode(
        header: &[u8; PROVIDER_FRAME_HEADER_BYTES],
        max_frame_bytes: usize,
    ) -> Result<ProviderFrameHeader, TransportError> {
        if header[..4] != PROVIDER_FRAME_MAGIC {
            return Err(TransportError::new("invalid Provider frame magic"));
        }
        if header[4] != PROVIDER_FRAME_VERSION {
            return Err(TransportError::new(format!(
                "unsupported Provider frame version: {}",
                header[4]
            )));
        }
        let encoding = ProviderFrameEncoding::from_byte(header[5])?;
        let payload_length =
            u32::from_be_bytes(header[6..10].try_into().expect("fixed header")) as usize;
        let frame_length = PROVIDER_FRAME_HEADER_BYTES
            .checked_add(payload_length)
            .ok_or_else(|| TransportError::new("Provider frame length overflow"))?;
        if frame_length > max_frame_bytes {
            return Err(TransportError::new(format!(
                "Provider frame exceeds {} bytes",
                max_frame_bytes
            )));
        }
        Ok(ProviderFrameHeader {
            payload_length,
            encoding,
        })
    }

}

pub(crate) fn encode_header(payload_length: usize, encoding: ProviderFrameEncoding) -> [u8; PROVIDER_FRAME_HEADER_BYTES] {
    let mut header = [0; PROVIDER_FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(&PROVIDER_FRAME_MAGIC);
    header[4] = PROVIDER_FRAME_VERSION;
    header[5] = encoding as u8;
    header[6..].copy_from_slice(&(payload_length as u32).to_be_bytes());
    header
}
