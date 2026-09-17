//! QR-authorized, end-to-end encrypted admission shared by LAN and rendezvous.
use super::*;
use codepet_lan_channel_sdk::{PairingRequestCreateRequest, PairingRequestState};
use ring::aead;

#[derive(Clone)]
pub(super) struct Invitation {
    secret: String,
    gateway_url: String,
    expires: u64,
    published_at: u64,
    result: Option<(String, String)>,
}

fn key(secret: &str, direction: &str) -> HostResult<aead::LessSafeKey> {
    let bytes = digest::digest(
        &digest::SHA256,
        format!("codepet-invite-v2:{direction}:{secret}").as_bytes(),
    );
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, bytes.as_ref()).map_err(|_| error())?,
    ))
}

fn open(secret: &str, aad: &str, sealed: &str) -> HostResult<Vec<u8>> {
    if sealed.len() > 16000 {
        return Err(error());
    }
    let mut bytes = B64.decode(sealed).map_err(|_| error())?;
    if bytes.len() < 28 {
        return Err(error());
    }
    let nonce: [u8; 12] = bytes[..12].try_into().map_err(|_| error())?;
    let plain = key(secret, "request")?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad.as_bytes()),
            &mut bytes[12..],
        )
        .map_err(|_| error())?;
    Ok(plain.to_vec())
}

fn seal(secret: &str, aad: &str, plain: &[u8]) -> HostResult<String> {
    let mut nonce = [0; 12];
    SystemRandom::new().fill(&mut nonce).map_err(|_| error())?;
    let mut bytes = plain.to_vec();
    key(secret, "result")?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad.as_bytes()),
            &mut bytes,
        )
        .map_err(|_| error())?;
    Ok(B64.encode([nonce.to_vec(), bytes].concat()))
}

impl Cloud {
    pub(crate) fn prepare_invitation(
        &self,
        session: &crate::PairingSession,
        gateway_url: String,
    ) -> HostResult<(String, String)> {
        let config = self.config.lock().map_err(|_| error())?;
        let mut invitations = self.invitations.lock().map_err(|_| error())?;
        invitations.retain(|_, entry| entry.expires > now());
        if invitations.len() >= 4 {
            return Err(error());
        }
        self.access
            .require_invitation_confirmation(&session.pairing_id)?;
        invitations.insert(
            session.pairing_id.clone(),
            Invitation {
                secret: session.pairing_secret.clone(),
                gateway_url,
                expires: session.expires_at / 1000,
                published_at: 0,
                result: None,
            },
        );
        Ok((
            config.service_url.clone(),
            B64.encode(self.key.public_key().as_ref()),
        ))
    }

