#[path = "../../test_support/mux_stdio.rs"]
mod mux_stdio;

use codepet_provider_codex::{
    CodexProvider, ExecutionLifecycleHook, ProviderEventSink, CODEX_INSTANCE_KIND,
    CODEX_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ConversationAcquireInteractionRequest,
    ContentBlock, ConversationCreateRequest, ConversationItem, ConversationListRequest,
    ConversationProjectFilter, ConversationProjectFilterAll, ConversationProjectFilterAllKind,
    ConversationProjectFilterProject,
    ConversationProjectFilterProjectKind,
    ConversationProjectFilterStandalone, ConversationProjectFilterStandaloneKind,
    ConversationGetRequest, ConversationSearchRequest, InstanceCapabilitiesRequest, InstanceCreateRequest,
    InstanceDestroyRequest, InstanceStartRequest, InstanceStopRequest, JsonObject, ProtocolEvent,
    ProjectCreateRequest, ProjectDeleteRequest, ProjectGetRequest, ProjectListRequest, ProjectRoot,
    ProjectUpdateRequest,
    ProtocolServer as ProviderProtocolServer, ProviderInitializeRequest,
    FlatModelCatalogKind, FlatModelSelection, ModelSelection, ProviderInstanceRoute,
    ProviderResourceId, ProviderShutdownRequest, TurnInput, TurnInputKind, TurnInterruptRequest,
    ToolOutcome, TurnSelection, TurnStartRequest, TurnSteerRequest, VersionRange, PROTOCOL_VERSION,
};
use serde_json::json;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn provider_executable() -> PathBuf {
    std::env::var_os("CODEPET_TEST_PROVIDER_EXE").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_codepet-provider-codex")))
}

fn item_resource_id(item: &ConversationItem) -> &str {
    match item {
        ConversationItem::MessageConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::ReasoningConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::CommandConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::FileChangeConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::ToolConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::ApprovalConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::UnknownConversationItem(value) => &value.resource.native_resource_id,
    }
}

fn item_status(item: &ConversationItem) -> codepet_provider_sdk::ConversationItemStatus {
    match item {
        ConversationItem::MessageConversationItem(value) => value.status,
        ConversationItem::ReasoningConversationItem(value) => value.status,
        ConversationItem::CommandConversationItem(value) => value.status,
        ConversationItem::FileChangeConversationItem(value) => value.status,
        ConversationItem::ToolConversationItem(value) => value.status,
        ConversationItem::ApprovalConversationItem(value) => value.status,
        ConversationItem::UnknownConversationItem(value) => value.status,
    }
}

