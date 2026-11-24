use super::*;

#[test]
fn repeated_thread_reads_do_not_load_or_configure_the_thread() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let operation_session = session.clone();
    let operation = thread::spawn(move || {
        (0..2)
            .map(|_| operation_session.thread_read("thread-writer-held").unwrap())
            .collect::<Vec<_>>()
    });

    for _ in 0..2 {
        let read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(read["method"], "thread/read");
        assert_eq!(read["params"]["threadId"], "thread-writer-held");
        assert_eq!(read["params"]["includeTurns"], true);
        peer_sender
            .send(json!({
                "id": read["id"],
                "result": {
                    "thread": thread_fixture(
                        "thread-writer-held",
                        "/tmp/project",
                        "idle",
                        vec![]
                    )
                }
            }))
            .unwrap();
    }

    let snapshots = operation.join().unwrap();
    assert_eq!(snapshots.len(), 2);
    assert!(snapshots
        .iter()
        .all(|snapshot| snapshot.thread.id == "thread-writer-held"));
    assert!(session.inner.loaded_threads.lock().unwrap().is_empty());
    assert!(session
        .inner
        .thread_configurations
        .lock()
        .unwrap()
        .is_empty());
    assert!(peer_receiver.try_recv().is_err());
    session.shutdown().unwrap();
}

#[test]
fn loaded_list_preserves_native_pagination_and_reads_metadata_without_turns() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let source = session.clone();
    let operation = thread::spawn(move || {
        let page = source
            .thread_loaded_list(Some("previous-id".into()), 100)
            .unwrap();
        assert_eq!(page, (vec!["loaded-id".into()], Some("loaded-id".into())));
        source.thread_read_metadata("loaded-id").unwrap()
    });
    let list = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(list["method"], "thread/loaded/list");
    assert_eq!(list["params"], json!({"cursor":"previous-id","limit":100}));
    peer_sender
        .send(json!({"id":list["id"],"result":{"data":["loaded-id"],"nextCursor":"loaded-id"}}))
        .unwrap();
    let read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(read["method"], "thread/read");
    assert_eq!(read["params"]["includeTurns"], false);
    peer_sender.send(json!({"id":read["id"],"result":{"thread":thread_fixture("loaded-id","/tmp/project","idle",vec![])}})).unwrap();
    assert_eq!(operation.join().unwrap().thread.id, "loaded-id");
    session.shutdown().unwrap();
}

#[test]
fn thread_list_preserves_native_cwds_from_the_wire() {
    let worktree = "/fixture/.codex/worktrees/linked/project";
    let deleted = "/fixture/.codex/worktrees/deleted/project";
    let (session, peer_receiver, peer_sender) = mock_session();
    let list_session = session.clone();
    let listed = thread::spawn(move || {
        list_session
            .thread_list(CodexThreadListRequest::default())
            .unwrap()
    });
    let request = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(request["method"], "thread/list");
    peer_sender
        .send(json!({
            "id": request["id"],
            "result": {
                "data": [
                    thread_fixture(
                        "thread-worktree",
                        worktree,
                        "idle",
                        vec![]
                    ),
                    thread_fixture(
                        "thread-deleted-worktree",
                        deleted,
                        "idle",
                        vec![]
                    )
                ],
                "nextCursor": null
            }
        }))
        .unwrap();

    let snapshots = listed.join().unwrap().data;

    assert_eq!(snapshots[0].thread.cwd, worktree);
    assert_eq!(snapshots[1].thread.cwd, deleted);
    assert_eq!(snapshots[0].workspace_root.as_deref(), Some(worktree));
    assert_eq!(snapshots[1].workspace_root.as_deref(), Some(deleted));
    session.shutdown().unwrap();
}