    pub(crate) async fn exchange_invitation(
        &self,
        id: &str,
        request_id: &str,
        sealed: &str,
    ) -> HostResult<Value> {
        if request_id.len() != 64
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(error());
        }
        let invitation = self
            .invitations
            .lock()
            .map_err(|_| error())?
            .get(id)
            .cloned()
            .ok_or_else(error)?;
        if invitation.expires <= now() {
            return Err(error());
        }
        let host = self.access.remote_host_identity();
        let aad = format!("{}:{id}:{request_id}", host.device_id);
        let bytes = open(&invitation.secret, &aad, sealed)?;
        let envelope: Value = serde_json::from_slice(&bytes).map_err(|_| error())?;
        let payload = B64
            .decode(envelope["payload"].as_str().ok_or_else(error)?)
            .map_err(|_| error())?;
        let body: Value = serde_json::from_slice(&payload).map_err(|_| error())?;
        let public = body["publicKey"].as_str().ok_or_else(error)?;
        signature::UnparsedPublicKey::new(
            &signature::ED25519,
            B64.decode(public).map_err(|_| error())?,
        )
        .verify(
            &payload,
            &B64.decode(envelope["signature"].as_str().ok_or_else(error)?)
                .map_err(|_| error())?,
        )
        .map_err(|_| error())?;
        if body["v"] != 2
            || body["host"] != host.device_id
            || body["invitationId"] != id
            || body["requestId"] != request_id
            || body["expires"] != invitation.expires
        {
            return Err(error());
        }
        let binding = hash(&payload);
        let request = self.access.create_invitation_request(
            id,
            &invitation.secret,
            &binding,
            PairingRequestCreateRequest {
                host_device_id: host.device_id.clone(),
                client_id: body["clientId"].as_str().ok_or_else(error)?.to_owned(),
                device: serde_json::from_value(body["device"].clone()).map_err(|_| error())?,
                client_nonce: request_id.to_owned(),
            },
        )?;
        if request.state == PairingRequestState::Pending {
            return Ok(json!({"result":null}));
        }
        let credential = request
            .bearer_token()
            .map(|bearer| self.access.validate_bearer(bearer))
            .transpose()?;
        if let Some((previous, result)) = invitation.result {
            if previous != binding {
                return Err(error());
            }
            return Ok(json!({"result":result}));
        }
        let mut result = json!({"v":2,"host":host.device_id,"invitationId":id,"requestId":request_id,
            "requestHash":binding,"expires":invitation.expires,"state":request.state});
        if request.state == PairingRequestState::Accepted {
            let bearer = request.bearer_token().ok_or_else(error)?;
            result["cloud"] = self.bind_peer(credential.as_ref().ok_or_else(error)?, public)?;
            result["pairing"] =
                json!({"device":host,"gatewayUrl":invitation.gateway_url,"credential":bearer});
        }
        let payload = serde_json::to_vec(&result).map_err(|_| error())?;
        let signed = serde_json::to_vec(&json!({"payload":B64.encode(&payload),
            "signature":B64.encode(self.key.sign(&payload).as_ref())}))
        .map_err(|_| error())?;
        let result = seal(&invitation.secret, &aad, &signed)?;
        let mut invitations = self.invitations.lock().map_err(|_| error())?;
        let entry = invitations.get_mut(id).ok_or_else(error)?;
        // Concurrent routes return one immutable encrypted terminal result.
        let cached = entry.result.get_or_insert((binding, result));
        Ok(json!({"result":cached.1}))
    }

    pub(super) async fn poll_invitations(&self) -> HostResult<()> {
        let pending = {
            let mut invitations = self.invitations.lock().map_err(|_| error())?;
            invitations.retain(|_, entry| entry.expires > now());
            invitations
                .iter()
                .filter(|(_, e)| e.published_at + 20 <= now())
                .map(|(id, e)| (id.clone(), e.clone()))
                .collect::<Vec<_>>()
        };
        for (id, invitation) in pending {
            let token = hash(format!("codepet-invite-v2:mailbox:{}", invitation.secret).as_bytes());
            self.request(
                reqwest::Method::PUT,
                "/v1/invitations",
                Some(json!({"id":id,
                "tokenHash":hash(token.as_bytes()),"expires":invitation.expires})),
            )
            .await?;
            if let Some(entry) = self.invitations.lock().map_err(|_| error())?.get_mut(&id) {
                entry.published_at = now();
            }
        }
        if self.invitations.lock().map_err(|_| error())?.is_empty() {
            return Ok(());
        }
        let batch = self
            .request(reqwest::Method::GET, "/v1/invitation-requests", None)
            .await?;
        if let Some(requests) = batch["requests"].as_array() {
            for request in requests.iter().take(4) {
                let (Some(id), Some(request_id), Some(sealed)) = (
                    request["id"].as_str(),
                    request["requestId"].as_str(),
                    request["sealed"].as_str(),
                ) else {
                    continue;
                };
                if let Ok(result) = self.exchange_invitation(id, request_id, sealed).await {
                    if let Some(result) = result["result"].as_str() {
                        let _ = self
                            .request(
                                reqwest::Method::POST,
                                "/v1/invitation-results",
                                Some(json!({"id":id,"requestId":request_id,"result":result})),
                            )
                            .await;
                    }
                }
            }
        }
        Ok(())
    }
}
