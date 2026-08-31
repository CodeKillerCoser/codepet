use crate::persistence::{persistence_io, write_json_atomically};
use crate::{HostError, HostResult};
use codepet_provider_sdk::{DeviceId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
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
        if !path.exists() {
            let identity = DeviceIdentity::generate(display_name)?;
            write_json_atomically(&path, &identity)?;
            return Ok(Self {
                identity,
                diagnostics: Vec::new(),
            });
        }

        match read_identity(&path) {
            Ok(mut identity) => {
                if identity.update_display_name(display_name)? {
                    write_json_atomically(&path, &identity)?;
                }
                Ok(Self {
                    identity,
                    diagnostics: Vec::new(),
                })
            }
            Err(error) => {
                let recovered_path = quarantine_corrupt_identity(&path)?;
                let identity = DeviceIdentity::generate(display_name)?;
                write_json_atomically(&path, &identity)?;
                Ok(Self {
                    identity,
                    diagnostics: vec![DeviceDiagnostic {
                        code: "device_identity_rebuilt".to_string(),
                        message: format!(
                            "device identity was invalid and was safely rebuilt: {error}"
                        ),
                        path,
                        recovered_path: Some(recovered_path),
                    }],
                })
            }
        }
    }

    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub fn diagnostics(&self) -> &[DeviceDiagnostic] {
        &self.diagnostics
    }
}

fn read_identity(path: &Path) -> HostResult<DeviceIdentity> {
    let payload = fs::read(path).map_err(|error| persistence_io("read device identity", path, error))?;
    let identity: DeviceIdentity = serde_json::from_slice(&payload).map_err(|error| {
        HostError::new(
            "invalid_device_identity",
            format!("decode device identity {}: {error}", path.display()),
        )
        .with_detail("path", path.display().to_string())
    })?;
    identity.validate()?;
    Ok(identity)
}

fn quarantine_corrupt_identity(path: &Path) -> HostResult<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        HostError::new(
            "invalid_persistence_path",
            format!("device identity path has no parent: {}", path.display()),
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("device-identity.json");
    let recovered_path = parent.join(format!("{file_name}.corrupt-{}", now_ms()));
    fs::rename(path, &recovered_path)
        .map_err(|error| persistence_io("quarantine corrupt device identity", path, error))?;
    Ok(recovered_path)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
