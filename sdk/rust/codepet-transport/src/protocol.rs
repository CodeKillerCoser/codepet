// Generated from protocol/v1/provider; do not edit.

use crate::generated::ProviderTransportLimits;
use crate::TransportError;
pub const JSONRPC_PROFILE: &str = "stdio-jsonrpc-v1";
pub const ZSTD_PROFILE: &str = "stdio-codepet-zstd-v1";
pub const MUX_PROFILE: &str = "stdio-codepet-mux-v1";
pub const DEFAULT_PROFILE: &str = "stdio-codepet-mux-v1";
pub const TRANSPORT_ENV: &str = "CODEPET_PROVIDER_TRANSPORT";
pub const BOOTSTRAP_MAGIC: [u8; 4] = *b"CPMX";
pub const BOOTSTRAP_VERSION: u8 = 1;
pub const BOOTSTRAP_CONFIRMATION: u8 = 1;
pub const BOOTSTRAP_MAX_JSON_BYTES: usize = 4096;
pub const BOOTSTRAP_TIMEOUT_MS: usize = 3000;
pub const BOOTSTRAP_MAX_SUPPORTED_VERSIONS: usize = 8;
pub const FRAME_MAGIC: [u8; 4] = *b"CPRF";
pub const FRAME_VERSION: u8 = 1;
pub const FRAME_HEADER_BYTES: usize = 10;
pub const FRAME_ZSTD_WINDOW_LOG_MAX: usize = 27;
pub const MESSAGE_VERSION: u8 = 1;
pub const MESSAGE_HEADER_BYTES: usize = 10;
pub const MESSAGE_GRANT: u8 = 1;
pub const MESSAGE_EVENT_ACK: u8 = 2;
pub const BOOTSTRAP_MAGIC_OFFSET: usize = 0;
pub const BOOTSTRAP_VERSION_OFFSET: usize = 4;
pub const BOOTSTRAP_PHASE_OFFSET: usize = 5;
pub const BOOTSTRAP_JSON_LENGTH_OFFSET: usize = 6;
pub const BOOTSTRAP_HEADER_BYTES: usize = 8;
pub const FRAME_MAGIC_OFFSET: usize = 0;
pub const FRAME_VERSION_OFFSET: usize = 4;
pub const FRAME_ENCODING_OFFSET: usize = 5;
pub const FRAME_PAYLOAD_LENGTH_OFFSET: usize = 6;
pub const MESSAGE_VERSION_OFFSET: usize = 0;
pub const MESSAGE_CLASS_OFFSET: usize = 1;
pub const MESSAGE_ENCODED_LENGTH_OFFSET: usize = 2;
pub const MESSAGE_DECODED_LENGTH_OFFSET: usize = 6;
pub const PHASE_HELLO: u8 = 1;
pub const PHASE_SELECT: u8 = 2;
pub const PHASE_CONFIRM: u8 = 3;
pub const PHASE_READY: u8 = 4;
pub const ENCODING_RAW: u8 = 0;
pub const ENCODING_ZSTD: u8 = 1;
pub const CLASS_NORMAL: u8 = 0;
pub const CLASS_SMALL: u8 = 1;
pub const CLASS_CONTROL: u8 = 2;
pub const COMPRESSION_THRESHOLD_BYTES: usize = 32768;
pub const COMPRESSION_LARGE_MESSAGE_BYTES: usize = 1048576;
pub const COMPRESSION_NORMAL_LEVEL: i32 = 1;
pub const COMPRESSION_LARGE_LEVEL: i32 = 3;
pub const COMPRESSION_MINIMUM_SAVED_BYTES: usize = 1024;
pub const COMPRESSION_MINIMUM_SAVED_PERCENT: usize = 10;
pub const FEATURES: &[&str] = &["raw-json","zstd-json","yamux-1","message-grant","reserved-lanes","bounded-decode","reset","ordered-events"];
pub const MAX_FRAME_PAYLOAD_BYTES: usize = 16384;
pub const MAX_ENCODED_MESSAGE_BYTES: usize = 16777216;
pub const MAX_DECODED_MESSAGE_BYTES: usize = 134217728;
pub const RECEIVE_BUDGET_BYTES: usize = 268435456;
pub const NORMAL_STREAMS: usize = 24;
pub const SMALL_STREAMS: usize = 16;
pub const CONTROL_STREAMS: usize = 4;
pub const SMALL_MESSAGE_BYTES: usize = 65536;
pub const SMALL_RECEIVE_BUDGET_BYTES: usize = 1048576;
pub const CONTROL_MESSAGE_BYTES: usize = 16384;
pub const CONTROL_RECEIVE_BUDGET_BYTES: usize = 262144;
pub const CONNECTION_WINDOW_BYTES: usize = 33554432;
pub const IDLE_TIMEOUT_MS: usize = 10000;
pub const STREAM_TIMEOUT_MS: usize = 60000;
pub fn default_transport_limits() -> ProviderTransportLimits { ProviderTransportLimits {
max_frame_payload_bytes: 16384,
max_encoded_message_bytes: 16777216,
max_decoded_message_bytes: 134217728,
receive_budget_bytes: 268435456,
normal_streams: 24,
small_streams: 16,
control_streams: 4,
small_message_bytes: 65536,
small_receive_budget_bytes: 1048576,
control_message_bytes: 16384,
control_receive_budget_bytes: 262144,
connection_window_bytes: 33554432,
idle_timeout_ms: 10000,
stream_timeout_ms: 60000,
} }
pub fn validate_limits(v: &ProviderTransportLimits) -> Result<(), TransportError> {
if v.max_frame_payload_bytes < 1024 || v.max_frame_payload_bytes > 65536 { return Err(TransportError::new("invalid negotiated maxFramePayloadBytes")); }
if v.max_encoded_message_bytes < 1024 || v.max_encoded_message_bytes > 16777216 { return Err(TransportError::new("invalid negotiated maxEncodedMessageBytes")); }
if v.max_decoded_message_bytes < 1024 || v.max_decoded_message_bytes > 134217728 { return Err(TransportError::new("invalid negotiated maxDecodedMessageBytes")); }
if v.receive_budget_bytes < 2048 || v.receive_budget_bytes > 1073741824 { return Err(TransportError::new("invalid negotiated receiveBudgetBytes")); }
if v.normal_streams < 1 || v.normal_streams > 32 { return Err(TransportError::new("invalid negotiated normalStreams")); }
if v.small_streams < 1 || v.small_streams > 16 { return Err(TransportError::new("invalid negotiated smallStreams")); }
if v.control_streams < 1 || v.control_streams > 8 { return Err(TransportError::new("invalid negotiated controlStreams")); }
if v.small_message_bytes < 1024 || v.small_message_bytes > 65536 { return Err(TransportError::new("invalid negotiated smallMessageBytes")); }
if v.small_receive_budget_bytes < 2048 || v.small_receive_budget_bytes > 16777216 { return Err(TransportError::new("invalid negotiated smallReceiveBudgetBytes")); }
if v.control_message_bytes < 1024 || v.control_message_bytes > 16384 { return Err(TransportError::new("invalid negotiated controlMessageBytes")); }
if v.control_receive_budget_bytes < 2048 || v.control_receive_budget_bytes > 1048576 { return Err(TransportError::new("invalid negotiated controlReceiveBudgetBytes")); }
if v.connection_window_bytes < 33554432 || v.connection_window_bytes > 268435456 { return Err(TransportError::new("invalid negotiated connectionWindowBytes")); }
if v.idle_timeout_ms < 100 || v.idle_timeout_ms > 120000 { return Err(TransportError::new("invalid negotiated idleTimeoutMs")); }
if v.stream_timeout_ms < 100 || v.stream_timeout_ms > 300000 { return Err(TransportError::new("invalid negotiated streamTimeoutMs")); }
if !(v.receive_budget_bytes >= (v.max_encoded_message_bytes + v.max_decoded_message_bytes)) { return Err(TransportError::new("normal 预算必须容纳最大编码和解码缓冲区。")); }
if !(v.small_receive_budget_bytes >= (2 * v.small_message_bytes)) { return Err(TransportError::new("small 预算至少容纳两倍消息上限。")); }
if !(v.control_receive_budget_bytes >= (2 * v.control_message_bytes)) { return Err(TransportError::new("control 预算至少容纳两倍消息上限。")); }
if !(v.small_message_bytes <= v.max_decoded_message_bytes) { return Err(TransportError::new("small 解码上限不得超过 normal 解码上限。")); }
if !(v.control_message_bytes <= v.small_message_bytes) { return Err(TransportError::new("control 消息上限不得超过 small 上限。")); }
if !(v.idle_timeout_ms <= v.stream_timeout_ms) { return Err(TransportError::new("失活超时不得大于流处理时限。")); }
Ok(()) }