fn item_contents(item: &ConversationItem) -> &[ContentBlock] {
    match item {
        ConversationItem::MessageConversationItem(value) => &value.contents,
        ConversationItem::ReasoningConversationItem(value) => &value.contents,
        ConversationItem::FileChangeConversationItem(value) => &value.contents,
        ConversationItem::CommandConversationItem(value) => match &value.tool.outcome {
            Some(ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
            Some(ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
            None => &[],
        },
        ConversationItem::ToolConversationItem(value) => match &value.tool.outcome {
            Some(ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
            Some(ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
            None => &[],
        },
        _ => &[],
    }
}

fn content_id(content: &ContentBlock) -> &str {
    match content {
        ContentBlock::TextContentBlock(value) => &value.content_id,
        ContentBlock::ReasoningSummaryContentBlock(value) => &value.content_id,
        ContentBlock::OutputContentBlock(value) => &value.content_id,
        ContentBlock::ActivitySummaryContentBlock(value) => &value.content_id,
        ContentBlock::StructuredJsonContentBlock(value) => &value.content_id,
        ContentBlock::ImageContentBlock(value) => &value.content_id,
        ContentBlock::AudioContentBlock(value) => &value.content_id,
        ContentBlock::ResourceLinkContentBlock(value) => &value.content_id,
        ContentBlock::EmbeddedResourceContentBlock(value) => &value.content_id,
    }
}

fn app_server_executable() -> PathBuf {
    std::env::var_os("CODEPET_TEST_APP_SERVER_EXE").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_codex-app-server-fixture")))
}

fn route(device_id: &str) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: device_id.to_string(),
        provider_plugin_id: CODEX_PLUGIN_ID.to_string(),
        provider_instance_id: "codex".to_string(),
    }
}

fn all_project_filter() -> ConversationProjectFilter {
    ConversationProjectFilter::ConversationProjectFilterAll(ConversationProjectFilterAll {
        kind: ConversationProjectFilterAllKind::All,
    })
}

fn instance_settings(app_server: &Path, approval_mode: &str, marker: &Path) -> JsonObject {
    [
        (
            "appServerExecutable".to_string(),
            json!(app_server.to_string_lossy()),
        ),
        (
            "appServerArgs".to_string(),
            json!([
                "--approval-mode",
                approval_mode,
                "--marker",
                marker.to_string_lossy()
            ]),
        ),
    ]
    .into_iter()
    .collect()
}

async fn configured_direct_provider(
    approval_mode: &str,
    marker: &Path,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    configured_direct_provider_with_events(approval_mode, marker, Arc::new(|_| Ok(()))).await
}

async fn configured_direct_provider_with_events(
    approval_mode: &str,
    marker: &Path,
    events: Arc<dyn ProviderEventSink>,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    configured_direct_provider_with_events_and_hook(approval_mode, marker, events, None).await
}

async fn configured_direct_provider_with_events_and_hook(
    approval_mode: &str,
    marker: &Path,
    events: Arc<dyn ProviderEventSink>,
    lifecycle_hook: Option<Arc<dyn ExecutionLifecycleHook>>,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    let provider = Arc::new(match lifecycle_hook {
        Some(lifecycle_hook) => {
            CodexProvider::new_with_execution_lifecycle_hook(events, lifecycle_hook)
        }
        None => CodexProvider::new(events),
    });
    let route = route("device-provider-direct");
    ProviderProtocolServer::provider_initialize(
        provider.as_ref(),
        ProviderInitializeRequest {
            host_client_id: "client-provider-direct".to_string(),
            host_device_id: route.device_id.clone(),
            host_version: "test".to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::instance_create(
        provider.as_ref(),
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CODEX_INSTANCE_KIND.to_string(),
            display_name: "Codex Direct Fixture".to_string(),
            settings: instance_settings(&app_server_executable(), approval_mode, marker),
        },
    )
    .await
    .unwrap();
    let started = ProviderProtocolServer::instance_start(
        provider.as_ref(),
        InstanceStartRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    (
        provider,
        route,
        started.instance.capabilities.revision,
    )
}

struct BlockHandleOnceHook {
    conversation_id: String,
    operation: &'static str,
    armed: AtomicBool,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl ExecutionLifecycleHook for BlockHandleOnceHook {
    fn after_handle_acquired(&self, conversation_id: &str, operation: &str) {
        if conversation_id == self.conversation_id
            && operation == self.operation
            && self.armed.swap(false, Ordering::SeqCst)
        {
            self.entered.wait();
            self.release.wait();
        }
    }
}

struct BlockResumeUntilCancelledHook {
    conversation_id: String,
    resume_entered: Arc<Barrier>,
    resume_release: Arc<Barrier>,
    cancelled_entered: Arc<Barrier>,
    cancelled_release: Arc<Barrier>,
}

impl ExecutionLifecycleHook for BlockResumeUntilCancelledHook {
    fn before_resume_linearization(&self, conversation_id: &str) {
        if conversation_id == self.conversation_id {
            self.resume_entered.wait();
            self.resume_release.wait();
        }
    }

    fn after_execution_cancelled(&self, conversation_id: &str) {
        if conversation_id == self.conversation_id {
            self.cancelled_entered.wait();
            self.cancelled_release.wait();
        }
    }
}

#[tokio::test]
async fn project_methods_and_project_owned_conversation_fail_closed_when_probe_is_unsupported() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let (provider, route, _) =
        configured_direct_provider("project-unsupported", marker.path()).await;
    let capabilities = ProviderProtocolServer::instance_capabilities(
        provider.as_ref(),
        InstanceCapabilitiesRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ProjectList));

    let project = ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: "project-unsupported".to_string(),
    };
    let error = ProviderProtocolServer::conversation_create(
        provider.as_ref(),
        ConversationCreateRequest {
            route: route.clone(),
            project: Some(project),
            title: None,
            permission_level: "workspace-write".to_string(),
            model: None,
            reasoning_effort: None,
            workspace_root: None,
            workspace_mode: None,
            extension: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "capability_unsupported");
    ProviderProtocolServer::instance_stop(provider.as_ref(), InstanceStopRequest { route })
        .await
        .unwrap();
}

#[tokio::test]
async fn project_owned_conversation_uses_native_project_id_without_inferring_cwd() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("project-owned-create.txt");
    let (provider, route, _) =
        configured_direct_provider("project-owned-create", &marker).await;
    let project = conversation_resource(&route, "project-fixture");

    let created = ProviderProtocolServer::conversation_create(
        provider.as_ref(),
        ConversationCreateRequest {
            route: route.clone(),
            project: Some(project.clone()),
            title: None,
            permission_level: "workspace-write".to_string(),
            model: Some("gpt-fixture".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: None,
            workspace_mode: None,
            extension: None,
        },
    )
    .await
    .unwrap()
    .conversation;
    assert_eq!(
        created.project.as_ref().map(|resource| resource.provider_id.as_str()),
        Some(project.provider_instance_id.as_str())
    );
    assert_eq!(
        created.project.as_ref().map(|resource| resource.native_resource_id.as_str()),
        Some(project.native_resource_id.as_str())
    );

    let fetched = ProviderProtocolServer::conversation_get(
        provider.as_ref(),
        ConversationGetRequest {
            conversation: conversation_resource(
                &route,
                &created.resource.native_resource_id,
            ),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fetched
            .conversation
            .project
            .as_ref()
            .map(|resource| resource.native_resource_id.as_str()),
        Some(project.native_resource_id.as_str())
    );

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test]
async fn provider_v1_round_trips_fixture_app_server_lifecycle_and_approval() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("approval.txt");
    let (event_sender, event_receiver) = mpsc::channel();
    let provider = CodexProvider::new(Arc::new(move |event| {
        event_sender.send(event).map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "test_event_sink_closed".to_string(),
            message: error.to_string(),
            retryable: false,
            details: None,
        })
    }));
    let device_id = "device-provider-fixture";
    let route = route(device_id);

    let initialized = ProviderProtocolServer::provider_initialize(
        &provider,
        ProviderInitializeRequest {
            host_client_id: "client-provider-fixture".to_string(),
            host_device_id: device_id.to_string(),
            host_version: "test".to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(initialized.plugin.plugin_id, CODEX_PLUGIN_ID);
    assert!(initialized.plugin.default_workspace_root.is_some());

    let created = ProviderProtocolServer::instance_create(
        &provider,
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CODEX_INSTANCE_KIND.to_string(),
            display_name: "Codex Fixture".to_string(),
            settings: instance_settings(&app_server_executable(), "normal", &marker),
        },
    )
    .await
    .unwrap();
    assert_eq!(created.instance.route, route);
    let started = ProviderProtocolServer::instance_start(
        &provider,
        InstanceStartRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(started.instance.status, codepet_provider_sdk::InstanceStatus::Ready);
    let capabilities = ProviderProtocolServer::instance_capabilities(
        &provider,
        InstanceCapabilitiesRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ApprovalResolve));
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ConversationSearch));
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::TurnStart));
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ProjectList));
    assert!(!capabilities.capabilities.revision.trim().is_empty());
    let reasoning = capabilities
        .capabilities
        .turn_send
        .as_ref()
        .and_then(|turn_send| turn_send.reasoning_effort.as_ref())
        .unwrap();
    assert_eq!(
        reasoning
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect::<Vec<_>>(),
        vec!["high"]
    );

    let projects = ProviderProtocolServer::project_list(
        &provider,
        ProjectListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let fixture_project = projects.projects[0].resource.clone();
    assert_eq!(projects.projects[0].metadata["fixture"], "true");
    assert_eq!(projects.projects[0].position, 1);
    let fetched_project = ProviderProtocolServer::project_get(
        &provider,
        ProjectGetRequest {
            project: conversation_resource(
                &route,
                &fixture_project.native_resource_id,
            ),
        },
    )
    .await
    .unwrap();
    assert_eq!(fetched_project.project.resource, fixture_project);
    let created_project = ProviderProtocolServer::project_create(
        &provider,
        ProjectCreateRequest {
            route: route.clone(),
            idempotency_key: "create-project-one".to_string(),
            name: "Created Project".to_string(),
            roots: vec![ProjectRoot {
                path: "/fixture/workspace".to_string(),
            }],
            metadata: BTreeMap::from([("owner".to_string(), "gateway".to_string())]),
        },
    )
    .await
    .unwrap();
    assert_eq!(created_project.project.resource.native_resource_id, "project-created");
    let updated_project = ProviderProtocolServer::project_update(
        &provider,
        ProjectUpdateRequest {
            project: conversation_resource(
                &route,
                &created_project.project.resource.native_resource_id,
            ),
            name: Some("Renamed Project".to_string()),
            roots: None,
            metadata: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated_project.project.name, "Renamed Project");
    ProviderProtocolServer::project_delete(
        &provider,
        ProjectDeleteRequest {
            project: conversation_resource(
                &route,
                &created_project.project.resource.native_resource_id,
            ),
        },
    )
    .await
    .unwrap();

    let listed = ProviderProtocolServer::conversation_list(
        &provider,
        ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(20),
            project_filter: all_project_filter(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        listed.conversations[0].resource.provider_id,
        "codex"
    );
    assert_eq!(
        listed.conversations[0].resource.native_resource_id,
        "thread-listed"
    );
    assert_eq!(listed.conversations[0].project.as_ref(), Some(&fixture_project));
    let project_conversations = ProviderProtocolServer::conversation_list(
        &provider,
        ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(20),
            project_filter: ConversationProjectFilter::ConversationProjectFilterProject(
                ConversationProjectFilterProject {
                    kind: ConversationProjectFilterProjectKind::Project,
                    project: conversation_resource(
                        &route,
                        &fixture_project.native_resource_id,
                    ),
                },
            ),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        project_conversations.conversations[0].project.as_ref(),
        Some(&fixture_project)
    );
    let standalone_conversations = ProviderProtocolServer::conversation_list(
        &provider,
        ConversationListRequest {
            route: route.clone(),
            cursor: None,
            limit: Some(20),
            project_filter: ConversationProjectFilter::ConversationProjectFilterStandalone(
                ConversationProjectFilterStandalone {
                    kind: ConversationProjectFilterStandaloneKind::Standalone,
                },
            ),
        },
    )
    .await
    .unwrap();
    assert!(standalone_conversations.conversations.is_empty());
    let searched = ProviderProtocolServer::conversation_search(
        &provider,
        ConversationSearchRequest {
            route: route.clone(),
            search_term: "gateway protocol".to_string(),
            cursor: Some("search-cursor".to_string()),
            limit: Some(7),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        searched.conversations[0].resource.native_resource_id,
        "thread-search-result"
    );
    assert_eq!(searched.page_info.next_cursor.as_deref(), Some("search-next"));

    let empty_search = ProviderProtocolServer::conversation_search(
        &provider,
        ConversationSearchRequest {
            route: route.clone(),
            search_term: " ".to_string(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(empty_search.code, "invalid_request");
    let fetched = ProviderProtocolServer::conversation_get(
        &provider,
        ConversationGetRequest {
            conversation: conversation_resource(
                &route,
                &listed.conversations[0].resource.native_resource_id,
            ),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fetched.conversation.resource.native_resource_id,
        "thread-listed"
    );
    assert_eq!(
        fetched
            .items
            .iter()
            .map(item_resource_id)
            .collect::<Vec<_>>(),
        vec![
            "user-one",
            "agent-one",
            "reasoning-one",
            "command-one",
            "file-one",
            "mcp-one",
            "dynamic-one",
            "unknown-one"
        ]
    );
    assert_eq!(content_id(&item_contents(&fetched.items[0])[0]), "user-one:input:0");
    assert_eq!(
        content_id(&item_contents(&fetched.items[2])[0]),
        "reasoning-one:summary:0"
    );
    assert!(matches!(fetched.items[7], ConversationItem::UnknownConversationItem(_)));
    assert!(fetched.items.iter().all(|item| !matches!(item, ConversationItem::ApprovalConversationItem(_))));
    let fetched_json = serde_json::to_string(&fetched).unwrap();
    assert!(!fetched_json.contains("private raw reasoning"));
    assert!(!fetched_json.contains("must not escape"));
    assert!(!fetched_json.contains("data:image/png"));
    assert!(!fetched_json.contains("private-a"));

    let unsupported_title = ProviderProtocolServer::conversation_create(
        &provider,
        ConversationCreateRequest {
            route: route.clone(),
            project: None,
            title: Some("unsupported title".to_string()),
            permission_level: "workspace-write".to_string(),
            model: None,
            reasoning_effort: None,
            workspace_root: None,
            workspace_mode: None,
            extension: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(unsupported_title.code, "capability_unsupported");

    let fixture_workspace = tempfile::tempdir().unwrap();
    let conversation = ProviderProtocolServer::conversation_create(
        &provider,
        ConversationCreateRequest {
            route: route.clone(),
            project: Some(conversation_resource(
                &route,
                &fixture_project.native_resource_id,
            )),
            title: None,
            permission_level: "workspace-write".to_string(),
            model: Some("gpt-fixture".to_string()),
            reasoning_effort: Some("high".to_string()),
            workspace_root: Some(fixture_workspace.path().to_string_lossy().into_owned()),
            workspace_mode: None,
            extension: None,
        },
    )
    .await
    .unwrap()
    .conversation;
    assert_eq!(conversation.project.as_ref(), Some(&fixture_project));
    let rejected_reasoning = ProviderProtocolServer::turn_start(
        &provider,
        TurnStartRequest {
            conversation: conversation_resource(
                &route,
                &conversation.resource.native_resource_id,
            ),
            client_request_id: "message-rejected".to_string(),
            capability_revision: capabilities.capabilities.revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "must not start".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: Some("workspace-write".to_string()),
                reasoning_effort_id: Some("low".to_string()),
                model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                    kind: FlatModelCatalogKind::Flat,
                    model_id: "gpt-fixture".to_string(),
                })),
            },
        },
    )
    .await
    .unwrap_err();
    assert_eq!(rejected_reasoning.code, "invalid_turn_selection");
    let started_turn = ProviderProtocolServer::turn_start(
        &provider,
        TurnStartRequest {
            conversation: conversation_resource(
                &route,
                &conversation.resource.native_resource_id,
            ),
            client_request_id: "message-one".to_string(),
            capability_revision: capabilities.capabilities.revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "run fixture".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: Some("workspace-write".to_string()),
                reasoning_effort_id: Some("high".to_string()),
                model: Some(ModelSelection::FlatModelSelection(FlatModelSelection {
                    kind: FlatModelCatalogKind::Flat,
                    model_id: "gpt-fixture".to_string(),
                })),
            },
        },
    )
    .await
    .unwrap();
    assert!(started_turn.accepted);
    assert!(started_turn.user_item.is_none());
    let refreshed = ProviderProtocolServer::conversation_get(
        &provider,
        ConversationGetRequest {
            conversation: conversation_resource(
                &route,
                &conversation.resource.native_resource_id,
            ),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert!(refreshed
        .items
        .iter()
        .any(|item| item_resource_id(item) == "user-one"));
    let turn = started_turn.turn;
    assert_eq!(turn.resource.native_resource_id, "turn-started");

    let mut approval = None;
    let mut saw_delta = false;
    let mut saw_title_update = false;
    for _ in 0..8 {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        match event {
            ProtocolEvent::EventConversationUpserted { params, .. } => {
                saw_title_update = params.conversation.title == "Renamed by Codex";
            }
            ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                saw_delta = params.delta == "fixture output"
                    && params.item_id == "agent-one"
                    && params.content_id == "agent-one:text";
            }
            ProtocolEvent::EventApprovalRequested { params, .. } => {
                approval = Some(params.approval);
                break;
            }
            _ => {}
        }
    }
    assert!(saw_delta);
    assert!(saw_title_update);
    let approval = approval.expect("fixture approval event");
    let resolved = ProviderProtocolServer::approval_resolve(
        &provider,
        ApprovalResolveRequest {
            approval: conversation_resource(
                &route,
                &approval.resource.native_resource_id,
            ),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        resolved.approval.status,
        codepet_provider_sdk::ApprovalStatus::Approved
    );
    let resolved_approval_id = resolved.approval.resource.native_resource_id.clone();
    let mut saw_resolved_event = false;
    for _ in 0..8 {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if matches!(
            event,
            ProtocolEvent::EventApprovalResolved { params, .. }
                if params.approval.resource.native_resource_id == resolved_approval_id
        ) {
            saw_resolved_event = true;
            break;
        }
    }
    assert!(saw_resolved_event);
    wait_for_file(&marker).await;
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "accept");

    let active_history = ProviderProtocolServer::conversation_get(
        &provider,
        ConversationGetRequest {
            conversation: conversation_resource(
                &route,
                &conversation.resource.native_resource_id,
            ),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        active_history
            .conversation
            .active_turn
            .as_ref()
            .map(|turn| turn.resource.native_resource_id.as_str()),
        Some("turn-started")
    );
    let agent = active_history
        .items
        .iter()
        .find(|item| item_resource_id(item) == "agent-one")
        .unwrap();
    let reasoning = active_history
        .items
        .iter()
        .find(|item| item_resource_id(item) == "reasoning-one")
        .unwrap();
    let command = active_history
        .items
        .iter()
        .find(|item| item_resource_id(item) == "command-one")
        .unwrap();
    let approval_item = active_history
        .items
        .iter()
        .find(|item| matches!(item, ConversationItem::ApprovalConversationItem(_)))
        .unwrap();
    assert!(item_contents(agent).is_empty());
    assert!(item_contents(reasoning).is_empty());
    assert!(item_contents(command).is_empty());
    let ConversationItem::ApprovalConversationItem(approval_item) = approval_item else { unreachable!() };
    assert_eq!(
        approval_item.status,
        codepet_provider_sdk::ConversationItemStatus::Approved
    );
    assert_eq!(
        approval_item
            .related_item
            .as_ref()
            .unwrap()
            .native_resource_id,
        "command-one"
    );
    assert_eq!(
        approval_item
            .approval
            .resource
            .native_resource_id,
        resolved_approval_id
    );

    let steered = ProviderProtocolServer::turn_steer(
        &provider,
        TurnSteerRequest {
            conversation: conversation_resource(
                &route,
                &conversation.resource.native_resource_id,
            ),
            turn: conversation_resource(&route, &turn.resource.native_resource_id),
            client_message_id: "message-two".to_string(),
            message: "steer fixture".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(steered.turn.resource.native_resource_id, "turn-started");
    let interrupted = ProviderProtocolServer::turn_interrupt(
        &provider,
        TurnInterruptRequest {
            conversation: conversation_resource(
                &route,
                &conversation.resource.native_resource_id,
            ),
            turn: conversation_resource(&route, &turn.resource.native_resource_id),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        interrupted.turn.status,
        codepet_provider_sdk::TurnStatus::Interrupted
    );

    let stopped = ProviderProtocolServer::instance_stop(
        &provider,
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(stopped.instance.status, codepet_provider_sdk::InstanceStatus::Stopped);
    assert!(ProviderProtocolServer::instance_destroy(
        &provider,
        InstanceDestroyRequest { route },
    )
    .await
    .unwrap()
    .destroyed);
    assert!(ProviderProtocolServer::provider_shutdown(
        &provider,
        ProviderShutdownRequest {},
    )
    .await
    .unwrap()
    .accepted);
}

#[test]
fn provider_binary_rejects_additional_network_permission_without_publishing_approval() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unsupported-approval.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("additional-network", &marker);

    let started = provider.request(
        "turn-unsafe",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "message-unsafe",
            "request unsafe approval",
            &capability_revision,
        ),
    );
    assert_eq!(started.pointer("/result/userItem"), Some(&Value::Null));
    wait_for_file_blocking(&marker);
    provider.collect_for(Duration::from_millis(100));

    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "error:-32601");
    assert!(provider.buffered.iter().all(|message| {
        message.get("method").and_then(Value::as_str) != Some("event.approvalRequested")
    }));

    provider.request("stop-unsafe", "instance.stop", json!({ "route": route_value() }));
    provider.request("shutdown-unsafe", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_rejects_stale_approval_when_app_server_request_id_is_reused() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("approval-generation.txt");
    let mut provider = ProviderBinary::spawn();
    let (first_conversation, first_capability_revision) = provider.configure("normal", &marker);

    provider.request(
        "turn-first",
        "turn.start",
        turn_start_params(
            first_conversation,
            "message-first",
            "first session",
            &first_capability_revision,
        ),
    );
    let first_approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();

    provider.request("stop-first", "instance.stop", json!({ "route": route_value() }));
    let second_start = provider.request(
        "start-second",
        "instance.start",
        json!({ "route": route_value() }),
    );
    let second_capability_revision = second_start
        .pointer("/result/instance/capabilities/revision")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let second_conversation = provider.create_conversation("conversation-second");
    provider.request(
        "turn-second",
        "turn.start",
        turn_start_params(
            second_conversation.clone(),
            "message-second",
            "second session",
            &second_capability_revision,
        ),
    );
    let second_approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();

    assert_ne!(
        first_approval["nativeResourceId"],
        second_approval["nativeResourceId"]
    );
    let history = provider.request(
        "history-second",
        "conversation.get",
        json!({ "conversation": second_conversation }),
    );
    let approval_items = history
        .pointer("/result/items")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter(|item| item.get("kind").and_then(Value::as_str) == Some("approval"))
        .collect::<Vec<_>>();
    assert_eq!(approval_items.len(), 1);
    assert_eq!(
        approval_items[0].pointer("/resource/nativeResourceId"),
        second_approval.get("nativeResourceId")
    );
    let stale = provider.request(
        "resolve-stale",
        "approval.resolve",
        json!({ "approval": first_approval, "decision": "approve" }),
    );
    assert_eq!(
        stale.pointer("/error/data/code").and_then(Value::as_str),
        Some("stale_approval_session")
    );
    assert!(!marker.exists());

    let resolved = provider.request(
        "resolve-current",
        "approval.resolve",
        json!({ "approval": second_approval, "decision": "approve" }),
    );
    assert_eq!(
        resolved
            .pointer("/result/approval/status")
            .and_then(Value::as_str),
        Some("approved")
    );
    wait_for_file_blocking(&marker);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "accept");

    provider.request("stop-second", "instance.stop", json!({ "route": route_value() }));
    provider.request("shutdown-second", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_conversation_get_is_pure_read_with_external_writer() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unused-approval.txt");
    let request_log = directory.path().join("app-server-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("none", &marker, Some(&request_log));
    std::fs::write(&request_log, "").unwrap();
    clear_session_log(&marker);
    let conversation = json!({
        "deviceId": "device-provider-binary",
        "providerPluginId": CODEX_PLUGIN_ID,
        "providerInstanceId": "codex",
        "nativeResourceId": "thread-writer-held"
    });

    let first = provider.request(
        "writer-held-first",
        "conversation.get",
        json!({ "conversation": conversation.clone() }),
    );
    let second = provider.request(
        "writer-held-second",
        "conversation.get",
        json!({ "conversation": conversation }),
    );

    for response in [&first, &second] {
        assert!(response.get("error").is_none());
        assert_eq!(
            response
                .pointer("/result/conversation/resource/nativeResourceId")
                .and_then(Value::as_str),
            Some("thread-writer-held")
        );
        assert_eq!(
            response
                .pointer("/result/items")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(8)
        );
    }
    assert_eq!(first.pointer("/result"), second.pointer("/result"));
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-writer-held",
            "thread/turns/list\tthread-writer-held",
            "thread/read\tthread-writer-held",
            "thread/turns/list\tthread-writer-held"
        ]
    );
    assert!(session_pids(&marker, "process/start", "").is_empty());
    assert!(session_pids(&marker, "thread/resume", "thread-writer-held").is_empty());

    provider.request("writer-held-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("writer-held-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_create_returns_before_conversation_is_materialized() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("create-readiness.txt");
    let request_log = directory.path().join("create-readiness-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure_with_request_log(
        "create-read-eventually",
        &marker,
        Some(&request_log),
    );

    assert_eq!(
        conversation.get("nativeResourceId").and_then(Value::as_str),
        Some("thread-created")
    );
    let fetched = provider.request(
        "created-readable",
        "conversation.get",
        json!({ "conversation": conversation }),
    );
    assert!(fetched.get("error").is_none(), "{fetched}");

    let requests = std::fs::read_to_string(&request_log).unwrap();
    let creation_requests = requests
        .lines()
        .skip_while(|line| *line != "thread/start")
        .collect::<Vec<_>>();
    assert_eq!(
        creation_requests,
        vec![
            "thread/start",
            "thread/read\tthread-created",
            "thread/turns/list\tthread-created"
        ]
    );

    provider.request("create-readiness-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("create-readiness-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_create_does_not_wait_for_history_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("observer-create-readiness.txt");
    let request_log = directory.path().join("observer-create-readiness-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure_with_request_log(
        "observer-create-read-eventually",
        &marker,
        Some(&request_log),
    );

    assert_eq!(
        conversation.get("nativeResourceId").and_then(Value::as_str),
        Some("thread-created")
    );
    let requests = std::fs::read_to_string(&request_log).unwrap();
    let created_reads = requests
        .lines()
        .filter(|line| *line == "thread/read\tthread-created")
        .count();
    assert_eq!(created_reads, 0, "{requests}");
    assert_eq!(
        requests
            .lines()
            .filter(|line| *line == "thread/turns/list\tthread-created")
            .count(),
        0,
        "{requests}"
    );

    provider.request("observer-create-readiness-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("observer-create-readiness-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_returns_empty_history_for_unmaterialized_new_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unmaterialized.txt");
    let request_log = directory.path().join("unmaterialized-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure_with_request_log(
        "unmaterialized-before-first-message",
        &marker,
        Some(&request_log),
    );

    let fetched = provider.request(
        "unmaterialized-get",
        "conversation.get",
        json!({ "conversation": conversation }),
    );

    assert!(fetched.get("error").is_none(), "{fetched}");
    assert_eq!(
        fetched
            .pointer("/result/conversation/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("thread-created")
    );
    assert_eq!(
        fetched.pointer("/result/items").and_then(Value::as_array),
        Some(&Vec::new())
    );
    assert!(fetched.pointer("/result/conversation/activeTurn").is_none());
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "initialize",
            "model/list",
            "project/list",
            "account/read",
            "account/rateLimits/read",
            "account/usage/read",
            "thread/start",
            "thread/read\tthread-created",
            "thread/turns/list\tthread-created"
        ]
    );

    provider.request("unmaterialized-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("unmaterialized-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_starts_first_turn_without_reading_unmaterialized_history() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unmaterialized-first-turn.txt");
    let request_log = directory.path().join("unmaterialized-first-turn-requests.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure_with_request_log(
        "unmaterialized-before-first-message",
        &marker,
        Some(&request_log),
    );
    std::fs::write(&request_log, "").unwrap();

    let started = provider.request(
        "unmaterialized-first-turn",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "unmaterialized-first-message",
            "start without persisted history",
            &capability_revision,
        ),
    );

    assert!(started.get("error").is_none(), "{started}");
    assert!(started.pointer("/result/turn/resource/nativeResourceId").is_some());
    let output = provider.event("event.turnOutputDelta");
    assert_eq!(
        output
            .pointer("/params/conversation/nativeResourceId")
            .and_then(Value::as_str),
        conversation.get("nativeResourceId").and_then(Value::as_str)
    );
    assert_eq!(
        output.pointer("/params/delta").and_then(Value::as_str),
        Some("fixture output")
    );
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["turn/start\tthread-created"]
    );

    provider.request("unmaterialized-first-turn-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("unmaterialized-first-turn-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_exposes_codex_authentication_usage_and_projects() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("provider-runtime-metadata.txt");
    let mut provider = ProviderBinary::spawn();
    provider.initialize_and_create_instance(
        &app_server_executable(),
        fixture_args("normal", &marker),
    );

    let started = provider.request(
        "runtime-metadata-start",
        "instance.start",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        started
            .pointer("/result/instance/authentication/status")
            .and_then(Value::as_str),
        Some("signed-in")
    );
    assert_eq!(
        started
            .pointer("/result/instance/authentication/displayText")
            .and_then(Value::as_str),
        Some("Signed in · ChatGPT Pro")
    );
    assert_eq!(
        started
            .pointer("/result/instance/usage/displayText")
            .and_then(Value::as_str),
        Some("5h 75% remaining · 7d 60% remaining")
    );
    assert_eq!(
        started
            .pointer("/result/instance/usage/details")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    let projects = provider.request(
        "runtime-metadata-projects",
        "project.list",
        json!({ "route": route_value(), "limit": 40 }),
    );
    assert_eq!(
        projects
            .pointer("/result/projects/0/name")
            .and_then(Value::as_str),
        Some("Fixture Project")
    );

    provider.request("runtime-metadata-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("runtime-metadata-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_acquire_interaction_reuses_created_session_and_returns_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("acquire-interaction.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("normal", &marker);
    clear_session_log(&marker);

    let first = provider.request(
        "acquire-interaction-first",
        "conversation.acquireInteraction",
        json!({ "conversation": conversation.clone() }),
    );
    let second = provider.request(
        "acquire-interaction-second",
        "conversation.acquireInteraction",
        json!({ "conversation": conversation.clone() }),
    );

    for response in [&first, &second] {
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(
            response
                .pointer("/result/selection/accessModeId")
                .and_then(Value::as_str),
            Some("workspace-write")
        );
        assert_eq!(
            response
                .pointer("/result/selection/reasoningEffortId")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            response
                .pointer("/result/selection/model/modelId")
                .and_then(Value::as_str),
            Some("gpt-fixture")
        );
        assert!(
            response
                .pointer("/result/leaseExpiresAt")
                .and_then(Value::as_u64)
                .is_none()
        );
    }
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-created").len(),
        0
    );

    let started = provider.request(
        "acquire-interaction-turn",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "acquire-interaction-message",
            "keep the leased writer",
            &capability_revision,
        ),
    );
    let turn = started.pointer("/result/turn/resource").cloned().unwrap();
    let approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();
    provider.request(
        "acquire-interaction-approval",
        "approval.resolve",
        json!({ "approval": approval, "decision": "approve" }),
    );
    provider.request(
        "acquire-interaction-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation.clone(), "turn": turn }),
    );
    let restarted = provider.request(
        "acquire-interaction-restart",
        "turn.start",
        turn_start_params(
            conversation,
            "acquire-interaction-message-two",
            "reuse the leased writer",
            &capability_revision,
        ),
    );
    assert!(restarted.get("error").is_none(), "{restarted}");
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-created").len(),
        0
    );

    provider.request("acquire-interaction-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("acquire-interaction-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn sdk_heartbeat_keeps_shared_server_until_last_client_disconnects_and_restarts_once() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("client-presence.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, revision) = provider.configure("none", &marker);
    let server = session_pids(&marker, "process/start", "");
    assert_eq!(server.len(), 1);
    let ping = |sequence: u64, clients: &[&str]| json!({
        "sequence": sequence, "hostSessionId": "test-host",
        "clients": { "revision": sequence, "connections": clients.iter().map(|id|
            json!({ "clientId": id, "connectionId": format!("connection-{id}") })).collect::<Vec<_>>() },
        "instances": [route_value()]
    });
    provider.request("two-clients", "provider.ping", ping(1, &["phone-a", "phone-b"]));
    for id in ["thread-a", "thread-b"] {
        let response = provider.request(id, "turn.start", turn_start_params(
            conversation_resource_value(id), &format!("message-{id}"), "keep running", &revision));
        assert!(response.get("error").is_none(), "{response}");
    }
    provider.request("one-client", "provider.ping", ping(2, &["phone-b"]));
    provider.collect_for(Duration::from_millis(80));
    assert!(process_is_running(server[0]));
    assert_eq!(session_pids(&marker, "process/start", ""), server);
    provider.request("no-clients", "provider.ping", ping(3, &[]));
    wait_for_processes_to_exit(&server, Duration::from_secs(2));
    let alive = provider.request("provider-still-alive", "provider.describe", json!({}));
    assert!(alive.get("error").is_none());
    provider.buffered.retain(|message| message.get("method").and_then(Value::as_str) != Some("event.instanceStatusChanged"));
    provider.request("reconnected", "provider.ping", ping(4, &["phone-a"]));
    provider.receive(Duration::from_secs(3), |message|
        message.get("method").and_then(Value::as_str) == Some("event.instanceStatusChanged")
        && message.pointer("/params/instance/status").and_then(Value::as_str) == Some("ready"));
    assert_eq!(session_pids(&marker, "process/start", "").len(), 2);
    provider.request("presence-stop", "provider.ping", ping(5, &[]));
    let all_servers = session_pids(&marker, "process/start", "");
    wait_for_processes_to_exit(&all_servers, Duration::from_secs(2));
    provider.request("presence-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_conversation_get_pages_history_without_resuming() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("paginated-history.txt");
    let request_log = directory.path().join("paginated-history-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("none", &marker, Some(&request_log));
    std::fs::write(&request_log, "").unwrap();
    clear_session_log(&marker);
    let conversation = conversation_resource_value("thread-paginated");

    let first = provider.request(
        "paginated-history-first",
        "conversation.get",
        json!({ "conversation": conversation.clone(), "limit": 1 }),
    );
    let wider = provider.request(
        "paginated-history-wider",
        "conversation.get",
        json!({ "conversation": conversation.clone(), "limit": 2 }),
    );
    let second = provider.request(
        "paginated-history-second",
        "conversation.get",
        json!({
            "conversation": conversation,
            "cursor": first.pointer("/result/pageInfo/nextCursor").unwrap(),
            "limit": 1
        }),
    );

    assert!(first.get("error").is_none(), "{first}");
    assert!(wider.get("error").is_none(), "{wider}");
    assert!(second.get("error").is_none(), "{second}");
    assert_eq!(
        first
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert_eq!(
        first
            .pointer("/result/items/0/contents/0/contentId")
            .and_then(Value::as_str),
        Some("agent-page-two:text")
    );
    assert_eq!(
        first
            .pointer("/result/pageInfo/nextCursor")
            .and_then(Value::as_str),
        Some("page-two")
    );
    assert_eq!(
        wider
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-one")
    );
    assert_eq!(
        wider
            .pointer("/result/items/1/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert!(wider.pointer("/result/pageInfo/nextCursor").is_none());
    assert_eq!(
        second
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-one")
    );
    assert!(second.pointer("/result/pageInfo/nextCursor").is_none());
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated",
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated",
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated"
        ]
    );
    assert!(session_pids(&marker, "process/start", "").is_empty());
    assert!(session_pids(&marker, "thread/resume", "thread-paginated").is_empty());

    provider.request("paginated-history-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("paginated-history-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reads_full_turns_for_app_server_0_152() {
    assert_provider_reads_full_turns("app-server-0.152-history");
}

#[test]
fn provider_binary_read_errors_preserve_shared_server_and_other_conversations() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("read-error-isolation.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("read-error-isolation", &marker);
    let original_pids = session_pids(&marker, "process/start", "");
    assert_eq!(original_pids.len(), 1);
    for (conversation, error_code) in [
        ("thread-oversized", "provider_protocol_error"),
        ("thread-invalid-json", "provider_protocol_error"),
        ("thread-invalid-envelope", "provider_protocol_error"),
        ("thread-rpc-error", "provider_error"),
    ] {
        let response = provider.request(conversation, "conversation.get", json!({
            "conversation": conversation_resource_value(conversation), "limit": 20,
        }));
        assert_eq!(response.pointer("/error/data/code").and_then(Value::as_str), Some(error_code), "{response}");
        let healthy = provider.request("healthy-after-error", "conversation.get", json!({
            "conversation": conversation_resource_value("thread-other"), "limit": 20,
        }));
        assert!(healthy.get("error").is_none(), "{healthy}");
        assert_eq!(session_pids(&marker, "process/start", ""), original_pids);
    }
    provider.request("isolation-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("isolation-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reads_full_turns_with_host_client_user_agent() {
    assert_provider_reads_full_turns("app-server-code-pet-history");
}

#[test]
fn provider_binary_forwards_turn_limit_and_truncates_only_tool_text_with_item_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("tool-text-policy.txt");
    let requests = directory.path().join("tool-text-policy-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log("app-server-code-pet-history", &marker, Some(&requests));
    let pids = session_pids(&marker, "process/start", "");
    std::fs::write(&requests, "").unwrap();
    let response = provider.request("tool-text-policy", "conversation.get", json!({
        "conversation": conversation_resource_value("thread-tool-text-policy"), "limit": 20,
    }));
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response.pointer("/result/pageInfo/nextCursor"), Some(&json!("native-next-page")));
    let items = response.pointer("/result/items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 3);
    let text = items[0].pointer("/tool/outcome/content/0/text").unwrap().as_str().unwrap();
    assert!(text.len() <= codepet_provider_sdk::DEFAULT_TOOL_TEXT_BYTES);
    assert_eq!(items[0].pointer("/_meta/truncations/0/originalBytes"), Some(&json!(20 * 1024 * 1024)));
    assert_eq!(items[0].pointer("/_meta/truncations/0/retainedBytes"), Some(&json!(text.len())));
    assert_eq!(items[1].pointer("/contents/0/text").unwrap().as_str().unwrap().len(), 300_000);
    assert_eq!(items[2].pointer("/tool/outcome/content/0/text").unwrap().as_str().unwrap().len(), 300_000);
    assert!(items[1].get("_meta").is_none() && items[2].get("_meta").is_none());
    assert_eq!(std::fs::read_to_string(&requests).unwrap().lines().collect::<Vec<_>>(), vec![
        "thread/read\tthread-tool-text-policy", "thread/turns/list\tthread-tool-text-policy",
    ]);
    assert_eq!(session_pids(&marker, "process/start", ""), pids);
    provider.request("tool-text-policy-shutdown", "provider.shutdown", json!({}));
}

#[tokio::test]
async fn oversized_event_keeps_shared_server_and_subsequent_events_available() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("event-error-isolation.txt");
    let rejected = Arc::new(AtomicBool::new(false));
    let sink_rejected = rejected.clone();
    let (sender, receiver) = mpsc::channel();
    let events = Arc::new(move |event| {
        if matches!(&event, ProtocolEvent::EventTurnUpserted { .. })
            && !sink_rejected.swap(true, Ordering::SeqCst)
        {
            return Err(codepet_provider_sdk::ProtocolError {
                code: "provider_frame_too_large".to_string(),
                message: "fixture event exceeds wire frame limit".to_string(),
                retryable: false, details: None,
            });
        }
        sender.send(event).unwrap();
        Ok(())
    });
    let (provider, route, capability_revision) = configured_direct_provider_with_events(
        "complete-on-approval", &marker, events,
    ).await;
    let pids = session_pids(&marker, "process/start", "");
    assert_eq!(pids.len(), 1);
    ProviderProtocolServer::turn_start(provider.as_ref(), TurnStartRequest {
        conversation: conversation_resource(&route, "thread-event-isolation"),
        client_request_id: "event-isolation-start".to_string(), capability_revision,
        input: TurnInput { kind: TurnInputKind::Text, text: "fixture turn".to_string() },
        selection: TurnSelection { access_mode_id: None, reasoning_effort_id: None, model: None },
    }).await.unwrap();
    loop {
        if matches!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), ProtocolEvent::EventApprovalRequested { .. }) { break; }
    }
    assert!(rejected.load(Ordering::SeqCst));
    ProviderProtocolServer::project_list(provider.as_ref(), ProjectListRequest {
        route, cursor: None, limit: Some(20),
    }).await.unwrap();
    assert_eq!(session_pids(&marker, "process/start", ""), pids);
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {}).await.unwrap();
}

fn assert_provider_reads_full_turns(mode: &str) {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("item-paginated-history.txt");
    let request_log = directory.path().join("item-paginated-history-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log(
        mode,
        &marker,
        Some(&request_log),
    );
    std::fs::write(&request_log, "").unwrap();

    let response = provider.request(
        "item-paginated-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-paginated"),
            "limit": 1
        }),
    );

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert_eq!(
        response
            .pointer("/result/pageInfo/nextCursor")
            .and_then(Value::as_str),
        Some("page-two")
    );
    assert_eq!(response.pointer("/result/conversation/project"), Some(&Value::Null));
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated"
        ]
    );

    provider.request("item-paginated-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("item-paginated-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reads_full_turns_without_item_probes_on_app_server_0_151() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("legacy-full-history.txt");
    let request_log = directory.path().join("legacy-full-history-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log(
        "app-server-0.151-history",
        &marker,
        Some(&request_log),
    );
    std::fs::write(&request_log, "").unwrap();

    let response = provider.request(
        "legacy-full-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-paginated"),
            "limit": 1
        }),
    );

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated"
        ]
    );

    provider.request("legacy-full-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("legacy-full-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reuses_one_execution_session_until_authoritative_terminal_state() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("turn-lifecycle.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("normal", &marker);
    let observer_pid = session_pids(&marker, "model/list", "")[0];
    let creation_pid = session_pids(&marker, "thread/start", "")[0];
    assert_eq!(observer_pid, creation_pid);
    clear_session_log(&marker);

    let started = provider.request(
        "lifecycle-turn-start",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "lifecycle-message-one",
            "start lifecycle",
            &capability_revision,
        ),
    );
    let turn = started
        .pointer("/result/turn/resource")
        .cloned()
        .unwrap();
    let approval = provider
        .event("event.approvalRequested")
        .pointer("/params/approval/resource")
        .cloned()
        .unwrap();

    provider.collect_for(Duration::from_millis(50));
    let resolved = provider.request(
        "lifecycle-approval",
        "approval.resolve",
        json!({ "approval": approval, "decision": "approve" }),
    );
    assert_eq!(
        resolved
            .pointer("/result/approval/status")
            .and_then(Value::as_str),
        Some("approved")
    );
    let steered = provider.request(
        "lifecycle-steer",
        "turn.steer",
        json!({
            "conversation": conversation.clone(),
            "turn": turn.clone(),
            "clientMessageId": "lifecycle-message-two",
            "message": "continue lifecycle"
        }),
    );
    assert!(steered.get("error").is_none());
    let interrupted = provider.request(
        "lifecycle-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation.clone(), "turn": turn }),
    );
    assert_eq!(
        interrupted
            .pointer("/result/turn/status")
            .and_then(Value::as_str),
        Some("interrupted")
    );

    let restarted = provider.request(
        "lifecycle-turn-restart",
        "turn.start",
        turn_start_params(
            conversation,
            "lifecycle-message-three",
            "restart after terminal snapshot",
            &capability_revision,
        ),
    );
    assert!(restarted.get("error").is_none());

    let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
    assert!(resume_pids.is_empty());
    let approval_pids = session_pids(&marker, "approval/response", "thread-created");
    assert_eq!(approval_pids.len(), 1);
    let first_execution = approval_pids[0];
    assert_eq!(first_execution, creation_pid);
    assert_eq!(observer_pid, first_execution);
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-created"),
        vec![first_execution]
    );
    assert_eq!(
        session_pids(&marker, "turn/interrupt", "thread-created"),
        vec![first_execution]
    );

    provider.request("lifecycle-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("lifecycle-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_keeps_waiting_user_input_execution_detached_from_remote_reads() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("waiting-user-input.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) =
        provider.configure("waiting-user-input", &marker);
    let observer_pid = session_pids(&marker, "model/list", "")[0];
    clear_session_log(&marker);

    let started = provider.request(
        "waiting-input-start",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "waiting-input-message",
            "wait for input",
            &capability_revision,
        ),
    );
    let turn = started
        .pointer("/result/turn/resource")
        .cloned()
        .unwrap();
    let execution_pid = session_pids(&marker, "turn/start", "thread-created")[0];
    let fetched = provider.request(
        "waiting-input-read",
        "conversation.get",
        json!({ "conversation": conversation.clone() }),
    );
    assert_eq!(
        fetched
            .pointer("/result/conversation/status")
            .and_then(Value::as_str),
        Some("waiting-user-input")
    );

    provider.collect_for(Duration::from_millis(50));
    let steered = provider.request(
        "waiting-input-steer",
        "turn.steer",
        json!({
            "conversation": conversation.clone(),
            "turn": turn.clone(),
            "clientMessageId": "waiting-input-follow-up",
            "message": "input supplied"
        }),
    );
    assert!(steered.get("error").is_none());
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-created"),
        vec![execution_pid]
    );
    let read_pids = session_pids(&marker, "thread/read", "thread-created");
    assert!(read_pids.contains(&observer_pid));
    assert!(read_pids.contains(&execution_pid));
    assert!(session_pids(&marker, "thread/resume", "thread-created").is_empty());

    provider.request(
        "waiting-input-interrupt",
        "turn.interrupt",
        json!({ "conversation": conversation, "turn": turn }),
    );
    provider.request("waiting-input-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("waiting-input-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_terminal_notification_preserves_other_conversations_in_shared_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("parallel-terminal.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("complete-on-approval", &marker);
    clear_session_log(&marker);
    let conversation_a = conversation_resource_value("thread-a");
    let conversation_b = conversation_resource_value("thread-b");

    let started_a = provider.request(
        "parallel-start-a",
        "turn.start",
        turn_start_params(
            conversation_a.clone(),
            "parallel-message-a",
            "run A",
            &capability_revision,
        ),
    );
    let turn_a = started_a.pointer("/result/turn/resource").cloned().unwrap();
    let approval_one = provider
        .event("event.approvalRequested")
        .pointer("/params/approval")
        .cloned()
        .unwrap();
    let started_b = provider.request(
        "parallel-start-b",
        "turn.start",
        turn_start_params(
            conversation_b.clone(),
            "parallel-message-b",
            "run B",
            &capability_revision,
        ),
    );
    let turn_b = started_b.pointer("/result/turn/resource").cloned().unwrap();
    let approval_two = provider
        .event("event.approvalRequested")
        .pointer("/params/approval")
        .cloned()
        .unwrap();
    let (approval_a, _approval_b) = if approval_one
        .pointer("/conversation/nativeResourceId")
        .and_then(Value::as_str)
        == Some("thread-a")
    {
        (approval_one, approval_two)
    } else {
        (approval_two, approval_one)
    };

    provider.request(
        "parallel-resolve-a",
        "approval.resolve",
        json!({ "approval": approval_a["resource"].clone(), "decision": "approve" }),
    );
    provider.receive(Duration::from_secs(5), |message| {
        message.get("method").and_then(Value::as_str) == Some("event.turnUpserted")
            && message
                .pointer("/params/turn/conversation/nativeResourceId")
                .and_then(Value::as_str)
                == Some("thread-a")
            && message
                .pointer("/params/turn/status")
                .and_then(Value::as_str)
                == Some("completed")
    });

    let steered_b = provider.request(
        "parallel-steer-b",
        "turn.steer",
        json!({
            "conversation": conversation_b.clone(),
            "turn": turn_b.clone(),
            "clientMessageId": "parallel-message-b-two",
            "message": "B remains active"
        }),
    );
    assert!(steered_b.get("error").is_none());
    let restarted_a = provider.request(
        "parallel-restart-a",
        "turn.start",
        turn_start_params(
            conversation_a,
            "parallel-message-a-two",
            "A starts again",
            &capability_revision,
        ),
    );
    assert!(restarted_a.get("error").is_none());

    let a_pids = session_pids(&marker, "thread/resume", "thread-a");
    let b_pids = session_pids(&marker, "thread/resume", "thread-b");
    assert_eq!(a_pids.len(), 1);
    assert_eq!(b_pids.len(), 1);
    assert_eq!(a_pids[0], b_pids[0]);
    assert_eq!(
        session_pids(&marker, "turn/steer", "thread-b"),
        vec![b_pids[0]]
    );

    provider.request("parallel-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("parallel-start-provider", "instance.start", json!({ "route": route_value() }));
    let resumed_b = provider.request(
        "parallel-steer-b-after-stop",
        "turn.steer",
        json!({
            "conversation": conversation_b,
            "turn": turn_b,
            "clientMessageId": "parallel-message-b-three",
            "message": "B resumes after Provider stop"
        }),
    );
    assert!(resumed_b.get("error").is_none());
    let b_pids_after_stop = session_pids(&marker, "thread/resume", "thread-b");
    assert_eq!(b_pids_after_stop.len(), 2);
    assert_ne!(b_pids_after_stop[0], b_pids_after_stop[1]);

    provider.request("parallel-stop-final", "instance.stop", json!({ "route": route_value() }));
    provider.request("parallel-shutdown", "provider.shutdown", json!({}));
    let _ = turn_a;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_publication_precedes_next_operation_without_closing_shared_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("terminal-closing-race.txt");
    let terminal_entered = Arc::new(Barrier::new(2));
    let terminal_release = Arc::new(Barrier::new(2));
    let sink_entered = terminal_entered.clone();
    let sink_release = terminal_release.clone();
    let (event_sender, event_receiver) = mpsc::channel();
    let events = Arc::new(move |event: ProtocolEvent| {
        let is_terminal = matches!(
            &event,
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.status == codepet_provider_sdk::TurnStatus::Completed
                    && params.turn.conversation.native_resource_id == "thread-terminal-race"
        );
        event_sender.send(event).map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "test_event_sink_closed".to_string(),
            message: error.to_string(),
            retryable: false,
            details: None,
        })?;
        if is_terminal {
            sink_entered.wait();
            sink_release.wait();
        }
        Ok(())
    });
    let (provider, route, capability_revision) = configured_direct_provider_with_events(
        "complete-on-approval",
        &marker,
        events,
    )
    .await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-terminal-race");
    ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.clone(),
            client_request_id: "terminal-race-first".to_string(),
            capability_revision: capability_revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "finish the first turn".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        },
    )
    .await
    .unwrap();
    let approval = loop {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if let ProtocolEvent::EventApprovalRequested { params, .. } = event {
            break params.approval.resource;
        }
    };
    ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: conversation_resource(&route, &approval.native_resource_id),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap();
    let entered = terminal_entered.clone();
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();

    let next_provider = provider.clone();
    let next_conversation = conversation.clone();
    let next = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            next_provider.as_ref(),
            TurnStartRequest {
                conversation: next_conversation,
                client_request_id: "terminal-race-second".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "start after terminal cleanup".to_string(),
                },
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!next.is_finished());
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-terminal-race").len(),
        1
    );

    let release = terminal_release.clone();
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), next)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let execution_pids = session_pids(&marker, "thread/resume", "thread-terminal-race");
    assert_eq!(execution_pids.len(), 1);
    assert!(process_is_running(execution_pids[0]));

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_server_keeps_writer_through_delayed_output_and_terminal_forwarding() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("delayed-user-item-output.txt");
    let (event_sender, event_receiver) = mpsc::channel();
    let terminal_forwarded_while_running = Arc::new(AtomicBool::new(false));
    let sink_terminal_forwarded_while_running = terminal_forwarded_while_running.clone();
    let sink_marker = marker.clone();
    let events = Arc::new(move |event: ProtocolEvent| {
        if matches!(
            &event,
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.resource.native_resource_id == "turn-started"
                    && params.turn.status == codepet_provider_sdk::TurnStatus::Completed
        ) {
            let execution_pids =
                session_pids(&sink_marker, "turn/start", "thread-delayed-user-item");
            sink_terminal_forwarded_while_running.store(
                execution_pids.len() == 1 && process_is_running(execution_pids[0]),
                Ordering::SeqCst,
            );
        }
        event_sender.send(event).map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "test_event_sink_closed".to_string(),
            message: error.to_string(),
            retryable: false,
            details: None,
        })
    });
    let (provider, route, capability_revision) =
        configured_direct_provider_with_events_and_hook(
            "delayed-output-after-user-item",
            &marker,
            events,
            None,
        )
        .await;
    let conversation = conversation_resource(&route, "thread-delayed-user-item");
    ProviderProtocolServer::conversation_acquire_interaction(
        provider.as_ref(),
        ConversationAcquireInteractionRequest {
            conversation: conversation.clone(),
        },
    )
    .await
    .unwrap();
    clear_session_log(&marker);

    let started = ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.clone(),
            client_request_id: "delayed-user-item".to_string(),
            capability_revision,
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "keep the writer while model output is delayed".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        },
    )
    .await
    .unwrap();
    assert!(started.user_item.is_none());
    let execution_pid = session_pids(&marker, "turn/start", "thread-delayed-user-item")[0];

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_completed_user_item = false;
    let mut saw_delayed_output = false;
    let mut saw_terminal = false;
    while !saw_terminal {
        let event = event_receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        match event {
            ProtocolEvent::EventConversationItemUpserted { params, .. }
                if item_resource_id(&params.item) == "user-one"
                    && item_status(&params.item)
                        == codepet_provider_sdk::ConversationItemStatus::Completed =>
            {
                saw_completed_user_item = true;
            }
            ProtocolEvent::EventTurnOutputDelta { params, .. }
                if params.turn.native_resource_id == "turn-started"
                    && params.delta == "delayed fixture output" =>
            {
                assert!(saw_completed_user_item);
                assert!(process_is_running(execution_pid));
                saw_delayed_output = true;
            }
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.resource.native_resource_id == "turn-started"
                    && params.turn.status == codepet_provider_sdk::TurnStatus::Completed =>
            {
                assert!(saw_delayed_output);
                saw_terminal = true;
            }
            _ => {}
        }
    }
    assert!(saw_completed_user_item);
    assert!(terminal_forwarded_while_running.load(Ordering::SeqCst));

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefetched_ready_handle_remains_valid_after_terminal_in_shared_server() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("prefetched-ready-handle.txt");
    let handle_entered = Arc::new(Barrier::new(2));
    let handle_release = Arc::new(Barrier::new(2));
    let hook = Arc::new(BlockHandleOnceHook {
        conversation_id: "thread-prefetched-handle".to_string(),
        operation: "turn.start",
        armed: AtomicBool::new(false),
        entered: handle_entered.clone(),
        release: handle_release.clone(),
    });
    let (event_sender, event_receiver) = mpsc::channel();
    let events = Arc::new(move |event: ProtocolEvent| {
        event_sender.send(event).map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "test_event_sink_closed".to_string(),
            message: error.to_string(),
            retryable: false,
            details: None,
        })
    });
    let (provider, route, capability_revision) =
        configured_direct_provider_with_events_and_hook(
            "complete-on-approval",
            &marker,
            events,
            Some(hook.clone()),
        )
        .await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-prefetched-handle");
    ProviderProtocolServer::turn_start(
        provider.as_ref(),
        TurnStartRequest {
            conversation: conversation.clone(),
            client_request_id: "prefetched-first".to_string(),
            capability_revision: capability_revision.clone(),
            input: TurnInput {
                kind: TurnInputKind::Text,
                text: "start the first turn".to_string(),
            },
            selection: TurnSelection {
                access_mode_id: None,
                reasoning_effort_id: None,
                model: None,
            },
        },
    )
    .await
    .unwrap();
    let approval = loop {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if let ProtocolEvent::EventApprovalRequested { params, .. } = event {
            break params.approval.resource;
        }
    };
    let first_generation_pids =
        session_pids(&marker, "thread/resume", "thread-prefetched-handle");
    assert_eq!(first_generation_pids.len(), 1);

    hook.armed.store(true, Ordering::SeqCst);
    let next_provider = provider.clone();
    let next_conversation = conversation.clone();
    let next = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            next_provider.as_ref(),
            TurnStartRequest {
                conversation: next_conversation,
                client_request_id: "prefetched-second".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "start on the next generation".to_string(),
                },
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
            },
        )
        .await
    });
    let entered = handle_entered.clone();
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();

    ProviderProtocolServer::approval_resolve(
        provider.as_ref(),
        ApprovalResolveRequest {
            approval: conversation_resource(&route, &approval.native_resource_id),
            decision: ApprovalDecision::Approve,
        },
    )
    .await
    .unwrap();
    loop {
        let event = event_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        if matches!(
            event,
            ProtocolEvent::EventTurnUpserted { params, .. }
                if params.turn.status == codepet_provider_sdk::TurnStatus::Completed
                    && params.turn.conversation.native_resource_id == "thread-prefetched-handle"
        ) {
            break;
        }
    }
    assert!(process_is_running(first_generation_pids[0]));

    let release = handle_release.clone();
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), next)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let execution_pids = session_pids(&marker, "thread/resume", "thread-prefetched-handle");
    assert_eq!(execution_pids.len(), 1);
    assert_eq!(execution_pids, first_generation_pids);

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_cancelled_before_resume_linearization_never_writes_resume_or_start() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("cancel-before-resume-linearization.txt");
    let resume_entered = Arc::new(Barrier::new(2));
    let resume_release = Arc::new(Barrier::new(2));
    let cancelled_entered = Arc::new(Barrier::new(2));
    let cancelled_release = Arc::new(Barrier::new(2));
    let hook = Arc::new(BlockResumeUntilCancelledHook {
        conversation_id: "thread-resume-pending-linearization".to_string(),
        resume_entered: resume_entered.clone(),
        resume_release: resume_release.clone(),
        cancelled_entered: cancelled_entered.clone(),
        cancelled_release: cancelled_release.clone(),
    });
    let (provider, route, capability_revision) =
        configured_direct_provider_with_events_and_hook(
            "resume-no-response",
            &marker,
            Arc::new(|_| Ok(())),
            Some(hook),
        )
        .await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-resume-pending-linearization");
    let operation_provider = provider.clone();
    let operation = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            operation_provider.as_ref(),
            TurnStartRequest {
                conversation,
                client_request_id: "cancel-before-resume-message".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "stop before resume is sent".to_string(),
                },
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
            },
        )
        .await
    });
    let entered = resume_entered.clone();
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();
    let execution_pids: Vec<u32> = vec![];
    assert!(execution_pids.is_empty());
    assert_eq!(session_method_count(&marker, "initialize"), 0);

    let stop_provider = provider.clone();
    let stop_route = route.clone();
    let stop = tokio::spawn(async move {
        ProviderProtocolServer::instance_stop(
            stop_provider.as_ref(),
            InstanceStopRequest { route: stop_route },
        )
        .await
    });
    let cancelled = cancelled_entered.clone();
    tokio::task::spawn_blocking(move || cancelled.wait())
        .await
        .unwrap();

    let release_resume = resume_release.clone();
    tokio::task::spawn_blocking(move || release_resume.wait())
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while session_pids(
        &marker,
        "thread/resume",
        "thread-resume-pending-linearization",
    )
    .is_empty()
        && !operation.is_finished()
        && Instant::now() < deadline
    {
        tokio::task::yield_now().await;
    }
    let resume_was_written = !session_pids(
        &marker,
        "thread/resume",
        "thread-resume-pending-linearization",
    )
    .is_empty();

    let release_cancelled = cancelled_release.clone();
    tokio::task::spawn_blocking(move || release_cancelled.wait())
        .await
        .unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(2), stop)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(stopped.instance.status, codepet_provider_sdk::InstanceStatus::Stopped);
    let operation_error = tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(operation_error.code, "provider_unavailable");
    assert!(!resume_was_written);
    assert!(
        session_pids(
            &marker,
            "thread/resume",
            "thread-resume-pending-linearization",
        )
        .is_empty()
    );
    assert!(
        session_pids(
            &marker,
            "turn/start",
            "thread-resume-pending-linearization",
        )
        .is_empty()
    );
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));

    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[test]
