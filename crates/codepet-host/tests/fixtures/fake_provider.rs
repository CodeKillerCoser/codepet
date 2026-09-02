use codepet_provider_sdk::{
    dispatch, ApprovalDecision, ApprovalResolveRequest, ApprovalResolveResponse, ApprovalStatus,
    ChoiceOption, ChoiceSet, ConversationAcquireInteractionRequest,
    ConversationAcquireInteractionResponse, ConversationCreateRequest, ConversationCreateResponse, ConversationGetRequest,
    ConversationGetResponse, ConversationListRequest, ConversationListResponse,
    ConversationSearchRequest, ConversationSearchResponse, ConversationContent,
    ConversationContentKind, ConversationCreateCapabilities, ConversationItem, ConversationItemKind,
    ConversationItemRole, ConversationItemStatus, ConversationStatus, ConversationUpsertedEvent,
    InstanceCapabilitiesRequest,
    InstanceCapabilitiesResponse, InstanceCreateRequest, InstanceCreateResponse,
    InstanceDestroyRequest, InstanceDestroyResponse, InstanceStartRequest,
    InstanceStartResponse, InstanceStatus, InstanceStopRequest, InstanceStopResponse,
    FlatModelCatalog, FlatModelCatalogKind, FlatModelSelection, HarnessDescriptor, JsonLineCodec,
    JsonObject, JsonRpcInboundRequest, JsonRpcNotification, ModelCatalog, ModelSelection, PageInfo, ProtocolEvent,
    ProtocolFuture, ProtocolRequest, ProtocolServer, ProviderApproval, ProviderCapabilities,
    ProviderCapability, ProviderConversation, ProviderDescribeRequest,
    ProviderDescribeResponse, ProviderInitializeRequest, ProviderInitializeResponse,
    ProviderInstance, ProviderInstanceRoute, ProviderPluginDescriptor, ProviderShutdownRequest,
    ProviderShutdownResponse, ProviderTurn, ProviderWireMessage, RoutedResourceId,
    TurnInterruptRequest, TurnInterruptResponse, TurnOutputDeltaEvent, TurnSendCapabilities,
    TurnSelection, TurnStartRequest, TurnStartResponse, TurnStatus, TurnSteerRequest,
    TurnSteerResponse, VersionRange,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct FakeProvider {
    plugin_id: String,
    instances: Mutex<BTreeMap<String, ProviderInstance>>,
    instance_settings: Mutex<BTreeMap<String, JsonObject>>,
}

impl FakeProvider {
    fn descriptor(&self) -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            plugin_id: self.plugin_id.clone(),
            display_name: "CodePet Fake Provider".to_string(),
            version: "1.0.0-test".to_string(),
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
        Box::pin(async move { Ok(ProviderDescribeResponse { plugin: descriptor }) })
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
                },
                status: InstanceStatus::Created,
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

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProtocolFuture<'a, ConversationListResponse> {
        Box::pin(async move {
            self.instance(&request.route)?;
            Ok(ConversationListResponse {
                conversations: vec![conversation(&request.route, "conversation-list")],
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
            let native_id = match request.conversation.native_resource_id.as_str() {
                "response-wrong-native" => "different-native-id",
                "response-empty-native" => "",
                native_id => native_id,
            };
            let response_route = if request.conversation.native_resource_id == "response-wrong-route" {
                ProviderInstanceRoute {
                    device_id: "device-other".to_string(),
                    provider_plugin_id: route.provider_plugin_id.clone(),
                    provider_instance_id: route.provider_instance_id.clone(),
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
            Ok(ConversationGetResponse {
                conversation: configured,
                items: history_items(&response_route, native_id),
            })
        })
    }

    fn conversation_acquire_interaction<'a>(
        &'a self,
        request: ConversationAcquireInteractionRequest,
    ) -> ProtocolFuture<'a, ConversationAcquireInteractionResponse> {
        Box::pin(async move {
            self.instance(&route_from_resource(&request.conversation))?;
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
            Ok(ConversationCreateResponse {
                conversation: conversation(&request.route, "conversation-created"),
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
                request.conversation
            };
            let turn = turn(&route, "turn-started", conversation.clone());
            let user_item_conversation = if wrong_user_item_conversation {
                resource(&route, "different-user-item-conversation")
            } else {
                conversation
            };
            let user_item = ConversationItem {
                resource: resource(&route, &format!("user-{}", request.client_request_id)),
                turn: turn.resource.clone(),
                conversation: user_item_conversation,
                kind: ConversationItemKind::Message,
                status: ConversationItemStatus::Completed,
                role: Some(ConversationItemRole::User),
                title: None,
                contents: vec![ConversationContent {
                    content_id: format!("{}:input:0", request.client_request_id),
                    kind: ConversationContentKind::Text,
                    text: request.input.text,
                }],
                related_item: None,
                approval: None,
            };
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
                request.conversation
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
                request.conversation,
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
                approval: ProviderApproval {
                    resource: request.approval,
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
                    extension: None,
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
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Ok(ProviderShutdownResponse { accepted: true })
        })
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Ok(path) = std::env::var("CODEPET_FAKE_PID_MARKER") {
        let _ = std::fs::write(path, format!("{}\n", std::process::id()));
    }
    let plugin_id = std::env::var("CODEPET_FAKE_PLUGIN_ID")
        .unwrap_or_else(|_| "dev.codepet.fake".to_string());
    let server = Arc::new(FakeProvider {
        plugin_id,
        instances: Mutex::new(BTreeMap::new()),
        instance_settings: Mutex::new(BTreeMap::new()),
    });
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let codec = JsonLineCodec::default();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    loop {
        let message = match codec.read_message(&mut input) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => {
                eprintln!("fake Provider input error: {}", error.error.message);
                std::process::exit(2);
            }
        };
        let ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) = message else {
            eprintln!("fake Provider expected a typed request");
            std::process::exit(3);
        };
        if matches!(request, ProtocolRequest::ProviderShutdown { .. }) {
            if let Ok(path) = std::env::var("CODEPET_FAKE_SHUTDOWN_MARKER") {
                let _ = std::fs::write(path, b"shutdown received\n");
            }
            if std::env::var_os("CODEPET_FAKE_SHUTDOWN_NOTIFICATION").is_some() {
                write_message(
                    &output,
                    codec,
                    ProviderWireMessage::Notification(JsonRpcNotification {
                        jsonrpc: "2.0".to_string(),
                        method: "fixture.shutdownProgress".to_string(),
                        params: serde_json::json!({ "stage": "stopping" }),
                    }),
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let response = dispatch(server.as_ref(), request).await;
            write_message(&output, codec, ProviderWireMessage::Response(response));
            close_stdout_pipe();
            if let Ok(path) = std::env::var("CODEPET_FAKE_STDOUT_CLOSED_MARKER") {
                let _ = std::fs::write(path, b"stdout closed\n");
            }
            eprintln!("fixture shutdown stderr tail");
            std::thread::sleep(Duration::from_millis(env_u64(
                "CODEPET_FAKE_SHUTDOWN_DELAY_MS",
                0,
            )));
            return;
        }
        let behavior = request_behavior(&request);
        if behavior == RequestBehavior::Timeout {
            continue;
        }
        if behavior == RequestBehavior::Crash {
            eprintln!("fixture crash requested");
            std::thread::sleep(Duration::from_millis(10));
            std::process::exit(17);
        }
        if behavior == RequestBehavior::Malformed {
            let mut output = output.lock().unwrap();
            let _ = output.write_all(b"{malformed-json}\n");
            let _ = output.flush();
            continue;
        }
        if behavior == RequestBehavior::Oversized {
            let mut output = output.lock().unwrap();
            let _ = output.write_all(&vec![b'x'; 2 * 1024 * 1024]);
            let _ = output.write_all(b"\n");
            let _ = output.flush();
            continue;
        }
        let server = server.clone();
        let output = output.clone();
        tokio::spawn(async move {
            match behavior {
                RequestBehavior::Slow => tokio::time::sleep(Duration::from_millis(120)).await,
                RequestBehavior::Fast => tokio::time::sleep(Duration::from_millis(5)).await,
                _ => {}
            }
            if matches!(
                behavior,
                RequestBehavior::EventFirst | RequestBehavior::SnapshotRace
            ) {
                if let Some(route) = request_route(&request) {
                    let conversation_id = match &request {
                        ProtocolRequest::ConversationList { .. } => {
                            "conversation-list-event-first"
                        }
                        _ => "conversation-event-first",
                    };
                    let event = ProtocolEvent::EventConversationUpserted {
                        jsonrpc: "2.0".to_string(),
                        params: ConversationUpsertedEvent {
                            conversation: conversation(&route, conversation_id),
                        },
                    };
                    write_message(&output, codec, ProviderWireMessage::Event(event));
                    write_message(
                        &output,
                        codec,
                        ProviderWireMessage::Notification(JsonRpcNotification {
                            jsonrpc: "2.0".to_string(),
                            method: "fixture.progress".to_string(),
                            params: serde_json::json!({ "step": 1 }),
                        }),
                    );
                    let delta = ProtocolEvent::EventTurnOutputDelta {
                        jsonrpc: "2.0".to_string(),
                        params: TurnOutputDeltaEvent {
                            turn: resource(&route, "turn-event-first"),
                            conversation: resource(&route, conversation_id),
                            item_id: "output-1".to_string(),
                            content_id: "output-1:text".to_string(),
                            kind: ConversationContentKind::Text,
                            delta: "hello".to_string(),
                            extension: None,
                        },
                    };
                    write_message(&output, codec, ProviderWireMessage::Event(delta));
                }
            }
            if behavior == RequestBehavior::SnapshotRace {
                wait_for_snapshot_release().await;
            }
            let response = dispatch(server.as_ref(), request).await;
            write_message(&output, codec, ProviderWireMessage::Response(response));
        });
    }
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum RequestBehavior {
    Normal,
    Slow,
    Fast,
    EventFirst,
    SnapshotRace,
    Timeout,
    Crash,
    Malformed,
    Oversized,
}

fn request_behavior(request: &ProtocolRequest) -> RequestBehavior {
    if matches!(request, ProtocolRequest::ConversationList { .. })
        && std::env::var("CODEPET_FAKE_CONVERSATION_LIST_SNAPSHOT_RACE").as_deref()
            == Ok("1")
    {
        return RequestBehavior::SnapshotRace;
    }
    let native_id = match request {
        ProtocolRequest::ConversationGet { params, .. } => {
            Some(params.conversation.native_resource_id.as_str())
        }
        _ => None,
    };
    match native_id {
        Some("slow") => RequestBehavior::Slow,
        Some("fast") => RequestBehavior::Fast,
        Some("event-first") => RequestBehavior::EventFirst,
        Some("snapshot-race") => RequestBehavior::SnapshotRace,
        Some("timeout") => RequestBehavior::Timeout,
        Some("crash") => RequestBehavior::Crash,
        Some("malformed") => RequestBehavior::Malformed,
        Some("oversized") => RequestBehavior::Oversized,
        _ => RequestBehavior::Normal,
    }
}

async fn wait_for_snapshot_release() {
    let marker = std::env::var("CODEPET_FAKE_SNAPSHOT_RELEASE_MARKER")
        .expect("snapshot race fixture requires a release marker");
    while !std::path::Path::new(&marker).exists() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn request_route(request: &ProtocolRequest) -> Option<ProviderInstanceRoute> {
    match request {
        ProtocolRequest::ConversationList { params, .. } => Some(params.route.clone()),
        ProtocolRequest::ConversationGet { params, .. } => {
            Some(route_from_resource(&params.conversation))
        }
        _ => None,
    }
}

fn write_message(
    output: &Arc<Mutex<std::io::Stdout>>,
    codec: JsonLineCodec,
    message: ProviderWireMessage,
) {
    let mut output = output.lock().unwrap();
    codec.write_message(&mut *output, &message).unwrap();
    output.flush().unwrap();
}

fn capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        revision: "fake-capabilities-v1".to_string(),
        methods: vec![
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

fn conversation(route: &ProviderInstanceRoute, native_id: &str) -> ProviderConversation {
    ProviderConversation {
        resource: resource(route, native_id),
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
        extension: None,
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
        ConversationItem {
            resource: resource(route, &user_item_id),
            turn: turn.clone(),
            conversation: conversation.clone(),
            kind: ConversationItemKind::Message,
            status: ConversationItemStatus::Completed,
            role: Some(ConversationItemRole::User),
            title: None,
            contents: vec![ConversationContent {
                content_id: format!("{user_item_id}:input:0"),
                kind: ConversationContentKind::Text,
                text: "fixture user message".to_string(),
            }],
            related_item: None,
            approval: None,
        },
        ConversationItem {
            resource: resource(route, &assistant_item_id),
            turn,
            conversation,
            kind: ConversationItemKind::Message,
            status: ConversationItemStatus::Completed,
            role: Some(ConversationItemRole::Assistant),
            title: None,
            contents: vec![ConversationContent {
                content_id: format!("{assistant_item_id}:text"),
                kind: ConversationContentKind::Text,
                text: "fixture assistant message".to_string(),
            }],
            related_item: None,
            approval: None,
        },
    ]
}

fn turn(
    route: &ProviderInstanceRoute,
    native_id: &str,
    conversation: RoutedResourceId,
) -> ProviderTurn {
    ProviderTurn {
        resource: resource(route, native_id),
        conversation,
        status: TurnStatus::Running,
        display_summary: Some("fixture turn".to_string()),
        started_at: Some(2),
        updated_at: Some(3),
        completed_at: None,
        extension: None,
    }
}

fn resource(route: &ProviderInstanceRoute, native_id: &str) -> RoutedResourceId {
    RoutedResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_id.to_string(),
    }
}

fn route_from_resource(resource: &RoutedResourceId) -> ProviderInstanceRoute {
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
