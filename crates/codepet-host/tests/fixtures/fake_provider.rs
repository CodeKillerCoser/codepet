use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ApprovalResolveResponse, ApprovalStatus,
    ChoiceOption, ChoiceSet, ConversationAcquireInteractionRequest,
    ConversationAcquireInteractionResponse, ConversationCreateRequest, ConversationCreateResponse, ConversationGetRequest,
    ConversationGetResponse, ConversationListRequest, ConversationListResponse,
    ConversationSearchRequest, ConversationSearchResponse, ContentBlock,
    ConversationContentKind, ConversationCreateCapabilities, ConversationItem,
    ConversationItemRole, ConversationItemStatus, ConversationItemUpsertedEvent, ConversationStatus,
    ConversationUpsertedEvent,
    InstanceCapabilitiesRequest,
    InstanceCapabilitiesResponse, InstanceCreateRequest, InstanceCreateResponse,
    InstanceDestroyRequest, InstanceDestroyResponse, InstanceStartRequest,
    InstanceStartResponse, InstanceStatus, InstanceStopRequest, InstanceStopResponse,
    FlatModelCatalog, FlatModelCatalogKind, FlatModelSelection, HarnessDescriptor,
    JsonObject, ModelCatalog, ModelSelection, PageInfo, ProtocolEvent,
    Project, ProjectChangeType, ProjectChangedEvent, ProjectCreateRequest, ProjectCreateResponse, ProjectDeleteRequest,
    ProjectDeleteResponse, ProjectGetRequest, ProjectGetResponse, ProjectListRequest,
    ProjectListResponse, ProjectRoot, ProjectUpdateRequest, ProjectUpdateResponse, ProtocolFuture,
    ProtocolServer, Approval, ProviderCapabilities,
    ProviderCapability, Conversation, ProviderDescribeRequest,
    ProviderDescribeResponse, ProviderInitializeRequest, ProviderInitializeResponse,
    ProviderAuthentication, ProviderAuthenticationStatus, ProviderInstance, ProviderInstanceRoute,
    ProviderPluginDescriptor, ProviderResourceId, ProviderShutdownRequest, ProviderShutdownResponse,
    TurnTask, ProviderUsage, ProviderUsageDetail, RoutedResourceId,
    MessageConversationItem, MessageConversationItemKind, TextContentBlock, TextContentBlockKind,
    TurnInterruptRequest, TurnInterruptResponse, TurnOutputDeltaEvent, TurnSendCapabilities,
    TurnSelection, TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest,
    TurnSteerResponse, VersionRange,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct FakeProvider {
    events: Option<Arc<dyn codepet_provider_sdk::ProviderEventSink>>,
    plugin_id: String,
    observation: Mutex<Option<(String, Arc<std::sync::atomic::AtomicBool>)>>,
    instances: Mutex<BTreeMap<String, ProviderInstance>>,
    instance_settings: Mutex<BTreeMap<String, JsonObject>>,
}

impl FakeProvider {
    fn descriptor(&self) -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            plugin_id: self.plugin_id.clone(),
            display_name: "CodePet Fake Provider".to_string(),
            version: "1.0.0-test".to_string(),
            default_workspace_root: Some("/workspace/fake".to_string()),
            supported_versions: VersionRange {
                min_version: env_u32("CODEPET_FAKE_SUPPORTED_MIN_VERSION", 1),
                max_version: env_u32("CODEPET_FAKE_SUPPORTED_MAX_VERSION", 1),
            },
            instance_kinds: vec!["fake".to_string()],
        }
    }

    fn instance(&self, route: &ProviderInstanceRoute) -> Result<ProviderInstance, codepet_provider_sdk::ProtocolError> {
        self.instances
            .lock()
            .ok()
            .and_then(|instances| instances.get(&route.provider_instance_id).cloned())
            .ok_or_else(|| protocol_error(
                "unknown_fake_instance",
                format!("fake instance is not registered: {}", route.provider_instance_id),
            ))
    }
}