fn shared_server_and_event_routing_are_stable_across_sixty_four_conversations() {
    const ITERATIONS: usize = 64;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("terminal-stress.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("complete-on-approval", &marker);
    clear_session_log(&marker);

    for index in 0..ITERATIONS {
        let thread_id = format!("thread-terminal-stress-{index}");
        let conversation = conversation_resource_value(&thread_id);
        let started = provider.request(
            &format!("terminal-stress-start-{index}"),
            "turn.start",
            turn_start_params(
                conversation,
                &format!("terminal-stress-message-{index}"),
                "complete this turn",
                &capability_revision,
            ),
        );
        assert!(started.get("error").is_none());
        let approval = provider
            .event("event.approvalRequested")
            .pointer("/params/approval/resource")
            .cloned()
            .unwrap();
        let resolved = provider.request(
            &format!("terminal-stress-resolve-{index}"),
            "approval.resolve",
            json!({ "approval": approval, "decision": "approve" }),
        );
        assert!(resolved.get("error").is_none());
        provider.receive(Duration::from_secs(2), |message| {
            message.get("method").and_then(Value::as_str) == Some("event.turnUpserted")
                && message
                    .pointer("/params/turn/conversation/nativeResourceId")
                    .and_then(Value::as_str)
                    == Some(thread_id.as_str())
                && message.pointer("/params/turn/status").and_then(Value::as_str)
                    == Some("completed")
        });
        let resume_pids = session_pids(&marker, "thread/resume", &thread_id);
        assert_eq!(resume_pids.len(), 1, "iteration {index} lost resume PID evidence");
        assert_eq!(
            session_pids(&marker, "approval/response", &thread_id),
            resume_pids,
            "iteration {index} crossed execution process evidence"
        );
    }
    assert_eq!(
        session_pids(&marker, "thread/resume", "*").len(),
        1
    );

    provider.request("terminal-stress-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("terminal-stress-shutdown", "provider.shutdown", json!({}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_first_turn_start_shares_one_resume_and_one_active_turn() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("concurrent-resume.txt");
    let (provider, route, capability_revision) =
        configured_direct_provider("none", &marker).await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-concurrent");
    let request = |client_request_id: &str| TurnStartRequest {
        conversation: conversation.clone(),
        client_request_id: client_request_id.to_string(),
        capability_revision: capability_revision.clone(),
        input: TurnInput {
            kind: TurnInputKind::Text,
            text: "concurrent start".to_string(),
        },
        selection: TurnSelection {
            access_mode_id: None,
            reasoning_effort_id: None,
            model: None,
        },
    };
    let first = ProviderProtocolServer::turn_start(provider.as_ref(), request("concurrent-one"));
    let second = ProviderProtocolServer::turn_start(provider.as_ref(), request("concurrent-two"));
    let (first, second) = tokio::join!(first, second);
    let results = [first, second];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(|error| error.code.as_str())
            .collect::<Vec<_>>(),
        vec!["turn_already_active"]
    );
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-concurrent").len(),
        1
    );
    assert_eq!(
        session_pids(&marker, "turn/start", "thread-concurrent").len(),
        1
    );

    ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_stop_interrupts_an_execution_still_waiting_for_resume() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stop-during-resume.txt");
    let (provider, route, capability_revision) =
        configured_direct_provider("resume-no-response", &marker).await;
    clear_session_log(&marker);
    let conversation = conversation_resource(&route, "thread-resume-pending");
    let operation_provider = provider.clone();
    let operation = tokio::spawn(async move {
        ProviderProtocolServer::turn_start(
            operation_provider.as_ref(),
            TurnStartRequest {
                conversation,
                client_request_id: "resume-pending-message".to_string(),
                capability_revision,
                input: TurnInput {
                    kind: TurnInputKind::Text,
                    text: "wait for resume".to_string(),
                },
                selection: TurnSelection {
                    access_mode_id: None,
                    reasoning_effort_id: None,
                    model: None,
                },
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while session_pids(&marker, "thread/resume", "thread-resume-pending").is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let stopped = ProviderProtocolServer::instance_stop(
        provider.as_ref(),
        InstanceStopRequest { route: route.clone() },
    )
    .await
    .unwrap();
    assert_eq!(stopped.instance.status, codepet_provider_sdk::InstanceStatus::Stopped);
    let operation_error = tokio::time::timeout(Duration::from_secs(3), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(operation_error.code, "provider_unavailable");
    assert_eq!(
        session_pids(&marker, "thread/resume", "thread-resume-pending").len(),
        1
    );

    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[test]
fn provider_binary_dispatches_other_conversations_and_stop_while_resume_hangs() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-concurrent-stop.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);
    let blocked_conversation = conversation_resource_value("thread-resume-pending-a");
    let other_conversation = conversation_resource_value("thread-b");

    provider.send_request(
        "stdio-blocked-a",
        "turn.start",
        turn_start_params(
            blocked_conversation,
            "stdio-blocked-message-a",
            "block A resume",
            &capability_revision,
        ),
    );
    wait_for_file_method(
        &marker,
        "thread/resume",
        "thread-resume-pending-a",
        Duration::from_secs(2),
    );
    provider.send_request(
        "stdio-other-b",
        "turn.start",
        turn_start_params(
            other_conversation,
            "stdio-other-message-b",
            "run B",
            &capability_revision,
        ),
    );
    let other_response = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("stdio-other-b")
    });
    assert_eq!(other_response["id"], "stdio-other-b");
    assert!(other_response.get("error").is_none());
    wait_for_file_method(
        &marker,
        "thread/resume",
        "thread-b",
        Duration::from_secs(2),
    );
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    provider.send_request("stdio-stop", "instance.stop", json!({ "route": route_value() }));
    let stopped = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("stdio-stop")
    });
    assert_eq!(stopped["id"], "stdio-stop");
    assert_eq!(
        stopped.pointer("/result/instance/status").and_then(Value::as_str),
        Some("stopped")
    );
    let blocked_response = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("stdio-blocked-a")
    });
    assert_eq!(blocked_response["id"], "stdio-blocked-a");
    assert!(blocked_response.get("error").is_some());
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));

    provider.request("stdio-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_reserves_dispatch_for_stop_when_all_normal_slots_are_blocked() {
    const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-saturated-stop.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);

    for index in 0..MAX_CONCURRENT_HOST_REQUESTS {
        provider.send_request(
            &format!("saturated-stop-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("saturated-stop-message-{index}"),
                "hold every normal dispatch slot",
                &capability_revision,
            ),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session_method_count(&marker, "thread/resume") < MAX_CONCURRENT_HOST_REQUESTS {
        assert!(Instant::now() < deadline, "bounded requests did not reach the dispatch limit");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        session_method_count(&marker, "thread/resume"),
        MAX_CONCURRENT_HOST_REQUESTS
    );
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    provider.send_request("saturated-heartbeat", "provider.ping", json!({
        "sequence": 1, "hostSessionId": "saturated-host",
        "clients": {"revision": 1, "connections": []}, "instances": []
    }));
    let pong = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("saturated-heartbeat")
    });
    assert_eq!(pong.pointer("/result/sequence"), Some(&json!(1)));
    provider.send_request(
        "saturated-instance-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    let stopped = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("saturated-instance-stop")
    });
    assert_eq!(stopped["id"], "saturated-instance-stop");
    assert_eq!(
        stopped.pointer("/result/instance/status").and_then(Value::as_str),
        Some("stopped")
    );
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));

    provider.request("saturated-provider-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_keeps_eof_visible_after_at_least_forty_nine_saturated_frames() {
    const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;
    const MAX_PENDING_HOST_MESSAGES: usize = 32;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-saturated-eof.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);

    for index in 0..MAX_CONCURRENT_HOST_REQUESTS {
        provider.send_request(
            &format!("saturated-eof-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("saturated-eof-message-{index}"),
                "saturate dispatch before EOF",
                &capability_revision,
            ),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session_method_count(&marker, "thread/resume") < MAX_CONCURRENT_HOST_REQUESTS {
        assert!(Instant::now() < deadline, "saturated requests did not reach the dispatch limit");
        std::thread::sleep(Duration::from_millis(5));
    }
    for index in MAX_CONCURRENT_HOST_REQUESTS
        ..(MAX_CONCURRENT_HOST_REQUESTS + MAX_PENDING_HOST_MESSAGES + 1)
    {
        provider.send_request(
            &format!("saturated-eof-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("saturated-eof-message-{index}"),
                "saturate dispatch before EOF",
                &capability_revision,
            ),
        );
    }
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    let status = provider.close_input_and_wait(Duration::from_secs(2));
    assert!(status.success());
    wait_for_processes_to_exit(&execution_pids, Duration::from_secs(2));
}

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
fn provider_binary_stop_linearizes_repeated_shared_server_initialization() {
    const ROUNDS: usize = 5;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("observer-initialize-stop.txt");
    let mut provider = ProviderBinary::spawn();
    provider.initialize_and_create_instance(
        &app_server_executable(),
        fixture_args("observer-initialize-no-response", &marker),
    );

    for round in 0..ROUNDS {
        let start_id = format!("observer-start-{round}");
        let stop_id = format!("observer-stop-{round}");
        provider.send_request(
            &start_id,
            "instance.start",
            json!({ "route": route_value() }),
        );
        wait_for_session_count(&marker, "initialize", round + 1, Duration::from_secs(2));
        let process_id = *session_pids(&marker, "process/start", "")
            .last()
            .expect("observer fixture process was not recorded");
        provider.send_request(
            &stop_id,
            "instance.stop",
            json!({ "route": route_value() }),
        );
        let stopped = provider.receive(Duration::from_secs(2), |message| {
            message.get("id").and_then(Value::as_str) == Some(stop_id.as_str())
        });
        let process_still_running = process_is_running(process_id);
        if process_still_running {
            terminate_processes(&[process_id]);
        }
        assert_eq!(stopped["id"], stop_id);
        assert_eq!(
            stopped.pointer("/result/instance/status").and_then(Value::as_str),
            Some("stopped")
        );
        assert!(
            !process_still_running,
            "round {round}: stop returned before observer {process_id} exited"
        );
        let start_response = provider.receive(Duration::from_secs(2), |message| {
            message.get("id").and_then(Value::as_str) == Some(start_id.as_str())
        });
        assert_eq!(start_response["id"], start_id);
        assert!(start_response.get("error").is_some());
    }
    provider.collect_for(Duration::from_millis(100));
    assert!(provider.buffered.iter().all(|message| {
        message.get("method").and_then(Value::as_str)
            != Some("event.instanceStatusChanged")
            || message
                .pointer("/params/instance/status")
                .and_then(Value::as_str)
                != Some("ready")
    }));

    let destroyed = provider.request(
        "observer-destroy",
        "instance.destroy",
        json!({ "route": route_value() }),
    );
    assert_eq!(destroyed["id"], "observer-destroy");
    provider.request("observer-shutdown", "provider.shutdown", json!({}));
}

#[cfg(unix)]
#[test]
fn provider_binary_async_event_broken_pipe_triggers_global_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("event-broken-pipe.txt");
    let mut provider = ProviderBinary::spawn_ignoring_sigpipe();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    let observer_pids = session_pids(&marker, "model/list", "");
    assert_eq!(observer_pids.len(), 1);
    clear_session_log(&marker);

    provider.send_request(
        "broken-pipe-turn",
        "turn.start",
        turn_start_params(
            conversation_resource_value("thread-broken-pipe"),
            "broken-pipe-message",
            "hold execution while stdout closes",
            &capability_revision,
        ),
    );
    wait_for_file_method(
        &marker,
        "thread/resume",
        "thread-broken-pipe",
        Duration::from_secs(2),
    );
    let execution_pids = session_pids(&marker, "thread/resume", "*");
    assert_eq!(execution_pids.len(), 1);

    provider.close_stdout_after_probe();
    terminate_processes(&observer_pids);
    let exit_status = provider.wait_for_exit(Duration::from_secs(2));
    let execution_processes_still_running = execution_pids
        .iter()
        .copied()
        .filter(|process_id| process_is_running(*process_id))
        .collect::<Vec<_>>();
    if exit_status.is_none() {
        let _ = provider.child.kill();
        let _ = provider.child.wait();
    }
    if !execution_processes_still_running.is_empty() {
        terminate_processes(&execution_processes_still_running);
    }
    assert!(
        exit_status.is_some(),
        "Provider ignored an async event stdout failure"
    );
    assert!(
        execution_processes_still_running.is_empty(),
        "stdout failure returned before execution sessions exited: {execution_processes_still_running:?}"
    );
}

