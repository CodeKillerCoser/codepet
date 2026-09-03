use crate::persistence::{
    persistence_io, protect_secret_file, write_secret_json_atomically,
};
use crate::{DeviceRegistry, HostError, HostResult};
use codepet_lan_channel_sdk::{
    DeviceDescriptor, LanHostIdentity, PairingExchangeRequest, PairingRequestCreateRequest,
    PairingRequestState,
};
use codepet_provider_sdk::{ClientId, TimestampMs};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::{Debug, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::sync::watch;
use x509_parser::parse_x509_certificate;

const LAN_TLS_IDENTITY_VERSION: u32 = 1;
const REMOTE_CREDENTIAL_STORE_VERSION: u32 = 2;
const RANDOM_SECRET_BYTES: usize = 32;
const RANDOM_ID_BYTES: usize = 16;
const SHA256_HEX_LENGTH: usize = 64;
const MAX_CLIENT_ID_LENGTH: usize = 256;
const MAX_DEVICE_NAME_LENGTH: usize = 256;
const MAX_OPERATING_SYSTEM_LENGTH: usize = 128;
const MAX_SYSTEM_VERSION_LENGTH: usize = 128;
const MAX_TLS_IDENTITY_FILE_BYTES: usize = 64 * 1024;
const MAX_CREDENTIAL_STORE_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REMOTE_CREDENTIALS: usize = 4096;

pub const PAIRING_SESSION_TTL: Duration = Duration::from_secs(5 * 60);
pub const PAIRING_REQUEST_TTL: Duration = Duration::from_secs(2 * 60);
const PAIRING_STATUS_RETENTION: Duration = Duration::from_secs(15 * 60);
const MAX_PAIRING_STATUS_RECORDS: usize = 64;
const MAX_PAIRING_REQUEST_RECORDS: usize = 64;

type Clock = Arc<dyn Fn() -> TimestampMs + Send + Sync>;

/// Explicit App-data paths used by the remote access persistence core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAccessConfig {
    pub tls_identity_path: PathBuf,
    pub credential_store_path: PathBuf,
}

impl RemoteAccessConfig {
    pub fn for_data_directory(directory: impl AsRef<Path>) -> Self {
        let directory = directory.as_ref();
        Self {
            tls_identity_path: directory.join("lan-tls-identity.json"),
            credential_store_path: directory.join("remote-credentials.json"),
        }
    }

    fn validate(&self) -> HostResult<()> {
        if self.tls_identity_path == self.credential_store_path {
            return Err(HostError::new(
                "invalid_remote_access_config",
                "LAN TLS identity and remote credential store paths must be different",
            ));
        }
        for path in [&self.tls_identity_path, &self.credential_store_path] {
            if path.parent().is_none() {
                return Err(HostError::new(
                    "invalid_persistence_path",
                    format!("remote access persistence path has no parent: {}", path.display()),
                ));
            }
        }
        Ok(())
    }
}

/// A safe recovery notice for invalid remote access persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAccessDiagnostic {
    pub code: String,
    pub message: String,
    pub path: PathBuf,
    pub recovered_path: Option<PathBuf>,
}

/// Persisted self-signed leaf certificate and matching PKCS#8 private key.
#[derive(Clone, PartialEq, Eq)]
pub struct LanTlsIdentity {
    certificate_der: Vec<u8>,
    private_key_der: Vec<u8>,
    certificate_fingerprint: String,
    created_at: TimestampMs,
}

impl LanTlsIdentity {
    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate_der
    }

    pub fn private_key_der(&self) -> &[u8] {
        &self.private_key_der
    }

    pub fn certificate_fingerprint(&self) -> &str {
        &self.certificate_fingerprint
    }

    pub fn created_at(&self) -> TimestampMs {
        self.created_at
    }
}

impl Debug for LanTlsIdentity {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LanTlsIdentity")
            .field("certificate_der_bytes", &self.certificate_der.len())
            .field("private_key_der", &"<redacted>")
            .field("certificate_fingerprint", &self.certificate_fingerprint)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct LanTlsIdentityDocument {
    version: u32,
    certificate_der: String,
    private_key_der: String,
    certificate_sha256: String,
    private_key_sha256: String,
    created_at: TimestampMs,
}

/// Client metadata bound to every issued remote credential.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RemoteClientIdentity {
    pub client_id: ClientId,
    pub descriptor: DeviceDescriptor,
}

impl RemoteClientIdentity {
    fn validate(&self) -> HostResult<()> {
        validate_text_field(
            "clientId",
            &self.client_id,
            MAX_CLIENT_ID_LENGTH,
        )?;
        validate_device_descriptor(&self.descriptor)
    }
}

/// One locally opened, five-minute pairing opportunity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PairingSession {
    pub pairing_id: String,
    pub pairing_secret: String,
    /// Wall-clock display hint for QR/UI; authorization uses a process-local monotonic deadline.
    pub expires_at: TimestampMs,
}

impl Debug for PairingSession {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PairingSession")
            .field("pairing_id", &self.pairing_id)
            .field("pairing_secret", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairingStatusKind {
    Active,
    Succeeded,
    Expired,
    Cancelled,
}

/// Non-secret, process-local pairing outcome retained for short-term UI queries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PairingStatus {
    pub pairing_id: String,
    pub state: PairingStatusKind,
    pub expires_at: TimestampMs,
}

/// Minimal Host-side signal used by lifecycle owners to synchronize discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingWatchState {
    pub pairing_id: Option<String>,
    pub pairing_available: bool,
    pub deadline: Option<Instant>,
}

impl PairingWatchState {
    fn unavailable() -> Self {
        Self {
            pairing_id: None,
            pairing_available: false,
            deadline: None,
        }
    }
}

/// Public credential metadata. Bearer hashes are never exposed through this type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RemoteCredential {
    pub credential_id: String,
    pub client_id: ClientId,
    pub descriptor: DeviceDescriptor,
    pub created_at: TimestampMs,
    pub last_seen_at: TimestampMs,
    pub revoked_at: Option<TimestampMs>,
}

/// A bearer returned exactly once after a successful pairing exchange.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct IssuedRemoteCredential {
    pub credential: RemoteCredential,
    pub bearer_token: String,
}

impl Debug for IssuedRemoteCredential {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedRemoteCredential")
            .field("credential", &self.credential)
            .field("bearer_token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct StoredRemoteCredential {
    credential_id: String,
    client_id: ClientId,
    descriptor: DeviceDescriptor,
    bearer_sha256: String,
    created_at: TimestampMs,
    last_seen_at: TimestampMs,
    revoked_at: Option<TimestampMs>,
}

impl StoredRemoteCredential {
    fn public(&self) -> RemoteCredential {
        RemoteCredential {
            credential_id: self.credential_id.clone(),
            client_id: self.client_id.clone(),
            descriptor: self.descriptor.clone(),
            created_at: self.created_at,
            last_seen_at: self.last_seen_at,
            revoked_at: self.revoked_at,
        }
    }

    fn validate(&self) -> HostResult<()> {
        validate_prefixed_random_id("credential", &self.credential_id)?;
        RemoteClientIdentity {
            client_id: self.client_id.clone(),
            descriptor: self.descriptor.clone(),
        }
        .validate()?;
        validate_sha256_hex("bearerSha256", &self.bearer_sha256)?;
        if self.last_seen_at < self.created_at
            || self
                .revoked_at
                .is_some_and(|revoked_at| revoked_at < self.created_at)
        {
            return Err(HostError::new(
                "invalid_remote_credential_store",
                "remote credential timestamps are inconsistent",
            )
            .with_detail("credentialId", self.credential_id.clone()));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct RemoteCredentialStoreDocument {
    version: u32,
    tls_certificate_sha256: String,
    credentials: Vec<StoredRemoteCredential>,
}

/// Durable hashed bearer records bound to one LAN TLS certificate fingerprint.
pub struct RemoteCredentialStore {
    path: PathBuf,
    tls_certificate_fingerprint: String,
    credentials: Mutex<BTreeMap<String, StoredRemoteCredential>>,
    diagnostics: Vec<RemoteAccessDiagnostic>,
    clock: Clock,
}

impl RemoteCredentialStore {
    pub fn open(
        path: impl Into<PathBuf>,
        tls_certificate_fingerprint: impl Into<String>,
    ) -> HostResult<Self> {
        Self::open_with_clock(
            path.into(),
            tls_certificate_fingerprint.into(),
            Arc::new(now_ms),
        )
    }

    fn open_with_clock(
        path: PathBuf,
        tls_certificate_fingerprint: String,
        clock: Clock,
    ) -> HostResult<Self> {
        validate_sha256_hex(
            "tlsCertificateSha256",
            &tls_certificate_fingerprint,
        )?;
        let (credentials, diagnostics) = if path.exists() {
            protect_secret_file(&path)?;
            match load_credentials(&path, &tls_certificate_fingerprint) {
                Ok(credentials) => (credentials, Vec::new()),
                Err(error) => {
                    let recovered_path = quarantine_corrupt_file(&path, clock())?;
                    let credentials = BTreeMap::new();
                    persist_credentials(
                        &path,
                        &tls_certificate_fingerprint,
                        &credentials,
                    )?;
                    (
                        credentials,
                        vec![RemoteAccessDiagnostic {
                            code: "remote_credentials_rebuilt".to_string(),
                            message: format!(
                                "remote credentials were invalid and were safely reset: {error}"
                            ),
                            path: path.clone(),
                            recovered_path: Some(recovered_path),
                        }],
                    )
                }
            }
        } else {
            let credentials = BTreeMap::new();
            persist_credentials(&path, &tls_certificate_fingerprint, &credentials)?;
            (credentials, Vec::new())
        };
        Ok(Self {
            path,
            tls_certificate_fingerprint,
            credentials: Mutex::new(credentials),
            diagnostics,
            clock,
        })
    }

    pub fn diagnostics(&self) -> &[RemoteAccessDiagnostic] {
        &self.diagnostics
    }

    pub fn list(&self) -> HostResult<Vec<RemoteCredential>> {
        let credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        let mut listed = credentials
            .values()
            .map(StoredRemoteCredential::public)
            .collect::<Vec<_>>();
        listed.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.credential_id.cmp(&right.credential_id))
        });
        Ok(listed)
    }

