use codepet_host::{PluginDescriptor, PluginProcess, PluginProcessOptions};
use codepet_provider_sdk::{
    ConversationGetRequest, InstanceCreateRequest, InstanceDestroyRequest, InstanceStartRequest,
    InstanceStatus, InstanceStopRequest, JsonObject, ProtocolEvent, ProviderDescribeRequest,
    ProviderInitializeRequest, ProviderInstanceRoute, RoutedResourceId, VersionRange,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn descriptor(plugin_id: &str) -> PluginDescriptor {
    PluginDescriptor {
        plugin_id: plugin_id.to_string(),
        display_name: "Fake Provider".to_string(),
        executable: env!("CARGO_BIN_EXE_codepet-host-fake-provider").into(),
        args: Vec::new(),
        env: BTreeMap::from([(
            "CODEPET_FAKE_PLUGIN_ID".to_string(),
            plugin_id.to_string(),
        )]),
        enabled: true,
        instances: Vec::new(),
    }
}

fn options() -> PluginProcessOptions {
    PluginProcessOptions {
        max_frame_bytes: 16 * 1024,
        request_timeout: Duration::from_secs(2),
        shutdown_timeout: Duration::from_secs(2),
        ..PluginProcessOptions::default()
    }
}

async fn ready_process(plugin_id: &str, instance_id: &str) -> PluginProcess {
    let process = PluginProcess::spawn(&descriptor(plugin_id), options())
        .unwrap();
    let initialized = process
        .client()
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "client-test".to_string(),
            host_device_id: "device-test".to_string(),
            host_version: "test".to_string(),
            supported_versions: VersionRange {
                min_version: 1,
                max_version: 1,
            },
        })
        .await
        .unwrap();
    assert_eq!(initialized.plugin.plugin_id, plugin_id);
    let described = process
        .client()
        .provider_describe(ProviderDescribeRequest {})
        .await
        .unwrap();
    assert_eq!(described.plugin, initialized.plugin);
    let route = ProviderInstanceRoute {
        device_id: "device-test".to_string(),
        provider_plugin_id: plugin_id.to_string(),
        provider_instance_id: instance_id.to_string(),
    };
    process
        .client()
        .instance_create(InstanceCreateRequest {
            route: route.clone(),
            instance_kind: "fake".to_string(),
            display_name: "Fake Instance".to_string(),
            settings: JsonObject::new(),
        })
        .await
        .unwrap();
    process
        .client()
        .instance_start(InstanceStartRequest { route })
        .await
        .unwrap();
    process
}

fn conversation(plugin_id: &str, instance_id: &str, native_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        device_id: "device-test".to_string(),
        provider_plugin_id: plugin_id.to_string(),
        provider_instance_id: instance_id.to_string(),
        native_resource_id: native_id.to_string(),
    }
}

#[tokio::test]
async fn real_stdio_lifecycle_correlates_concurrent_responses_and_separates_events() {
    let process = ready_process("dev.codepet.concurrent", "instance-concurrent").await;
    let mut inbound = process.take_inbound().await.unwrap();
    let slow = process
        .client()
        .conversation_get(ConversationGetRequest {
            conversation: conversation("dev.codepet.concurrent", "instance-concurrent", "slow"),
        });
    let fast = process
        .client()
        .conversation_get(ConversationGetRequest {
            conversation: conversation("dev.codepet.concurrent", "instance-concurrent", "fast"),
        });
    let (slow, fast) = tokio::join!(slow, fast);
    assert_eq!(
        slow.unwrap().conversation.resource.native_resource_id,
        "slow"
    );
    assert_eq!(
        fast.unwrap().conversation.resource.native_resource_id,
        "fast"
    );

    let response = process
        .client()
        .conversation_get(ConversationGetRequest {
            conversation: conversation(
                "dev.codepet.concurrent",
                "instance-concurrent",
                "event-first",
            ),
        })
        .await
        .unwrap();
    assert_eq!(
        response.conversation.resource.native_resource_id,
        "event-first"
    );
    let first = inbound.recv().await.unwrap();
    let second = inbound.recv().await.unwrap();
    let third = inbound.recv().await.unwrap();
    assert!(matches!(
        first,
        codepet_provider_sdk::ProviderWireMessage::Event(
            ProtocolEvent::EventConversationUpserted { .. }
        )
    ));
    assert!(matches!(
        second,
        codepet_provider_sdk::ProviderWireMessage::Notification(_)
    ));
    assert!(matches!(
        third,
        codepet_provider_sdk::ProviderWireMessage::Event(
            ProtocolEvent::EventTurnOutputDelta { .. }
        )
    ));
    let route = ProviderInstanceRoute {
        device_id: "device-test".to_string(),
        provider_plugin_id: "dev.codepet.concurrent".to_string(),
        provider_instance_id: "instance-concurrent".to_string(),
    };
    let stopped = process
        .client()
        .instance_stop(InstanceStopRequest {
            route: route.clone(),
        })
        .await
        .unwrap();
    assert_eq!(stopped.instance.status, InstanceStatus::Stopped);
    let restarted = process
        .client()
        .instance_start(InstanceStartRequest {
            route: route.clone(),
        })
        .await
        .unwrap();
    assert_eq!(restarted.instance.status, InstanceStatus::Ready);
    let destroyed = process
        .client()
        .instance_destroy(InstanceDestroyRequest { route })
        .await
        .unwrap();
    assert!(destroyed.destroyed);
    assert!(process.shutdown().await.unwrap().success);
}

