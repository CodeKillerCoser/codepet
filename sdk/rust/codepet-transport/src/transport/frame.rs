//! CPRF byte layout, shared by compressed serial records and mux message bodies.
use crate::protocol::{FRAME_VERSION_OFFSET, FRAME_ENCODING_OFFSET, FRAME_PAYLOAD_LENGTH_OFFSET};
use super::error::TransportError;

pub const PROVIDER_FRAME_MAGIC: [u8; 4] = crate::protocol::FRAME_MAGIC;
pub const PROVIDER_FRAME_VERSION: u8 = crate::protocol::FRAME_VERSION;
pub const PROVIDER_FRAME_HEADER_BYTES: usize = crate::protocol::FRAME_HEADER_BYTES;
pub const MAX_PROVIDER_FRAME_BYTES: usize = crate::protocol::MAX_ENCODED_MESSAGE_BYTES;
pub const PROVIDER_FRAME_COMPRESSION_THRESHOLD_BYTES: usize = crate::protocol::COMPRESSION_THRESHOLD_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProviderFrameEncoding {
    RawJson = crate::protocol::ENCODING_RAW,
    ZstdJson = crate::protocol::ENCODING_ZSTD,
}

impl ProviderFrameEncoding {
    pub fn from_byte(value: u8) -> Result<Self, TransportError> {
        match value {
            crate::protocol::ENCODING_RAW => Ok(Self::RawJson),
            crate::protocol::ENCODING_ZSTD => Ok(Self::ZstdJson),
            _ => Err(TransportError::new(format!(
                "unsupported Provider frame encoding: {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFrameHeader {
    pub payload_length: usize,
    pub encoding: ProviderFrameEncoding,
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
    pub fn decode(
        header: &[u8; PROVIDER_FRAME_HEADER_BYTES],
        max_frame_bytes: usize,
    ) -> Result<ProviderFrameHeader, TransportError> {
        if header[..4] != PROVIDER_FRAME_MAGIC {
            return Err(TransportError::new("invalid Provider frame magic"));
        }
        if header[FRAME_VERSION_OFFSET] != PROVIDER_FRAME_VERSION {
            return Err(TransportError::new(format!(
                "unsupported Provider frame version: {}",
                header[FRAME_VERSION_OFFSET]
            )));
        }
        let encoding = ProviderFrameEncoding::from_byte(header[FRAME_ENCODING_OFFSET])?;
        let payload_length =
            u32::from_be_bytes(header[FRAME_PAYLOAD_LENGTH_OFFSET..PROVIDER_FRAME_HEADER_BYTES].try_into().expect("fixed header")) as usize;
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

pub fn encode_header(
    payload_length: usize,
    encoding: ProviderFrameEncoding,
) -> [u8; PROVIDER_FRAME_HEADER_BYTES] {
    let mut header = [0; PROVIDER_FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(&PROVIDER_FRAME_MAGIC);
    header[FRAME_VERSION_OFFSET] = PROVIDER_FRAME_VERSION;
    header[FRAME_ENCODING_OFFSET] = encoding as u8;
    header[FRAME_PAYLOAD_LENGTH_OFFSET..].copy_from_slice(&(payload_length as u32).to_be_bytes());
    header
}
