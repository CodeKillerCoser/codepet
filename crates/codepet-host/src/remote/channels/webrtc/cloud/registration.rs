use super::*;

fn random_secret() -> HostResult<String> {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| error())?;
    Ok(B64.encode(bytes))
}

pub(super) fn load_or_create_config(access: &RemoteAccessManager) -> HostResult<Option<(PathBuf, Config)>> {
    use rusqlite::{params, OptionalExtension};
    let Some(service) = access.signaling_service.lock().map_err(|_| error())?.clone() else { return Ok(None) };
    let service = authorized_service_url(&service, Some(&service))?;
    let path = access.rtc_cloud_config_path();
    let mut db = crate::storage::open(&path)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(crate::storage::error)?;
    let seed: Option<Vec<u8>> = tx.query_row("SELECT signing_seed FROM signaling_identity WHERE id=1",[],|r|r.get(0)).optional().map_err(crate::storage::error)?;
    let seed = match seed {
        Some(seed) => seed,
        None => {
            let mut seed = vec![0;32];
            SystemRandom::new().fill(&mut seed).map_err(|_| error())?;
            tx.execute("INSERT INTO signaling_identity VALUES(1,?1)",[&seed]).map_err(crate::storage::error)?;
            seed
        }
    };
    signature::Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| error())?;
    tx.execute("INSERT OR IGNORE INTO signaling_services VALUES(?1,?2,1)",params![service,random_secret()?]).map_err(crate::storage::error)?;
    let (host_token, auto_register) = tx.query_row("SELECT host_token,auto_register FROM signaling_services WHERE service_url=?1",[&service],|r|Ok((r.get(0)?,r.get(1)?))).map_err(crate::storage::error)?;
    let mut peers = BTreeMap::new();
    {
        let mut statement = tx.prepare("SELECT credential_id,public_key,token FROM signaling_peers WHERE service_url=?1").map_err(crate::storage::error)?;
        for row in statement.query_map([&service],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?))).map_err(crate::storage::error)? {
            let (id,public_key,token) = row.map_err(crate::storage::error)?;
            if public_key.len()!=32 { return Err(error()) }
            peers.insert(id,Peer { public_key:B64.encode(public_key),token });
        }
    }
    tx.commit().map_err(crate::storage::error)?;
    Ok(Some((path,Config { service_url:service,host_token,seed:B64.encode(seed),peers,auto_register })))
}

pub(super) fn persist_peer(path: &std::path::Path, service: &str, id: &str, peer: &Peer) -> HostResult<()> {
    let public_key = B64.decode(&peer.public_key).map_err(|_| error())?;
    crate::storage::open(path)?.execute("INSERT INTO signaling_peers VALUES(?1,?2,?3,?4)", rusqlite::params![service,id,public_key,peer.token]).map_err(crate::storage::error)?;
    Ok(())
}

