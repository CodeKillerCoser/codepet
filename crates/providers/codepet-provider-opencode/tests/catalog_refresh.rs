//! Native catalog snapshots may be nonempty before custom providers settle.
use codepet_provider_opencode::{OpenCodeProvider, OPENCODE_INSTANCE_KIND, OPENCODE_PLUGIN_ID};
use codepet_provider_sdk::*;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{mpsc, Arc},
    time::Duration,
};

struct Fixture {
    directory: tempfile::TempDir,
    provider: OpenCodeProvider,
    route: ProviderInstanceRoute,
    events: mpsc::Receiver<ProtocolEvent>,
}

impl Fixture {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        write_json(
            directory.path(),
            "opencode-fixture-startup.json",
            json!({"catalogControlDirectory":directory.path()}),
        );
        let (sender, events) = mpsc::channel();
        let provider = OpenCodeProvider::new(Arc::new(move |event| {
            let _ = sender.send(event);
            Ok(())
        }));
        provider
            .provider_initialize(ProviderInitializeRequest {
                directories: None,
                host_client_id: "catalog-host".into(),
                host_device_id: "catalog-device".into(),
                host_version: "test".into(),
                protocol: ProtocolVersionOffer {
                    min_version: PROTOCOL_VERSION,
                    version: PROTOCOL_VERSION,
                },
            })
            .await
            .unwrap();
        let route = ProviderInstanceRoute {
            device_id: "catalog-device".into(),
            provider_plugin_id: OPENCODE_PLUGIN_ID.into(),
            provider_instance_id: "catalog".into(),
        };
        provider
            .instance_create(InstanceCreateRequest {
                route: route.clone(),
                instance_kind: OPENCODE_INSTANCE_KIND.into(),
                display_name: "Catalog".into(),
                settings: BTreeMap::from([
                    (
                        "serverExecutable".into(),
                        json!(env!("CARGO_BIN_EXE_opencode-server-fixture")),
                    ),
                    ("serverVersion".into(), json!("2.2.6")),
                    ("serverArgs".into(), json!(["serve"])),
                    ("workspaceRoot".into(), json!(directory.path())),
                ]),
            })
            .await
            .unwrap();
        provider
            .instance_start(InstanceStartRequest {
                route: route.clone(),
            })
            .await
            .unwrap();
        let fixture = Self {
            directory,
            provider,
            route,
            events,
        };
        fixture.wait_models(&["fixture/fixture-model"]).await;
        fixture
    }

    async fn capabilities(&self) -> ProviderCapabilities {
        self.provider
            .instance_capabilities(InstanceCapabilitiesRequest {
                route: self.route.clone(),
            })
            .await
            .unwrap()
            .capabilities
    }

    async fn wait_models(&self, expected: &[&str]) -> ProviderCapabilities {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let capabilities = self.capabilities().await;
                if model_ids(&capabilities) == expected {
                    return capabilities;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("updated catalog")
    }

    fn response(&self, models: &[(&str, bool)], status: u16) {
        write_json(
            self.directory.path(),
            "catalog-response.json",
            json!({"status":status,
            "providers":[{"id":"fixture","name":"Fixture Provider"},{"id":"custom","name":"Configured Provider"}],
            "models": std::iter::once(json!({"id":"fixture-model","providerID":"fixture","name":"Fixture Model","status":"active","enabled":true,"variants":[{"id":"high"}]}))
                .chain(models.iter().map(|(id,enabled)|json!({"id":id,"providerID":"custom","name":id,"status":"active","enabled":enabled,"variants":[{"id":"high"}]})))
                .collect::<Vec<_>>() }),
        );
    }

    async fn event(&self, serial: u64, kind: &str, count: u64) {
        write_json(
            self.directory.path(),
            "catalog-event.json",
            json!({"serial":serial,"type":kind,"count":count}),
        );
        wait_until(|| {
            std::fs::read_to_string(self.directory.path().join("catalog-event-ack"))
                .ok()
                .as_deref()
                == Some(&serial.to_string())
        })
        .await;
    }

    fn reads(&self) -> u64 {
        std::fs::read_to_string(self.directory.path().join("model-reads"))
            .unwrap()
            .parse()
            .unwrap()
    }

    async fn shutdown(&self) {
        self.provider
            .provider_shutdown(ProviderShutdownRequest {})
            .await
            .unwrap();
    }
}

fn write_json(root: &Path, name: &str, value: Value) {
    let temporary = root.join(format!("{name}.tmp"));
    std::fs::write(&temporary, serde_json::to_vec(&value).unwrap()).unwrap();
    std::fs::rename(temporary, root.join(name)).unwrap();
}

