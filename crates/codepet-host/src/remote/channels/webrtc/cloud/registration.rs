use super::*;

fn random_secret() -> HostResult<String> {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| error())?;
    Ok(B64.encode(bytes))
}

// New files are private and installed atomically before the first network request.
fn create_private(path: &std::path::Path, bytes: &[u8]) -> HostResult<()> {
    let parent = path.parent().ok_or_else(error)?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(failure) if failure.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(error()),
    }
}

pub(super) fn load_or_create_config(
    access: &RemoteAccessManager,
) -> HostResult<Option<(PathBuf, Config)>> {
    let legacy_path = access.rtc_cloud_config_path();
    let selected = access
        .signaling_service
        .lock()
        .map_err(|_| error())?
        .clone();
    let legacy: Option<Config> = if legacy_path.exists() {
        Some(serde_json::from_slice(&std::fs::read(&legacy_path)?)?)
    } else {
        None
    };
    let Some(service) = selected else {
        return Ok(legacy.map(|config| (legacy_path, config)));
    };
    if let Some(config) = &legacy {
        if authorized_service_url(&config.service_url, Some(&service)).is_ok() {
            let mut config = config.clone();
            config.service_url = service;
            config.auto_register = true;
            return Ok(Some((legacy_path, config)));
        }
    }
    let parent = legacy_path.parent().ok_or_else(error)?;
    let identity_path = parent.join("rtc-device-key.json");
    if !identity_path.exists() {
        let seed = match legacy.as_ref() {
            Some(c) => c.seed.clone(),
            None => random_secret()?,
        };
        signature::Ed25519KeyPair::from_seed_unchecked(&B64.decode(&seed).map_err(|_| error())?)
            .map_err(|_| error())?;
        create_private(&identity_path, &serde_json::to_vec(&json!({"seed": seed}))?)?;
    }
    let identity: Value = serde_json::from_slice(&std::fs::read(&identity_path)?)?;
    let seed = identity["seed"].as_str().ok_or_else(error)?.to_owned();
    let path = parent
        .join("rtc-services")
        .join(format!("{}.json", hash(service.as_bytes())));
    if !path.exists() {
        let config = Config {
            service_url: service.clone(),
            host_token: random_secret()?,
            seed: seed.clone(),
            peers: BTreeMap::new(),
            auto_register: true,
        };
        create_private(&path, &serde_json::to_vec(&config)?)?;
    }
    let config: Config = serde_json::from_slice(&std::fs::read(&path)?)?;
    if config.service_url != service || config.seed != seed {
        return Err(error());
    }
    Ok(Some((path, config)))
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
    fn legacy_identity_is_preserved_when_registering_another_service() {
        let directory = tempfile::tempdir().unwrap();
        let manager = access(directory.path());
        let legacy = Config {
            service_url: "https://old.example.com".into(),
            host_token: random_secret().unwrap(),
            seed: random_secret().unwrap(),
            peers: BTreeMap::new(),
            auto_register: false,
        };
        create_private(
            &manager.rtc_cloud_config_path(),
            &serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        assert!(
            !load_or_create_config(&manager)
                .unwrap()
                .unwrap()
                .1
                .auto_register
        );
        *manager.signaling_service.lock().unwrap() = Some("https://new.example.com".into());
        let (_, new) = load_or_create_config(&manager).unwrap().unwrap();
        assert_eq!(new.seed, legacy.seed);
        assert_ne!(new.host_token, legacy.host_token);
        let old: Config =
            serde_json::from_slice(&std::fs::read(manager.rtc_cloud_config_path()).unwrap())
                .unwrap();
        assert_eq!(old.host_token, legacy.host_token);
    }
}
