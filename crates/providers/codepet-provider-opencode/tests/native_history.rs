//! Opt-in history check against an isolated copy of native OpenCode data.
use codepet_provider_opencode::*;
use codepet_provider_sdk::*;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires an explicitly prepared data snapshot and installed OpenCode"]
async fn native_history_snapshot_is_readable() {
    let executable = std::env::var("CODEPET_OPENCODE_EXECUTABLE").unwrap();
    let data = std::env::var("CODEPET_OPENCODE_HISTORY_SNAPSHOT").unwrap();
    let provider = OpenCodeProvider::new(Arc::new(|_| Ok(())));
    provider
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "history-diagnostic".into(),
            host_device_id: "history-diagnostic".into(),
            host_version: "test".into(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        })
        .await
        .unwrap();
    let route = ProviderInstanceRoute {
        device_id: "history-diagnostic".into(),
        provider_plugin_id: OPENCODE_PLUGIN_ID.into(),
        provider_instance_id: "opencode".into(),
    };
    provider
        .instance_create(InstanceCreateRequest {
            route: route.clone(),
            instance_kind: OPENCODE_INSTANCE_KIND.into(),
            display_name: "History diagnostic".into(),
            settings: [
                ("serverExecutable".into(), json!(executable)),
                ("serverArgs".into(), json!(["serve"])),
                ("dataDirectory".into(), json!(data)),
                ("workspaceRoot".into(), json!(data)),
            ]
            .into(),
        })
        .await
        .unwrap();
    let outcome = async {
        provider
            .instance_start(InstanceStartRequest {
                route: route.clone(),
            })
            .await?;
        let page = provider
            .conversation_list(ConversationListRequest {
                route: route.clone(),
                cursor: None,
                limit: Some(20),
                project_filter: serde_json::from_value(json!({"kind":"all"})).unwrap(),
            })
            .await?;
        assert!(
            !page.conversations.is_empty(),
            "snapshot must contain conversations"
        );
        println!("Conversations: {}", page.conversations.len());
        let mut failures = Vec::new();
        for conversation in page.conversations {
            let resource = ProviderResourceId {
                device_id: route.device_id.clone(),
                provider_plugin_id: route.provider_plugin_id.clone(),
                provider_instance_id: route.provider_instance_id.clone(),
                native_resource_id: conversation.resource.native_resource_id,
            };
            match provider
                .conversation_get(ConversationGetRequest {
                    conversation: resource,
                    cursor: None,
                    limit: Some(20),
                })
                .await
            {
                Ok(history) => println!("History OK: {} items", history.items.len()),
                Err(error) => {
                    println!("History error: {error:?}");
                    failures.push(error);
                }
            }
        }
        Ok::<_, ProtocolError>(failures)
    }
    .await;
    provider
        .provider_shutdown(ProviderShutdownRequest {})
        .await
        .unwrap();
    assert!(
        outcome.as_ref().is_ok_and(|failures| failures.is_empty()),
        "{outcome:?}"
    );
}