fn model_ids(capabilities: &ProviderCapabilities) -> Vec<String> {
    let Some(ModelCatalog::GroupedModelCatalog(catalog)) = capabilities
        .turn_send
        .as_ref()
        .and_then(|s| s.model_catalog.as_ref())
    else {
        return vec![];
    };
    let mut ids = catalog
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.id, m.id)))
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

async fn wait_until(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture condition");
}

#[tokio::test]
async fn late_custom_models_refresh_create_and_send_catalogs_without_reprobing_accounts() {
    let f = Fixture::start().await;
    let initial = f.capabilities().await;
    f.response(&[("custom-a", true), ("custom-b", true)], 200);
    let reads = f.reads();
    f.event(1, "model.updated", 100).await;
    let updated = f
        .wait_models(&[
            "custom/custom-a",
            "custom/custom-b",
            "fixture/fixture-model",
        ])
        .await;
    assert_ne!(initial.revision, updated.revision);
    let Some(ModelCatalog::FlatModelCatalog(create)) = updated
        .conversation_create
        .as_ref()
        .unwrap()
        .selection
        .as_ref()
        .unwrap()
        .model_catalog
        .as_ref()
    else {
        panic!("flat create catalog");
    };
    assert!(create.models.iter().any(|m| m.id == "custom/custom-a"));
    assert!(f.events.try_iter().any(|e|matches!(e,ProtocolEvent::EventInstanceStatusChanged{params,..} if model_ids(&params.instance.capabilities).contains(&"custom/custom-a".into()))));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(f.reads() - reads <= 2, "event bursts must coalesce");

    let reads = f.reads();
    f.event(2, "provider.updated", 1).await;
    wait_until(|| f.reads() > reads).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        updated.revision,
        f.capabilities().await.revision,
        "unchanged snapshot keeps revision"
    );
    f.response(&[("custom-a", false)], 200); // disable a and remove b
    f.event(3, "config.updated", 1).await;
    let removed = f.wait_models(&["fixture/fixture-model"]).await;
    assert_ne!(updated.revision, removed.revision);
    assert_eq!(
        std::fs::read_to_string(f.directory.path().join("metadata-calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    f.shutdown().await;
}

#[tokio::test]
async fn update_during_inflight_catalog_is_not_lost_and_transient_errors_keep_last_snapshot() {
    let f = Fixture::start().await;
    let root = f.directory.path();
    f.response(&[("older", true)], 200);
    std::fs::write(root.join("block-model"), "").unwrap();
    f.event(1, "model.updated", 1).await;
    wait_until(|| root.join("model-blocked").exists()).await;
    f.response(&[("newer", true)], 200);
    f.event(2, "provider.updated", 1).await;
    std::fs::write(root.join("release-model"), "").unwrap();
    let last = f
        .wait_models(&["custom/newer", "fixture/fixture-model"])
        .await;
    f.response(&[], 503);
    let reads = f.reads();
    f.event(3, "model.updated", 1).await;
    wait_until(|| f.reads() > reads).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(last.revision, f.capabilities().await.revision);
    f.response(&[("recovered", true)], 200); // retry must recover without another event
    f.wait_models(&["custom/recovered", "fixture/fixture-model"])
        .await;
    f.shutdown().await;
}

#[tokio::test]
async fn stopping_inflight_catalog_cannot_publish_ready_or_overwrite_restart() {
    let f = Fixture::start().await;
    let root = f.directory.path();
    f.response(&[("stale", true)], 200);
    std::fs::write(root.join("block-model"), "").unwrap();
    f.event(1, "config.updated", 1).await;
    wait_until(|| root.join("model-blocked").exists()).await;
    f.provider
        .instance_stop(InstanceStopRequest {
            route: f.route.clone(),
        })
        .await
        .unwrap();
    let _ = f.events.try_iter().collect::<Vec<_>>();
    std::fs::write(root.join("release-model"), "").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!f.events.try_iter().any(|e|matches!(e,ProtocolEvent::EventInstanceStatusChanged{params,..} if params.instance.status==InstanceStatus::Ready)));
    f.response(&[("restarted", true)], 200);
    f.provider
        .instance_start(InstanceStartRequest {
            route: f.route.clone(),
        })
        .await
        .unwrap();
    f.wait_models(&["custom/restarted", "fixture/fixture-model"])
        .await;
    f.shutdown().await;
}
