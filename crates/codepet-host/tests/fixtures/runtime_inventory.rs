use super::*;
use codepet_provider_sdk::{RuntimeGetInstalledRequest, RuntimeGetInstalledResponse};

impl FakeProvider {
    pub(super) async fn runtime_inventory(&self, request: RuntimeGetInstalledRequest)
        -> Result<RuntimeGetInstalledResponse, codepet_provider_sdk::ProtocolError> {
        let path = std::env::var("CODEPET_FAKE_RUNTIME_CONTROL")
            .map_err(|_| protocol_error("unsupported_method", "Runtime fixture is not enabled"))?;
        let control: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let installation = |version: &str| serde_json::json!({
            "executablePath":"/fixture/runtime", "version":version, "source":"current-path"
        });
        if request.refresh == Some(true) {
            if let Some(marker) = control["marker"].as_str() { std::fs::write(marker, b"requested").unwrap(); }
            if let Some(version) = control["notifyVersion"].as_str() {
                self.events.as_ref().unwrap().publish(ProtocolEvent::RuntimeInventoryChanged {
                    jsonrpc: "2.0".into(), params: serde_json::from_value(serde_json::json!({
                        "harnessList":[installation(version)], "selected":installation(version), "scanning":false
                    })).unwrap(),
                })?;
            }
            if let Some(gate) = control["gate"].as_str() {
                while !std::path::Path::new(gate).exists() { tokio::time::sleep(Duration::from_millis(5)).await; }
            }
            if control["error"].as_bool() == Some(true) {
                return Err(protocol_error("fixture_scan_failed", "Fixture scan failed"));
            }
        }
        let version = control["version"].as_str().unwrap_or("1.0.0");
        let installed = installation(version);
        Ok(serde_json::from_value(serde_json::json!({
            "harnessList": if control["empty"].as_bool() == Some(true) { vec![] } else { vec![installed.clone()] },
            "selected": if control["empty"].as_bool() == Some(true) { serde_json::Value::Null } else { installed },
            "scanning":control["scanning"].as_bool().unwrap_or(false),
            "scanError":control["scanError"],
        })).unwrap())
    }
}