impl ProtocolServer for FakeProvider {
    fn provider_initialize<'a>(
        &'a self,
        _request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        let descriptor = self.descriptor();
        let delay = env_u64("CODEPET_FAKE_INITIALIZE_DELAY_MS", 0);
        let marker = std::env::var("CODEPET_FAKE_INITIALIZE_MARKER").ok();
        Box::pin(async move {
            if let Some(marker) = marker {
                let _ = std::fs::write(marker, b"initialize received\n");
            }
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Ok(ProviderInitializeResponse {
                selected_version: env_u32("CODEPET_FAKE_SELECTED_VERSION", 1),
                plugin: descriptor,
            })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        _request: ProviderDescribeRequest,
    ) -> ProtocolFuture<'a, ProviderDescribeResponse> {
        let descriptor = self.descriptor();
        Box::pin(async move {
            if std::env::var_os("CODEPET_FAKE_MUX_EVENTS").is_some() {
                if let Some(events) = &self.events {
                    for i in 0..8 {
                        events.publish(ProtocolEvent::EventProjectChanged { jsonrpc: "2.0".into(), params: ProjectChangedEvent {
                            project: ProviderResourceId { device_id: "test".into(), provider_plugin_id: self.plugin_id.clone(), provider_instance_id: "test".into(), native_resource_id: i.to_string() }, change_type: ProjectChangeType::Updated,
                        } })?;
                    }
                }
            }
            Ok(ProviderDescribeResponse { plugin: descriptor })
        })
    }