    /// Validates an active bearer and durably advances its `lastSeenAt` value.
    pub fn validate_bearer(&self, bearer_token: &str) -> HostResult<RemoteCredential> {
        validate_bearer_token(bearer_token)?;
        let bearer_hash = sha256_bytes(bearer_token.as_bytes());
        let now = (self.clock)();
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        let credential_id = credentials.values().find_map(|credential| {
            let stored_hash = decode_sha256_hex(&credential.bearer_sha256).ok()?;
            (credential.revoked_at.is_none()
                && constant_time_equal(&stored_hash, &bearer_hash))
            .then(|| credential.credential_id.clone())
        });
        let Some(credential_id) = credential_id else {
            return Err(invalid_remote_credential());
        };
        let mut updated = credentials.clone();
        let credential = updated.get_mut(&credential_id).ok_or_else(|| {
            HostError::new(
                "remote_credential_store_unavailable",
                "validated remote credential disappeared from the credential store",
            )
            .retryable(true)
        })?;
        credential.last_seen_at = now.max(credential.last_seen_at);
        let validated = credential.public();
        persist_credentials(
            &self.path,
            &self.tls_certificate_fingerprint,
            &updated,
        )?;
        *credentials = updated;
        Ok(validated)
    }

    /// Revokes every active credential bound to the specified client id.
    pub fn revoke_client(&self, client_id: &str) -> HostResult<Vec<RemoteCredential>> {
        validate_text_field("clientId", client_id, MAX_CLIENT_ID_LENGTH)?;
        let now = (self.clock)();
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        let mut updated = credentials.clone();
        let mut revoked = Vec::new();
        for credential in updated.values_mut() {
            if credential.client_id == client_id && credential.revoked_at.is_none() {
                credential.revoked_at = Some(now.max(credential.created_at));
                revoked.push(credential.public());
            }
        }
        if revoked.is_empty() {
            return Ok(revoked);
        }
        persist_credentials(
            &self.path,
            &self.tls_certificate_fingerprint,
            &updated,
        )?;
        *credentials = updated;
        Ok(revoked)
    }

    /// Revokes one credential by its public id. Repeating a successful revoke is a no-op.
    pub fn revoke_credential(&self, credential_id: &str) -> HostResult<RemoteCredential> {
        validate_prefixed_random_id("credential", credential_id)?;
        let now = (self.clock)();
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        let Some(existing) = credentials.get(credential_id) else {
            return Err(HostError::new(
                "remote_credential_not_found",
                "Remote credential does not exist",
            ));
        };
        if existing.revoked_at.is_some() {
            return Ok(existing.public());
        }
        let mut updated = credentials.clone();
        let credential = updated.get_mut(credential_id).ok_or_else(|| {
            HostError::new(
                "remote_credential_store_unavailable",
                "remote credential disappeared from the credential store",
            )
            .retryable(true)
        })?;
        credential.revoked_at = Some(now.max(credential.created_at));
        let revoked = credential.public();
        persist_credentials(
            &self.path,
            &self.tls_certificate_fingerprint,
            &updated,
        )?;
        *credentials = updated;
        Ok(revoked)
    }

    /// Revokes the active credential represented by the supplied bearer itself.
    pub fn revoke_current(&self, bearer_token: &str) -> HostResult<RemoteCredential> {
        validate_bearer_token(bearer_token)?;
        let bearer_hash = sha256_bytes(bearer_token.as_bytes());
        let now = (self.clock)();
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        let credential_id = credentials.values().find_map(|credential| {
            let stored_hash = decode_sha256_hex(&credential.bearer_sha256).ok()?;
            (credential.revoked_at.is_none()
                && constant_time_equal(&stored_hash, &bearer_hash))
            .then(|| credential.credential_id.clone())
        });
        let Some(credential_id) = credential_id else {
            return Err(invalid_remote_credential());
        };
        let mut updated = credentials.clone();
        let credential = updated.get_mut(&credential_id).ok_or_else(|| {
            HostError::new(
                "remote_credential_store_unavailable",
                "current remote credential disappeared from the credential store",
            )
            .retryable(true)
        })?;
        credential.revoked_at = Some(now.max(credential.created_at));
        let revoked = credential.public();
        persist_credentials(
            &self.path,
            &self.tls_certificate_fingerprint,
            &updated,
        )?;
        *credentials = updated;
        Ok(revoked)
    }

    fn update_descriptor(
        &self,
        credential_id: &str,
        descriptor: DeviceDescriptor,
    ) -> HostResult<RemoteCredential> {
        validate_prefixed_random_id("credential", credential_id)?;
        validate_device_descriptor(&descriptor)?;
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        let Some(existing) = credentials.get(credential_id) else {
            return Err(HostError::new(
                "remote_credential_not_found",
                "Remote credential does not exist",
            ));
        };
        if existing.descriptor == descriptor {
            return Ok(existing.public());
        }
        let mut updated = credentials.clone();
        let credential = updated.get_mut(credential_id).ok_or_else(|| {
            HostError::new(
                "remote_credential_store_unavailable",
                "remote credential disappeared from the credential store",
            )
            .retryable(true)
        })?;
        credential.descriptor = descriptor;
        let refreshed = credential.public();
        persist_credentials(
            &self.path,
            &self.tls_certificate_fingerprint,
            &updated,
        )?;
        *credentials = updated;
        Ok(refreshed)
    }

    fn issue(&self, client: RemoteClientIdentity) -> HostResult<IssuedRemoteCredential> {
        client.validate()?;
        let credential_id = random_prefixed_id("credential")?;
        let bearer_token = random_hex::<RANDOM_SECRET_BYTES>()?;
        let bearer_sha256 = sha256_hex(bearer_token.as_bytes());
        let created_at = (self.clock)();
        let stored = StoredRemoteCredential {
            credential_id: credential_id.clone(),
            client_id: client.client_id,
            descriptor: client.descriptor,
            bearer_sha256,
            created_at,
            last_seen_at: created_at,
            revoked_at: None,
        };
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| credential_store_lock_error())?;
        if credentials.len() >= MAX_REMOTE_CREDENTIALS {
            return Err(HostError::new(
                "remote_credential_limit_reached",
                "remote credential store has reached its credential limit",
            ));
        }
        if credentials.contains_key(&credential_id)
            || credentials
                .values()
                .any(|credential| credential.bearer_sha256 == stored.bearer_sha256)
        {
            return Err(HostError::new(
                "remote_credential_random_collision",
                "secure random credential material collided with an existing credential",
            )
            .retryable(true));
        }
        let mut updated = credentials.clone();
        updated.insert(credential_id, stored.clone());
        persist_credentials(
            &self.path,
            &self.tls_certificate_fingerprint,
            &updated,
        )?;
        *credentials = updated;
        Ok(IssuedRemoteCredential {
            credential: stored.public(),
            bearer_token,
        })
    }
}

struct ActivePairingSession {
    pairing_id: String,
    pairing_secret_sha256: [u8; 32],
    deadline: Instant,
    expires_at: TimestampMs,
}

struct PairingOutcomeRecord {
    status: PairingStatus,
    recorded_at: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemotePairingRequest {
    pub request_id: String,
    pub state: PairingRequestState,
    pub client: RemoteClientIdentity,
    pub expires_at: TimestampMs,
    pub confirmation_code: String,
    bearer_token: Option<String>,
}

impl RemotePairingRequest {
    pub fn bearer_token(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }
}

struct PairingRequestRecord {
    request: RemotePairingRequest,
    client_nonce: String,
    host_pairing_id: Option<String>,
    deadline: Instant,
    recorded_at: Instant,
}

struct PairingState {
    active: Option<ActivePairingSession>,
    outcomes: BTreeMap<String, PairingOutcomeRecord>,
    outcome_order: VecDeque<String>,
    requests: BTreeMap<String, PairingRequestRecord>,
    request_order: VecDeque<String>,
}

/// Host security boundary for a future LAN HTTP/WSS listener.
///
/// Device identity is always read through the retained `DeviceRegistry`; pairing
/// state is process-local and credentials are persisted only as SHA-256 hashes.
pub struct RemoteAccessManager {
    device: Arc<DeviceRegistry>,
    local_device_descriptor: DeviceDescriptor,
    tls_identity: LanTlsIdentity,
    credential_store: RemoteCredentialStore,
    pairing: Mutex<PairingState>,
    pairing_watch: watch::Sender<PairingWatchState>,
    diagnostics: Vec<RemoteAccessDiagnostic>,
    clock: Clock,
}

impl RemoteAccessManager {
    pub fn open(
        config: RemoteAccessConfig,
        device: Arc<DeviceRegistry>,
        local_device_descriptor: DeviceDescriptor,
    ) -> HostResult<Self> {
        Self::open_with_clock(
            config,
            device,
            local_device_descriptor,
            Arc::new(now_ms),
        )
    }