#[test]
fn typed_operations_preserve_normal_and_project_thread_start_parameters() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let operation_session = session.clone();
    let operations = thread::spawn(move || {
        let page = operation_session
            .thread_list(CodexThreadListRequest {
                cursor: Some("cursor-one".to_string()),
                limit: Some(20),
                project_id: None,
                workspace_root: Some("/work/project".to_string()),
                search_term: None,
            })
            .unwrap();
        assert_eq!(page.data.len(), 1);
        let read = operation_session.thread_read("thread-read").unwrap();
        assert_eq!(read.thread.id, "thread-read");
        let normal = operation_session
            .thread_start(CodexThreadStartRequest {
                workspace_root: None,
                project_id: None,
                permission_level: CodexPermissionLevel::ReadOnly,
                model: None,
                reasoning_effort: None,
            })
            .unwrap();
        assert_eq!(normal.workspace_root.as_deref(), Some("/tmp"));
        assert_eq!(
            normal.permission_level,
            Some(CodexPermissionLevel::ReadOnly)
        );
        let project = operation_session
            .thread_start(CodexThreadStartRequest {
                workspace_root: Some("/work/project".to_string()),
                project_id: Some("project-fixture".to_string()),
                permission_level: CodexPermissionLevel::FullAccess,
                model: Some("gpt-fixture".to_string()),
                reasoning_effort: Some("high".to_string()),
            })
            .unwrap();
        assert_eq!(project.workspace_root.as_deref(), Some("/work/project"));
        assert_eq!(
            project.permission_level,
            Some(CodexPermissionLevel::FullAccess)
        );
        assert_eq!(project.model.as_deref(), Some("gpt-fixture"));
        assert_eq!(project.reasoning_effort.as_deref(), Some("high"));
        let turn = operation_session
            .turn_start(CodexTurnStartRequest {
                thread_id: "thread-project".to_string(),
                message: "hello".to_string(),
                client_message_id: Some("message-one".to_string()),
                permission_level: Some(CodexPermissionLevel::FullAccess),
                model: Some("gpt-fixture".to_string()),
                reasoning_effort: Some("high".to_string()),
            })
            .unwrap();
        assert_eq!(turn.id, "turn-started");
        let steered = operation_session
            .turn_steer(CodexTurnSteerRequest {
                thread_id: "thread-project".to_string(),
                expected_turn_id: "turn-started".to_string(),
                message: "steer".to_string(),
                client_message_id: Some("message-two".to_string()),
            })
            .unwrap();
        assert_eq!(steered.id, "turn-started");
        let interrupted = operation_session
            .turn_interrupt("thread-project", "turn-started")
            .unwrap();
        assert_eq!(interrupted.status, CodexTurnStatus::Interrupted);
    });

    let list = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(list["method"], "thread/list");
    assert_eq!(list["params"]["cwd"], "/work/project");
    assert_eq!(list["params"]["sortKey"], "updated_at");
    assert_eq!(list["params"]["sortDirection"], "desc");
    assert_eq!(list["params"]["useStateDbOnly"], true);
    assert!(list["params"].get("searchTerm").is_none());
    peer_sender
        .send(json!({
            "id": list["id"],
            "result": {
                "data": [thread_fixture("thread-listed", "/work/project", "idle", vec![])],
                "nextCursor": null
            }
        }))
        .unwrap();

    let read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(read["method"], "thread/read");
    assert_eq!(read["params"]["includeTurns"], true);
    peer_sender
        .send(json!({
            "id": read["id"],
            "result": {
                "thread": thread_fixture("thread-read", "/tmp", "idle", vec![])
            }
        }))
        .unwrap();

    let normal = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(normal["method"], "thread/start");
    assert!(normal["params"].get("cwd").is_none());
    assert!(normal["params"].get("projectId").is_none());
    assert_eq!(normal["params"]["sandbox"], "read-only");
    assert_eq!(normal["params"]["approvalPolicy"], "on-request");
    peer_sender
        .send(json!({
            "id": normal["id"],
            "result": {
                "thread": thread_fixture("thread-normal", "/tmp", "idle", vec![]),
                "model": "default",
                "modelProvider": "openai",
                "cwd": "/tmp",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "reasoningEffort": "medium",
                "sandbox": {
                    "type": "readOnly",
                    "networkAccess": false
                }
            }
        }))
        .unwrap();

    let project = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(project["method"], "thread/start");
    assert_eq!(project["params"]["cwd"], "/work/project");
    assert_eq!(project["params"]["projectId"], "project-fixture");
    assert_eq!(project["params"]["sandbox"], "danger-full-access");
    assert_eq!(project["params"]["approvalPolicy"], "never");
    assert_eq!(
        project["params"]["config"]["model_reasoning_effort"],
        "high"
    );
    peer_sender
        .send(json!({
            "id": project["id"],
            "result": {
                "thread": thread_fixture("thread-project", "/work/project", "idle", vec![]),
                "model": "gpt-fixture",
                "modelProvider": "openai",
                "cwd": "/work/project",
                "approvalPolicy": "never",
                "approvalsReviewer": "user",
                "reasoningEffort": "high",
                "sandbox": {
                    "type": "dangerFullAccess"
                }
            }
        }))
        .unwrap();

    let start = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(start["method"], "turn/start");
    assert_eq!(start["params"]["input"][0]["text"], "hello");
    assert_eq!(start["params"]["clientUserMessageId"], "message-one");
    assert_eq!(start["params"]["sandboxPolicy"]["type"], "dangerFullAccess");
    assert_eq!(start["params"]["approvalPolicy"], "never");
    assert_eq!(start["params"]["model"], "gpt-fixture");
    assert_eq!(start["params"]["effort"], "high");
    peer_sender
        .send(json!({
            "id": start["id"],
            "result": { "turn": turn_fixture("turn-started", "inProgress") }
        }))
        .unwrap();

    let steer = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(steer["method"], "turn/steer");
    assert_eq!(steer["params"]["expectedTurnId"], "turn-started");
    peer_sender
        .send(json!({
            "id": steer["id"],
            "result": { "turnId": "turn-started" }
        }))
        .unwrap();

    let steer_read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(steer_read["method"], "thread/read");
    peer_sender
        .send(json!({
            "id": steer_read["id"],
            "result": {
                "thread": thread_fixture(
                    "thread-project",
                    "/work/project",
                    "active",
                    vec![turn_fixture("turn-started", "inProgress")]
                )
            }
        }))
        .unwrap();

    let interrupt = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(interrupt["method"], "turn/interrupt");
    assert_eq!(interrupt["params"]["turnId"], "turn-started");
    peer_sender
        .send(json!({
            "id": interrupt["id"],
            "result": {}
        }))
        .unwrap();

    let interrupt_read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(interrupt_read["method"], "thread/read");
    peer_sender
        .send(json!({
            "id": interrupt_read["id"],
            "result": {
                "thread": thread_fixture(
                    "thread-project",
                    "/work/project",
                    "idle",
                    vec![turn_fixture("turn-started", "interrupted")]
                )
            }
        }))
        .unwrap();

    operations.join().unwrap();
    session.shutdown().unwrap();
}

