use rusqlite::{params, OptionalExtension};
use crate::{HostError, HostResult};
use codepet_provider_sdk::{DeviceId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DEVICE_IDENTITY_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    pub version: u32,
    pub device_id: DeviceId,
    pub display_name: String,
    pub created_at: TimestampMs,
}

impl DeviceIdentity {
    fn generate(display_name: impl Into<String>) -> HostResult<Self> {
        let display_name = display_name.into();
        Self::validate_display_name(&display_name)?;
        Ok(Self {
            version: DEVICE_IDENTITY_VERSION,
            device_id: format!("device-{}", Uuid::new_v4()),
            display_name,
            created_at: now_ms(),
        })
    }

    fn validate_display_name(display_name: &str) -> HostResult<()> {
        if display_name.trim().is_empty() {
            return Err(HostError::new(
                "invalid_device_identity",
                "device display name must not be empty",
            ));
        }
        Ok(())
    }

    fn update_display_name(&mut self, display_name: String) -> HostResult<bool> {
        Self::validate_display_name(&display_name)?;
        if self.display_name == display_name {
            return Ok(false);
        }
        self.display_name = display_name;
        Ok(true)
    }

    fn validate(&self) -> HostResult<()> {
        if self.version != DEVICE_IDENTITY_VERSION {
            return Err(HostError::new(
                "unsupported_device_identity_version",
                format!("unsupported device identity version: {}", self.version),
            ));
        }
        if self.device_id.trim().is_empty() || self.display_name.trim().is_empty() {
            return Err(HostError::new(
                "invalid_device_identity",
                "device id and display name must not be empty",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceDiagnostic {
    pub code: String,
    pub message: String,
    pub path: PathBuf,
    pub recovered_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct DeviceRegistry {
    identity: DeviceIdentity,
    diagnostics: Vec<DeviceDiagnostic>,
}

impl DeviceRegistry {
    pub fn open(path: impl Into<PathBuf>, display_name: impl Into<String>) -> HostResult<Self> {
        let path = path.into();
        let display_name = display_name.into();
        DeviceIdentity::validate_display_name(&display_name)?;
        let mut db = crate::storage::open(&path)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(crate::storage::error)?;
        let saved = tx.query_row("SELECT device_id,display_name,created_at FROM host_identity WHERE id=1", [], |r| Ok(DeviceIdentity {
            version: DEVICE_IDENTITY_VERSION, device_id:r.get(0)?,display_name:r.get(1)?,created_at:r.get(2)?,
        })).optional().map_err(crate::storage::error)?;
        let mut identity = match saved { Some(identity) => identity, None => DeviceIdentity::generate(&display_name)? };
        identity.validate()?;
        identity.update_display_name(display_name)?;
        tx.execute("INSERT INTO host_identity VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name WHERE display_name!=excluded.display_name",
            params![identity.device_id,identity.display_name,identity.created_at]).map_err(crate::storage::error)?;
        tx.commit().map_err(crate::storage::error)?;
        Ok(Self { identity, diagnostics:Vec::new() })
    }

    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub fn diagnostics(&self) -> &[DeviceDiagnostic] {
        &self.diagnostics
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