    fn open_with_clock(
        config: RemoteAccessConfig,
        device: Arc<DeviceRegistry>,
        local_device_descriptor: DeviceDescriptor,
        clock: Clock,
    ) -> HostResult<Self> {
        config.validate()?;
        validate_device_descriptor(&local_device_descriptor)?;
        let (tls_identity, mut diagnostics) =
            open_tls_identity(&config.tls_identity_path, clock.clone())?;
        let credential_store = RemoteCredentialStore::open_with_clock(
            config.credential_store_path,
            tls_identity.certificate_fingerprint.clone(),
            clock.clone(),
        )?;
        diagnostics.extend_from_slice(credential_store.diagnostics());
        let (pairing_watch, _) = watch::channel(PairingWatchState::unavailable());
        Ok(Self {
            device,
            local_device_descriptor,
            tls_identity,
            credential_store,
            pairing: Mutex::new(PairingState {
                active: None,
                outcomes: BTreeMap::new(),
                outcome_order: VecDeque::new(),
                requests: BTreeMap::new(),
                request_order: VecDeque::new(),
            }),
            pairing_watch,
            diagnostics,
            clock,
        })
    }

    pub fn device_registry(&self) -> &DeviceRegistry {
        self.device.as_ref()
    }

    pub fn tls_identity(&self) -> &LanTlsIdentity {
        &self.tls_identity
    }

    pub fn credential_store(&self) -> &RemoteCredentialStore {
        &self.credential_store
    }

    pub fn diagnostics(&self) -> &[RemoteAccessDiagnostic] {
        &self.diagnostics
    }

    pub fn remote_host_identity(&self) -> LanHostIdentity {
        let device = self.device.identity();
        LanHostIdentity {
            device_id: device.device_id.clone(),
            descriptor: self.local_device_descriptor.clone(),
            identity_fingerprint: self.tls_identity.certificate_fingerprint.clone(),
        }
    }

    /// Opens one five-minute pairing session. A second active session is rejected.
    pub fn begin_pairing(&self) -> HostResult<PairingSession> {
        self.begin_pairing_with_ttl(PAIRING_SESSION_TTL)
    }

    fn begin_pairing_with_ttl(&self, ttl: Duration) -> HostResult<PairingSession> {
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_locked(&mut pairing, Instant::now());
        if let Some(active) = pairing.active.as_ref() {
            return Err(HostError::new(
                "pairing_session_active",
                "Only one remote pairing session may be active at a time",
            )
            .with_detail("pairingId", active.pairing_id.clone()));
        }
        let pairing_id = random_prefixed_id("pairing")?;
        let pairing_secret = random_hex::<RANDOM_SECRET_BYTES>()?;
        let deadline = Instant::now().checked_add(ttl).ok_or_else(|| {
            HostError::new(
                "invalid_pairing_ttl",
                "remote pairing TTL exceeds the monotonic clock range",
            )
        })?;
        let expires_at = (self.clock)().saturating_add(duration_ms(ttl));
        let active = ActivePairingSession {
            pairing_id: pairing_id.clone(),
            pairing_secret_sha256: sha256_bytes(pairing_secret.as_bytes()),
            deadline,
            expires_at,
        };
        pairing.active = Some(active);
        self.pairing_watch.send_replace(PairingWatchState {
            pairing_id: Some(pairing_id.clone()),
            pairing_available: true,
            deadline: Some(deadline),
        });
        Ok(PairingSession {
            pairing_id,
            pairing_secret,
            expires_at,
        })
    }

    /// Atomically consumes a valid pairing session and persists one new credential.
    pub fn complete_pairing(
        &self,
        pairing_id: &str,
        request: PairingExchangeRequest,
    ) -> HostResult<IssuedRemoteCredential> {
        if validate_prefixed_random_id("pairing", pairing_id).is_err()
            || !is_canonical_hex(&request.pairing_secret, SHA256_HEX_LENGTH)
        {
            return Err(invalid_pairing_session());
        }
        let supplied_secret_hash = sha256_bytes(request.pairing_secret.as_bytes());
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        let Some(session) = pairing.active.as_ref() else {
            return Err(invalid_pairing_session());
        };
        if Instant::now() >= session.deadline {
            self.expire_locked(&mut pairing, Instant::now());
            return Err(HostError::new(
                "pairing_session_expired",
                "remote pairing session has expired",
            ));
        }
        if session.pairing_id != pairing_id
            || !constant_time_equal(
                &session.pairing_secret_sha256,
                &supplied_secret_hash,
            )
        {
            return Err(invalid_pairing_session());
        }
        let client = RemoteClientIdentity {
            client_id: request.client_id,
            descriptor: request.device,
        };
        client.validate()?;
        let issued = self.credential_store.issue(client)?;
        let completed = pairing.active.take().ok_or_else(|| {
            HostError::new(
                "pairing_session_unavailable",
                "Remote pairing session changed before it could be consumed",
            )
            .retryable(true)
        })?;
        self.record_outcome_locked(
            &mut pairing,
            PairingStatus {
                pairing_id: completed.pairing_id,
                state: PairingStatusKind::Succeeded,
                expires_at: completed.expires_at,
            },
            Instant::now(),
        );
        self.pairing_watch
            .send_replace(PairingWatchState::unavailable());
        Ok(issued)
    }

    pub fn subscribe_pairing_state(&self) -> watch::Receiver<PairingWatchState> {
        self.pairing_watch.subscribe()
    }

    pub fn create_pairing_request(
        &self,
        request: PairingRequestCreateRequest,
    ) -> HostResult<RemotePairingRequest> {
        if request.host_device_id != self.device.identity().device_id {
            return Err(HostError::new(
                "pairing_host_identity_mismatch",
                "Pairing request targeted a different Host device",
            ));
        }
        if !is_canonical_hex(&request.client_nonce, SHA256_HEX_LENGTH) {
            return Err(HostError::new(
                "invalid_pairing_request",
                "Pairing request clientNonce must be 64 lowercase hexadecimal characters",
            ));
        }
        let client = RemoteClientIdentity {
            client_id: request.client_id,
            descriptor: request.device,
        };
        client.validate()?;

        let now = Instant::now();
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_locked(&mut pairing, now);
        self.expire_pairing_requests_locked(&mut pairing, now);
        if let Some(existing) = pairing.requests.values().find(|record| {
            record.request.client.client_id == client.client_id
                && record.client_nonce == request.client_nonce
        }) {
            return Ok(existing.request.clone());
        }
        if pairing
            .requests
            .values()
            .filter(|record| record.request.state == PairingRequestState::Pending)
            .count()
            >= MAX_PAIRING_REQUEST_RECORDS
        {
            return Err(HostError::new(
                "pairing_request_limit_reached",
                "Too many pairing requests are retained",
            ));
        }

        let request_id = random_prefixed_id("request")?;
        let confirmation_code = pairing_confirmation_code(
            &request_id,
            &request.client_nonce,
            self.tls_identity.certificate_fingerprint(),
        );
        let deadline = now.checked_add(PAIRING_REQUEST_TTL).ok_or_else(|| {
            HostError::new(
                "invalid_pairing_ttl",
                "remote pairing request TTL exceeds the monotonic clock range",
            )
        })?;
        let expires_at = (self.clock)().saturating_add(duration_ms(PAIRING_REQUEST_TTL));
        let pairing_request = RemotePairingRequest {
            request_id: request_id.clone(),
            state: PairingRequestState::Pending,
            client: client.clone(),
            expires_at,
            confirmation_code,
            bearer_token: None,
        };

        let host_pairing_id = pairing
            .active
            .as_ref()
            .map(|active| active.pairing_id.clone());
        pairing.requests.insert(
            request_id.clone(),
            PairingRequestRecord {
                request: pairing_request.clone(),
                client_nonce: request.client_nonce,
                host_pairing_id,
                deadline,
                recorded_at: now,
            },
        );
        pairing.request_order.push_back(request_id);
        Ok(pairing_request)
    }

    pub fn pairing_request_status(
        &self,
        request_id: &str,
    ) -> HostResult<RemotePairingRequest> {
        validate_prefixed_random_id("request", request_id)?;
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_pairing_requests_locked(&mut pairing, Instant::now());
        pairing
            .requests
            .get(request_id)
            .map(|record| record.request.clone())
            .ok_or_else(pairing_request_not_found)
    }

