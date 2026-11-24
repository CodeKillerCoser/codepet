use super::*;
use serde_json::json;
fn create(root: &Path) -> CreateTaskInput {
    serde_json::from_value(json!({"title":"任务目标","description":"任务背景","agentWorkspace":{"path":root},"initialRun":{"kind":"bugfix","title":"首次定位","goal":"检查菜单","solution":"排查事件","description":"本次排查","validationPlan":{"items":[{"title":"检查菜单","steps":"点击","expected":"打开"}]}}})).unwrap()
}
#[test]
fn task_and_run_are_independent_and_retries_are_atomic() {
    let d = tempfile::tempdir().unwrap();
    let l = Ledger::new(d.path());
    let input = create(d.path());
    let a = Actor::agent("codex").unwrap();
    let r = l.create_task(input.clone(), a.clone()).unwrap();
    assert_eq!(l.create_task(input, a.clone()).unwrap().task_id, r.task_id);
    let u = UpdateTaskRunInput {
        task_id: r.task_id.clone(),
        run_id: r.run_id.clone(),
        expected_revision: r.revision,
        description: Some("确认原因并修复".into()),
        status: Some(RunStatus::Published),
        delivery: Some("1.0".into()),
        ..Default::default()
    };
    let r = l.update_task_run(u, a).unwrap();
    let task = l.get(&r.task_id).unwrap();
    assert_eq!(task.description, "任务背景");
    assert_eq!(task.runs[0].description, "确认原因并修复");
    assert_eq!(task.created_by, "codex");
    assert!(serde_json::to_value(task).unwrap().get("rounds").is_none());
    let input: CreateTaskRunInput = serde_json::from_value(
        json!({"taskId":r.task_id,"expectedRevision":r.revision,"title":"发布后复发","kind":"bugfix"}),
    )
    .unwrap();
    let next = l
        .create_task_run(input, Actor::agent("other-agent").unwrap())
        .unwrap();
    let task = l.get(&next.task_id).unwrap();
    assert_eq!(task.runs.len(), 2);
    assert_eq!(
        l.find(FindTasksInput {
            query: Some("本次排查".into()),
            ..Default::default()
        })
        .unwrap()["total"],
        1
    );
    assert_eq!(task.runs[1].created_by, "other-agent");
    assert_eq!(task.created_by, "codex");
    assert_ne!(
        task.runs[0].validation_plan.items[0].id,
        task.runs[1].validation_plan.items[0].id
    );
    assert!(task.runs[1].validation_plan.records.is_empty());
    assert!(l
        .update_task_run(
            UpdateTaskRunInput {
                task_id: task.id,
                run_id: r.run_id,
                expected_revision: next.revision,
                ..Default::default()
            },
            Actor::agent("codex").unwrap()
        )
        .is_err());
}

