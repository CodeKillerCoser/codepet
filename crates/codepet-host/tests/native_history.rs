//! Explicit snapshot-only diagnostic across the installed Provider mux boundary.
use codepet_host::{PluginDescriptor, PluginProcess, PluginProcessOptions};
use codepet_provider_sdk::*;
use serde_json::json;
use std::path::PathBuf;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Provider, OpenCode and isolated history snapshot"]
async fn installed_opencode_history_through_host_transport() {
    let binary = PathBuf::from(std::env::var_os("CODEPET_OPENCODE_PROVIDER").unwrap());
    let executable = std::env::var("CODEPET_OPENCODE_EXECUTABLE").unwrap();
    let data = std::env::var("CODEPET_OPENCODE_HISTORY_SNAPSHOT").unwrap();
    let process = PluginProcess::spawn(
        &PluginDescriptor {
            plugin_id: "dev.codepet.opencode".into(),
            display_name: "History diagnostic".into(),
            icon: None,
            executable: binary,
            args: vec![],
            env: Default::default(),
            enabled: true,
            instances: vec![],
        },
        PluginProcessOptions {
            request_timeout: std::time::Duration::from_secs(
                std::env::var("CODEPET_HISTORY_DIAGNOSTIC_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(10),
            ),
            ..PluginProcessOptions::default()
        },
    )
    .unwrap();
    let mut inbound = process.take_inbound().await.unwrap();
    let events = tokio::spawn(async move { while inbound.recv().await.is_some() {} });
    let client = process.client();
    let result = async {
        client
            .provider_initialize(ProviderInitializeRequest {
                host_client_id: "history-check".into(),
                host_device_id: "history-check".into(),
                host_version: "test".into(),
                supported_versions: VersionRange {
                    min_version: PROTOCOL_VERSION,
                    max_version: PROTOCOL_VERSION,
                },
            })
            .await?;
        let route = ProviderInstanceRoute {
            device_id: "history-check".into(),
            provider_plugin_id: "dev.codepet.opencode".into(),
            provider_instance_id: "opencode".into(),
        };
        client
            .instance_create(InstanceCreateRequest {
                route: route.clone(),
                instance_kind: "opencode".into(),
                display_name: "History check".into(),
                settings: [
                    ("serverExecutable".into(), json!(executable)),
                    ("serverArgs".into(), json!(["serve"])),
                    ("dataDirectory".into(), json!(data)),
                    ("workspaceRoot".into(), json!(data)),
                ]
                .into(),
            })
            .await?;
        let started = std::time::Instant::now();
        client
            .instance_start(InstanceStartRequest {
                route: route.clone(),
            })
            .await?;
        println!("Instance started in {:?}", started.elapsed());
        let listed = client
            .conversation_list(ConversationListRequest {
                route: route.clone(),
                cursor: None,
                limit: Some(20),
                project_filter: serde_json::from_value(json!({"kind":"all"})).unwrap(),
                query: None,
                reader_scope: None,
            })
            .await?;
        assert!(
            !listed.conversations.is_empty(),
            "snapshot must contain conversations"
        );
        for item in listed.conversations {
            let conversation = ProviderResourceId {
                device_id: route.device_id.clone(),
                provider_plugin_id: route.provider_plugin_id.clone(),
                provider_instance_id: route.provider_instance_id.clone(),
                native_resource_id: item.resource.native_resource_id,
            };
            let response = client
                .conversation_get(ConversationGetRequest {
                    conversation,
                    cursor: None,
                    limit: Some(20),
                })
                .await?;
            println!("Installed Provider history: {} items", response.items.len());
        }
        Ok::<_, ProtocolError>(())
    }
    .await;
    let shutdown = process.shutdown().await;
    events.abort();
    assert!(result.is_ok(), "{result:?}");
    assert!(shutdown.is_ok(), "{shutdown:?}");
}