    pub fn pending_pairing_requests(&self) -> HostResult<Vec<RemotePairingRequest>> {
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_pairing_requests_locked(&mut pairing, Instant::now());
        Ok(pairing
            .request_order
            .iter()
            .filter_map(|request_id| pairing.requests.get(request_id))
            .filter(|record| record.request.state == PairingRequestState::Pending)
            .map(|record| record.request.clone())
            .collect())
    }

    pub fn resolve_pairing_request(
        &self,
        request_id: &str,
        accept: bool,
    ) -> HostResult<RemotePairingRequest> {
        validate_prefixed_random_id("request", request_id)?;
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        let now = Instant::now();
        self.expire_pairing_requests_locked(&mut pairing, now);
        let (client, host_pairing_id) = {
            let record = pairing
                .requests
                .get(request_id)
                .ok_or_else(pairing_request_not_found)?;
            if record.request.state != PairingRequestState::Pending {
                return Ok(record.request.clone());
            }
            (
                record.request.client.clone(),
                record.host_pairing_id.clone(),
            )
        };
        let issued = if accept {
            Some(self.credential_store.issue(client)?)
        } else {
            None
        };
        let record = pairing
            .requests
            .get_mut(request_id)
            .ok_or_else(pairing_request_not_found)?;
        if let Some(issued) = issued {
            record.request.state = PairingRequestState::Accepted;
            record.request.bearer_token = Some(issued.bearer_token);
        } else {
            record.request.state = PairingRequestState::Rejected;
        }
        record.recorded_at = now;
        let resolved = record.request.clone();
        if accept {
            if let Some(active) = pairing.active.take_if(|active| {
                host_pairing_id
                    .as_deref()
                    .is_some_and(|pairing_id| pairing_id == active.pairing_id)
            }) {
                self.record_outcome_locked(
                    &mut pairing,
                    PairingStatus {
                        pairing_id: active.pairing_id,
                        state: PairingStatusKind::Succeeded,
                        expires_at: active.expires_at,
                    },
                    now,
                );
                self.pairing_watch
                    .send_replace(PairingWatchState::unavailable());
            }
        }
        self.expire_pairing_requests_locked(&mut pairing, now);
        Ok(resolved)
    }

    pub fn pairing_status(&self, pairing_id: &str) -> HostResult<PairingStatus> {
        validate_prefixed_random_id("pairing", pairing_id)?;
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_locked(&mut pairing, Instant::now());
        if let Some(active) = pairing
            .active
            .as_ref()
            .filter(|active| active.pairing_id == pairing_id)
        {
            return Ok(PairingStatus {
                pairing_id: active.pairing_id.clone(),
                state: PairingStatusKind::Active,
                expires_at: active.expires_at,
            });
        }
        pairing
            .outcomes
            .get(pairing_id)
            .map(|record| record.status.clone())
            .ok_or_else(pairing_status_not_found)
    }

    /// Runs one synchronous operation while the matching pairing remains active.
    /// Pairing completion, cancellation, and expiry wait for the operation to finish.
    pub fn run_while_pairing_active<T, E>(
        &self,
        pairing_id: &str,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> HostResult<Result<T, E>> {
        validate_prefixed_random_id("pairing", pairing_id)?;
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_locked(&mut pairing, Instant::now());
        if pairing
            .active
            .as_ref()
            .is_some_and(|active| active.pairing_id == pairing_id)
        {
            return Ok(operation());
        }
        if let Some(record) = pairing.outcomes.get(pairing_id) {
            return Err(pairing_not_active(&record.status));
        }
        Err(pairing_status_not_found())
    }

    pub fn cancel_pairing(&self, pairing_id: &str) -> HostResult<PairingStatus> {
        validate_prefixed_random_id("pairing", pairing_id)?;
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_locked(&mut pairing, Instant::now());
        if let Some(status) = pairing
            .outcomes
            .get(pairing_id)
            .map(|record| record.status.clone())
        {
            return if status.state == PairingStatusKind::Cancelled {
                Ok(status)
            } else {
                Err(pairing_not_active(&status))
            };
        }
        let Some(active) = pairing.active.take() else {
            return Err(pairing_status_not_found());
        };
        if active.pairing_id != pairing_id {
            pairing.active = Some(active);
            return Err(pairing_status_not_found());
        }
        let status = PairingStatus {
            pairing_id: active.pairing_id,
            state: PairingStatusKind::Cancelled,
            expires_at: active.expires_at,
        };
        self.record_outcome_locked(&mut pairing, status.clone(), Instant::now());
        self.pairing_watch
            .send_replace(PairingWatchState::unavailable());
        Ok(status)
    }

    pub fn expire_pairing(&self, pairing_id: &str) -> HostResult<PairingStatus> {
        validate_prefixed_random_id("pairing", pairing_id)?;
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| pairing_session_lock_error())?;
        self.expire_locked(&mut pairing, Instant::now());
        if let Some(active) = pairing
            .active
            .as_ref()
            .filter(|active| active.pairing_id == pairing_id)
        {
            return Ok(PairingStatus {
                pairing_id: active.pairing_id.clone(),
                state: PairingStatusKind::Active,
                expires_at: active.expires_at,
            });
        }
        pairing
            .outcomes
            .get(pairing_id)
            .map(|record| record.status.clone())
            .ok_or_else(pairing_status_not_found)
    }

    fn expire_locked(&self, pairing: &mut PairingState, now: Instant) {
        self.prune_outcomes_locked(pairing, now);
        if !pairing
            .active
            .as_ref()
            .is_some_and(|active| now >= active.deadline)
        {
            return;
        }
        let Some(expired) = pairing.active.take() else {
            return;
        };
        self.record_outcome_locked(
            pairing,
            PairingStatus {
                pairing_id: expired.pairing_id,
                state: PairingStatusKind::Expired,
                expires_at: expired.expires_at,
            },
            now,
        );
        self.pairing_watch
            .send_replace(PairingWatchState::unavailable());
    }

    fn record_outcome_locked(
        &self,
        pairing: &mut PairingState,
        status: PairingStatus,
        now: Instant,
    ) {
        let pairing_id = status.pairing_id.clone();
        pairing.outcomes.insert(
            pairing_id.clone(),
            PairingOutcomeRecord {
                status,
                recorded_at: now,
            },
        );
        pairing.outcome_order.push_back(pairing_id);
        self.prune_outcomes_locked(pairing, now);
    }

    fn prune_outcomes_locked(&self, pairing: &mut PairingState, now: Instant) {
        while let Some(pairing_id) = pairing.outcome_order.front() {
            let remove = pairing.outcome_order.len() > MAX_PAIRING_STATUS_RECORDS
                || pairing
                    .outcomes
                    .get(pairing_id)
                    .map_or(true, |record| {
                        now.saturating_duration_since(record.recorded_at)
                            >= PAIRING_STATUS_RETENTION
                    });
            if !remove {
                break;
            }
            let Some(pairing_id) = pairing.outcome_order.pop_front() else {
                break;
            };
            pairing.outcomes.remove(&pairing_id);
        }
    }

