use super::*;

#[tokio::test]
async fn provider_v1_round_trips_fixture_app_server_lifecycle_and_approval() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("approval.txt");
    let (event_sender, event_receiver) = mpsc::channel();
    let provider = CodexProvider::new(Arc::new(move |event| {
        event_sender
            .send(event)
            .map_err(|error| codepet_provider_sdk::ProtocolError {
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
            directories: None,
            host_client_id: "client-provider-fixture".to_string(),
            host_device_id: device_id.to_string(),
            host_version: "test".to_string(),
            protocol: ProtocolVersionOffer {
                min_version: PROTOCOL_VERSION,
                version: PROTOCOL_VERSION,
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
    assert_eq!(
        started.instance.status,
        codepet_provider_sdk::InstanceStatus::Starting
    );
    wait_for_capabilities(&provider, &route).await;
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
            project: conversation_resource(&route, &fixture_project.native_resource_id),
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
    assert_eq!(
        created_project.project.resource.native_resource_id,
        "project-created"
    );
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
            query: None,
            reader_scope: None,
            route: route.clone(),
            cursor: None,
            limit: Some(20),
            project_filter: all_project_filter(),
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.conversations[0].resource.provider_id, "codex");
    assert_eq!(
        listed.conversations[0].resource.native_resource_id,
        "thread-listed"
    );
    assert_eq!(
        listed.conversations[0].project.as_ref(),
        Some(&fixture_project)
    );
    let updated_boundary = listed.conversations[0].updated_at.unwrap();
    let date_query: ConversationListRequest = serde_json::from_value(json!({
        "route": route, "projectFilter": {"kind":"all"}, "limit":20,
        "query":{"kind":"updatedAfter","updatedAfter":updated_boundary}
    }))
    .unwrap();
    let dated = ProviderProtocolServer::conversation_list(&provider, date_query)
        .await
        .unwrap();
    assert!(dated
        .conversations
        .iter()
        .any(|row| row.resource.native_resource_id == "thread-listed"));
    let ids_query: ConversationListRequest = serde_json::from_value(json!({
        "route":route,"projectFilter":{"kind":"all"},"limit":20,
        "query":{"kind":"ids","ids":["thread-listed"]}
    }))
    .unwrap();
    let selected = ProviderProtocolServer::conversation_list(&provider, ids_query)
        .await
        .unwrap();
    assert_eq!(selected.conversations.len(), 1);
    assert_eq!(
        selected.conversations[0].resource.native_resource_id,
        "thread-listed"
    );
    let project_conversations = ProviderProtocolServer::conversation_list(
        &provider,
        ConversationListRequest {
            query: None,
            reader_scope: None,
            route: route.clone(),
            cursor: None,
            limit: Some(20),
            project_filter: ConversationProjectFilter::ConversationProjectFilterProject(
                ConversationProjectFilterProject {
                    kind: ConversationProjectFilterProjectKind::Project,
                    project: conversation_resource(&route, &fixture_project.native_resource_id),
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
            query: None,
            reader_scope: None,
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
    assert_eq!(
        searched.page_info.next_cursor.as_deref(),
        Some("search-next")
    );

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
    assert_eq!(
        content_id(&item_contents(&fetched.items[0])[0]),
        "user-one:input:0"
    );
    assert_eq!(
        content_id(&item_contents(&fetched.items[2])[0]),
        "reasoning-one:summary:0"
    );
    assert!(matches!(
        fetched.items[7],
        ConversationItem::UnknownConversationItem(_)
    ));
    assert!(fetched
        .items
        .iter()
        .all(|item| !matches!(item, ConversationItem::ApprovalConversationItem(_))));
    let fetched_json = serde_json::to_string(&fetched).unwrap();
    assert!(!fetched_json.contains("private raw reasoning"));
    assert!(!fetched_json.contains("must not escape"));
    let ContentBlock::ImageContentBlock(image) = &item_contents(&fetched.items[0])[1] else {
        panic!("native user image must remain an explicit image block");
    };
    assert_eq!(image.uri, "data:image/png;base64,private");
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
            conversation: conversation_resource(&route, &conversation.resource.native_resource_id),
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
            conversation: conversation_resource(&route, &conversation.resource.native_resource_id),
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
            conversation: conversation_resource(&route, &conversation.resource.native_resource_id),
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
    let deadline = Instant::now() + Duration::from_secs(3);
    while approval.is_none() || !saw_delta || !saw_title_update {
        let event = event_receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        match event {
            ProtocolEvent::EventConversationUpserted { params, .. } => {
                saw_title_update |= params.conversation.title == "Renamed by Codex";
            }
            ProtocolEvent::EventTurnOutputDelta { params, .. } => {
                saw_delta = params.delta == "fixture output"
                    && params.item_id == "agent-one"
                    && params.content_id == "agent-one:text";
            }
            ProtocolEvent::EventApprovalRequested { params, .. } => {
                approval = Some(params.approval);
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
            approval: conversation_resource(&route, &approval.resource.native_resource_id),
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
            conversation: conversation_resource(&route, &conversation.resource.native_resource_id),
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
    let ConversationItem::ApprovalConversationItem(approval_item) = approval_item else {
        unreachable!()
    };
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
        approval_item.approval.resource.native_resource_id,
        resolved_approval_id
    );

    let steered = ProviderProtocolServer::turn_steer(
        &provider,
        TurnSteerRequest {
            conversation: conversation_resource(&route, &conversation.resource.native_resource_id),
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
            conversation: conversation_resource(&route, &conversation.resource.native_resource_id),
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
    assert_eq!(
        stopped.instance.status,
        codepet_provider_sdk::InstanceStatus::Stopped
    );
    assert!(
        ProviderProtocolServer::instance_destroy(&provider, InstanceDestroyRequest { route },)
            .await
            .unwrap()
            .destroyed
    );
    assert!(
        ProviderProtocolServer::provider_shutdown(&provider, ProviderShutdownRequest {},)
            .await
            .unwrap()
            .accepted
    );
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

    provider.request(
        "stop-unsafe",
        "instance.stop",
        json!({ "route": route_value() }),
    );
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

    provider.request(
        "stop-first",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request(
        "start-second",
        "instance.start",
        json!({ "route": route_value() }),
    );
    let second_conversation = provider.create_conversation("conversation-second");
    let second_capabilities = provider.request(
        "catalog-second",
        "instance.capabilities",
        json!({ "route": route_value() }),
    );
    let second_capability_revision = second_capabilities
        .pointer("/result/capabilities/revision")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
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

    provider.request(
        "stop-second",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("shutdown-second", "provider.shutdown", json!({}));
}