#[test]
fn project_operations_use_the_experimental_app_server_contract() {
    let (session, peer_receiver, peer_sender) = mock_session();
    *session.inner.harness_version.lock().unwrap() = Some("0.166.0-alpha.5.1".into());
    let operation_session = session.clone();
    let operations = thread::spawn(move || {
        let listed = operation_session
            .project_list(Some("cursor-one".to_string()), Some(5))
            .unwrap();
        assert_eq!(listed.next_cursor.as_deref(), Some("cursor-two"));
        assert_eq!(listed.data[0].id, "project-one");
        assert_eq!(
            operation_session
                .project_read("project-one")
                .unwrap()
                .position,
            3
        );
        operation_session
            .project_create(CodexProjectCreateRequest {
                idempotency_key: "create-one".to_string(),
                name: "Project One".to_string(),
                roots: vec![crate::protocol::CodexProjectRoot {
                    path: "/fixture/project".to_string(),
                }],
                metadata: std::collections::BTreeMap::from([(
                    "team".to_string(),
                    "gateway".to_string(),
                )]),
            })
            .unwrap();
        operation_session
            .project_update(CodexProjectUpdateRequest {
                project_id: "project-one".to_string(),
                name: Some("Project Renamed".to_string()),
                roots: None,
                metadata: None,
            })
            .unwrap();
        operation_session.project_delete("project-one").unwrap();
    });

    let project_json = || {
        json!({
            "id": "project-one",
            "name": "Project One",
            "roots": [{ "path": "/fixture/project" }],
            "metadata": { "team": "gateway" },
            "position": 3,
            "createdAt": 11,
            "updatedAt": 12
        })
    };

    let list = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(list["method"], "project/list");
    assert_eq!(
        list["params"],
        json!({ "cursor": "cursor-one", "limit": 5 })
    );
    peer_sender
        .send(json!({
            "id": list["id"],
            "result": { "data": [project_json()], "nextCursor": "cursor-two" }
        }))
        .unwrap();

    let read = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(read["method"], "project/read");
    assert_eq!(read["params"], json!({ "projectId": "project-one" }));
    peer_sender
        .send(json!({ "id": read["id"], "result": { "project": project_json() } }))
        .unwrap();

    let create = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(create["method"], "project/create");
    assert_eq!(create["params"]["idempotencyKey"], "create-one");
    assert_eq!(create["params"]["roots"][0]["path"], "/fixture/project");
    assert_eq!(create["params"]["metadata"]["team"], "gateway");
    peer_sender
        .send(json!({ "id": create["id"], "result": { "project": project_json() } }))
        .unwrap();

    let update = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(update["method"], "project/update");
    assert_eq!(
        update["params"],
        json!({ "projectId": "project-one", "name": "Project Renamed" })
    );
    peer_sender
        .send(json!({ "id": update["id"], "result": { "project": project_json() } }))
        .unwrap();

    let delete = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(delete["method"], "project/delete");
    assert_eq!(delete["params"], json!({ "projectId": "project-one" }));
    peer_sender
        .send(json!({ "id": delete["id"], "result": {} }))
        .unwrap();

    operations.join().unwrap();
    session.shutdown().unwrap();
}