    fn expire_pairing_requests_locked(&self, pairing: &mut PairingState, now: Instant) {
        for record in pairing.requests.values_mut() {
            if record.request.state == PairingRequestState::Pending && now >= record.deadline {
                record.request.state = PairingRequestState::Expired;
                record.recorded_at = now;
            }
        }

        let mut removable = pairing
            .request_order
            .iter()
            .filter(|request_id| {
                pairing.requests.get(*request_id).is_none_or(|record| {
                    record.request.state != PairingRequestState::Pending
                        && now.saturating_duration_since(record.recorded_at)
                            >= PAIRING_STATUS_RETENTION
                })
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let retained_terminal_count = pairing
            .request_order
            .iter()
            .filter(|request_id| !removable.contains(*request_id))
            .filter(|request_id| {
                pairing.requests.get(*request_id).is_some_and(|record| {
                    record.request.state != PairingRequestState::Pending
                })
            })
            .count();
        let excess_terminal_count =
            retained_terminal_count.saturating_sub(MAX_PAIRING_STATUS_RECORDS);
        let excess_terminal_ids = pairing
            .request_order
            .iter()
            .filter(|request_id| !removable.contains(*request_id))
            .filter(|request_id| {
                pairing.requests.get(*request_id).is_some_and(|record| {
                    record.request.state != PairingRequestState::Pending
                })
            })
            .take(excess_terminal_count)
            .cloned()
            .collect::<Vec<_>>();
        for request_id in excess_terminal_ids {
            removable.insert(request_id);
        }
        for request_id in &removable {
            pairing.requests.remove(request_id);
        }
        pairing.request_order.retain(|request_id| {
            !removable.contains(request_id) && pairing.requests.contains_key(request_id)
        });
    }

    pub fn validate_bearer(&self, bearer_token: &str) -> HostResult<RemoteCredential> {
        self.credential_store.validate_bearer(bearer_token)
    }

    pub fn list_credentials(&self) -> HostResult<Vec<RemoteCredential>> {
        self.credential_store.list()
    }

    pub fn update_credential_descriptor(
        &self,
        credential_id: &str,
        descriptor: DeviceDescriptor,
    ) -> HostResult<RemoteCredential> {
        self.credential_store
            .update_descriptor(credential_id, descriptor)
    }

    pub fn revoke_client(&self, client_id: &str) -> HostResult<Vec<RemoteCredential>> {
        self.credential_store.revoke_client(client_id)
    }

    pub fn revoke_credential(&self, credential_id: &str) -> HostResult<RemoteCredential> {
        self.credential_store.revoke_credential(credential_id)
    }

    pub fn revoke_current_credential(
        &self,
        bearer_token: &str,
    ) -> HostResult<RemoteCredential> {
        self.credential_store.revoke_current(bearer_token)
    }
}

fn open_tls_identity(
    path: &Path,
    clock: Clock,
) -> HostResult<(LanTlsIdentity, Vec<RemoteAccessDiagnostic>)> {
    if !path.exists() {
        let identity = generate_tls_identity(clock())?;
        persist_tls_identity(path, &identity)?;
        return Ok((identity, Vec::new()));
    }
    protect_secret_file(path)?;
    match load_tls_identity(path) {
        Ok(identity) => Ok((identity, Vec::new())),
        Err(error) => {
            let recovered_path = quarantine_corrupt_file(path, clock())?;
            let identity = generate_tls_identity(clock())?;
            persist_tls_identity(path, &identity)?;
            Ok((
                identity,
                vec![RemoteAccessDiagnostic {
                    code: "lan_tls_identity_rebuilt".to_string(),
                    message: format!(
                        "LAN TLS identity was invalid and was safely rebuilt; clients must pair again: {error}"
                    ),
                    path: path.to_path_buf(),
                    recovered_path: Some(recovered_path),
                }],
            ))
        }
    }
}

fn generate_tls_identity(created_at: TimestampMs) -> HostResult<LanTlsIdentity> {
    let mut params = CertificateParams::new(vec!["localhost".to_string()]).map_err(|error| {
        HostError::new(
            "lan_tls_identity_generation_failed",
            format!("prepare self-signed LAN TLS certificate: {error}"),
        )
    })?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "CodePet LAN Remote Access");
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    let key_pair = KeyPair::generate().map_err(|error| {
        HostError::new(
            "lan_tls_identity_generation_failed",
            format!("generate LAN TLS private key: {error}"),
        )
    })?;
    let certificate = params.self_signed(&key_pair).map_err(|error| {
        HostError::new(
            "lan_tls_identity_generation_failed",
            format!("sign LAN TLS certificate: {error}"),
        )
    })?;
    let certificate_der = certificate.der().to_vec();
    let private_key_der = key_pair.serialize_der();
    Ok(LanTlsIdentity {
        certificate_fingerprint: sha256_hex(&certificate_der),
        certificate_der,
        private_key_der,
        created_at,
    })
}

fn load_tls_identity(path: &Path) -> HostResult<LanTlsIdentity> {
    let payload = fs::read(path)
        .map_err(|error| persistence_io("read LAN TLS identity", path, error))?;
    if payload.len() > MAX_TLS_IDENTITY_FILE_BYTES {
        return Err(HostError::new(
            "invalid_lan_tls_identity",
            "LAN TLS identity exceeds the persisted size limit",
        ));
    }
    let document: LanTlsIdentityDocument = serde_json::from_slice(&payload).map_err(|error| {
        HostError::new(
            "invalid_lan_tls_identity",
            format!("decode LAN TLS identity {}: {error}", path.display()),
        )
        .with_detail("path", path.display().to_string())
    })?;
    if document.version != LAN_TLS_IDENTITY_VERSION {
        return Err(HostError::new(
            "unsupported_lan_tls_identity_version",
            format!(
                "unsupported LAN TLS identity version: {}",
                document.version
            ),
        ));
    }
    validate_sha256_hex("certificateSha256", &document.certificate_sha256)?;
    validate_sha256_hex("privateKeySha256", &document.private_key_sha256)?;
    let certificate_der = decode_hex(&document.certificate_der).map_err(|message| {
        HostError::new("invalid_lan_tls_identity", message)
            .with_detail("field", "certificateDer")
    })?;
    let private_key_der = decode_hex(&document.private_key_der).map_err(|message| {
        HostError::new("invalid_lan_tls_identity", message)
            .with_detail("field", "privateKeyDer")
    })?;
    if certificate_der.is_empty() || private_key_der.is_empty() {
        return Err(HostError::new(
            "invalid_lan_tls_identity",
            "LAN TLS certificate and private key DER must not be empty",
        ));
    }
    if sha256_hex(&certificate_der) != document.certificate_sha256
        || sha256_hex(&private_key_der) != document.private_key_sha256
    {
        return Err(HostError::new(
            "invalid_lan_tls_identity",
            "LAN TLS identity checksum validation failed",
        ));
    }
    validate_certificate_and_key(&certificate_der, &private_key_der)?;
    Ok(LanTlsIdentity {
        certificate_der,
        private_key_der,
        certificate_fingerprint: document.certificate_sha256,
        created_at: document.created_at,
    })
}

fn persist_tls_identity(path: &Path, identity: &LanTlsIdentity) -> HostResult<()> {
    let document = LanTlsIdentityDocument {
        version: LAN_TLS_IDENTITY_VERSION,
        certificate_der: encode_hex(&identity.certificate_der),
        private_key_der: encode_hex(&identity.private_key_der),
        certificate_sha256: identity.certificate_fingerprint.clone(),
        private_key_sha256: sha256_hex(&identity.private_key_der),
        created_at: identity.created_at,
    };
    write_secret_json_atomically(path, &document)
}

fn validate_certificate_and_key(certificate_der: &[u8], private_key_der: &[u8]) -> HostResult<()> {
    let key_pair = KeyPair::try_from(private_key_der).map_err(|error| {
        HostError::new(
            "invalid_lan_tls_identity",
            format!("parse LAN TLS private key: {error}"),
        )
    })?;
    let (remaining, certificate) = parse_x509_certificate(certificate_der).map_err(|error| {
        HostError::new(
            "invalid_lan_tls_identity",
            format!("parse LAN TLS certificate: {error}"),
        )
    })?;
    if !remaining.is_empty() {
        return Err(HostError::new(
            "invalid_lan_tls_identity",
            "LAN TLS certificate contains trailing DER data",
        ));
    }
    certificate
        .verify_signature(Some(certificate.public_key()))
        .map_err(|error| {
            HostError::new(
                "invalid_lan_tls_identity",
                format!("verify self-signed LAN TLS certificate: {error}"),
            )
        })?;
    if certificate.public_key().subject_public_key.data.as_ref() != key_pair.public_key_raw() {
        return Err(HostError::new(
            "invalid_lan_tls_identity",
            "LAN TLS certificate does not match the persisted private key",
        ));
    }
    Ok(())
}

fn load_credentials(
    path: &Path,
    expected_tls_fingerprint: &str,
) -> HostResult<BTreeMap<String, StoredRemoteCredential>> {
    let payload = fs::read(path)
        .map_err(|error| persistence_io("read remote credential store", path, error))?;
    if payload.len() > MAX_CREDENTIAL_STORE_FILE_BYTES {
        return Err(HostError::new(
            "invalid_remote_credential_store",
            "remote credential store exceeds the persisted size limit",
        ));
    }
    let document: RemoteCredentialStoreDocument =
        serde_json::from_slice(&payload).map_err(|error| {
            HostError::new(
                "invalid_remote_credential_store",
                format!("decode remote credential store {}: {error}", path.display()),
            )
            .with_detail("path", path.display().to_string())
        })?;
    if document.version != REMOTE_CREDENTIAL_STORE_VERSION {
        return Err(HostError::new(
            "unsupported_remote_credential_store_version",
            format!(
                "unsupported remote credential store version: {}",
                document.version
            ),
        ));
    }
    validate_sha256_hex(
        "tlsCertificateSha256",
        &document.tls_certificate_sha256,
    )?;
    if document.tls_certificate_sha256 != expected_tls_fingerprint {
        return Err(HostError::new(
            "remote_credential_tls_identity_mismatch",
            "remote credentials belong to a different LAN TLS identity",
        ));
    }
    if document.credentials.len() > MAX_REMOTE_CREDENTIALS {
        return Err(HostError::new(
            "invalid_remote_credential_store",
            "remote credential store contains too many credentials",
        ));
    }
    let mut credentials = BTreeMap::new();
    let mut bearer_hashes = BTreeSet::new();
    for credential in document.credentials {
        credential.validate()?;
        if !bearer_hashes.insert(credential.bearer_sha256.clone()) {
            return Err(HostError::new(
                "duplicate_remote_credential",
                "remote credential store contains duplicate bearer hashes",
            ));
        }
        if credentials
            .insert(credential.credential_id.clone(), credential)
            .is_some()
        {
            return Err(HostError::new(
                "duplicate_remote_credential",
                "remote credential store contains duplicate credential ids",
            ));
        }
    }
    Ok(credentials)
}

fn persist_credentials(
    path: &Path,
    tls_certificate_fingerprint: &str,
    credentials: &BTreeMap<String, StoredRemoteCredential>,
) -> HostResult<()> {
    let document = RemoteCredentialStoreDocument {
        version: REMOTE_CREDENTIAL_STORE_VERSION,
        tls_certificate_sha256: tls_certificate_fingerprint.to_string(),
        credentials: credentials.values().cloned().collect(),
    };
    write_secret_json_atomically(path, &document)
}

fn quarantine_corrupt_file(path: &Path, timestamp: TimestampMs) -> HostResult<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        HostError::new(
            "invalid_persistence_path",
            format!("remote access persistence path has no parent: {}", path.display()),
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("remote-access-data");
    let recovered_path = parent.join(format!("{file_name}.corrupt-{timestamp}"));
    fs::rename(path, &recovered_path)
        .map_err(|error| persistence_io("quarantine corrupt remote access data", path, error))?;
    Ok(recovered_path)
}

fn validate_text_field(field: &str, value: &str, maximum_length: usize) -> HostResult<()> {
    if value.trim().is_empty() || value.len() > maximum_length {
        return Err(HostError::new(
            "invalid_remote_client_identity",
            format!(
                "remote client {field} must contain 1 to {maximum_length} bytes"
            ),
        )
        .with_detail("field", field.to_string()));
    }
    Ok(())
}

fn validate_device_descriptor(descriptor: &DeviceDescriptor) -> HostResult<()> {
    validate_text_field(
        "deviceName",
        &descriptor.device_name,
        MAX_DEVICE_NAME_LENGTH,
    )?;
    validate_text_field(
        "operatingSystem",
        &descriptor.operating_system,
        MAX_OPERATING_SYSTEM_LENGTH,
    )?;
    validate_text_field(
        "systemVersion",
        &descriptor.system_version,
        MAX_SYSTEM_VERSION_LENGTH,
    )
}

fn validate_prefixed_random_id(prefix: &str, value: &str) -> HostResult<()> {
    let expected_prefix = format!("{prefix}-");
    let suffix = value.strip_prefix(&expected_prefix).unwrap_or_default();
    if !is_canonical_hex(suffix, RANDOM_ID_BYTES * 2) {
        return Err(HostError::new(
            "invalid_remote_access_id",
            format!("remote access id must use the canonical {prefix} wire format"),
        ));
    }
    Ok(())
}

fn validate_bearer_token(bearer_token: &str) -> HostResult<()> {
    if !is_canonical_hex(bearer_token, RANDOM_SECRET_BYTES * 2) {
        return Err(invalid_remote_credential());
    }
    Ok(())
}

fn validate_sha256_hex(field: &str, value: &str) -> HostResult<()> {
    if !is_canonical_hex(value, SHA256_HEX_LENGTH) {
        return Err(HostError::new(
            "invalid_remote_access_hash",
            format!("remote access {field} must be 64 lowercase hexadecimal characters"),
        )
        .with_detail("field", field.to_string()));
    }
    Ok(())
}

fn random_prefixed_id(prefix: &str) -> HostResult<String> {
    Ok(format!("{prefix}-{}", random_hex::<RANDOM_ID_BYTES>()?))
}

fn random_hex<const LENGTH: usize>() -> HostResult<String> {
    let mut bytes = [0_u8; LENGTH];
    SystemRandom::new().fill(&mut bytes).map_err(|_| {
        HostError::new(
            "secure_random_unavailable",
            "read operating-system secure random source",
        )
        .retryable(true)
    })?;
    Ok(encode_hex(&bytes))
}

fn sha256_bytes(value: &[u8]) -> [u8; 32] {
    ring::digest::digest(&ring::digest::SHA256, value)
        .as_ref()
        .try_into()
        .expect("SHA-256 always produces 32 bytes")
}

fn sha256_hex(value: &[u8]) -> String {
    encode_hex(&sha256_bytes(value))
}

fn pairing_confirmation_code(
    request_id: &str,
    client_nonce: &str,
    certificate_fingerprint: &str,
) -> String {
    let digest = sha256_bytes(
        format!("{request_id}\0{client_nonce}\0{certificate_fingerprint}").as_bytes(),
    );
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{value:06}")
}

fn decode_sha256_hex(value: &str) -> Result<[u8; 32], String> {
    let decoded = decode_hex(value)?;
    decoded
        .try_into()
        .map_err(|_| "SHA-256 value must contain exactly 32 bytes".to_string())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.as_bytes().chunks_exact(2).remainder().is_empty()
        || !value.as_bytes().iter().all(|byte| is_lower_hex(*byte))
    {
        return Err("hex value must contain an even number of lowercase hexadecimal characters"
            .to_string());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0]).ok_or_else(|| "invalid hexadecimal value".to_string())?;
            let low = hex_nibble(pair[1]).ok_or_else(|| "invalid hexadecimal value".to_string())?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn is_canonical_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length && value.as_bytes().iter().all(|byte| is_lower_hex(*byte))
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    bool::from(left.ct_eq(right))
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn now_ms() -> TimestampMs {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn credential_store_lock_error() -> HostError {
    HostError::new(
        "remote_credential_store_unavailable",
        "remote credential store lock is unavailable",
    )
    .retryable(true)
}

fn pairing_session_lock_error() -> HostError {
    HostError::new(
        "pairing_session_unavailable",
        "remote pairing session lock is unavailable",
    )
    .retryable(true)
}

fn pairing_status_not_found() -> HostError {
    HostError::new(
        "pairing_session_not_found",
        "Remote pairing status is no longer available in this Host process",
    )
}

fn pairing_request_not_found() -> HostError {
    HostError::new(
        "pairing_request_not_found",
        "Remote pairing request is no longer available in this Host process",
    )
}

fn pairing_not_active(status: &PairingStatus) -> HostError {
    HostError::new(
        "pairing_session_not_active",
        "Remote pairing session is no longer active",
    )
    .with_detail("state", format!("{:?}", status.state).to_ascii_lowercase())
}

fn invalid_pairing_session() -> HostError {
    HostError::new(
        "invalid_pairing_session",
        "remote pairing id or secret is invalid",
    )
}

fn invalid_remote_credential() -> HostError {
    HostError::new(
        "invalid_remote_credential",
        "remote bearer credential is invalid or revoked",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestClock {
        value: Arc<AtomicU64>,
    }

    impl TestClock {
        fn new(value: TimestampMs) -> Self {
            Self {
                value: Arc::new(AtomicU64::new(value)),
            }
        }

        fn clock(&self) -> Clock {
            let value = self.value.clone();
            Arc::new(move || value.load(Ordering::SeqCst))
        }

        fn set(&self, value: TimestampMs) {
            self.value.store(value, Ordering::SeqCst);
        }
    }

    fn open_manager(
        directory: &Path,
        clock: &TestClock,
    ) -> (RemoteAccessConfig, Arc<DeviceRegistry>, RemoteAccessManager) {
        let device = Arc::new(
            DeviceRegistry::open(directory.join("device.json"), "Remote Test Device").unwrap(),
        );
        let config = RemoteAccessConfig::for_data_directory(directory.join("remote-access"));
        let manager = RemoteAccessManager::open_with_clock(
            config.clone(),
            device.clone(),
            device_descriptor("Remote Test Device", "TestOS", "1.0"),
            clock.clock(),
        )
        .unwrap();
        (config, device, manager)
    }

    fn device_descriptor(
        device_name: &str,
        operating_system: &str,
        system_version: &str,
    ) -> DeviceDescriptor {
        DeviceDescriptor {
            device_name: device_name.to_string(),
            operating_system: operating_system.to_string(),
            system_version: system_version.to_string(),
        }
    }

    fn paired_device_descriptor(client_id: &str) -> DeviceDescriptor {
        device_descriptor(&format!("Client {client_id}"), "TestOS", "1.0")
    }

    fn pair(
        manager: &RemoteAccessManager,
        client_id: &str,
    ) -> IssuedRemoteCredential {
        let session = manager.begin_pairing().unwrap();
        assert!(is_canonical_hex(
            &session.pairing_secret,
            RANDOM_SECRET_BYTES * 2
        ));
        let pairing_id = session.pairing_id;
        manager
            .complete_pairing(&pairing_id, PairingExchangeRequest {
                pairing_secret: session.pairing_secret,
                client_id: client_id.to_string(),
                device: paired_device_descriptor(client_id),
            })
            .unwrap()
    }

    fn pairing_request(
        manager: &RemoteAccessManager,
        client_id: &str,
        nonce: char,
    ) -> PairingRequestCreateRequest {
        PairingRequestCreateRequest {
            host_device_id: manager.device_registry().identity().device_id.clone(),
            client_id: client_id.to_string(),
            device: paired_device_descriptor(client_id),
            client_nonce: nonce.to_string().repeat(SHA256_HEX_LENGTH),
        }
    }

    #[test]
    fn incoming_pairing_request_requires_explicit_resolution_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(1_000);
        let (_, _, manager) = open_manager(directory.path(), &clock);
        let request = pairing_request(&manager, "client-request", 'a');

        let pending = manager.create_pairing_request(request.clone()).unwrap();
        assert_eq!(pending.state, PairingRequestState::Pending);
        assert_eq!(pending.confirmation_code.len(), 6);
        assert!(pending
            .confirmation_code
            .bytes()
            .all(|byte| byte.is_ascii_digit()));
        assert!(pending.bearer_token().is_none());
        assert_eq!(
            manager.create_pairing_request(request).unwrap().request_id,
            pending.request_id
        );
        assert_eq!(manager.pending_pairing_requests().unwrap().len(), 1);

        let accepted = manager
            .resolve_pairing_request(&pending.request_id, true)
            .unwrap();
        assert_eq!(accepted.state, PairingRequestState::Accepted);
        assert!(manager
            .validate_bearer(accepted.bearer_token().unwrap())
            .is_ok());
        assert!(manager.pending_pairing_requests().unwrap().is_empty());
        assert_eq!(
            manager
                .pairing_request_status(&pending.request_id)
                .unwrap()
                .bearer_token(),
            accepted.bearer_token()
        );

        let rejected = manager
            .create_pairing_request(pairing_request(&manager, "client-rejected", 'b'))
            .unwrap();
        let rejected = manager
            .resolve_pairing_request(&rejected.request_id, false)
            .unwrap();
        assert_eq!(rejected.state, PairingRequestState::Rejected);
        assert!(rejected.bearer_token().is_none());
    }

    #[test]
    fn pairing_confirmation_code_matches_the_cross_language_vector() {
        assert_eq!(
            pairing_confirmation_code(
                "request-0123456789abcdef",
                &"1234567890abcdef".repeat(4),
                &"0123456789abcdef".repeat(4),
            ),
            "777533"
        );
    }

    #[test]
    fn host_initiated_pairing_completes_after_matching_code_is_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(2_000);
        let (_, _, manager) = open_manager(directory.path(), &clock);
        let pairing = manager.begin_pairing().unwrap();

        let pending = manager
            .create_pairing_request(pairing_request(&manager, "client-host-invite", 'c'))
            .unwrap();

        assert_eq!(pending.state, PairingRequestState::Pending);
        assert!(manager.subscribe_pairing_state().borrow().pairing_available);
        let accepted = manager
            .resolve_pairing_request(&pending.request_id, true)
            .unwrap();
        assert_eq!(accepted.state, PairingRequestState::Accepted);
        assert!(manager
            .validate_bearer(accepted.bearer_token().unwrap())
            .is_ok());
        assert_eq!(
            manager.pairing_status(&pairing.pairing_id).unwrap().state,
            PairingStatusKind::Succeeded
        );
        assert!(!manager.subscribe_pairing_state().borrow().pairing_available);
    }

    #[test]
    fn pairing_request_retention_never_evicts_a_pending_request() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(3_000);
        let (_, _, manager) = open_manager(directory.path(), &clock);
        let pending = manager
            .create_pairing_request(pairing_request(&manager, "client-pending", 'd'))
            .unwrap();

        for index in 0..(MAX_PAIRING_STATUS_RECORDS + 8) {
            let resolved = manager
                .create_pairing_request(pairing_request(
                    &manager,
                    &format!("client-resolved-{index}"),
                    'e',
                ))
                .unwrap();
            manager
                .resolve_pairing_request(&resolved.request_id, false)
                .unwrap();
        }

        assert_eq!(
            manager
                .pairing_request_status(&pending.request_id)
                .unwrap()
                .state,
            PairingRequestState::Pending
        );
        assert_eq!(manager.pending_pairing_requests().unwrap().len(), 1);
        let pairing = manager.pairing.lock().unwrap();
        assert!(pairing.requests.len() <= MAX_PAIRING_STATUS_RECORDS + 1);
    }

