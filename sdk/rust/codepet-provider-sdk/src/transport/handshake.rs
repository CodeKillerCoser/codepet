use crate::generated::{ProviderTransportHello, ProviderTransportLimits, ProviderTransportSelection};
use super::error::TransportError;
use futures::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use serde::{Serialize, de::DeserializeOwned};
use std::time::Duration;

pub const MUX_PROFILE: &str = "stdio-codepet-mux-v1";
pub const TRANSPORT_ENV: &str = "CODEPET_PROVIDER_TRANSPORT";
const FEATURES: &[&str] = &["raw-json", "zstd-json", "yamux-1", "message-grant", "reserved-lanes", "bounded-decode", "reset", "ordered-events"];
const BOOTSTRAP_LIMIT: usize = 4096;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(3);

fn error(message: impl ToString) -> TransportError { TransportError::new(message) }

pub fn default_transport_limits() -> ProviderTransportLimits {
    ProviderTransportLimits {
        max_frame_payload_bytes: 16 * 1024, max_encoded_message_bytes: 16 * 1024 * 1024,
        max_decoded_message_bytes: 128 * 1024 * 1024, receive_budget_bytes: 256 * 1024 * 1024,
        normal_streams: 24, small_streams: 16, control_streams: 4,
        small_message_bytes: 64 * 1024, small_receive_budget_bytes: 1024 * 1024,
        control_message_bytes: 16 * 1024, control_receive_budget_bytes: 256 * 1024,
        connection_window_bytes: 32 * 1024 * 1024, idle_timeout_ms: 10_000, stream_timeout_ms: 60_000,
    }
}

fn validate_limits(v: &ProviderTransportLimits) -> Result<(), TransportError> {
    for (n, lo, hi) in [
        (v.max_frame_payload_bytes, 1024, 65536), (v.max_encoded_message_bytes, 1024, 16*1024*1024),
        (v.max_decoded_message_bytes, 1024, 128*1024*1024), (v.receive_budget_bytes, 2048, 1024*1024*1024),
        (v.normal_streams, 1, 32), (v.small_streams, 1, 16), (v.control_streams, 1, 8),
        (v.small_message_bytes, 1024, 65536), (v.small_receive_budget_bytes, 2048, 16*1024*1024),
        (v.control_message_bytes, 1024, 16384), (v.control_receive_budget_bytes, 2048, 1024*1024),
        (v.connection_window_bytes, 32*1024*1024, 256*1024*1024),
        (v.idle_timeout_ms, 100, 120000), (v.stream_timeout_ms, 100, 300000),
    ] { if n < lo || n > hi { return Err(error("invalid negotiated transport limit")); } }
    if v.receive_budget_bytes < v.max_encoded_message_bytes + v.max_decoded_message_bytes
        || v.small_receive_budget_bytes < 2 * v.small_message_bytes
        || v.control_receive_budget_bytes < 2 * v.control_message_bytes
        || v.small_message_bytes > v.max_decoded_message_bytes
        || v.control_message_bytes > v.small_message_bytes
        || v.idle_timeout_ms > v.stream_timeout_ms
    { return Err(error("inconsistent negotiated transport budgets")); }
    Ok(())
}

fn validate_features(v: &[String]) -> Result<(), TransportError> {
    if v.len() != FEATURES.len() || FEATURES.iter().any(|f| !v.iter().any(|v| v == f)) {
        return Err(error("required transport features do not match mux v1"));
    }
    Ok(())
}

async fn boot_write<I: futures::AsyncWrite + Unpin, T: Serialize>(io: &mut I, tag: u8, v: &T) -> Result<(), TransportError> {
    let bytes = serde_json::to_vec(v).map_err(error)?;
    if bytes.len() > BOOTSTRAP_LIMIT { return Err(error("bootstrap exceeds limit")); }
    io.write_all(b"CPMX").await.map_err(error)?;
    io.write_all(&[1, tag]).await.map_err(error)?;
    io.write_all(&(bytes.len() as u16).to_be_bytes()).await.map_err(error)?;
    io.write_all(&bytes).await.map_err(error)?;
    io.flush().await.map_err(error)
}
async fn boot_read<I: futures::AsyncRead + Unpin, T: DeserializeOwned>(io: &mut I, tag: u8) -> Result<T, TransportError> {
    let mut header = [0; 8]; io.read_exact(&mut header).await.map_err(error)?;
    if &header[..6] != [b'C', b'P', b'M', b'X', 1, tag] { return Err(error("invalid bootstrap phase or profile")); }
    let n = u16::from_be_bytes([header[6], header[7]]) as usize;
    if n > BOOTSTRAP_LIMIT { return Err(error("bootstrap exceeds limit")); }
    let mut bytes = vec![0; n]; io.read_exact(&mut bytes).await.map_err(error)?;
    serde_json::from_slice(&bytes).map_err(error)
}

pub(super) async fn negotiate<I: AsyncRead + AsyncWrite + Unpin>(
    io: &mut I, host: bool, limits: &ProviderTransportLimits,
) -> Result<ProviderTransportLimits, TransportError> {
    validate_limits(limits)?;
    let features: Vec<String> = FEATURES.iter().map(|v| v.to_string()).collect();
    let remote = tokio::time::timeout(BOOTSTRAP_TIMEOUT, async {
        if host {
            boot_write(io, 1, &ProviderTransportHello { supported_versions: vec![1], features: features.clone(), receive: limits.clone() }).await?;
            let v: ProviderTransportSelection = boot_read(io, 2).await?;
            if v.selected_version != 1 { return Err(error("unsupported transport selection")); }
            validate_features(&v.features)?; validate_limits(&v.receive)?;
            boot_write(io, 3, &1u8).await?;
            if boot_read::<_, u8>(io, 4).await? != 1 { return Err(error("invalid READY")); }
            Ok(v.receive)
        } else {
            let v: ProviderTransportHello = boot_read(io, 1).await?;
            if !v.supported_versions.contains(&1) || v.supported_versions.len() > 8 { return Err(error("no common transport version")); }
            validate_features(&v.features)?; validate_limits(&v.receive)?;
            boot_write(io, 2, &ProviderTransportSelection { selected_version: 1, features, receive: limits.clone() }).await?;
            if boot_read::<_, u8>(io, 3).await? != 1 { return Err(error("invalid CONFIRM")); }
            boot_write(io, 4, &1u8).await?;
            Ok(v.receive)
        }
    }).await.map_err(|_| error("transport handshake timed out"))??;
    eprintln!("{}", serde_json::json!({"schema":"codepet.provider.transport.v1", "name":"provider.mux.ready",
        "profile":MUX_PROFILE, "role":if host {"host"} else {"provider"}, "receive":limits, "send":remote}));
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::compat::TokioAsyncReadCompatExt;
    #[tokio::test]
    async fn handshake_rejects_version_and_limits_before_business_data() {
        let (a, b) = tokio::io::duplex(512);
        let server = tokio::spawn(async move { negotiate(&mut b.compat(), false, &default_transport_limits()).await });
        let mut a = a.compat();
        boot_write(&mut a, 1, &ProviderTransportHello { supported_versions: vec![99], features: FEATURES.iter().map(|s| s.to_string()).collect(), receive: default_transport_limits() }).await.unwrap();
        assert!(matches!(server.await.unwrap(), Err(e) if e.message.contains("common transport version")));
        let mut invalid = default_transport_limits(); invalid.receive_budget_bytes = 2048;
        assert!(validate_limits(&invalid).is_err());
        assert!(validate_features(&["raw-json".into()]).is_err());
    }
}