    fn event_subscribe<'a>(&'a self, request: codepet_provider_sdk::EventSubscribeRequest) -> ProtocolFuture<'a, codepet_provider_sdk::EventSubscribeResponse> {
        Box::pin(async move {
            let mut observation = self.observation.lock().unwrap();
            if observation.is_some() { return Err(protocol_error("duplicate_wire_subscription", "Host must fan out locally")); }
            let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
            *observation = Some((request.subscription_id.clone(), active.clone()));
            let events = self.events.clone().unwrap();
            let id = request.subscription_id.clone();
            tokio::spawn(async move {
                let mut sequence = 0u64;
                while active.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    sequence += 1;
                    let event = codepet_provider_sdk::ProviderNotificationEvent {
                        subscription_id: id.clone(), event_id: sequence.to_string(), received_at: sequence,
                        payload: serde_json::json!({"hook_event_name":"UserPromptSubmit", "session_id":"global-session", "prompt":"Observed outside Remote"}).as_object().unwrap().clone().into_iter().collect(),
                    };
                    if events.publish(ProtocolEvent::EventNotification { jsonrpc: "2.0".into(), params: event }).is_err() { break; }
                }
            });
            Ok(codepet_provider_sdk::EventSubscribeResponse { subscription_id: request.subscription_id, message: "fixture installed".into() })
        })
    }
    fn event_unsubscribe<'a>(&'a self, request: codepet_provider_sdk::EventUnsubscribeRequest) -> ProtocolFuture<'a, codepet_provider_sdk::EventUnsubscribeResponse> {
        Box::pin(async move {
            let mut observation = self.observation.lock().unwrap();
            if observation.as_ref().is_some_and(|(id, _)| id == &request.subscription_id) {
                observation.take().unwrap().1.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(codepet_provider_sdk::EventUnsubscribeResponse { subscription_id: request.subscription_id })
        })
    }

    fn instance_create<'a>(
        &'a self,
        request: InstanceCreateRequest,
    ) -> ProtocolFuture<'a, InstanceCreateResponse> {
        Box::pin(async move {
            let instance_id = request.route.provider_instance_id.clone();
            let instance = ProviderInstance {
                route: request.route,
                plugin_id: self.plugin_id.clone(),
                instance_kind: request.instance_kind,
                display_name: request.display_name,
                harness: HarnessDescriptor {
                    id: "fake-harness".to_string(),
                    display_name: "Fake Harness".to_string(),
                    version: Some("1.0.0-fixture".to_string()),
                    executable_path: Some("/fixture/fake-harness".to_string()),
                },
                status: InstanceStatus::Created,
                authentication: Some(ProviderAuthentication {
                    status: ProviderAuthenticationStatus::SignedIn,
                    display_text: Some("Signed in to fixture".to_string()),
                }),
                usage: Some(ProviderUsage {
                    display_text: "Fixture usage 42%".to_string(),
                    observed_at: Some(1_788_450_000_000),
                    details: Some(vec![ProviderUsageDetail {
                        namespace: "dev.codepet.fixture.usage".to_string(),
                        schema_version: "1".to_string(),
                        data: BTreeMap::from([
                            ("usedPercent".to_string(), serde_json::json!(42)),
                            ("accessToken".to_string(), serde_json::json!("must-not-leak")),
                            ("nested".to_string(), serde_json::json!({"cookie": "must-not-leak", "safe": true})),
                        ]),
                    }]),
                }),
                capabilities: capabilities(),
            };
            self.instances
                .lock()
                .map_err(|_| protocol_error("fake_state_error", "fake instance lock failed"))?
                .insert(instance.route.provider_instance_id.clone(), instance.clone());
            self.instance_settings
                .lock()
                .map_err(|_| protocol_error("fake_state_error", "fake settings lock failed"))?
                .insert(instance_id, request.settings);
            Ok(InstanceCreateResponse { instance })
        })
    }

    fn instance_start<'a>(
        &'a self,
        request: InstanceStartRequest,
    ) -> ProtocolFuture<'a, InstanceStartResponse> {
        Box::pin(async move {
            if let Ok(path) = std::env::var("CODEPET_FAKE_INSTANCE_START_MARKER") {
                if let Ok(mut marker) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(marker, "{}", request.route.provider_instance_id);
                }
            }
            if std::env::var("CODEPET_FAKE_INSTANCE_START_ERROR_ID").as_deref()
                == Ok(request.route.provider_instance_id.as_str())
            {
                return Err(protocol_error(
                    "fixture_instance_start_failed",
                    "fixture rejected instance.start",
                ));
            }
            let mut instance = self.instance(&request.route)?;
            instance.status = InstanceStatus::Ready;
            self.instances
                .lock()
                .map_err(|_| protocol_error("fake_state_error", "fake instance lock failed"))?
                .insert(instance.route.provider_instance_id.clone(), instance.clone());
            Ok(InstanceStartResponse { instance })
        })
    }

    fn instance_stop<'a>(
        &'a self,
        request: InstanceStopRequest,
    ) -> ProtocolFuture<'a, InstanceStopResponse> {
        Box::pin(async move {
            let mut instance = self.instance(&request.route)?;
            instance.status = InstanceStatus::Stopped;
            self.instances
                .lock()
                .map_err(|_| protocol_error("fake_state_error", "fake instance lock failed"))?
                .insert(instance.route.provider_instance_id.clone(), instance.clone());
            Ok(InstanceStopResponse { instance })
        })
    }

    fn instance_destroy<'a>(
        &'a self,
        request: InstanceDestroyRequest,
    ) -> ProtocolFuture<'a, InstanceDestroyResponse> {
        Box::pin(async move {
            let destroyed = self
                .instances
                .lock()
                .map_err(|_| protocol_error("fake_state_error", "fake instance lock failed"))?
                .remove(&request.route.provider_instance_id)
                .is_some();
            Ok(InstanceDestroyResponse { destroyed })
        })
    }

    fn instance_capabilities<'a>(
        &'a self,
        request: InstanceCapabilitiesRequest,
    ) -> ProtocolFuture<'a, InstanceCapabilitiesResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            Ok(InstanceCapabilitiesResponse {
                capabilities: capabilities(),
            })
        })
    }

    fn project_list<'a>(
        &'a self,
        request: ProjectListRequest,
    ) -> ProtocolFuture<'a, ProjectListResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            Ok(ProjectListResponse {
                projects: vec![project(&request.route, "project-list", "Listed Project")],
                page_info: PageInfo {
                    next_cursor: Some("project-next".to_string()),
                },
            })
        })
    }

    fn project_get<'a>(
        &'a self,
        request: ProjectGetRequest,
    ) -> ProtocolFuture<'a, ProjectGetResponse> {
        Box::pin(async move {
            let route = route_from_resource(&request.project);
            self.instance(&route)?;
            if request.project.native_resource_id == "event-project" {
                self.events.as_ref().unwrap().publish(ProtocolEvent::EventProjectChanged { jsonrpc: "2.0".into(),
                    params: ProjectChangedEvent { project: request.project.clone(), change_type: ProjectChangeType::Updated } })?;
            }
            Ok(ProjectGetResponse {
                project: project(&route, &request.project.native_resource_id, "Fetched Project"),
            })
        })
    }

    fn project_create<'a>(
        &'a self,
        request: ProjectCreateRequest,
    ) -> ProtocolFuture<'a, ProjectCreateResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            let mut created = project(&request.route, "project-created", &request.name);
            created.roots = request.roots;
            created.metadata = request.metadata;
            Ok(ProjectCreateResponse { project: created })
        })
    }

    fn project_update<'a>(
        &'a self,
        request: ProjectUpdateRequest,
    ) -> ProtocolFuture<'a, ProjectUpdateResponse> {
        Box::pin(async move {
            let route = route_from_resource(&request.project);
            self.instance(&route)?;
            let mut updated = project(
                &route,
                &request.project.native_resource_id,
                request.name.as_deref().unwrap_or("Updated Project"),
            );
            if let Some(roots) = request.roots {
                updated.roots = roots;
            }
            if let Some(metadata) = request.metadata {
                updated.metadata = metadata;
            }
            Ok(ProjectUpdateResponse { project: updated })
        })
    }

    fn project_delete<'a>(
        &'a self,
        request: ProjectDeleteRequest,
    ) -> ProtocolFuture<'a, ProjectDeleteResponse> {
        Box::pin(async move {
            self.instance(&route_from_resource(&request.project))?;
            Ok(ProjectDeleteResponse {})
        })
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            if std::env::var("CODEPET_FAKE_CONVERSATION_LIST_SNAPSHOT_RACE").as_deref() == Ok("1") {
                self.publish_conversation(&request.route, "conversation-list-event-first")?;
                wait_for_snapshot_release().await;
            }
            let mut listed = conversation(&request.route, "conversation-list");
            if let codepet_provider_sdk::ConversationProjectFilter::ConversationProjectFilterProject(
                filter,
            ) = request.project_filter
            {
                listed.project = Some(resource(
                    &request.route,
                    &filter.project.native_resource_id,
                ));
            }
            Ok(ConversationListResponse {
                conversations: vec![listed],
                page_info: PageInfo { next_cursor: None },
            })
        })
    }

    fn conversation_search<'a>(
        &'a self,
        request: ConversationSearchRequest,
    ) -> ProtocolFuture<'a, ConversationSearchResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            if request.search_term != "gateway protocol"
                || request.cursor.as_deref() != Some("search-cursor")
                || request.limit != Some(7)
            {
                return Err(protocol_error(
                    "fixture_search_params_mismatch",
                    "conversation.search params were not preserved",
                ));
            }
            Ok(ConversationSearchResponse {
                conversations: vec![conversation(&request.route, "conversation-search")],
                page_info: PageInfo {
                    next_cursor: Some("search-next".to_string()),
                },
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProtocolFuture<'a, ConversationGetResponse> {
        Box::pin(async move {
            let route = route_from_resource(&request.conversation);
            self.instance(&route)?;
            self.before_conversation(&route, &request.conversation.native_resource_id).await?;
            let native_id = match request.conversation.native_resource_id.as_str() {
                "response-wrong-native" => "different-native-id",
                "response-empty-native" => "",
                native_id => native_id,
            };
            let response_route = if request.conversation.native_resource_id == "response-wrong-route" {
                ProviderInstanceRoute {
                    device_id: route.device_id.clone(),
                    provider_plugin_id: route.provider_plugin_id.clone(),
                    provider_instance_id: "instance-other".to_string(),
                }
            } else {
                route
            };
            let configured_revision = self
                .instance_settings
                .lock()
                .map_err(|_| protocol_error("fake_state_error", "fake settings lock failed"))?
                .get(&response_route.provider_instance_id)
                .and_then(|settings| settings.get("fixtureRevision"))
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let mut configured = conversation(&response_route, native_id);
            if let Some(configured_revision) = configured_revision {
                configured.preview = Some(configured_revision);
            }
            if request.conversation.native_resource_id == "compressed" {
                configured.preview = Some("provider-frame-v1-".repeat(4_096));
            }
            Ok(ConversationGetResponse {
                conversation: configured,
                items: history_items(&response_route, native_id),
                page_info: None,
            })
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: ConversationAcquireInteractionRequest,
    ) -> ProtocolFuture<'a, ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            self.instance(&route_from_resource(&request.conversation))?;
            if request.conversation.native_resource_id == "resume-denied" {
                return Err(protocol_error("conversation_write_conflict", "fixture writer held"));
            }
            Ok(ConversationAcquireInteractionResponse {
                selection: TurnSelection {
                    access_mode_id: Some("workspace-write".to_string()),
                    reasoning_effort_id: Some("high".to_string()),
                    model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                        kind: FlatModelCatalogKind::Flat,
                        model_id: "model-a".to_string(),
                    })),
                },
                lease_expires_at: Some(2_000),
            })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProtocolFuture<'a, ConversationCreateResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            let mut created = conversation(&request.route, "conversation-created");
            created.project = request
                .project
                .map(|project| resource(&request.route, &project.native_resource_id));
            Ok(ConversationCreateResponse {
                conversation: created,
            })
        })
    }

    fn turn_start<'a>(
        &'a self,
        request: TurnStartRequest,
    ) -> ProtocolFuture<'a, TurnStartResponse> {
        Box::pin(async move {
            if let Ok(path) = std::env::var("CODEPET_FAKE_TURN_START_MARKER") {
                if let Ok(mut marker) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(marker, "{}", request.client_request_id);
                }
            }
            let route = route_from_resource(&request.conversation);
            self.instance(&route)?;
            let wrong_user_item_conversation = request.conversation.native_resource_id
                == "response-wrong-user-item-conversation";
            let conversation = if request.conversation.native_resource_id
                == "response-wrong-conversation"
            {
                resource(&route, "different-conversation")
            } else {
                resource(&route, &request.conversation.native_resource_id)
            };
            let turn = turn(&route, "turn-started", conversation.clone());
            let user_item_conversation = if wrong_user_item_conversation {
                resource(&route, "different-user-item-conversation")
            } else {
                conversation
            };
            let user_item = ConversationItem::MessageConversationItem(MessageConversationItem { meta: None,
                resource: resource(&route, &format!("user-{}", request.client_request_id)),
                turn: turn.resource.clone(),
                conversation: user_item_conversation,
                kind: MessageConversationItemKind::Message,
                status: ConversationItemStatus::Completed,
                role: ConversationItemRole::User,
                contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                    content_id: format!("{}:input:0", request.client_request_id),
                    kind: TextContentBlockKind::Text,
                    text: request.input.text,
                    truncation: None,
                })],
            });
            Ok(TurnStartResponse {
                accepted: true,
                turn,
                user_item: Some(user_item),
                effective_selection: request.selection,
            })
        })
    }

    fn turn_steer<'a>(
        &'a self,
        request: TurnSteerRequest,
    ) -> ProtocolFuture<'a, TurnSteerResponse> {
        Box::pin(async move {
            let route = route_from_resource(&request.turn);
            self.instance(&route)?;
            let conversation = if request.turn.native_resource_id == "steer-wrong-conversation" {
                resource(&route, "conversation-b")
            } else {
                resource(&route, &request.conversation.native_resource_id)
            };
            Ok(TurnSteerResponse {
                turn: turn(
                    &route,
                    &request.turn.native_resource_id,
                    conversation,
                ),
            })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProtocolFuture<'a, TurnInterruptResponse> {
        Box::pin(async move {
            let route = route_from_resource(&request.turn);
            self.instance(&route)?;
            let mut interrupted = turn(
                &route,
                &request.turn.native_resource_id,
                resource(&route, &request.conversation.native_resource_id),
            );
            interrupted.status = TurnStatus::Interrupted;
            Ok(TurnInterruptResponse { turn: interrupted })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProtocolFuture<'a, ApprovalResolveResponse> {
        Box::pin(async move {
            let route = route_from_resource(&request.approval);
            self.instance(&route)?;
            Ok(ApprovalResolveResponse {
                approval: Approval {
                    resource: resource(&route, &request.approval.native_resource_id),
                    conversation: resource(&route, "conversation-event-first"),
                    turn: resource(&route, "turn-event-first"),
                    kind: "command-execution".to_string(),
                    title: "Run test command".to_string(),
                    description: None,
                    status: match request.decision {
                        ApprovalDecision::Approve => ApprovalStatus::Approved,
                        ApprovalDecision::Deny => ApprovalStatus::Denied,
                    },
                    decisions: vec![ApprovalDecision::Approve, ApprovalDecision::Deny],
                    requested_at: Some(1),
                    resolved_at: Some(2),
                    decision: Some(request.decision),
                },
            })
        })
    }

    fn provider_shutdown<'a>(
        &'a self,
        _request: ProviderShutdownRequest,
    ) -> ProtocolFuture<'a, ProviderShutdownResponse> {
        let delay = env_u64("CODEPET_FAKE_SHUTDOWN_RESPONSE_DELAY_MS", 0);
        Box::pin(async move {
            if let Ok(path) = std::env::var("CODEPET_FAKE_SHUTDOWN_MARKER") {
                let _ = std::fs::write(path, b"shutdown received\n");
            }
            if std::env::var_os("CODEPET_FAKE_SHUTDOWN_NOTIFICATION").is_some() {
                self.publish_conversation(&ProviderInstanceRoute { device_id: "shutdown".into(), provider_plugin_id: self.plugin_id.clone(), provider_instance_id: "shutdown".into() }, "shutdown")?;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Ok(ProviderShutdownResponse { accepted: true })
        })
    }
}

impl FakeProvider {
    async fn before_conversation(&self, route: &ProviderInstanceRoute, id: &str) -> Result<(), codepet_provider_sdk::ProtocolError> {
        match id {
            "slow" => tokio::time::sleep(Duration::from_millis(120)).await,
            "fast" => tokio::time::sleep(Duration::from_millis(5)).await,
            "timeout" => std::future::pending::<()>().await,
            "crash" => { eprintln!("fixture crash requested"); std::process::exit(17); },
            "malformed" | "oversized" => {
                // Deliberately corrupt the physical mux transport for Host failure-isolation tests.
                let mut header = [0u8; 12];
                if id == "malformed" { header[0] = 255; }
                else { header[8..].copy_from_slice(&u32::MAX.to_be_bytes()); }
                let mut output = std::io::stdout();
                output.write_all(&header).unwrap(); output.flush().unwrap();
                std::future::pending::<()>().await;
            }
            "event-first" | "snapshot-race" => {
                self.publish_conversation(route, id)?;
                if id == "snapshot-race" { wait_for_snapshot_release().await; }
            }
            _ => {}
        }
        Ok(())
    }

    fn publish_conversation(&self, route: &ProviderInstanceRoute, id: &str) -> Result<(), codepet_provider_sdk::ProtocolError> {
        let sink = self.events.as_ref().unwrap();
        sink.publish(ProtocolEvent::EventConversationUpserted { jsonrpc: "2.0".into(), params: ConversationUpsertedEvent { conversation: conversation(route, id) } })?;
        let item_id = format!("{id}-assistant");
        let content_id = format!("{item_id}:text");
        let item = ConversationItem::MessageConversationItem(MessageConversationItem { meta: None,
            resource: resource(route, &item_id), turn: resource(route, &format!("{id}-turn")), conversation: resource(route, id),
            kind: MessageConversationItemKind::Message, status: ConversationItemStatus::Completed, role: ConversationItemRole::Assistant,
            contents: vec![ContentBlock::TextContentBlock(TextContentBlock { content_id: content_id.clone(), kind: TextContentBlockKind::Text,
                text: "fixture assistant message".into(), truncation: None })],
        });
        sink.publish(ProtocolEvent::EventConversationItemUpserted { jsonrpc: "2.0".into(), params: ConversationItemUpsertedEvent { item } })?;
        sink.publish(ProtocolEvent::EventTurnOutputDelta { jsonrpc: "2.0".into(), params: TurnOutputDeltaEvent {
            turn: provider_resource(route, &format!("{id}-turn")), conversation: provider_resource(route, id), item_id, content_id,
            kind: ConversationContentKind::Text, delta: "hello".into(), extension: None,
        } })
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Ok(path) = std::env::var("CODEPET_FAKE_PID_MARKER") {
        let _ = std::fs::write(path, format!("{}\n", std::process::id()));
    }
    let plugin_id = std::env::var("CODEPET_FAKE_PLUGIN_ID")
        .unwrap_or_else(|_| "dev.codepet.fake".to_string());
    codepet_provider_sdk::serve_stdio(codepet_provider_sdk::StdioServerOptions::default(), |events| FakeProvider {
        events: Some(events), plugin_id, observation: Mutex::new(None), instances: Mutex::new(BTreeMap::new()), instance_settings: Mutex::new(BTreeMap::new()),
    }).await.unwrap();
    close_stdout_pipe();
    if let Ok(path) = std::env::var("CODEPET_FAKE_STDOUT_CLOSED_MARKER") {
        let _ = std::fs::write(path, b"stdout closed\n");
    }
    eprintln!("fixture shutdown stderr tail");
    std::thread::sleep(Duration::from_millis(env_u64("CODEPET_FAKE_SHUTDOWN_DELAY_MS", 0)));
}

fn env_u32(name: &str, fallback: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

#[cfg(unix)]
fn close_stdout_pipe() {
    extern "C" {
        fn close(fd: i32) -> i32;
    }

    let result = unsafe { close(1) };
    if result != 0 {
        std::process::exit(4);
    }
}

#[cfg(windows)]
fn close_stdout_pipe() {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    extern "system" {
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    let handle = std::io::stdout().as_raw_handle();
    if unsafe { CloseHandle(handle) } == 0 {
        std::process::exit(4);
    }
}

async fn wait_for_snapshot_release() {
    let marker = std::env::var("CODEPET_FAKE_SNAPSHOT_RELEASE_MARKER")
        .expect("snapshot race fixture requires a release marker");
    while !std::path::Path::new(&marker).exists() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        revision: "fake-capabilities-v1".to_string(),
        methods: vec![
            ProviderCapability::ProjectList,
            ProviderCapability::ProjectGet,
            ProviderCapability::ProjectCreate,
            ProviderCapability::ProjectUpdate,
            ProviderCapability::ProjectDelete,
            ProviderCapability::ConversationList,
            ProviderCapability::ConversationSearch,
            ProviderCapability::ConversationGet,
            ProviderCapability::ConversationCreate,
            ProviderCapability::TurnStart,
            ProviderCapability::TurnSteer,
            ProviderCapability::TurnInterrupt,
            ProviderCapability::ApprovalResolve,
        ],
        turn_send: Some(TurnSendCapabilities {
            access_mode: Some(ChoiceSet {
                options: vec![choice("workspace-write", "Workspace write")],
                default_id: Some("workspace-write".to_string()),
            }),
            reasoning_effort: Some(ChoiceSet {
                options: vec![choice("medium", "Medium")],
                default_id: Some("medium".to_string()),
            }),
            model_catalog: Some(ModelCatalog::FlatModelCatalog(FlatModelCatalog {
                kind: FlatModelCatalogKind::Flat,
                models: vec![choice("fake-model", "Fake model")],
                default_selection: Some(FlatModelSelection {
                    kind: FlatModelCatalogKind::Flat,
                    model_id: "fake-model".to_string(),
                }),
            })),
        }),
        conversation_create: Some(ConversationCreateCapabilities {
            supports_title: true,
            selection: None,
            workspace_mode: Some(ChoiceSet {
                options: vec![choice("main", "Main workspace")],
                default_id: Some("main".to_string()),
            }),
        }),
        extensions: Vec::new(),
    }
}

fn choice(id: &str, display_name: &str) -> ChoiceOption {
    ChoiceOption {
        id: id.to_string(),
        display_name: display_name.to_string(),
        description: None,
        enabled: Some(true),
        disabled_reason: None,
    }
}

fn conversation(route: &ProviderInstanceRoute, native_id: &str) -> Conversation {
    Conversation {
        resource: resource(route, native_id),
        project: None,
        title: format!("Fake {native_id}"),
        preview: Some("fixture conversation".to_string()),
        status: ConversationStatus::Idle,
        permission_level: Some("workspace-write".to_string()),
        model: Some("fake-model".to_string()),
        reasoning_effort: Some("medium".to_string()),
        selection: Some(codepet_provider_sdk::TurnSelection {
            access_mode_id: Some("workspace-write".to_string()),
            reasoning_effort_id: Some("medium".to_string()),
            model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                kind: FlatModelCatalogKind::Flat,
                model_id: "fake-model".to_string(),
            })),
        }),
        workspace_root: Some("/fixture".to_string()),
        created_at: Some(1),
        updated_at: Some(2),
        active_turn: None,
        read_state: None,
    }
}