#[test]
fn provider_binary_backpressures_excess_mux_requests_and_keeps_control_available() {
    const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;
    const MAX_PENDING_HOST_MESSAGES: usize = 32;
    const OVERLOADED_INDEX: usize = MAX_CONCURRENT_HOST_REQUESTS + MAX_PENDING_HOST_MESSAGES;

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("stdio-overload-response.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("resume-no-response", &marker);
    clear_session_log(&marker);

    for index in 0..MAX_CONCURRENT_HOST_REQUESTS {
        provider.send_request(
            &format!("overload-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("overload-message-{index}"),
                "verify explicit overload",
                &capability_revision,
            ),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session_method_count(&marker, "thread/resume") < MAX_CONCURRENT_HOST_REQUESTS {
        assert!(Instant::now() < deadline, "overload requests did not fill dispatch slots");
        std::thread::sleep(Duration::from_millis(5));
    }
    for index in MAX_CONCURRENT_HOST_REQUESTS..=OVERLOADED_INDEX {
        provider.send_request(
            &format!("overload-request-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(&format!("thread-resume-pending-{index}")),
                &format!("overload-message-{index}"),
                "verify explicit overload",
                &capability_revision,
            ),
        );
    }
    // Mux holds excess senders before opening streams; it does not dispatch them
    // and then synthesize the old flat-queue provider_overloaded response.
    provider.collect_for(Duration::from_millis(100));
    assert_eq!(session_method_count(&marker, "thread/resume"), MAX_CONCURRENT_HOST_REQUESTS);
    assert!(
        session_pids(
            &marker,
            "thread/resume",
            &format!("thread-resume-pending-{OVERLOADED_INDEX}")
        )
        .is_empty()
    );

    provider.send_request(
        "overload-instance-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    let stopped = provider.receive(Duration::from_secs(2), |message| {
        message.get("id").and_then(Value::as_str) == Some("overload-instance-stop")
    });
    assert_eq!(
        stopped.pointer("/result/instance/status").and_then(Value::as_str),
        Some("stopped")
    );
    assert!(
        session_pids(
            &marker,
            "thread/resume",
            &format!("thread-resume-pending-{OVERLOADED_INDEX}")
        )
        .is_empty()
    );
    provider.request("overload-provider-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_standardizes_writer_conflict_and_removes_failed_execution() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("writer-conflict.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("none", &marker);
    clear_session_log(&marker);
    let conversation = conversation_resource_value("thread-active-writer");

    for (id, client_request_id) in [
        ("writer-conflict-first", "writer-conflict-message-one"),
        ("writer-conflict-second", "writer-conflict-message-two"),
    ] {
        let response = provider.request(
            id,
            "turn.start",
            turn_start_params(
                conversation.clone(),
                client_request_id,
                "writer conflict",
                &capability_revision,
            ),
        );
        assert_eq!(
            response.pointer("/error/data/code").and_then(Value::as_str),
            Some("conversation_write_conflict")
        );
        assert_eq!(
            response
                .pointer("/error/data/retryable")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            response
                .pointer("/error/data/details/operation")
                .and_then(Value::as_str),
            Some("thread/resume")
        );
        assert_eq!(
            response
                .pointer("/error/data/details/reason")
                .and_then(Value::as_str),
            Some("owned-by-other-runtime")
        );
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(!encoded.contains("thread-active-writer already has an active writer"));
    }
    let resume_pids = session_pids(&marker, "thread/resume", "thread-active-writer");
    assert_eq!(resume_pids.len(), 1);
    assert_eq!(session_method_count(&marker, "thread/resume"), 2);
    assert!(session_pids(&marker, "turn/start", "thread-active-writer").is_empty());

    provider.request("writer-conflict-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("writer-conflict-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_does_not_guess_writer_conflicts_from_near_match_errors() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("writer-conflict-near-matches.txt");
    let mut provider = ProviderBinary::spawn();
    let (_, capability_revision) = provider.configure("none", &marker);
    clear_session_log(&marker);

    for (index, thread_id) in [
        "thread-active-writer-wrong-code",
        "thread-active-writer-wrong-message",
        "thread-active-writer-with-data",
        "thread-active-writer-other-thread",
    ]
    .into_iter()
    .enumerate()
    {
        let response = provider.request(
            &format!("writer-near-match-{index}"),
            "turn.start",
            turn_start_params(
                conversation_resource_value(thread_id),
                &format!("writer-near-match-message-{index}"),
                "writer near match",
                &capability_revision,
            ),
        );
        assert_eq!(
            response.pointer("/error/data/code").and_then(Value::as_str),
            Some("provider_error"),
            "near match {thread_id} must retain the ordinary Provider error"
        );
    }

    provider.request("writer-near-match-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("writer-near-match-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_keeps_rpc_reject_local_and_never_retries_unknown_outcome() {
    for (mode, expected_code) in [
        ("turn-reject", "provider_error"),
        ("turn-sent-unknown", "provider_unavailable"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join(format!("{mode}.txt"));
        let mut provider = ProviderBinary::spawn();
        let (conversation, capability_revision) = provider.configure(mode, &marker);
        clear_session_log(&marker);

        let first = provider.request(
            "failed-turn-first",
            "turn.start",
            turn_start_params(
                conversation.clone(),
                "failed-message-one",
                "fail once",
                &capability_revision,
            ),
        );
        assert_eq!(
            first.pointer("/error/data/code").and_then(Value::as_str),
            Some(expected_code)
        );
        assert_eq!(
            session_pids(&marker, "thread/resume", "thread-created").len(),
            0
        );
        assert_eq!(
            session_pids(&marker, "turn/start", "thread-created").len(),
            1
        );

        let second = provider.request(
            "failed-turn-second",
            "turn.start",
            turn_start_params(
                conversation,
                "failed-message-two",
                "explicit caller retry",
                &capability_revision,
            ),
        );
        assert_eq!(
            second.pointer("/error/data/code").and_then(Value::as_str),
            Some(expected_code)
        );
        let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
        assert!(resume_pids.is_empty());
        assert_eq!(
            session_method_count(&marker, "turn/start"),
            if mode == "turn-reject" { 2 } else { 1 }
        );

        provider.request("failed-turn-stop", "instance.stop", json!({ "route": route_value() }));
        provider.request("failed-turn-shutdown", "provider.shutdown", json!({}));
    }
}

#[test]
fn provider_binary_shared_server_crash_requires_instance_restart() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("execution-crash.txt");
    let mut provider = ProviderBinary::spawn();
    let (conversation, capability_revision) = provider.configure("crash-after-start", &marker);
    clear_session_log(&marker);

    let started = provider.request(
        "crash-start",
        "turn.start",
        turn_start_params(
            conversation.clone(),
            "crash-message-one",
            "start before crash",
            &capability_revision,
        ),
    );
    assert!(started.get("error").is_none());
    let turn = started
        .pointer("/result/turn/resource")
        .cloned()
        .unwrap();
    provider.collect_for(Duration::from_millis(100));

    let steered = provider.request(
        "crash-steer",
        "turn.steer",
        json!({
            "conversation": conversation,
            "turn": turn,
            "clientMessageId": "crash-message-two",
            "message": "resume after execution crash"
        }),
    );
    assert!(steered.get("error").is_some());
    let resume_pids = session_pids(&marker, "thread/resume", "thread-created");
    assert!(resume_pids.is_empty());

    provider.request("crash-stop", "instance.stop", json!({ "route": route_value() }));
    provider.request("crash-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_rejects_legacy_framing_before_initialization() {
    let mut child = Command::new(provider_executable())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut oversized_header = Vec::from(codepet_provider_sdk::PROVIDER_FRAME_MAGIC);
    oversized_header.push(codepet_provider_sdk::PROVIDER_FRAME_VERSION);
    oversized_header.push(codepet_provider_sdk::ProviderFrameEncoding::RawJson as u8);
    oversized_header.extend_from_slice(
        &(codepet_provider_sdk::MAX_PROVIDER_FRAME_BYTES as u32).to_be_bytes(),
    );
    stdin.write_all(&oversized_header).unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Provider did not fail-stop");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!status.success());
    let mut stdout = child.stdout.take().unwrap();
    let mut output = Vec::new();
    stdout.read_to_end(&mut output).unwrap();
    assert!(output.is_empty(), "Unnegotiated input must not dispatch or emit business messages");
}

#[test]
fn provider_binary_preserves_large_history_content_and_keeps_serving() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unused.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("none", &marker);

    let fetched = provider.request(
        "large-history",
        "conversation.get",
        json!({
            "conversation": {
                "deviceId": "device-provider-binary",
                "providerPluginId": CODEX_PLUGIN_ID,
                "providerInstanceId": "codex",
                "nativeResourceId": "thread-large"
            }
        }),
    );
    assert!(fetched.get("error").is_none(), "{fetched}");
    let text = fetched
        .pointer("/result/items/1/contents/0/text")
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(text.len(), 1024 * 1024 + 4096);
    assert!(fetched
        .pointer("/result/items/1/contents/0/truncation")
        .is_none());

    let described = provider.request("after-large-history", "provider.describe", json!({}));
    assert_eq!(
        described
            .pointer("/result/plugin/pluginId")
            .and_then(Value::as_str),
        Some(CODEX_PLUGIN_ID)
    );
    provider.request("large-history-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_pages_large_turns_without_transport_budgeting() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("oversized-history.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure("none", &marker);

    let fetched = provider.request(
        "oversized-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-output-too-large")
        }),
    );
    assert_eq!(fetched["id"], "oversized-history");
    assert!(fetched.get("error").is_none(), "{fetched}");
    assert_eq!(
        fetched
            .pointer("/result/items")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|item| item
                .pointer("/resource/nativeResourceId")
                .and_then(Value::as_str)
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["agent-large-one", "agent-large-two", "agent-large-three"]
    );
    assert!(fetched
        .pointer("/result/items/0/contents/0/truncation")
        .is_none());

    let reduced = provider.request(
        "reduced-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-output-too-large"),
            "limit": 2
        }),
    );
    assert!(reduced.get("error").is_none(), "{reduced}");
    assert_eq!(
        reduced
            .pointer("/result/items")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|item| item
                .pointer("/resource/nativeResourceId")
                .and_then(Value::as_str)
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["agent-large-two", "agent-large-three"]
    );
    assert_eq!(
        reduced
            .pointer("/result/pageInfo/nextCursor")
            .and_then(Value::as_str),
        Some("large-page-three")
    );

    let remainder = provider.request(
        "remaining-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-output-too-large"),
            "cursor": "large-page-three",
            "limit": 2
        }),
    );
    assert!(remainder.get("error").is_none(), "{remainder}");
    assert_eq!(
        remainder
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-large-one")
    );
    assert!(remainder.pointer("/result/pageInfo/nextCursor").is_none());

    let described = provider.request("after-oversized-history", "provider.describe", json!({}));
    assert_eq!(
        described
            .pointer("/result/plugin/pluginId")
            .and_then(Value::as_str),
        Some(CODEX_PLUGIN_ID)
    );
    provider.request("oversized-history-shutdown", "provider.shutdown", json!({}));
}

