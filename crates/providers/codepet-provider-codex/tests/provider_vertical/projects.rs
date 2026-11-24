use super::*;

#[tokio::test]
async fn project_methods_and_project_owned_conversation_fail_closed_when_probe_is_unsupported() {
    verify_unsupported_project("project-unsupported").await;
}

#[tokio::test]
async fn legacy_unknown_project_method_preserves_chat_capabilities() {
    verify_unsupported_project("project-unsupported-legacy").await;
}

#[tokio::test]
async fn project_probe_error_preserves_chat_capabilities() {
    verify_unsupported_project("project-probe-error").await;
}

#[tokio::test]
async fn malformed_project_probe_preserves_chat_capabilities() {
    verify_unsupported_project("project-probe-malformed").await;
    verify_unsupported_project("project-no-position-legacy").await;
}

#[tokio::test]
async fn project_order_resolution_rejects_repeated_cursors_and_missing_projects() {
    for mode in ["project-no-position-loop", "project-no-position-missing"] {
        let directory = tempfile::tempdir().unwrap();
        let (provider, route, _) =
            configured_direct_provider(mode, &directory.path().join("projects")).await;
        let error = provider
            .project_list(ProjectListRequest {
                route: route.clone(),
                cursor: Some("project-page-two".into()),
                limit: Some(1),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "provider_protocol_error");
        assert!(provider
            .instance_capabilities(InstanceCapabilitiesRequest {
                route: route.clone()
            })
            .await
            .unwrap()
            .capabilities
            .turn_send
            .is_some());
        provider
            .instance_stop(InstanceStopRequest { route })
            .await
            .unwrap();
        provider
            .provider_shutdown(ProviderShutdownRequest {})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn empty_project_continuation_does_not_scan_ordering() {
    let directory = tempfile::tempdir().unwrap();
    let (provider, route, _) = configured_direct_provider(
        "project-no-position-empty",
        &directory.path().join("projects"),
    )
    .await;
    let page = provider
        .project_list(ProjectListRequest {
            route: route.clone(),
            cursor: Some("end".into()),
            limit: Some(1),
        })
        .await
        .unwrap();
    assert!(page.projects.is_empty());
    assert!(page.page_info.next_cursor.is_none());
    provider
        .instance_stop(InstanceStopRequest { route })
        .await
        .unwrap();
    provider
        .provider_shutdown(ProviderShutdownRequest {})
        .await
        .unwrap();
}

#[tokio::test]
async fn projects_without_native_positions_preserve_order_across_pages_and_get() {
    for mode in [
        "project-no-position",
        "project-no-position-next",
        "project-no-position-future",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (provider, route, _) =
            configured_direct_provider(mode, &directory.path().join("projects")).await;
        wait_for_capabilities(&provider, &route).await;
        let first = provider
            .project_list(ProjectListRequest {
                route: route.clone(),
                cursor: None,
                limit: Some(1),
            })
            .await
            .unwrap();
        assert_eq!(first.projects[0].resource.native_resource_id, "project-z");
        assert_eq!(first.projects[0].position, 0);
        assert_eq!(
            first.page_info.next_cursor.as_deref(),
            Some("project-page-two")
        );
        let second = provider
            .project_list(ProjectListRequest {
                route: route.clone(),
                cursor: first.page_info.next_cursor,
                limit: Some(1),
            })
            .await
            .unwrap();
        assert_eq!(second.projects[0].resource.native_resource_id, "project-a");
        assert_eq!(second.projects[0].position, 1);
        assert!(second.page_info.next_cursor.is_none());
        let fetched = provider
            .project_get(ProjectGetRequest {
                project: conversation_resource(&route, "project-a"),
            })
            .await
            .unwrap();
        assert_eq!(fetched.project.position, 1);
        let created = provider
            .project_create(ProjectCreateRequest {
                route: route.clone(),
                idempotency_key: "project-create-new-shape".into(),
                name: "Second Project".into(),
                roots: vec![ProjectRoot {
                    path: "/fixture/workspace".into(),
                }],
                metadata: BTreeMap::new(),
            })
            .await
            .unwrap();
        assert_eq!(created.project.position, 1);
        let updated = provider
            .project_update(ProjectUpdateRequest {
                project: conversation_resource(&route, "project-a"),
                name: Some("Updated".into()),
                roots: None,
                metadata: None,
            })
            .await
            .unwrap();
        assert_eq!(updated.project.position, 1);
        provider
            .instance_stop(InstanceStopRequest { route })
            .await
            .unwrap();
        provider
            .provider_shutdown(ProviderShutdownRequest {})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn project_owned_conversation_uses_native_project_id_without_inferring_cwd() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("project-owned-create.txt");
    let (provider, route, _) = configured_direct_provider("project-owned-create", &marker).await;
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
        created
            .project
            .as_ref()
            .map(|resource| resource.provider_id.as_str()),
        Some(project.provider_instance_id.as_str())
    );
    assert_eq!(
        created
            .project
            .as_ref()
            .map(|resource| resource.native_resource_id.as_str()),
        Some(project.native_resource_id.as_str())
    );

    let fetched = ProviderProtocolServer::conversation_get(
        provider.as_ref(),
        ConversationGetRequest {
            conversation: conversation_resource(&route, &created.resource.native_resource_id),
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
        InstanceStopRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::provider_shutdown(provider.as_ref(), ProviderShutdownRequest {})
        .await
        .unwrap();
}
