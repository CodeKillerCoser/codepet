use super::error::TransportError;
use crate::generated::{
    ProviderTransportHello, ProviderTransportLimits, ProviderTransportSelection,
};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

pub use crate::protocol::{default_transport_limits, MUX_PROFILE, TRANSPORT_ENV};
use crate::protocol::*;
const BOOTSTRAP_LIMIT: usize = BOOTSTRAP_MAX_JSON_BYTES;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_millis(BOOTSTRAP_TIMEOUT_MS as u64);
fn error(message: impl ToString) -> TransportError { TransportError::new(message) }

fn validate_features(v: &[String]) -> Result<(), TransportError> {
    if v.len() != FEATURES.len() || FEATURES.iter().any(|f| !v.iter().any(|v| v == f)) {
        return Err(error("required transport features do not match mux v1"));
    }
    Ok(())
}

async fn boot_write<I: futures::AsyncWrite + Unpin, T: Serialize>(
    io: &mut I,
    tag: u8,
    v: &T,
) -> Result<(), TransportError> {
    let bytes = serde_json::to_vec(v).map_err(error)?;
    if bytes.len() > BOOTSTRAP_LIMIT {
        return Err(error("bootstrap exceeds limit"));
    }
    io.write_all(&BOOTSTRAP_MAGIC).await.map_err(error)?;
    io.write_all(&[BOOTSTRAP_VERSION, tag]).await.map_err(error)?;
    io.write_all(&(bytes.len() as u16).to_be_bytes())
        .await
        .map_err(error)?;
    io.write_all(&bytes).await.map_err(error)?;
    io.flush().await.map_err(error)
}
async fn boot_read<I: futures::AsyncRead + Unpin, T: DeserializeOwned>(
    io: &mut I,
    tag: u8,
) -> Result<T, TransportError> {
    let mut header = [0; BOOTSTRAP_HEADER_BYTES];
    io.read_exact(&mut header).await.map_err(error)?;
    if &header[..6] != [BOOTSTRAP_MAGIC[0], BOOTSTRAP_MAGIC[1], BOOTSTRAP_MAGIC[2], BOOTSTRAP_MAGIC[3], BOOTSTRAP_VERSION, tag] {
        return Err(error("invalid bootstrap phase or profile"));
    }
    let n = u16::from_be_bytes([header[BOOTSTRAP_JSON_LENGTH_OFFSET], header[BOOTSTRAP_JSON_LENGTH_OFFSET+1]]) as usize;
    if n > BOOTSTRAP_LIMIT {
        return Err(error("bootstrap exceeds limit"));
    }
    let mut bytes = vec![0; n];
    io.read_exact(&mut bytes).await.map_err(error)?;
    serde_json::from_slice(&bytes).map_err(error)
}

pub(super) async fn negotiate<I: AsyncRead + AsyncWrite + Unpin>(
    io: &mut I,
    host: bool,
    limits: &ProviderTransportLimits,
) -> Result<ProviderTransportLimits, TransportError> {
    validate_limits(limits)?;
    let features: Vec<String> = FEATURES.iter().map(|v| v.to_string()).collect();
    let remote = tokio::time::timeout(BOOTSTRAP_TIMEOUT, async {
        if host {
            boot_write(
                io,
                PHASE_HELLO,
                &ProviderTransportHello {
                    supported_versions: vec![BOOTSTRAP_VERSION as u64],
                    features: features.clone(),
                    receive: limits.clone(),
                },
            )
            .await?;
            let v: ProviderTransportSelection = boot_read(io, PHASE_SELECT).await?;
            if v.selected_version != BOOTSTRAP_VERSION as u64 {
                return Err(error("unsupported transport selection"));
            }
            validate_features(&v.features)?;
            validate_limits(&v.receive)?;
            boot_write(io, PHASE_CONFIRM, &BOOTSTRAP_CONFIRMATION).await?;
            if boot_read::<_, u8>(io, PHASE_READY).await? != BOOTSTRAP_CONFIRMATION {
                return Err(error("invalid READY"));
            }
            Ok(v.receive)
        } else {
            let v: ProviderTransportHello = boot_read(io, PHASE_HELLO).await?;
            if !v.supported_versions.contains(&(BOOTSTRAP_VERSION as u64)) || v.supported_versions.len() > BOOTSTRAP_MAX_SUPPORTED_VERSIONS {
                return Err(error("no common transport version"));
            }
            validate_features(&v.features)?;
            validate_limits(&v.receive)?;
            boot_write(
                io,
                PHASE_SELECT,
                &ProviderTransportSelection {
                    selected_version: BOOTSTRAP_VERSION as u64,
                    features,
                    receive: limits.clone(),
                },
            )
            .await?;
            if boot_read::<_, u8>(io, PHASE_CONFIRM).await? != BOOTSTRAP_CONFIRMATION {
                return Err(error("invalid CONFIRM"));
            }
            boot_write(io, PHASE_READY, &BOOTSTRAP_CONFIRMATION).await?;
            Ok(v.receive)
        }
    })
    .await
    .map_err(|_| error("transport handshake timed out"))??;
    eprintln!(
        "{}",
        serde_json::json!({"schema":"codepet.provider.transport.v1", "name":"provider.mux.ready",
        "profile":MUX_PROFILE, "role":if host {"host"} else {"provider"}, "receive":limits, "send":remote})
    );
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::compat::TokioAsyncReadCompatExt;
    #[tokio::test]
    async fn handshake_rejects_version_and_limits_before_business_data() {
        let (a, b) = tokio::io::duplex(512);
        let server = tokio::spawn(async move {
            negotiate(&mut b.compat(), false, &default_transport_limits()).await
        });
        let mut a = a.compat();
        boot_write(
            &mut a,
            1,
            &ProviderTransportHello {
                supported_versions: vec![99],
                features: FEATURES.iter().map(|s| s.to_string()).collect(),
                receive: default_transport_limits(),
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(server.await.unwrap(), Err(e) if e.message.contains("common transport version"))
        );
        let mut invalid = default_transport_limits();
        invalid.receive_budget_bytes = 2048;
        assert!(validate_limits(&invalid).is_err());
        assert!(validate_features(&["raw-json".into()]).is_err());
    }
}