#[test]
fn historical_threads_resume_once_per_app_server_session() {
    for session_number in 1..=2 {
        let (session, peer_receiver, peer_sender) = mock_session();
        let operation_session = session.clone();
        let operation = thread::spawn(move || {
            for message_number in 1..=2 {
                operation_session
                    .turn_start(CodexTurnStartRequest {
                        thread_id: "thread-historical".to_string(),
                        message: format!("message {message_number}"),
                        client_message_id: Some(format!(
                            "session-{session_number}-message-{message_number}"
                        )),
                        permission_level: None,
                        model: None,
                        reasoning_effort: None,
                    })
                    .unwrap();
            }
        });

        let resume = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(resume["method"], "thread/resume");
        assert_eq!(resume["params"]["threadId"], "thread-historical");
        assert_eq!(resume["params"]["excludeTurns"], true);
        peer_sender
            .send(json!({
                "id": resume["id"],
                "result": {
                    "thread": thread_fixture(
                        "thread-historical",
                        "/tmp/project",
                        "idle",
                        vec![]
                    ),
                    "model": "default",
                    "modelProvider": "openai",
                    "cwd": "/tmp/project",
                    "approvalPolicy": "on-request",
                    "approvalsReviewer": "user",
                    "reasoningEffort": null,
                    "sandbox": {
                        "type": "workspaceWrite",
                        "writableRoots": [],
                        "networkAccess": false,
                        "excludeTmpdirEnvVar": false,
                        "excludeSlashTmp": false
                    }
                },
                "emittedAtMs": 1234
            }))
            .unwrap();

        for message_number in 1..=2 {
            let start = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(start["method"], "turn/start");
            assert_eq!(
                start["params"]["clientUserMessageId"],
                format!("session-{session_number}-message-{message_number}")
            );
            peer_sender
                .send(json!({
                    "id": start["id"],
                    "result": {
                        "turn": turn_fixture(
                            &format!("turn-{session_number}-{message_number}"),
                            "inProgress"
                        )
                    }
                }))
                .unwrap();
        }

        operation.join().unwrap();
        assert!(peer_receiver.try_recv().is_err());
        session.shutdown().unwrap();
    }
}

#[test]
fn failed_resume_wakes_waiter_to_retry_without_caching_failure() {
    let (session, peer_receiver, peer_sender) = mock_session();
    let first_session = session.clone();
    let first = thread::spawn(move || first_session.ensure_thread_loaded_outcome("thread-missing"));
    let first_resume = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(first_resume["method"], "thread/resume");

    let second_session = session.clone();
    let second =
        thread::spawn(move || second_session.ensure_thread_loaded_outcome("thread-missing"));
    assert!(peer_receiver
        .recv_timeout(Duration::from_millis(20))
        .is_err());
    peer_sender
        .send(json!({
            "id": first_resume["id"],
            "error": {
                "code": -32600,
                "message": "no rollout found for thread id thread-missing"
            }
        }))
        .unwrap();

    let second_resume = peer_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(second_resume["method"], "thread/resume");
    assert_eq!(second_resume["params"]["threadId"], "thread-missing");
    peer_sender
        .send(json!({
            "id": second_resume["id"],
            "error": {
                "code": -32600,
                "message": "no rollout found for thread id thread-missing"
            }
        }))
        .unwrap();

    for outcome in [first.join().unwrap(), second.join().unwrap()] {
        assert!(matches!(
            outcome,
            CodexRequestOutcome::ExplicitRpcReject(CodexAppServerError::Rpc {
                code: -32600,
                message,
                data: None
            }) if message == "no rollout found for thread id thread-missing"
        ));
    }
    assert!(peer_receiver.try_recv().is_err());
    session.shutdown().unwrap();
}