fn project(route: &ProviderInstanceRoute, native_id: &str, name: &str) -> Project {
    Project {
        resource: resource(route, native_id),
        name: name.to_string(),
        roots: vec![ProjectRoot {
            path: "/fixture/project".to_string(),
        }],
        metadata: BTreeMap::from([("fixture".to_string(), "true".to_string())]),
        position: 4,
        created_at: 10,
        updated_at: 20,
    }
}

fn history_items(route: &ProviderInstanceRoute, native_id: &str) -> Vec<ConversationItem> {
    let turn = resource(route, &format!("{native_id}-turn"));
    let conversation = resource(
        route,
        if native_id == "response-wrong-item-conversation" {
            "different-thread"
        } else {
            native_id
        },
    );
    let user_item_id = format!("{native_id}-user");
    let assistant_item_id = format!("{native_id}-assistant");
    vec![
        ConversationItem::MessageConversationItem(MessageConversationItem { meta: None,
            resource: resource(route, &user_item_id),
            turn: turn.clone(),
            conversation: conversation.clone(),
            kind: MessageConversationItemKind::Message,
            status: ConversationItemStatus::Completed,
            role: ConversationItemRole::User,
            contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                content_id: format!("{user_item_id}:input:0"),
                kind: TextContentBlockKind::Text,
                text: "fixture user message".to_string(),
                truncation: None,
            })],
        }),
        ConversationItem::MessageConversationItem(MessageConversationItem { meta: None,
            resource: resource(route, &assistant_item_id),
            turn,
            conversation,
            kind: MessageConversationItemKind::Message,
            status: ConversationItemStatus::Completed,
            role: ConversationItemRole::Assistant,
            contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                content_id: format!("{assistant_item_id}:text"),
                kind: TextContentBlockKind::Text,
                text: "fixture assistant message".to_string(),
                truncation: None,
            })],
        }),
    ]
}

fn turn(
    route: &ProviderInstanceRoute,
    native_id: &str,
    conversation: RoutedResourceId,
) -> TurnTask {
    TurnTask {
        resource: resource(route, native_id),
        conversation,
        status: TurnStatus::Running,
        display_summary: Some("fixture turn".to_string()),
        started_at: Some(2),
        updated_at: Some(3),
        completed_at: None,
    }
}

fn resource(route: &ProviderInstanceRoute, native_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        provider_id: route.provider_instance_id.clone(),
        native_resource_id: native_id.to_string(),
    }
}

fn provider_resource(route: &ProviderInstanceRoute, native_id: &str) -> ProviderResourceId {
    ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_id.to_string(),
    }
}

fn route_from_resource(resource: &ProviderResourceId) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: resource.device_id.clone(),
        provider_plugin_id: resource.provider_plugin_id.clone(),
        provider_instance_id: resource.provider_instance_id.clone(),
    }
}

fn protocol_error(
    code: &str,
    message: impl Into<String>,
) -> codepet_provider_sdk::ProtocolError {
    codepet_provider_sdk::ProtocolError {
        code: code.to_string(),
        message: message.into(),
        retryable: false,
        details: None,
    }
}