#[test]
#[ignore = "requires CODEPET_CODEX_EXECUTABLE pointing to a real Codex CLI"]
fn provider_real_codex_app_server_smoke() {
    let executable = std::env::var_os("CODEPET_CODEX_EXECUTABLE")
        .map(PathBuf::from)
        .expect("CODEPET_CODEX_EXECUTABLE must point to the resolved Codex executable");
    assert!(executable.is_absolute());
    assert!(executable.is_file());
    let mut provider = ProviderBinary::spawn();

    provider.request(
        "real-initialize",
        "provider.initialize",
        json!({
            "hostClientId": "provider-real-codex-smoke",
            "hostDeviceId": "device-provider-binary",
            "hostVersion": "test",
            "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
        }),
    );
    provider.request(
        "real-create",
        "instance.create",
        json!({
            "route": route_value(),
            "instanceKind": CODEX_INSTANCE_KIND,
            "displayName": "Codex Real Smoke",
            "settings": {
                "appServerExecutable": executable.to_string_lossy(),
                "appServerArgs": ["app-server", "--listen", "stdio://"]
            }
        }),
    );
    let started = provider.request(
        "real-start",
        "instance.start",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        started.pointer("/result/instance/status").and_then(Value::as_str),
        Some("ready")
    );
    let capability_revision = started
        .pointer("/result/instance/capabilities/revision")
        .and_then(Value::as_str)
        .expect("real Provider must advertise a capability revision")
        .to_string();
    let listed = provider.request(
        "real-list",
        "conversation.list",
        json!({
            "route": route_value(),
            "projectFilter": { "kind": "all" },
            "limit": 1
        }),
    );
    assert!(listed.pointer("/result/conversations").is_some());
    let projects_response = provider.request(
        "real-project-list",
        "project.list",
        json!({
            "route": route_value(),
            "limit": 100
        }),
    );
    let projects = projects_response
        .pointer("/result/projects")
        .and_then(Value::as_array)
        .expect("Codex 0.152 project.list must return projects");
    let standalone = provider.request(
        "real-standalone-list",
        "conversation.list",
        json!({
            "route": route_value(),
            "projectFilter": { "kind": "standalone" },
            "limit": 100
        }),
    );
    eprintln!(
        "real Codex projects={} standalone conversations={}",
        projects.len(),
        standalone
            .pointer("/result/conversations")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default()
    );
    assert!(
        standalone
            .pointer("/result/conversations")
            .and_then(Value::as_array)
            .is_some_and(|conversations| conversations.iter().all(|conversation| {
                conversation.get("project").is_none()
                    || conversation.get("project").is_some_and(Value::is_null)
            })),
        "standalone list returned a project-owned conversation: {standalone}"
    );
    for (index, project) in projects.iter().enumerate() {
        let resource = project
            .get("resource")
            .cloned()
            .expect("project must have a routed resource");
        let conversations = provider.request(
            &format!("real-project-conversations-{index}"),
            "conversation.list",
            json!({
                "route": route_value(),
                "projectFilter": { "kind": "project", "project": resource.clone() },
                "limit": 100
            }),
        );
        eprintln!(
            "project {:?} conversations={} response_error={:?}",
            project.get("name").and_then(Value::as_str),
            conversations
                .pointer("/result/conversations")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
            conversations.get("error")
        );
        assert!(conversations.get("error").is_none(), "{conversations}");
        assert!(
            conversations
                .pointer("/result/conversations")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().all(|conversation| {
                    conversation.get("project") == Some(&resource)
                })),
            "project list returned a conversation with the wrong membership: {conversations}"
        );
    }
    if let Some(workspace) = std::env::var_os("CODEPET_REAL_WORKSPACE") {
        let created = provider.request(
            "real-conversation-create",
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "workspaceRoot": workspace.to_string_lossy(),
                "workspaceMode": "worktree"
            }),
        );
        assert!(created
            .pointer("/result/conversation/resource/nativeResourceId")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty()));
        assert!(created
            .pointer("/result/conversation/workspaceRoot")
            .and_then(Value::as_str)
            .is_some_and(|path| path == workspace.to_string_lossy()));
    }
    if let Some(workspace) = std::env::var_os("CODEPET_REAL_UNMATERIALIZED_WORKSPACE") {
        let created = provider.request(
            "real-unmaterialized-create",
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "workspaceRoot": workspace.to_string_lossy(),
                "workspaceMode": "main"
            }),
        );
        assert!(created.get("error").is_none(), "{created}");
        let resource = created
            .pointer("/result/conversation/resource")
            .cloned()
            .expect("real conversation.create must return a resource");
        let fetched = provider.request(
            "real-unmaterialized-get",
            "conversation.get",
            json!({ "conversation": resource.clone() }),
        );
        assert!(fetched.get("error").is_none(), "{fetched}");
        assert_eq!(
            fetched.pointer("/result/conversation/resource"),
            Some(&resource)
        );
        assert_eq!(
            fetched
                .pointer("/result/items")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        const USER_MARKER: &str = "CODEPET_PROVIDER_USER_MARKER_0152";
        const ASSISTANT_MARKER: &str = "CODEPET_PROVIDER_ASSISTANT_OK_0152";
        provider.send_request(
            "real-unmaterialized-turn-start",
            "turn.start",
            turn_start_params(
                resource.clone(),
                "real-unmaterialized-client-message",
                &format!("{USER_MARKER}. Reply with exactly {ASSISTANT_MARKER}"),
                &capability_revision,
            ),
        );
        let turn_started = provider.receive(Duration::from_secs(30), |message| {
            message.get("id").and_then(Value::as_str)
                == Some("real-unmaterialized-turn-start")
        });
        assert!(turn_started.get("error").is_none(), "{turn_started}");
        let conversation_id = resource
            .get("nativeResourceId")
            .and_then(Value::as_str)
            .expect("created resource must have a native id");
        provider.receive(Duration::from_secs(120), |message| {
            message.get("method").and_then(Value::as_str) == Some("event.turnUpserted")
                && message
                    .pointer("/params/turn/conversation/nativeResourceId")
                    .and_then(Value::as_str)
                    == Some(conversation_id)
                && message.pointer("/params/turn/status").and_then(Value::as_str)
                    == Some("completed")
        });
        let materialized = provider.request(
            "real-materialized-get",
            "conversation.get",
            json!({ "conversation": resource, "limit": 20 }),
        );
        assert!(materialized.get("error").is_none(), "{materialized}");
        let serialized = materialized.to_string();
        assert!(serialized.contains(USER_MARKER), "user message was not materialized");
        assert!(
            serialized.contains(ASSISTANT_MARKER),
            "assistant response was not exposed by conversation.get: {materialized}"
        );
    }
    if let Some(conversation_id) = std::env::var_os("CODEPET_REAL_CONVERSATION_ID") {
        let conversation_id = conversation_id.to_string_lossy();
        let fetched = provider.request(
            "real-existing-conversation-get",
            "conversation.get",
            json!({
                "conversation": conversation_resource_value(&conversation_id),
                "limit": 1
            }),
        );
        assert!(fetched.get("error").is_none(), "{fetched}");
        assert_eq!(
            fetched
                .pointer("/result/conversation/resource/nativeResourceId")
                .and_then(Value::as_str),
            Some(conversation_id.as_ref())
        );
        assert!(fetched
            .pointer("/result/items")
            .is_some_and(Value::is_array));
        assert!(fetched.pointer("/result/pageInfo").is_some_and(Value::is_object));
    }
    let stopped = provider.request(
        "real-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    assert_eq!(
        stopped.pointer("/result/instance/status").and_then(Value::as_str),
        Some("stopped")
    );
    provider.request("real-shutdown", "provider.shutdown", json!({}));
}