impl Cloud {
    pub(super) async fn ensure_registered(&self) -> HostResult<()> {
        let mut registered = self.registered.lock().await;
        if *registered {
            return Ok(());
        }
        let (service, token, automatic) = {
            let config = self.config.lock().map_err(|_| error())?;
            (
                config.service_url.clone(),
                config.host_token.clone(),
                config.auto_register,
            )
        };
        if !automatic {
            *registered = true;
            return Ok(());
        }
        let diagnostic = super::super::diagnostics::Diagnostic::new("");
        diagnostic.emit("signal.registration.start", json!({}));
        let result = async {
            let host = self.access.remote_host_identity().device_id;
            let payload = serde_json::to_vec(&json!({"v":1,"kind":"device.register","deviceId":host,
                "role":"host","tokenHash":hash(token.as_bytes()),"expires":now()+60}))?;
            let response = self.http.post(format!("{service}/v1/devices/register"))
                .bearer_auth(token)
                .json(&json!({"payload":B64.encode(&payload),"signature":B64.encode(self.key.sign(&payload).as_ref()),
                    "publicKey":B64.encode(self.key.public_key().as_ref())}))
                .send().await.map_err(|_| HostError::new("signal_registration_network", "地址已保存，暂时无法连接信令服务，App 将自动重试").retryable(true))?;
            if !response.status().is_success() {
                let status = response.status().as_u16();
                diagnostic.emit("signal.registration.rejected", json!({"status":status}));
                return Err(HostError::new("signal_registration_rejected", match status {
                    403 | 404 => "信令服务尚未启用自动登记，请更新服务端",
                    409 => "设备登记身份冲突，原有身份不会被覆盖",
                    429 => "登记请求过于频繁，App 将稍后重试",
                    _ => "信令服务登记失败，App 将自动重试",
                }).retryable(status != 409));
            }
            let mut response = response;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| error())? {
                if bytes.len()+chunk.len()>16384 { return Err(error()); }
                bytes.extend_from_slice(&chunk);
            }
            let body: Value = serde_json::from_slice(&bytes)?;
            if body["protocol"] != "codepet-device-registration-v1" || body["deviceId"] != host
                || body["role"] != "host" || body["status"] != "registered" { return Err(error()); }
            Ok(())
        }.await;
        diagnostic.emit(
            if result.is_ok() {
                "signal.registration.complete"
            } else {
                "signal.registration.failed"
            },
            json!({"code":result.as_ref().err().map(|e: &HostError| e.code.as_str())}),
        );
        if result.is_ok() {
            *registered = true;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn access(path: &std::path::Path) -> RemoteAccessManager {
        RemoteAccessManager::open(
            crate::RemoteAccessConfig::for_data_directory(path.join("connections")),
            Arc::new(
                crate::DeviceRegistry::open(path.join("device.json"), "Enrollment test").unwrap(),
            ),
            codepet_gateway_sdk::DeviceDescriptor {
                device_name: "Enrollment test".into(),
                operating_system: "test".into(),
                system_version: "1".into(),
            },
        )
        .unwrap()
    }
    #[test]
    fn automatic_registration_persists_identity_and_isolates_service_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let manager = access(directory.path());
        assert!(load_or_create_config(&manager).unwrap().is_none());
        *manager.signaling_service.lock().unwrap() = Some("https://first.example.com".into());
        let (path, first) = load_or_create_config(&manager).unwrap().unwrap();
        let (_, retry) = load_or_create_config(&manager).unwrap().unwrap();
        assert_eq!(first.seed, retry.seed);
        assert_eq!(first.host_token, retry.host_token);
        assert!(first.auto_register);
        assert_eq!(B64.decode(&first.seed).unwrap().len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        *manager.signaling_service.lock().unwrap() = Some("https://second.example.com".into());
        let (_, second) = load_or_create_config(&manager).unwrap().unwrap();
        assert_eq!(first.seed, second.seed);
        assert_ne!(first.host_token, second.host_token);
        let reopened = access(directory.path());
        *reopened.signaling_service.lock().unwrap() = Some("https://first.example.com".into());
        let (_, restored) = load_or_create_config(&reopened).unwrap().unwrap();
        assert_eq!(first.host_token, restored.host_token);
        assert_eq!(
            manager.remote_host_identity().device_id,
            reopened.remote_host_identity().device_id
        );
    }
    #[test]
    fn legacy_files_are_ignored_and_peer_rows_survive_reopen() {
        let directory=tempfile::tempdir().unwrap();
        let manager=access(directory.path());
        let legacy_path=manager.rtc_cloud_config_path().with_file_name("rtc-cloud.json");
        std::fs::write(&legacy_path,b"invalid legacy JSON").unwrap();
        assert!(load_or_create_config(&manager).unwrap().is_none());
        *manager.signaling_service.lock().unwrap()=Some("https://new.example.com".into());
        let (path,config)=load_or_create_config(&manager).unwrap().unwrap();
        let peer=Peer { public_key:B64.encode([1;32]),token:random_secret().unwrap() };
        persist_peer(&path,&config.service_url,"credential-test",&peer).unwrap();
        let (_,restored)=load_or_create_config(&manager).unwrap().unwrap();
        assert_eq!(restored.peers["credential-test"].token,peer.token);
        assert_eq!(restored.peers["credential-test"].public_key,peer.public_key);
        assert_eq!(std::fs::read(&legacy_path).unwrap(),b"invalid legacy JSON");
    }
}