#[tokio::test]
async fn shutdown_waits_for_a_provider_that_closes_stdout_before_delayed_clean_exit() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("shutdown-received");
    let stdout_closed = directory.path().join("stdout-closed");
    let mut descriptor = descriptor("dev.codepet.delayed-shutdown");
    descriptor.env.insert(
        "CODEPET_FAKE_SHUTDOWN_DELAY_MS".to_string(),
        "200".to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_SHUTDOWN_MARKER".to_string(),
        marker.display().to_string(),
    );
    descriptor.env.insert(
        "CODEPET_FAKE_STDOUT_CLOSED_MARKER".to_string(),
        stdout_closed.display().to_string(),
    );
    let process = Arc::new(PluginProcess::spawn(
        &descriptor,
        PluginProcessOptions {
            shutdown_timeout: Duration::from_secs(1),
            ..options()
        },
    )
    .unwrap());
    process
        .client()
        .provider_initialize(ProviderInitializeRequest {
            host_client_id: "client-test".to_string(),
            host_device_id: "device-test".to_string(),
            host_version: "test".to_string(),
            supported_versions: VersionRange {
                min_version: 1,
                max_version: 1,
            },
        })
        .await
        .unwrap();

    let shutdown_process = process.clone();
    let shutdown = tokio::spawn(async move { shutdown_process.shutdown().await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !stdout_closed.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(marker.exists());
    assert!(!shutdown.is_finished(), "stdout EOF must not complete shutdown");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !shutdown.is_finished(),
        "Provider remains alive during its configured clean-exit delay"
    );
    let exit = shutdown.await.unwrap().unwrap();
    assert!(exit.success, "delayed clean exit must not be killed: {exit:?}");
    assert!(process
        .stderr_diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.line == "fixture shutdown stderr tail"));
}

#[tokio::test]
async fn timeout_does_not_poison_later_requests() {
    let process = ready_process("dev.codepet.timeout", "instance-timeout").await;
    let error = process
        .client()
        .conversation_get(ConversationGetRequest {
            conversation: conversation("dev.codepet.timeout", "instance-timeout", "timeout"),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "provider_request_timeout");

    let recovered = process
        .client()
        .conversation_get(ConversationGetRequest {
            conversation: conversation(
                "dev.codepet.timeout",
                "instance-timeout",
                "after-timeout",
            ),
        })
        .await
        .unwrap();
    assert_eq!(
        recovered.conversation.resource.native_resource_id,
        "after-timeout"
    );
    process.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_oversized_and_crashed_plugins_close_only_their_process() {
    for native_id in ["malformed", "oversized", "crash"] {
        let instance_id = format!("instance-{native_id}");
        let process = ready_process(&format!("dev.codepet.{native_id}"), &instance_id).await;
        let healthy = ready_process(
            &format!("dev.codepet.healthy-{native_id}"),
            &format!("healthy-{native_id}"),
        )
        .await;
        let error = process
            .client()
            .conversation_get(ConversationGetRequest {
                conversation: conversation(
                    &format!("dev.codepet.{native_id}"),
                    &instance_id,
                    native_id,
                ),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error.code.as_str(),
            "provider_invalid_frame"
                | "provider_frame_too_large"
                | "provider_stdout_eof"
                | "provider_process_exited"
        ));

        let healthy_instance = format!("healthy-{native_id}");
        let response = healthy
            .client()
            .conversation_get(ConversationGetRequest {
                conversation: conversation(
                    &format!("dev.codepet.healthy-{native_id}"),
                    &healthy_instance,
                    "healthy",
                ),
            })
            .await
            .unwrap();
        assert_eq!(response.conversation.resource.native_resource_id, "healthy");
        healthy.shutdown().await.unwrap();
    }
}