#[test]
fn provider_binary_public_mux_smoke() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = ProviderBinary::spawn();
    let (conversation, _) = provider.configure("normal", &directory.path().join("smoke.txt"));
    let described = provider.request("smoke-describe", "provider.describe", json!({}));
    assert_eq!(described["result"]["plugin"]["pluginId"], CODEX_PLUGIN_ID);
    let listed = provider.request("smoke-list", "conversation.list", json!({
        "route": route_value(), "limit": 10, "projectFilter": { "kind": "all" }
    }));
    assert!(listed["result"]["conversations"].is_array());
    let fetched = provider.request("smoke-get", "conversation.get", json!({ "conversation": conversation }));
    assert_eq!(fetched["result"]["conversation"]["resource"], conversation);
    assert!(fetched["result"]["items"].is_array());
    let stopped = provider.request("smoke-stop", "instance.stop", json!({ "route": route_value() }));
    assert_eq!(stopped["result"]["instance"]["status"], "stopped");
    let shutdown = provider.request("smoke-shutdown", "provider.shutdown", json!({}));
    assert_eq!(shutdown["result"]["accepted"], true);
    assert!(provider.close_input_and_wait(Duration::from_secs(3)).success());
}

#[test]
fn automatic_title_is_saved_once_without_blocking_execution_or_publishing_ephemeral_events() {
    for mode in ["auto-title", "auto-title-failure"] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("title.marker");
        let requests = directory.path().join("requests.log");
        let mut provider = ProviderBinary::spawn();
        let (conversation, revision) = provider.configure_with_request_log(mode, &marker, Some(&requests));
        let started = provider.request("first-title-turn", "turn.start",
            turn_start_params(conversation.clone(), "first", "需要一个简短标题", &revision));
        assert_eq!(started["result"]["accepted"], true, "{started}");
        let output = provider.event("event.turnOutputDelta");
        assert_eq!(output["params"]["delta"], "fixture output");
        wait_for_session_count(&marker, "thread/unsubscribe", 1, Duration::from_secs(3));
        if mode == "auto-title" {
            let named = provider.receive(Duration::from_secs(3), |event| {
                event["method"] == "event.conversationUpserted" && event["params"]["conversation"]["title"] == "测试自动会话标题"
            });
            assert_eq!(named["params"]["conversation"]["resource"]["nativeResourceId"], "thread-created");
        }
        let interrupted = provider.request("interrupt-title-turn", "turn.interrupt", json!({
            "conversation": conversation, "turn": started["result"]["turn"]["resource"]
        }));
        assert!(interrupted.get("error").is_none(), "{interrupted}");
        let second = provider.request("second-title-turn", "turn.start",
            turn_start_params(conversation, "second", "继续对话", &revision));
        assert_eq!(second["result"]["accepted"], true, "{second}");
        provider.collect_for(Duration::from_millis(150));
        let log = std::fs::read_to_string(&requests).unwrap();
        assert_eq!(log.lines().filter(|line| *line == "turn/start\ttitle-job").count(), 1, "{log}");
        assert!(provider.buffered.iter().all(|event| !event.to_string().contains("title-job")));
        provider.request("title-stop", "instance.stop", json!({"route": route_value()}));
        provider.request("title-shutdown", "provider.shutdown", json!({}));
    }
}

