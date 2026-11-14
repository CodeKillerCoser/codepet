//! Opt-in test: scripts/test_opencode_observation.py --remote-turns supplies isolated
//! OpenCode config and a localhost model fixture. It never uses the user's models.
use codepet_provider_opencode::{OpenCodeProvider, OPENCODE_INSTANCE_KIND, OPENCODE_PLUGIN_ID};
use codepet_provider_sdk::*;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

#[tokio::test]
#[ignore = "requires isolated OpenCode and a local model fixture"]
async fn native_remote_turns_keep_the_instance_ready() {
    let executable = std::env::var("CODEPET_OPENCODE_EXECUTABLE").unwrap();
    let workspace =
        std::env::var("CODEPET_OPENCODE_TEST_WORKSPACE").expect("isolated workspace required");
    let (sender, receiver) = mpsc::channel();
    let provider = Arc::new(OpenCodeProvider::new(Arc::new(move |event| {
        let _ = sender.send(event);
        Ok(())
    })));
    let owner = provider.clone();
    let result = tokio::spawn(async move {
        let provider = owner;
        provider
            .provider_initialize(ProviderInitializeRequest {
                directories: None,
                host_client_id: "native-smoke".into(),
                host_device_id: "native-smoke-device".into(),
                host_version: "0.1.0".into(),
                protocol: ProtocolVersionOffer {
                    min_version: PROTOCOL_VERSION,
                    version: PROTOCOL_VERSION,
                },
            })
            .await
            .unwrap();
        let route = ProviderInstanceRoute {
            device_id: "native-smoke-device".into(),
            provider_plugin_id: OPENCODE_PLUGIN_ID.into(),
            provider_instance_id: "opencode".into(),
        };
        provider
            .instance_create(InstanceCreateRequest {
                route: route.clone(),
                instance_kind: OPENCODE_INSTANCE_KIND.into(),
                display_name: "Native smoke".into(),
                settings: BTreeMap::from([
                    ("serverExecutable".into(), json!(executable)),
                    (
                        "serverVersion".into(),
                        json!(std::env::var("CODEPET_OPENCODE_TEST_VERSION").unwrap()),
                    ),
                    ("serverArgs".into(), json!(["serve"])),
                    ("workspaceRoot".into(), json!(workspace)),
                ]),
            })
            .await
            .unwrap();
        let started = tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                let snapshot = provider
                    .instance_start(InstanceStartRequest {
                        route: route.clone(),
                    })
                    .await
                    .unwrap();
                if snapshot.instance.status == InstanceStatus::Ready {
                    break snapshot;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("instance did not become Ready");
        let conversation = provider
            .conversation_create(ConversationCreateRequest {
                route: route.clone(),
                project: None,
                title: None,
                permission_level: "build".into(),
                model: Some("codepet-smoke/smoke".into()),
                reasoning_effort: Some("default".into()),
                workspace_root: Some(workspace.clone()),
                workspace_mode: None,
                extension: None,
            })
            .await
            .unwrap()
            .conversation;
        let resource = ProviderResourceId {
            device_id: route.device_id.clone(),
            provider_plugin_id: route.provider_plugin_id.clone(),
            provider_instance_id: route.provider_instance_id.clone(),
            native_resource_id: conversation.resource.native_resource_id.clone(),
        };
        let listing = provider
            .conversation_list(ConversationListRequest {
                route: route.clone(),
                query: None,
                reader_scope: None,
                cursor: None,
                limit: Some(10),
                project_filter: serde_json::from_value(json!({"kind":"all"})).unwrap(),
            })
            .await
            .unwrap();
        assert!(listing
            .conversations
            .iter()
            .any(|item| item.resource == conversation.resource));
        assert!(started
            .instance
            .capabilities
            .turn_send
            .as_ref()
            .unwrap()
            .model_catalog
            .is_some());
        let decisions = if std::env::var_os("CODEPET_OPENCODE_TEST_DENY").is_some() {
            vec![
                ApprovalDecision::Approve,
                ApprovalDecision::Deny,
                ApprovalDecision::Approve,
            ]
        } else {
            vec![ApprovalDecision::Approve, ApprovalDecision::Approve]
        };
        for (index, decision) in decisions.into_iter().enumerate() {
            let denied = decision == ApprovalDecision::Deny;
            let marker = std::path::Path::new(&workspace).join("codepet-smoke-executed");
            if marker.exists() {
                std::fs::remove_file(&marker).unwrap();
            }
            let mut approvals = 0;
            let mut deltas = Vec::new();
            let mut user_items = Vec::new();
            let mut steered = false;
            let response = provider
                .turn_start(TurnStartRequest {
                    conversation: resource.clone(),
                    client_request_id: format!("native-turn-{index}"),
                    capability_revision: started.instance.capabilities.revision.clone(),
                    input: TurnInput {
                        kind: TurnInputKind::Text,
                        text: "remote-smoke".into(),
                    },
                    selection: TurnSelection {
                        access_mode_id: None,
                        reasoning_effort_id: None,
                        model: None,
                    },
                })
                .await
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let event = receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .expect("turn did not finish");
                match event {
                    ProtocolEvent::EventInstanceStatusChanged { params, .. } => assert_ne!(
                        params.instance.status,
                        InstanceStatus::Error,
                        "native instance failed while sending"
                    ),
                    ProtocolEvent::EventTurnUpserted { params, .. }
                        if params.turn.resource == response.turn.resource
                            && matches!(
                                params.turn.status,
                                TurnStatus::Completed
                                    | TurnStatus::Failed
                                    | TurnStatus::Interrupted
                            ) =>
                    {
                        if !denied {
                            assert_eq!(params.turn.status, TurnStatus::Completed);
                        } else {
                            assert!(matches!(
                                params.turn.status,
                                TurnStatus::Completed | TurnStatus::Failed
                            ));
                        }
                        assert!(params.turn.started_at.is_some());
                        break;
                    }
                    ProtocolEvent::EventConversationItemUpserted { params, .. } => {
                        if let Some(ConversationItem::MessageConversationItem(item)) = params.item {
                            if item.role == ConversationItemRole::User && item.turn == response.turn.resource {
                                user_items.push(item);
                            }
                        }
                    }
                    ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                        deltas.push((params.content_id, params.delta));
                    }
                    ProtocolEvent::EventApprovalRequested { params, .. } => {
                        approvals += 1;
                        if index == 0 && !steered {
                            let mut turn = resource.clone();
                            turn.native_resource_id = response.turn.resource.native_resource_id.clone();
                            provider.turn_steer(TurnSteerRequest {
                                conversation: resource.clone(), turn,
                                client_message_id: "native-steer".into(), message: "steer-smoke".into(),
                            }).await.unwrap();
                            steered = true;
                        }

                        provider
                            .approval_resolve(ApprovalResolveRequest {
                                approval: ProviderResourceId {
                                    device_id: route.device_id.clone(),
                                    provider_plugin_id: route.provider_plugin_id.clone(),
                                    provider_instance_id: route.provider_instance_id.clone(),
                                    native_resource_id: params.approval.resource.native_resource_id,
                                },
                                decision: decision.clone(),
                            })
                            .await
                            .unwrap();
                    }
                    _ => {}
                }
            }
            let snapshot = provider
                .conversation_get(ConversationGetRequest {
                    conversation: resource.clone(),
                    cursor: None,
                    limit: Some(100),
                })
                .await
                .unwrap();
            assert!(!snapshot.items.is_empty());
            let json = serde_json::to_value(snapshot.items).unwrap();
            fn collect(value: &serde_json::Value, ids: &mut std::collections::HashSet<String>) {
                match value {
                    serde_json::Value::Object(map) => {
                        if let Some(id) = map.get("contentId").and_then(serde_json::Value::as_str) {
                            assert!(ids.insert(id.into()), "duplicate content identity {id}");
                        }
                        for v in map.values() {
                            collect(v, ids);
                        }
                    }
                    serde_json::Value::Array(values) => {
                        for v in values {
                            collect(v, ids);
                        }
                    }
                    _ => {}
                }
            }
            let mut ids = std::collections::HashSet::new();
            collect(&json, &mut ids);
            assert_eq!(
                marker.exists(),
                !denied,
                "permission decision did not control tool execution"
            );
            assert!(approvals > 0, "turn {index} never requested permission");
            {
                assert_eq!(user_items.len(), if steered { 2 } else { 1 }, "every native delivered input must be streamed once");
                let Some(ConversationItem::MessageConversationItem(receipt)) = &response.user_item else {
                    panic!("expected canonical user receipt");
                };
                assert_eq!(user_items[0].resource, receipt.resource);
                assert_eq!(user_items[0].contents, receipt.contents);
                if steered {
                    assert_ne!(user_items[0].resource, user_items[1].resource);
                    assert!(serde_json::to_string(&user_items[1]).unwrap().contains("steer-smoke"));
                }
            }

            if !denied {
                assert!(!deltas.is_empty(), "turn {index} emitted no streaming text");
                assert!(deltas.iter().any(|(_, text)| text.contains("Smoke reasoning")),
                    "the local fixture must exercise reasoning and text together");
                assert_eq!(deltas.iter().map(|(id, _)| id).collect::<std::collections::HashSet<_>>().len(), 2);
                assert!(deltas
                    .iter()
                    .any(|(_, text)| text.contains("Smoke complete")));
            }
            for (id, _) in &deltas {
                assert!(
                    ids.contains(id),
                    "stream content {id} is absent from history"
                );
            }
            if index == 0 {
                assert!(
                    json.to_string().contains("codepet-smoke"),
                    "approved tool output absent"
                );
            }
            println!(
                "turn {index}: approvals={approvals}, deltas={}, unique history contents={}",
                deltas.len(),
                ids.len()
            );
            provider
                .conversation_acquire_interaction(ConversationAcquireInteractionRequest {
                    force: None,
                    conversation: resource.clone(),
                })
                .await
                .unwrap();
        }
        if std::env::var_os("CODEPET_OPENCODE_TEST_ERROR").is_some() {
            let failed = provider.turn_start(TurnStartRequest {
                conversation: resource.clone(), client_request_id: "terminal-error-smoke".into(),
                capability_revision: started.instance.capabilities.revision.clone(),
                input: TurnInput { kind: TurnInputKind::Text, text: "terminal-error-smoke".into() },
                selection: TurnSelection { access_mode_id: None, reasoning_effort_id: None, model: None },
            }).await.unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let event = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("native failure missing");
                if let ProtocolEvent::EventTurnUpserted { params, .. } = event {
                    if params.turn.resource == failed.turn.resource && params.turn.status == TurnStatus::Failed {
                        let error = params.turn.error.as_ref().expect("structured native error missing");
                        assert!(error.message.contains("CodePet test quota exhausted"), "native failure reason lost: {error:?}");
                        assert!(!error.retryable);
                        assert_eq!(params.turn.display_summary.as_deref(), Some(error.message.as_str()));
                        println!("native terminal failure preserved: {} / {}", error.code, error.message);
                        break;
                    }
                }
            }
            let history = provider.conversation_get(ConversationGetRequest { conversation: resource.clone(), cursor: None, limit: Some(100) }).await.unwrap();
            assert!(serde_json::to_string(&history).unwrap().contains("CodePet test quota exhausted"));
        }
        provider
            .instance_stop(InstanceStopRequest { route })
            .await
            .unwrap();
    })
    .await;
    provider
        .provider_shutdown(ProviderShutdownRequest {})
        .await
        .unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}
