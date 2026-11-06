//! Public rendezvous authenticated by QR invitations or existing pinned LAN pairing.
mod invitation;
mod registration;
mod service;
pub use service::SignalingServiceController;
use super::super::session::SessionRegistry;
use super::path_policy::PathPolicy;
use crate::{HostError, HostResult, ProviderGatewayService, RemoteAccessManager, RemoteCredential};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
    signature::{self, KeyPair},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use webrtc::{
    ice_transport::ice_server::RTCIceServer,
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
    },
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    service_url: String,
    host_token: String,
    seed: String,
    #[serde(default)]
    peers: BTreeMap<String, Peer>,
    #[serde(default)]
    auto_register: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Peer {
    public_key: String,
    token: String,
}

pub(crate) struct Cloud {
    invitations: Mutex<BTreeMap<String, invitation::Invitation>>,
    path: PathBuf,
    config: Mutex<Config>,
    key: signature::Ed25519KeyPair,
    access: Arc<RemoteAccessManager>,
    http: reqwest::Client,
    sync_lock: tokio::sync::Mutex<()>,
    registered: tokio::sync::Mutex<bool>,
}

fn error() -> HostError {
    HostError::new(
        "rtc_cloud_unavailable",
        "RTC cloud configuration or authorization failed",
    )
    .retryable(true)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    digest::digest(&digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Cloud {
    pub(crate) fn load(access: Arc<RemoteAccessManager>) -> HostResult<Option<Arc<Self>>> {
        let result = Self::load_config(access);
        let fields = match &result {
            Ok(Some(_)) => json!({"state":"configured"}),
            Ok(None) => json!({"state":"notConfigured"}),
            Err(failure) => json!({"state":"unavailable","code":failure.code}),
        };
        super::diagnostics::Diagnostic::new("").emit("signal.configuration", fields);
        result
    }

    fn load_config(access: Arc<RemoteAccessManager>) -> HostResult<Option<Arc<Self>>> {
        let Some((path, config)) = registration::load_or_create_config(&access)? else { return Ok(None); };
        let url = url::Url::parse(&config.service_url).map_err(|_| error())?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || config.host_token.len() < 32
        {
            return Err(error());
        }
        let key = signature::Ed25519KeyPair::from_seed_unchecked(
            &B64.decode(&config.seed).map_err(|_| error())?,
        )
        .map_err(|_| error())?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| error())?;
        Ok(Some(Arc::new(Self {
            path,
            invitations: Mutex::new(BTreeMap::new()),
            config: Mutex::new(config),
            key,
            access,
            http,
            sync_lock: tokio::sync::Mutex::new(()),
            registered: tokio::sync::Mutex::new(false),
        })))
    }

    fn active(&self, id: &str) -> HostResult<RemoteCredential> {
        self.access
            .list_credentials()?
            .into_iter()
            .find(|c| c.credential_id == id && c.revoked_at.is_none())
            .ok_or_else(error)
    }

    pub(crate) async fn bootstrap(
        &self,
        credential: &RemoteCredential,
        public_key: &str,
    ) -> HostResult<Value> {
        let result = self.bind_peer(credential, public_key)?;
        self.sync_clients().await?;
        self.active(&credential.credential_id)?;
        Ok(result)
    }

    fn bind_peer(&self, credential: &RemoteCredential, public_key: &str) -> HostResult<Value> {
        if B64.decode(public_key).map_err(|_| error())?.len() != 32 {
            return Err(error());
        }
        self.active(&credential.credential_id)?;
        let result = {
            let mut config = self.config.lock().map_err(|_| error())?;
            if let Some(peer) = config.peers.get(&credential.credential_id) {
                if peer.public_key != public_key {
                    return Err(error());
                }
            } else {
                if config.peers.len() >= 32 {
                    return Err(error());
                }
                let mut token = [0u8; 32];
                SystemRandom::new().fill(&mut token).map_err(|_| error())?;
                config.peers.insert(
                    credential.credential_id.clone(),
                    Peer {
                        public_key: public_key.to_owned(),
                        token: B64.encode(token),
                    },
                );
                let mut file =
                    tempfile::NamedTempFile::new_in(self.path.parent().ok_or_else(error)?)
                        .map_err(|_| error())?;
                file.write_all(&serde_json::to_vec(&*config).map_err(|_| error())?)
                    .map_err(|_| error())?;
                file.as_file().sync_all().map_err(|_| error())?;
                if file.persist(&self.path).is_err() {
                    config.peers.remove(&credential.credential_id);
                    return Err(error());
                }
            }
            let peer = &config.peers[&credential.credential_id];
            json!({"serviceUrl":config.service_url,"host":self.access.remote_host_identity().device_id,
                "client":credential.credential_id,"hostPublicKey":B64.encode(self.key.public_key().as_ref()),"token":peer.token})
        };
        Ok(result)
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> HostResult<Value> {
        let (url, token) = {
            let config = self.config.lock().map_err(|_| error())?;
            (
                format!("{}{}", config.service_url.trim_end_matches('/'), path),
                config.host_token.clone(),
            )
        };
        let mut request = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let route = path.split('?').next().unwrap_or("unknown");
        let response = request.send().await.map_err(|failure| {
            super::diagnostics::Diagnostic::new("").emit("signal.http.failed", json!({"route":route,"timeout":failure.is_timeout(),"connectError":failure.is_connect()}));
            error()
        })?;
        if !response.status().is_success() {
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                *self.registered.lock().await = false;
            }
            super::diagnostics::Diagnostic::new("").emit(
                "signal.http.failed",
                json!({"route":route,"status":response.status().as_u16()}),
            );
        }
        let mut response = response.error_for_status().map_err(|_| error())?;
        if response.content_length().unwrap_or(0) > 4 * 1024 * 1024 {
            return Err(error());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| error())? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(error());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| error())
    }

    async fn sync_clients(&self) -> HostResult<()> {
        self.ensure_registered().await?;
        // Snapshot after acquiring the lock: an older periodic snapshot must
        // never overwrite the bootstrap response's freshly registered client.
        let _sync = self.sync_lock.lock().await;
        let active: HashSet<_> = self
            .access
            .list_credentials()?
            .into_iter()
            .filter(|c| c.revoked_at.is_none())
            .map(|c| c.credential_id)
            .collect();
        let clients = {
            let config = self.config.lock().map_err(|_| error())?;
            config
                .peers
                .iter()
                .filter(|(id, _)| active.contains(*id))
                .map(|(id, p)| json!({"id":id,"tokenHash":hash(p.token.as_bytes())}))
                .collect::<Vec<_>>()
        };
        self.request(
            reqwest::Method::PUT,
            "/v1/clients",
            Some(json!({"clients":clients})),
        )
        .await?;
        Ok(())
    }

    pub(crate) fn start(
        self: &Arc<Self>,
        gateway: Arc<ProviderGatewayService>,
        sessions: Arc<SessionRegistry>,
    ) -> tokio::task::JoinHandle<()> {
        let cloud = self.clone();
        tokio::spawn(async move {
            let seen = Arc::new(Mutex::new(BTreeMap::new()));
            let mut negotiations = tokio::task::JoinSet::new();
            loop {
                while negotiations.try_join_next().is_some() {}
                if cloud.ensure_registered().await.is_err() {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
                if let Ok(mut seen) = seen.lock() {
                    seen.retain(|_, expires| *expires > now());
                }
                let _ = cloud.poll_invitations().await;
                if cloud.sync_clients().await.is_ok() && negotiations.len() < 32 {
                    if let Ok(batch) = cloud
                        .request(
                            reqwest::Method::GET,
                            &format!("/v1/offers?limit={}", 32 - negotiations.len()),
                            None,
                        )
                        .await
                    {
                        if let Some(offers) = batch.get("offers").and_then(Value::as_array) {
                            for offer in offers.iter().take(32) {
                                // Independent paths must not queue behind another path's ICE timeout.
                                // Bound pending setups as well as live peers in SessionRegistry.
                                if negotiations.len() >= 32 {
                                    break;
                                }
                                let (cloud, gateway, sessions, seen, offer) = (
                                    cloud.clone(),
                                    gateway.clone(),
                                    sessions.clone(),
                                    seen.clone(),
                                    offer.clone(),
                                );
                                negotiations.spawn(async move {
                                    let _ = cloud.accept(&offer, gateway, sessions, &seen).await;
                                });
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
    }

    async fn accept(
        &self,
        offer: &Value,
        gateway: Arc<ProviderGatewayService>,
        sessions: Arc<SessionRegistry>,
        seen: &Mutex<BTreeMap<String, u64>>,
    ) -> HostResult<()> {
        let mut stage = "verifyOffer";
        let mut diagnostic = super::diagnostics::Diagnostic::new("");
        let result: HostResult<()> = async {
            let client = offer["client"].as_str().ok_or_else(error)?;
            let attempt = offer["attempt"].as_str().ok_or_else(error)?;
            let peer = {
                self.config
                    .lock()
                    .map_err(|_| error())?
                    .peers
                    .get(client)
                    .cloned()
                    .ok_or_else(error)?
            };
            let (body, bytes, expires) = verify_offer(
                &peer.public_key,
                &self.access.remote_host_identity().device_id,
                client,
                attempt,
                &offer["envelope"],
            )?;
            {
                let mut seen = seen.lock().map_err(|_| error())?;
                if seen.contains_key(attempt) || seen.len() >= 256 {
                    return Err(error());
                }
                seen.insert(attempt.to_owned(), expires);
            }
            let policy: PathPolicy = body
                .get("pathPolicy")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|_| error())?
                .unwrap_or_default();
            let description: RTCSessionDescription =
                serde_json::from_value(body["description"].clone()).map_err(|_| error())?;
            diagnostic = super::diagnostics::Diagnostic::new(&description.sdp);
            diagnostic.emit("signal.offer.verified", json!({"attempt":attempt,"pathPolicy":policy}));
            stage = "admission";
            let credential = self.active(client)?;
            let registration = sessions.register(client).map_err(|_| error())?;
            self.active(client)?;
            stage = "iceConfiguration";
            let servers: Vec<RTCIceServer> = if policy == PathPolicy::Ipv6Direct {
                Vec::new()
            } else {
                let ice = self.request(reqwest::Method::GET, "/v1/ice", None).await?;
                serde_json::from_value(ice["iceServers"].clone()).map_err(|_| error())?
            };
            diagnostic.emit(
                "signal.offer.accepted",
                json!({"attempt":attempt,"pathPolicy":policy}),
            );
            stage = "negotiation";
            let answer = super::answer_offer_with_policy(
                description,
                gateway,
                self.access.clone(),
                credential,
                registration,
                RTCConfiguration {
                    ice_servers: servers,
                    ..Default::default()
                },
                policy,
            )
            .await?;
            diagnostic.emit("signal.answer.ready", json!({"attempt":attempt}));
            let payload=serde_json::to_vec(&json!({"v":1,"kind":"answer","host":self.access.remote_host_identity().device_id,"client":client,"attempt":attempt,"expires":expires,"offerHash":hash(&bytes),"pathPolicy":policy,"description":answer})).map_err(|_|error())?;
            stage = "answerPublication";
            self.request(reqwest::Method::POST,"/v1/answers",Some(json!({"client":client,"attempt":attempt,"envelope":{"payload":B64.encode(&payload),"signature":B64.encode(self.key.sign(&payload).as_ref())}}))).await?;
            diagnostic.emit("signal.answer.published", json!({"attempt":attempt,"pathPolicy":policy}));
            Ok(())
        }.await;
        if let Err(failure) = &result {
            let attempt = offer["attempt"].as_str().filter(|v| {
                v.len() <= 100
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            });
            diagnostic.emit("signal.offer.failed", json!({"attempt":attempt,"stage":stage,"code":failure.code,"retryable":failure.retryable}));
        }
        result
    }
}

fn authorization_required() -> HostError {
    HostError::new("rtc_cloud_authorization_required", "信令服务地址已配置，但当前设备尚未获得该服务的授权")
}

fn authorized_service_url(authorized: &str, selected: Option<&str>) -> HostResult<String> {
    // Existing credentials remain scoped to the service which issued them.
    let bound = crate::normalize_signaling_service(authorized)?.ok_or_else(error)?;
    if let Some(selected) = selected {
        let selected = crate::normalize_signaling_service(selected)?.ok_or_else(error)?;
        if selected != bound { return Err(authorization_required()); }
        return Ok(selected);
    }
    Ok(bound)
}

fn verify_offer(
    public_key: &str,
    host: &str,
    client: &str,
    attempt: &str,
    envelope: &Value,
) -> HostResult<(Value, Vec<u8>, u64)> {
    let payload = envelope["payload"].as_str().ok_or_else(error)?;
    if payload.len() > 90000 || attempt.len() > 100 {
        return Err(error());
    }
    let bytes = B64.decode(payload).map_err(|_| error())?;
    if bytes.len() > 65536 {
        return Err(error());
    }
    let sig = B64
        .decode(envelope["signature"].as_str().ok_or_else(error)?)
        .map_err(|_| error())?;
    signature::UnparsedPublicKey::new(
        &signature::ED25519,
        B64.decode(public_key).map_err(|_| error())?,
    )
    .verify(&bytes, &sig)
    .map_err(|_| error())?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| error())?;
    if let Some(policy) = body.get("pathPolicy") {
        serde_json::from_value::<PathPolicy>(policy.clone()).map_err(|_| error())?;
    }
    let expires = body["expires"].as_u64().ok_or_else(error)?;
    if body["v"] != 1
        || body["kind"] != "offer"
        || body["host"] != host
        || body["client"] != client
        || body["attempt"] != attempt
        || expires <= now()
        || expires > now() + 90
    {
        return Err(error());
    }
    Ok((body, bytes, expires))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_service_preserves_legacy_authorization_without_forwarding_it() {
        assert_eq!(authorized_service_url("https://signal.example.com:443/", None).unwrap(), "https://signal.example.com");
        assert_eq!(authorized_service_url("https://172.96.254.12:8443", Some("172.96.254.12")).unwrap(), "https://172.96.254.12:8443");
        assert_eq!(authorized_service_url("https://old.example.com", Some("https://new.example.com")).unwrap_err().code, "rtc_cloud_authorization_required");
    }
    #[test]
    fn signed_offers_bind_identity_attempt_and_expiry() {
        let key = signature::Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let public = B64.encode(key.public_key().as_ref());
        let body = json!({"v":1,"kind":"offer","host":"host","client":"client","attempt":"attempt","expires":now()+60,"description":{"type":"offer","sdp":"test"}});
        let sign = |body: &Value| {
            let bytes = serde_json::to_vec(body).unwrap();
            json!({"payload":B64.encode(&bytes),"signature":B64.encode(key.sign(&bytes).as_ref())})
        };
        assert!(verify_offer(&public, "host", "client", "attempt", &sign(&body)).is_ok());
        for (field, value) in [
            ("host", json!("other")),
            ("client", json!("other")),
            ("attempt", json!("old")),
            ("expires", json!(0)),
            ("kind", json!("answer")),
            ("pathPolicy", json!("unknown")),
        ] {
            let mut changed = body.clone();
            changed[field] = value;
            assert!(verify_offer(&public, "host", "client", "attempt", &sign(&changed)).is_err());
        }
        let mut tampered = sign(&body);
        tampered["signature"] = json!(B64.encode([0; 64]));
        assert!(verify_offer(&public, "host", "client", "attempt", &tampered).is_err());
    }
}