struct ProviderBinary {
    child: Child,
    stdin: Option<mux_stdio::Writer>,
    stdout: mux_stdio::Reader,
    buffered: VecDeque<Value>,
}

impl ProviderBinary {
    fn spawn() -> Self {
        let mut command = Command::new(provider_executable());
        Self::spawn_command(&mut command)
    }

    #[cfg(unix)]
    fn spawn_ignoring_sigpipe() -> Self {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "trap '' PIPE; exec \"$1\"", "provider-without-sigpipe"])
            .arg(provider_executable());
        Self::spawn_command(&mut command)
    }

    fn spawn_command(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (stdin, stdout) = mux_stdio::connect(child.stdin.take().unwrap(), child.stdout.take().unwrap());
        Self { child, stdin: Some(stdin), stdout, buffered: VecDeque::new() }
    }

    fn configure(&mut self, approval_mode: &str, marker: &Path) -> (Value, String) {
        self.configure_with_request_log(approval_mode, marker, None)
    }

    fn configure_with_request_log(
        &mut self,
        approval_mode: &str,
        marker: &Path,
        request_log: Option<&Path>,
    ) -> (Value, String) {
        let mut app_server_args = vec![
            "--approval-mode".to_string(),
            approval_mode.to_string(),
            "--marker".to_string(),
            marker.to_string_lossy().into_owned(),
        ];
        if let Some(request_log) = request_log {
            app_server_args.push("--request-log".to_string());
            app_server_args.push(request_log.to_string_lossy().into_owned());
        }
        self.initialize_and_create_instance(&app_server_executable(), app_server_args);
        let started = self.request("start", "instance.start", json!({ "route": route_value() }));
        let capability_revision = started
            .pointer("/result/instance/capabilities/revision")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        (
            self.create_conversation("conversation-first"),
            capability_revision,
        )
    }

    fn initialize_and_create_instance(&mut self, executable: &Path, app_server_args: Vec<String>) {
        self.request(
            "initialize",
            "provider.initialize",
            json!({
                "hostClientId": "provider-binary-test",
                "hostDeviceId": "device-provider-binary",
                "hostVersion": "test",
                "supportedVersions": { "minVersion": 1, "maxVersion": 1 }
            }),
        );
        self.request(
            "create",
            "instance.create",
            json!({
                "route": route_value(),
                "instanceKind": CODEX_INSTANCE_KIND,
                "displayName": "Codex Binary Fixture",
                "settings": {
                    "appServerExecutable": executable,
                    "appServerArgs": app_server_args
                }
            }),
        );
    }

    fn create_conversation(&mut self, id: &str) -> Value {
        let workspace = std::env::temp_dir().join(format!("codepet-codex-provider-{}-{id}", self.child.id()));
        std::fs::create_dir_all(&workspace).unwrap();
        self.request(
            id,
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "model": "gpt-fixture",
                "reasoningEffort": "high",
                "workspaceRoot": workspace.to_string_lossy()
            }),
        )
        .pointer("/result/conversation/resource")
        .cloned()
        .unwrap()
    }

    fn request(&mut self, id: &str, method: &str, params: Value) -> Value {
        self.send_request(id, method, provider_request_params(method, params));
        self.receive(Duration::from_secs(5), |message| {
            message.get("id").and_then(Value::as_str) == Some(id)
        })
    }

    fn send_request(&mut self, id: &str, method: &str, params: Value) {
        let stdin = self.stdin.as_mut().expect("Provider stdin is closed");
        stdin.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    fn close_input_and_wait(&mut self, timeout: Duration) -> std::process::ExitStatus {
        self.close_input_and_wait_result(timeout)
            .expect("Provider did not exit after stdin EOF")
    }

    fn close_input_and_wait_result(
        &mut self,
        timeout: Duration,
    ) -> Option<std::process::ExitStatus> {
        drop(self.stdin.take());
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn close_stdout_after_probe(&mut self) {
        let described = self.request("close-stdout-probe", "provider.describe", json!({}));
        assert_eq!(described["id"], "close-stdout-probe");
        self.stdout.close();
    }

    fn event(&mut self, method: &str) -> Value {
        self.receive(Duration::from_secs(5), |message| {
            message.get("method").and_then(Value::as_str) == Some(method)
        })
    }

    fn receive<F>(&mut self, timeout: Duration, predicate: F) -> Value
    where
        F: Fn(&Value) -> bool,
    {
        if let Some(index) = self.buffered.iter().position(&predicate) {
            return self.buffered.remove(index).unwrap();
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self.stdout.messages.recv_timeout(remaining).unwrap();
            if predicate(&message) {
                return message;
            }
            self.buffered.push_back(message);
        }
    }

    fn collect_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            match self.stdout.messages.recv_timeout(remaining) {
                Ok(message) => self.buffered.push_back(message),
                Err(mpsc::RecvTimeoutError::Timeout) => return,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn provider_request_params(method: &str, mut params: Value) -> Value {
    let resource_fields: &[&str] = match method {
        "conversation.get" | "conversation.acquireInteraction" => &["conversation"],
        "turn.start" => &["conversation"],
        "turn.steer" | "turn.interrupt" => &["conversation", "turn"],
        "approval.resolve" => &["approval"],
        "project.get" | "project.update" | "project.delete" => &["project"],
        _ => &[],
    };
    for field in resource_fields {
        let Some(resource) = params.get_mut(*field) else {
            continue;
        };
        if resource.get("deviceId").is_some() {
            continue;
        }
        let native_resource_id = resource.get("nativeResourceId").cloned();
        if let Some(native_resource_id) = native_resource_id {
            *resource = json!({
                "deviceId": "device-provider-binary",
                "providerPluginId": CODEX_PLUGIN_ID,
                "providerInstanceId": "codex",
                "nativeResourceId": native_resource_id,
            });
        }
    }
    params
}

impl Drop for ProviderBinary {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn route_value() -> Value {
    json!({
        "deviceId": "device-provider-binary",
        "providerPluginId": CODEX_PLUGIN_ID,
        "providerInstanceId": "codex"
    })
}

fn fixture_args(approval_mode: &str, marker: &Path) -> Vec<String> {
    vec![
        "--approval-mode".to_string(),
        approval_mode.to_string(),
        "--marker".to_string(),
        marker.to_string_lossy().into_owned(),
    ]
}

fn conversation_resource_value(native_resource_id: &str) -> Value {
    let mut resource = route_value();
    resource["nativeResourceId"] = json!(native_resource_id);
    resource
}

fn conversation_resource(route: &ProviderInstanceRoute, native_resource_id: &str) -> ProviderResourceId {
    ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_resource_id.to_string(),
    }
}

fn clear_session_log(marker: &Path) {
    for path in session_log_paths(marker) {
        std::fs::remove_file(path).unwrap();
    }
}

fn session_pids(marker: &Path, method: &str, thread_id: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for path in session_log_paths(marker) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in contents.lines() {
            let mut fields = line.splitn(3, '\t');
            let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            let recorded_method = fields.next();
            let recorded_thread = fields.next();
            if recorded_method != Some(method) || (thread_id != "*" && recorded_thread != Some(thread_id)) {
                continue;
            }
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

fn session_log_paths(marker: &Path) -> Vec<PathBuf> {
    let Some(parent) = marker.parent() else {
        return Vec::new();
    };
    let Some(stem) = marker.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.sessions.");
    let mut paths = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
                .then(|| entry.path())
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn session_method_count(marker: &Path, method: &str) -> usize {
    session_log_paths(marker)
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|contents| {
            contents
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|line| line.split('\t').nth(1) == Some(method))
        .count()
}

fn wait_for_file_method(marker: &Path, method: &str, thread_id: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while session_pids(marker, method, thread_id).is_empty() {
        assert!(
            Instant::now() < deadline,
            "fixture did not record {method} for {thread_id}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_session_count(marker: &Path, method: &str, expected: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while session_method_count(marker, method) < expected {
        assert!(
            Instant::now() < deadline,
            "fixture did not record {expected} {method} requests"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn process_is_running(process_id: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(process_id.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
fn terminate_processes(process_ids: &[u32]) {
    for process_id in process_ids {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &process_id.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    wait_for_processes_to_exit(process_ids, Duration::from_secs(2));
}

#[cfg(windows)]
fn process_is_running(process_id: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {process_id}"), "/FO", "CSV", "/NH"])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(&process_id.to_string()))
}

fn wait_for_processes_to_exit(process_ids: &[u32], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let running = process_ids
            .iter()
            .copied()
            .filter(|process_id| process_is_running(*process_id))
            .collect::<Vec<_>>();
        if running.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "App Server processes did not exit: {running:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn turn_start_params(
    conversation: Value,
    client_request_id: &str,
    text: &str,
    capability_revision: &str,
) -> Value {
    json!({
        "conversation": conversation,
        "clientRequestId": client_request_id,
        "capabilityRevision": capability_revision,
        "input": { "kind": "text", "text": text },
        "selection": {}
    })
}

fn wait_for_file_blocking(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture marker was not written");
        std::thread::sleep(Duration::from_millis(5));
    }
}

async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !path.is_file() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