    #[test]
    fn tls_identity_persists_and_rotation_requires_pairing_again() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(10_000);
        let (config, device, manager) = open_manager(directory.path(), &clock);
        let fingerprint = manager
            .tls_identity()
            .certificate_fingerprint()
            .to_string();
        let certificate = manager.tls_identity().certificate_der().to_vec();
        let private_key = manager.tls_identity().private_key_der().to_vec();
        assert!(is_canonical_hex(&fingerprint, SHA256_HEX_LENGTH));
        let independent_fingerprint = ring::digest::digest(
            &ring::digest::SHA256,
            &certificate,
        )
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
        assert_eq!(fingerprint, independent_fingerprint);
        assert_eq!(manager.device_registry().identity(), device.identity());
        let device_id = device.identity().device_id.clone();
        let issued = pair(&manager, "client-persisted");
        drop(manager);

        let renamed_device = Arc::new(
            DeviceRegistry::open(
                directory.path().join("device.json"),
                "Renamed Remote Test Device",
            )
            .unwrap(),
        );
        assert_eq!(renamed_device.identity().device_id, device_id);
        let renamed_descriptor =
            device_descriptor("Renamed Remote Test Device", "TestOS", "1.0");
        let reopened = RemoteAccessManager::open_with_clock(
            config.clone(),
            renamed_device.clone(),
            renamed_descriptor.clone(),
            clock.clock(),
        )
        .unwrap();
        assert_eq!(
            reopened.tls_identity().certificate_fingerprint(),
            fingerprint
        );
        assert_eq!(reopened.tls_identity().certificate_der(), certificate);
        assert_eq!(reopened.tls_identity().private_key_der(), private_key);
        let renamed_identity = reopened.remote_host_identity();
        assert_eq!(renamed_identity.device_id, device_id);
        assert_eq!(renamed_identity.descriptor, renamed_descriptor);
        assert_eq!(renamed_identity.identity_fingerprint, fingerprint);
        assert!(reopened.validate_bearer(&issued.bearer_token).is_ok());
        drop(reopened);