#[test]
fn migration_preserves_ids_evidence_and_v2_bytes() {
    let d = tempfile::tempdir().unwrap();
    let old = legacy::Ledger::new(d.path());
    let input:legacy_model::SaveInput=serde_json::from_value(json!({"workspace":d.path(),"title":"旧任务","objective":"旧目标详情","description":"旧背景","kind":"bug","plan":{"summary":"旧方案","criteria":[{"title":"检查","steps":"操作","expected":"通过"}]}})).unwrap();
    let r = old.save(input, legacy::Actor::Agent).unwrap();
    let historical:legacy_model::SaveInput=serde_json::from_value(json!({"taskId":r.task_id,"expectedRevision":r.revision,"phase":"published","delivery":"历史版本 1.0","verification":{"planRevision":1,"stage":"local","target":"历史工作区","environment":"macOS","results":[{"criterionId":r.criterion_ids[0],"result":"passed","evidence":"旧验收证据"}]}})).unwrap();
    let r = old.save(historical, legacy::Actor::Agent).unwrap();
    let path = d.path().join(format!(
        "tasks/v2/projects/{}/tasks/{}.json",
        old.get(&r.task_id).unwrap().project_id,
        r.task_id
    ));
    let bytes = std::fs::read(&path).unwrap();
    let l = Ledger::new(d.path());
    let task = l.get(&r.task_id).unwrap();
    assert_eq!(task.id, r.task_id);
    assert_eq!(task.runs[0].id, r.round_id);
    assert_eq!(task.created_by, "unknown");
    assert_eq!(task.runs[0].kind, TaskKind::Bugfix);
    assert!(task.description.contains("旧目标详情"));
    assert!(task.runs[0].description.contains("旧方案"));
    assert_eq!(task.runs[0].validation_plan.items[0].id, r.criterion_ids[0]);
    assert_eq!(
        task.runs[0].validation_plan.records[0].input.results[0].evidence,
        "旧验收证据"
    );
    assert!(task.runs[0]
        .timeline
        .iter()
        .any(|event| event.kind == "imported_delivery"
            && event.reference.as_deref() == Some("历史版本 1.0")));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(d.path().join(format!(
            "tasks/v4/projects/{}/tasks/{}.json",
            task.project_id, task.id
        )))
        .unwrap(),
    )
    .unwrap();
    assert!(saved.get("runs").is_some());
    assert!(saved.get("objective").is_none());
    assert_eq!(Ledger::new(d.path()).list().unwrap().len(), 1);
}
#[test]
fn permissions_versions_and_human_acceptance_survive_the_new_shape() {
    let d = tempfile::tempdir().unwrap();
    let l = Ledger::new(d.path());
    let r = l
        .create_task(create(d.path()), Actor::agent("codex").unwrap())
        .unwrap();
    assert!(Actor::agent("human").is_err());
    assert!(Actor::agent("codex\ninjected").is_err());
    assert!(serde_json::from_value::<UpdateTaskRunInput>(
        json!({"taskId":r.task_id,"runId":r.run_id,"expectedRevision":1,"taskTitle":"不能改任务"})
    )
    .is_err());
    let verification = |target: &str| VerificationInput {
        plan_revision: 1,
        stage: Scope::Release,
        target: target.into(),
        environment: "macOS".into(),
        results: vec![CheckResult {
            criterion_id: r.criterion_ids[0].clone(),
            result: TestResult::Passed,
            evidence: "实际验证".into(),
        }],
    };
    let update = |rev, record| UpdateTaskRunInput {
        task_id: r.task_id.clone(),
        run_id: r.run_id.clone(),
        expected_revision: rev,
        verification: Some(record),
        ..Default::default()
    };
    let r2 = l
        .update_task_run(
            update(1, verification("1.0")),
            Actor::agent("codex").unwrap(),
        )
        .unwrap();
    assert!(l
        .close(&r.task_id, &r.run_id, r2.revision, true, "试图接受自测")
        .is_err());
    assert!(l
        .update_task_run(
            UpdateTaskRunInput {
                task_id: r.task_id.clone(),
                run_id: r.run_id.clone(),
                expected_revision: r2.revision,
                status: Some(RunStatus::Closed),
                ..Default::default()
            },
            Actor::agent("codex").unwrap()
        )
        .is_err());
    let r3 = l
        .update_task_run(update(r2.revision, verification("1.0")), Actor::human())
        .unwrap();
    assert!(human_acceptance_ready(&l.get(&r.task_id).unwrap().runs[0]));
    let closed = l
        .close(&r.task_id, &r.run_id, r3.revision, true, "人工通过")
        .unwrap();
    assert_eq!(closed.status, RunStatus::Closed);
}
#[test]
fn conflicting_writers_and_retries_do_not_duplicate_milestones() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let l = Ledger::new(data.path());
    let input = create(work.path());
    let r = l
        .create_task(input.clone(), Actor::agent("codex").unwrap())
        .unwrap();
    let jobs = (0..2)
        .map(|i| {
            let l = l.clone();
            let r = r.clone();
            std::thread::spawn(move || {
                l.update_task_run(
                    UpdateTaskRunInput {
                        task_id: r.task_id,
                        run_id: r.run_id,
                        expected_revision: r.revision,
                        note: Some(format!("writer {i}")),
                        ..Default::default()
                    },
                    Actor::agent("codex").unwrap(),
                )
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        jobs.into_iter()
            .map(|j| j.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    std::fs::remove_dir_all(work.path()).unwrap();
    assert_eq!(
        l.create_task(input, Actor::agent("codex").unwrap())
            .unwrap()
            .task_id,
        r.task_id
    );
    assert_eq!(l.get(&r.task_id).unwrap().revision, 2);
}
fn git(path: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn workspace_observes_rename_untracked_staged_and_conflicts() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    assert!(git(p, &["init", "-b", "main"]).status.success());
    git(p, &["config", "user.name", "QA"]);
    git(p, &["config", "user.email", "qa@example.invalid"]);
    for name in ["conflict.txt", "old name.txt", "stage.txt"] {
        std::fs::write(p.join(name), "base\n").unwrap();
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    git(p, &["checkout", "-b", "other"]);
    std::fs::write(p.join("conflict.txt"), "other\n").unwrap();
    git(p, &["commit", "-am", "other"]);
    git(p, &["checkout", "main"]);
    std::fs::write(p.join("conflict.txt"), "main\n").unwrap();
    git(p, &["commit", "-am", "main"]);
    assert!(!git(p, &["merge", "other"]).status.success());
    assert!(git(p, &["mv", "old name.txt", "新 name.txt"])
        .status
        .success());
    std::fs::write(p.join("stage.txt"), "changed").unwrap();
    git(p, &["add", "stage.txt"]);
    std::fs::write(p.join("untracked\nfile.txt"), "new").unwrap();
    let observed = workspace::inspect(&WorkspaceInput {
        path: p.display().to_string(),
        related_commits: vec![],
    })
    .unwrap()
    .observation;
    assert_eq!(observed.change_status.has_uncommitted_changes, Some(true));
    assert_eq!(observed.change_status.has_unpushed_commits, None);
    assert!(observed
        .changed_files
        .iter()
        .any(|f| f.path == "conflict.txt" && f.change_type == "conflicted" && !f.staged));
    assert!(observed
        .changed_files
        .iter()
        .any(|f| f.path == "新 name.txt" && f.original_path.as_deref() == Some("old name.txt")));
    assert!(observed
        .changed_files
        .iter()
        .any(|f| f.path == "stage.txt" && f.staged));
    assert!(observed
        .changed_files
        .iter()
        .any(|f| f.path == "untracked\nfile.txt" && f.change_type == "untracked"));
}

#[test]
fn human_metadata_edits_leave_run_description_and_creator_unchanged() {
    let data = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(data.path());
    let receipt = ledger
        .create_task(create(data.path()), Actor::agent("codex").unwrap())
        .unwrap();
    let before = ledger.get(&receipt.task_id).unwrap();
    let edited = ledger
        .edit_task(EditTaskInput {
            task_id: before.id.clone(),
            expected_revision: before.revision,
            title: "修订标题".into(),
            description: "补充背景".into(),
        })
        .unwrap();
    let after = ledger.get(&edited.task_id).unwrap();
    assert_eq!(after.description, "补充背景");
    assert_eq!(after.runs[0].kind, TaskKind::Bugfix);
    assert_eq!(after.created_by, "codex");
    assert_eq!(after.runs[0].description, before.runs[0].description);
    assert_eq!(after.runs[0].created_by, "codex");
}

#[test]
fn mcp_detail_does_not_repeat_full_git_snapshots_in_progress_and_tests() {
    let data = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(data.path());
    let receipt = ledger
        .create_task(create(data.path()), Actor::agent("codex").unwrap())
        .unwrap();
    let mut task = ledger.get(&receipt.task_id).unwrap();
    let run = &mut task.runs[0];
    run.agent_workspace.changed_files = (0..40)
        .map(|n| ChangedFile {
            path: format!("file-{n}.rs"),
            change_type: "modified".into(),
            staged: false,
            original_path: None,
        })
        .collect();
    run.timeline.last_mut().unwrap().agent_workspace = Some(run.agent_workspace.clone());
    run.validation_plan.records.push(Verification {
        id: id(),
        actor: "agent".into(),
        agent: "codex".into(),
        created_at: now(),
        agent_workspace: Some(run.agent_workspace.clone()),
        input: VerificationInput {
            plan_revision: 1,
            stage: Scope::Local,
            target: "workspace".into(),
            environment: "test".into(),
            results: vec![],
        },
    });
    ledger
        .store()
        .unwrap()
        .write(&Ledger::key(&task.project_id, &task.id), &task)
        .unwrap();
    let detail = ledger
        .find(FindTasksInput {
            task_id: Some(task.id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        detail["run"]["agentWorkspace"]["changedFiles"]
            .as_array()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(detail["run"]["agentWorkspace"]["changedFileCount"], 40);
    assert!(detail["run"]["latestProgress"]
        .get("agentWorkspace")
        .is_none());
    assert!(detail["run"]["validationPlan"]["latestVerification"]
        .get("agentWorkspace")
        .is_none());
}

#[test]
fn run_fields_and_single_item_verification_are_independent() {
    let data = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(data.path());
    let mut input = create(data.path());
    let mut second = input.initial_run.validation_plan.as_ref().unwrap().items[0].clone();
    second.id.clear();
    second.title = "另一个验收点".into();
    input
        .initial_run
        .validation_plan
        .as_mut()
        .unwrap()
        .items
        .push(second);
    let created = ledger
        .create_task(input, Actor::agent("codex").unwrap())
        .unwrap();
    let update = UpdateTaskRunInput {
        task_id: created.task_id.clone(),
        run_id: created.run_id.clone(),
        expected_revision: 1,
        title: Some("已定位原因".into()),
        kind: Some(TaskKind::Feature),
        goal: Some("独立目标".into()),
        solution: Some("独立方案".into()),
        ..Default::default()
    };
    let updated = ledger
        .update_task_run(update, Actor::agent("codex").unwrap())
        .unwrap();
    let task = ledger.get(&created.task_id).unwrap();
    assert_eq!(task.title, "任务目标");
    assert_eq!(task.description, "任务背景");
    let run = &task.runs[0];
    assert_eq!(
        (&run.title, &run.goal, &run.solution),
        (&"已定位原因".into(), &"独立目标".into(), &"独立方案".into())
    );
    assert_eq!(run.kind, TaskKind::Feature);
    let record = |criterion_id: String| VerificationInput {
        plan_revision: 1,
        stage: Scope::Release,
        target: "1.0".into(),
        environment: "macOS".into(),
        results: vec![CheckResult {
            criterion_id,
            result: TestResult::Passed,
            evidence: "实际操作结果符合预期".into(),
        }],
    };
    let save = |revision, verification| {
        ledger
            .update_task_run(
                UpdateTaskRunInput {
                    task_id: task.id.clone(),
                    run_id: run.id.clone(),
                    expected_revision: revision,
                    verification: Some(verification),
                    ..Default::default()
                },
                Actor::human(),
            )
            .unwrap()
    };
    let first = save(updated.revision, record(created.criterion_ids[0].clone()));
    assert!(!human_acceptance_ready(
        &ledger.get(&task.id).unwrap().runs[0]
    ));
    save(first.revision, record(created.criterion_ids[1].clone()));
    let task = ledger.get(&task.id).unwrap();
    assert!(human_acceptance_ready(&task.runs[0]));
    assert_eq!(task.runs[0].validation_plan.records.len(), 2);
    assert!(serde_json::to_value(task).unwrap().get("kind").is_none());
}

#[test]
fn v3_migration_preserves_unstructured_text_evidence_and_original_bytes() {
    let seed = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(seed.path());
    let created = ledger
        .create_task(create(seed.path()), Actor::agent("codex").unwrap())
        .unwrap();
    ledger
        .update_task_run(
            UpdateTaskRunInput {
                task_id: created.task_id.clone(),
                run_id: created.run_id.clone(),
                expected_revision: 1,
                verification: Some(VerificationInput {
                    plan_revision: 1,
                    stage: Scope::Local,
                    target: "迁移前工作区".into(),
                    environment: "macOS".into(),
                    results: vec![CheckResult {
                        criterion_id: created.criterion_ids[0].clone(),
                        result: TestResult::Passed,
                        evidence: "迁移前的验证证据".into(),
                    }],
                }),
                ..Default::default()
            },
            Actor::agent("codex").unwrap(),
        )
        .unwrap();
    let task = ledger.get(&created.task_id).unwrap();
    let projects = Ledger::projects(&ledger.store().unwrap()).unwrap();
    let mut v3 = json!(task);
    v3["schemaVersion"] = json!(3);
    v3["kind"] = json!("bugfix");
    for run in v3["runs"].as_array_mut().unwrap() {
        for name in ["title", "kind", "goal", "solution"] {
            run.as_object_mut().unwrap().remove(name);
        }
    }
    let target = tempfile::tempdir().unwrap();
    {
        let source = Store::open(&target.path().join("tasks/v3")).unwrap();
        source.write("projects.json", &projects).unwrap();
        source
            .write(&Ledger::key(&task.project_id, &task.id), &v3)
            .unwrap();
    }
    let source_path = target
        .path()
        .join("tasks/v3")
        .join(Ledger::key(&task.project_id, &task.id));
    let original = std::fs::read(&source_path).unwrap();
    let imported = Ledger::new(target.path()).get(&task.id).unwrap();
    assert_eq!(imported.schema_version, 4);
    assert_eq!(imported.revision, task.revision);
    assert_eq!(imported.runs[0].id, task.runs[0].id);
    assert_eq!(imported.runs[0].description, task.runs[0].description);
    assert_eq!(imported.runs[0].goal, task.title);
    assert_eq!(imported.runs[0].solution, "");
    assert_eq!(imported.runs[0].kind, TaskKind::Bugfix);
    assert_eq!(
        json!(imported.runs[0].validation_plan),
        json!(task.runs[0].validation_plan)
    );
    assert_eq!(std::fs::read(source_path).unwrap(), original);
}