        let mismatched_identity = generate_tls_identity(15_000).unwrap();
        let mut mismatched_document: serde_json::Value = serde_json::from_slice(
            &fs::read(&config.tls_identity_path).unwrap(),
        )
        .unwrap();
        mismatched_document["privateKeyDer"] =
            serde_json::json!(encode_hex(mismatched_identity.private_key_der()));
        mismatched_document["privateKeySha256"] =
            serde_json::json!(sha256_hex(mismatched_identity.private_key_der()));
        fs::write(
            &config.tls_identity_path,
            serde_json::to_vec_pretty(&mismatched_document).unwrap(),
        )
        .unwrap();
        assert_eq!(
            load_tls_identity(&config.tls_identity_path)
                .unwrap_err()
                .code,
            "invalid_lan_tls_identity"
        );
        clock.set(20_000);
        let recovered = RemoteAccessManager::open_with_clock(
            config.clone(),
            renamed_device,
            renamed_descriptor,
            clock.clock(),
        )
        .unwrap();
        assert_ne!(
            recovered.tls_identity().certificate_fingerprint(),
            fingerprint
        );
        assert_eq!(
            recovered
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            vec!["lan_tls_identity_rebuilt", "remote_credentials_rebuilt"]
        );
        assert_eq!(
            recovered
                .validate_bearer(&issued.bearer_token)
                .unwrap_err()
                .code,
            "invalid_remote_credential"
        );
        assert!(recovered
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic
                .recovered_path
                .as_ref()
                .is_some_and(|path| path.exists())));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&config.tls_identity_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&config.credential_store_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn tls_identity_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(1_000);
        let device = Arc::new(
            DeviceRegistry::open(directory.path().join("device.json"), "Symlink Test")
                .unwrap(),
        );
        let config = RemoteAccessConfig::for_data_directory(
            directory.path().join("remote-access"),
        );
        fs::create_dir_all(config.tls_identity_path.parent().unwrap()).unwrap();
        let target = directory.path().join("tls-target.json");
        fs::write(&target, b"{}").unwrap();
        symlink(&target, &config.tls_identity_path).unwrap();

        let error = RemoteAccessManager::open_with_clock(
            config,
            device,
            device_descriptor("Symlink Test", "TestOS", "1.0"),
            clock.clock(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "invalid_secret_file_type");
    }

    #[test]
    fn pairing_uses_monotonic_deadline_and_secret_is_not_consumed_on_error() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(1_000);
        let (config, device, manager) = open_manager(directory.path(), &clock);
        let manager = Arc::new(manager);
        let session = manager.begin_pairing().unwrap();
        assert_eq!(
            session.expires_at,
            1_000 + duration_ms(PAIRING_SESSION_TTL)
        );
        let pairing_id = session.pairing_id.clone();
        let request = PairingExchangeRequest {
            pairing_secret: session.pairing_secret.clone(),
            client_id: "client-once".to_string(),
            device: device_descriptor("Once", "iOS", "18.0"),
        };
        clock.set(u64::MAX);
        let mut wrong_request = request.clone();
        let replacement = if wrong_request.pairing_secret.starts_with('0') {
            "1"
        } else {
            "0"
        };
        wrong_request.pairing_secret.replace_range(0..1, replacement);
        assert_eq!(
            manager
                .complete_pairing(&pairing_id, wrong_request)
                .unwrap_err()
                .code,
            "invalid_pairing_session"
        );
        let mut invalid_descriptor_request = request.clone();
        invalid_descriptor_request.device.system_version.clear();
        assert_eq!(
            manager
                .complete_pairing(&pairing_id, invalid_descriptor_request)
                .unwrap_err()
                .code,
            "invalid_remote_client_identity"
        );
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let attempts = (0..2)
            .map(|_| {
                let manager = manager.clone();
                let barrier = barrier.clone();
                let pairing_id = pairing_id.clone();
                let request = request.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    manager.complete_pairing(&pairing_id, request)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let outcomes = attempts
            .into_iter()
            .map(|attempt| attempt.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter_map(|outcome| outcome.as_ref().err())
                .map(|error| error.code.as_str())
                .collect::<Vec<_>>(),
            vec!["invalid_pairing_session"]
        );

        clock.set(10_000);
        let expired = manager
            .begin_pairing_with_ttl(Duration::ZERO)
            .unwrap();
        assert_eq!(expired.expires_at, 10_000);
        let expired_pairing_id = expired.pairing_id;
        assert_eq!(
            manager
                .complete_pairing(&expired_pairing_id, PairingExchangeRequest {
                    pairing_secret: expired.pairing_secret,
                    client_id: "client-expired".to_string(),
                    device: device_descriptor("Expired", "Android", "15"),
                })
                .unwrap_err()
                .code,
            "pairing_session_expired"
        );

        clock.set(500_000);
        let lost_on_restart = manager.begin_pairing().unwrap();
        drop(manager);
        let restarted = RemoteAccessManager::open_with_clock(
            config,
            device,
            device_descriptor("Remote Test Device", "TestOS", "1.0"),
            clock.clock(),
        )
        .unwrap();
        let lost_pairing_id = lost_on_restart.pairing_id;
        assert_eq!(
            restarted
                .complete_pairing(&lost_pairing_id, PairingExchangeRequest {
                    pairing_secret: lost_on_restart.pairing_secret,
                    client_id: "client-restart".to_string(),
                    device: device_descriptor("Restart", "Android", "15"),
                })
                .unwrap_err()
                .code,
            "invalid_pairing_session"
        );
        assert_eq!(
            restarted
                .pairing_status(&lost_pairing_id)
                .unwrap_err()
                .code,
            "pairing_session_not_found"
        );
    }

    #[test]
    fn remote_device_descriptor_persists_refreshes_and_rejects_v1_store() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(1_000);
        let (config, device, manager) = open_manager(directory.path(), &clock);
        let issued = pair(&manager, "descriptor-client");
        assert_eq!(
            issued.credential.descriptor,
            paired_device_descriptor("descriptor-client")
        );

        let refreshed = device_descriptor("Renamed Device", "TestOS", "2.0");
        assert_eq!(
            manager
                .update_credential_descriptor(
                    &issued.credential.credential_id,
                    refreshed.clone(),
                )
                .unwrap()
                .descriptor,
            refreshed
        );
        let persisted_after_refresh =
            fs::read_to_string(&config.credential_store_path).unwrap();
        manager
            .update_credential_descriptor(
                &issued.credential.credential_id,
                refreshed.clone(),
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(&config.credential_store_path).unwrap(),
            persisted_after_refresh
        );
        drop(manager);

        let reopened = RemoteAccessManager::open_with_clock(
            config.clone(),
            device.clone(),
            device_descriptor("Remote Test Device", "TestOS", "1.0"),
            clock.clock(),
        )
        .unwrap();
        assert_eq!(
            reopened.list_credentials().unwrap()[0].descriptor,
            refreshed
        );
        drop(reopened);

        let mut legacy_document: serde_json::Value = serde_json::from_slice(
            &fs::read(&config.credential_store_path).unwrap(),
        )
        .unwrap();
        legacy_document["version"] = serde_json::json!(1);
        fs::write(
            &config.credential_store_path,
            serde_json::to_vec_pretty(&legacy_document).unwrap(),
        )
        .unwrap();
        let reset = RemoteAccessManager::open_with_clock(
            config,
            device,
            device_descriptor("Remote Test Device", "TestOS", "1.0"),
            clock.clock(),
        )
        .unwrap();
        assert!(reset.list_credentials().unwrap().is_empty());
        assert_eq!(
            reset.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            vec!["remote_credentials_rebuilt"]
        );
    }

    #[test]
    fn pairing_status_watch_and_single_active_session_are_process_local() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(1_000);
        let (_config, _device, manager) = open_manager(directory.path(), &clock);
        let mut pairing_watch = manager.subscribe_pairing_state();

        let cancelled_session = manager.begin_pairing().unwrap();
        assert!(pairing_watch.borrow().pairing_available);
        assert_eq!(
            pairing_watch.borrow().pairing_id.as_deref(),
            Some(cancelled_session.pairing_id.as_str())
        );
        assert_eq!(
            manager.begin_pairing().unwrap_err().code,
            "pairing_session_active"
        );
        let cancelled = manager
            .cancel_pairing(&cancelled_session.pairing_id)
            .unwrap();
        assert_eq!(cancelled.state, PairingStatusKind::Cancelled);
        assert!(!pairing_watch.borrow_and_update().pairing_available);
        assert_eq!(
            manager
                .cancel_pairing(&cancelled_session.pairing_id)
                .unwrap(),
            cancelled
        );

        let newer_session = manager.begin_pairing().unwrap();
        assert_eq!(
            manager
                .cancel_pairing(&cancelled_session.pairing_id)
                .unwrap(),
            cancelled
        );
        assert_eq!(
            manager.pairing_status(&newer_session.pairing_id).unwrap().state,
            PairingStatusKind::Active
        );
        assert!(pairing_watch.borrow().pairing_available);
        assert_eq!(
            pairing_watch.borrow().pairing_id.as_deref(),
            Some(newer_session.pairing_id.as_str())
        );
        manager.cancel_pairing(&newer_session.pairing_id).unwrap();

        let expired_session = manager
            .begin_pairing_with_ttl(Duration::ZERO)
            .unwrap();
        let expired = manager
            .expire_pairing(&expired_session.pairing_id)
            .unwrap();
        assert_eq!(expired.state, PairingStatusKind::Expired);
        assert_eq!(
            manager
                .run_while_pairing_active(&expired_session.pairing_id, || Ok::<(), ()>(()))
                .unwrap_err()
                .code,
            "pairing_session_not_active"
        );
        assert!(!pairing_watch.borrow().pairing_available);

        let succeeded_session = manager.begin_pairing().unwrap();
        let succeeded_id = succeeded_session.pairing_id.clone();
        manager
            .complete_pairing(
                &succeeded_id,
                PairingExchangeRequest {
                    pairing_secret: succeeded_session.pairing_secret,
                    client_id: "status-client".to_string(),
                    device: device_descriptor("Status Client", "TestOS", "1.0"),
                },
            )
            .unwrap();
        assert_eq!(
            manager.pairing_status(&succeeded_id).unwrap().state,
            PairingStatusKind::Succeeded
        );
        assert!(!pairing_watch.borrow().pairing_available);
    }

    #[test]
    fn bearer_hash_validation_and_both_revocation_paths_persist() {
        let directory = tempfile::tempdir().unwrap();
        let clock = TestClock::new(1_000);
        let (config, _device, manager) = open_manager(directory.path(), &clock);
        let current = pair(&manager, "client-current");
        assert!(is_canonical_hex(
            &current.bearer_token,
            RANDOM_SECRET_BYTES * 2
        ));
        let persisted = fs::read_to_string(&config.credential_store_path).unwrap();
        assert!(!persisted.contains(&current.bearer_token));
        assert!(persisted.contains(&sha256_hex(current.bearer_token.as_bytes())));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&persisted).unwrap()["version"],
            REMOTE_CREDENTIAL_STORE_VERSION
        );

        clock.set(2_000);
        let validated = manager.validate_bearer(&current.bearer_token).unwrap();
        assert_eq!(validated.last_seen_at, 2_000);
        let revoked = manager
            .revoke_current_credential(&current.bearer_token)
            .unwrap();
        assert_eq!(revoked.revoked_at, Some(2_000));
        assert_eq!(
            manager
                .revoke_credential(&current.credential.credential_id)
                .unwrap()
                .revoked_at,
            Some(2_000)
        );
        assert_eq!(
            manager
                .validate_bearer(&current.bearer_token)
                .unwrap_err()
                .code,
            "invalid_remote_credential"
        );

        let client_a_first = pair(&manager, "client-a");
        let client_a_second = pair(&manager, "client-a");
        let client_b = pair(&manager, "client-b");
        clock.set(3_000);
        let revoked_client = manager.revoke_client("client-a").unwrap();
        assert_eq!(revoked_client.len(), 2);
        assert_eq!(
            manager
                .validate_bearer(&client_a_first.bearer_token)
                .unwrap_err()
                .code,
            "invalid_remote_credential"
        );
        assert_eq!(
            manager
                .validate_bearer(&client_a_second.bearer_token)
                .unwrap_err()
                .code,
            "invalid_remote_credential"
        );
        assert!(manager.validate_bearer(&client_b.bearer_token).is_ok());

        drop(manager);
        let store = RemoteCredentialStore::open(
            config.credential_store_path,
            sha256_hex(
                &load_tls_identity(&config.tls_identity_path)
                    .unwrap()
                    .certificate_der,
            ),
        )
        .unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 4);
        let persisted_current = listed
            .iter()
            .find(|credential| {
                credential.credential_id == current.credential.credential_id
            })
            .unwrap();
        assert_eq!(persisted_current.client_id, "client-current");
        assert_eq!(
            persisted_current.descriptor,
            paired_device_descriptor("client-current")
        );
        assert_eq!(persisted_current.last_seen_at, 2_000);
        assert_eq!(persisted_current.revoked_at, Some(2_000));
        assert_eq!(
            listed
                .iter()
                .filter(|credential| credential.revoked_at.is_some())
                .count(),
            3
        );
    }
}
