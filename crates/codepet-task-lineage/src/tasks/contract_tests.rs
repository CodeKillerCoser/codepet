use super::*;
use serde_json::json;
const CHAT: &str = "00000000-0000-4000-8000-000000000002";
fn actor() -> Actor {
    Actor::agent("codex").unwrap()
}
fn create(path: &Path) -> CreateTaskInput {
    serde_json::from_value(json!({"title":"合同验收","agentWorkspace":{"path":path},"initialRun":{"agentConversation":[format!("codex://threads/{CHAT}")],"validationPlan":{"items":[{"title":"本地检查","steps":"执行","expected":"成功","scope":"local"},{"title":"另一个检查","steps":"执行","expected":"成功","scope":"both"}]}}})).unwrap()
}
fn verification(r: &Receipt, ids: &[usize], result: TestResult) -> VerificationInput {
    VerificationInput {
        plan_revision: r.plan_revision,
        stage: Scope::Local,
        target: "worktree@commit".into(),
        environment: "macOS".into(),
        results: ids
            .iter()
            .map(|i| CheckResult {
                criterion_id: r.criterion_ids[*i].clone(),
                result: result.clone(),
                evidence: "实际测试结论".into(),
            })
            .collect(),
    }
}
fn finish(r: &Receipt, ids: &[usize], result: TestResult) -> FinishTaskRunInput {
    FinishTaskRunInput {
        task_id: r.task_id.clone(),
        run_id: r.run_id.clone(),
        expected_revision: r.revision,
        conclusion: "开发完成，逐项验证结果已记录".into(),
        changed_files: vec![],
        agent_conversation: vec![CHAT.into()],
        verification: verification(r, ids, result),
        agent_workspace: None,
        operation_id: None,
    }
}
#[test]
fn schema_requires_conversation_and_result_with_conclusion() {
    let update = serde_json::to_value(schemars::schema_for!(UpdateTaskRunInput)).unwrap();
    assert!(update["required"]
        .as_array()
        .unwrap()
        .contains(&json!("agentConversation")));
    assert_eq!(update["properties"]["agentConversation"]["minItems"], 1);
    let create = serde_json::to_value(schemars::schema_for!(CreateTaskInput)).unwrap();
    assert!(create["required"]
        .as_array()
        .unwrap()
        .contains(&json!("initialRun")));
    let initial = serde_json::to_value(schemars::schema_for!(InitialRunInput)).unwrap();
    assert!(initial["required"]
        .as_array()
        .unwrap()
        .contains(&json!("agentConversation")));
    let result = serde_json::to_value(schemars::schema_for!(CheckResult)).unwrap();
    for required in ["criterionId", "result", "evidence"] {
        assert!(
            result["required"]
                .as_array()
                .unwrap()
                .contains(&json!(required)),
            "{result}"
        );
    }
    let finish = serde_json::to_value(schemars::schema_for!(FinishTaskRunInput)).unwrap();
    for required in [
        "conclusion",
        "verification",
        "agentConversation",
        "changedFiles",
    ] {
        assert!(finish["required"]
            .as_array()
            .unwrap()
            .contains(&json!(required)));
    }
}
#[test]
fn agent_writes_require_nonempty_valid_conversations_even_when_already_linked() {
    let d = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(d.path());
    for ids in [
        vec![],
        vec![" ".into()],
        vec!["codex://threads/".into()],
        vec!["not-a-thread".into()],
    ] {
        let mut input = create(d.path());
        input.initial_run.agent_conversation = ids;
        assert!(ledger.create_task(input, actor()).is_err());
        assert!(ledger.list().unwrap().is_empty());
    }
    let r = ledger.create_task(create(d.path()), actor()).unwrap();
    assert_eq!(
        ledger.get(&r.task_id).unwrap().runs[0].agent_conversation[0].conversation_id,
        CHAT
    );
    assert!(ledger
        .update_task_run(
            UpdateTaskRunInput {
                task_id: r.task_id.clone(),
                run_id: r.run_id.clone(),
                expected_revision: 1,
                note: Some("不能省略本次对话".into()),
                ..Default::default()
            },
            actor()
        )
        .is_err());
    assert_eq!(ledger.get(&r.task_id).unwrap().revision, 1);
    let mut f = finish(&r, &[0, 1], TestResult::Passed);
    f.agent_conversation.clear();
    assert!(ledger.finish_task_run(f, actor()).is_err());
    assert_eq!(ledger.get(&r.task_id).unwrap().revision, 1);
}
#[test]
fn verification_status_and_conclusion_commit_atomically() {
    let d = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(d.path());
    let r = ledger.create_task(create(d.path()), actor()).unwrap();
    for evidence in ["", "   "] {
        let mut f = finish(&r, &[0, 1], TestResult::Passed);
        f.verification.results[1].evidence = evidence.into();
        assert!(ledger.finish_task_run(f, actor()).is_err());
        let task = ledger.get(&r.task_id).unwrap();
        assert_eq!(task.revision, 1);
        assert!(task.runs[0].validation_plan.records.is_empty());
    }
    let mut f = finish(&r, &[0, 1], TestResult::Passed);
    f.conclusion = " ".into();
    assert!(ledger.finish_task_run(f, actor()).is_err());
    let mut v = json!(verification(&r, &[0], TestResult::Passed));
    v["results"][0].as_object_mut().unwrap().remove("result");
    assert!(serde_json::from_value::<VerificationInput>(v).is_err());
    let mut f = finish(&r, &[0], TestResult::Passed);
    f.verification.plan_revision = 999;
    assert!(ledger.finish_task_run(f, actor()).is_err());
    assert_eq!(ledger.get(&r.task_id).unwrap().revision, 1);
}
#[test]
fn finish_keeps_incomplete_or_failed_runs_active_and_never_accepts_for_user() {
    let d = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(d.path());
    let r = ledger.create_task(create(d.path()), actor()).unwrap();
    let partial = ledger
        .finish_task_run(finish(&r, &[0], TestResult::Passed), actor())
        .unwrap();
    assert_eq!(partial.status, RunStatus::InProgress);
    let input = finish(&partial, &[1], TestResult::Passed);
    let ready = ledger.finish_task_run(input.clone(), actor()).unwrap();
    assert_eq!(ready.status, RunStatus::ReadyForDelivery);
    assert_eq!(
        ledger.finish_task_run(input, actor()).unwrap().revision,
        ready.revision
    );
    let task = ledger.get(&r.task_id).unwrap();
    assert_eq!(task.runs[0].validation_plan.records.len(), 2);
    assert!(task.runs[0].closure.is_none());
    assert!(!human_acceptance_ready(&task.runs[0]));
    assert!(ledger
        .close(
            &r.task_id,
            &r.run_id,
            ready.revision,
            true,
            "不能把 Agent 完成当成人工通过"
        )
        .is_err());
    let failed = ledger
        .finish_task_run(finish(&ready, &[0], TestResult::Failed), actor())
        .unwrap();
    assert_eq!(failed.status, RunStatus::InProgress);
    let task = ledger.get(&r.task_id).unwrap();
    assert_eq!(
        task.runs[0]
            .validation_plan
            .records
            .last()
            .unwrap()
            .input
            .results[0]
            .evidence,
        "实际测试结论"
    );
}
#[test]
fn committed_files_survive_a_clean_worktree_and_later_milestones() {
    use std::process::Command;
    let d = tempfile::tempdir().unwrap();
    let path = d.path();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{:?}", out);
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "QA"]);
    git(&["config", "user.email", "qa@example.invalid"]);
    std::fs::write(path.join("old\nname.txt"), "unchanged contents").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    let root_sha = git(&["rev-parse", "HEAD"]);
    git(&["mv", "old\nname.txt", "new name.txt"]);
    git(&["commit", "-m", "rename"]);
    let sha = git(&["rev-parse", "HEAD"]);
    let data = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(data.path());
    let mut input = create(path);
    input.agent_workspace.related_commits = vec![root_sha, sha.clone()];
    let r = ledger.create_task(input, actor()).unwrap();
    let task = ledger.get(&r.task_id).unwrap();
    let ws = &task.runs[0].agent_workspace;
    assert!(ws.changed_files.is_empty());
    assert_eq!(ws.commit_changes.len(), 2);
    assert_eq!(ws.commit_changes[0].files[0].path, "old\nname.txt");
    assert_eq!(ws.commit_changes[1].files[0].path, "new name.txt");
    let key = Ledger::key(&task.project_id, &task.id);
    let mut old = json!(task);
    old["runs"][0]["agentWorkspace"]
        .as_object_mut()
        .unwrap()
        .remove("commitChanges");
    old["runs"][0]["agentWorkspace"]
        .as_object_mut()
        .unwrap()
        .remove("runFiles");
    ledger.store().unwrap().write(&key, &old).unwrap();
    let before = std::fs::read(ledger.root.join(&key)).unwrap();
    let read = ledger.commit_changes(&r.task_id, &r.run_id).unwrap();
    assert_eq!(read[1].files[0].path, "new name.txt");
    assert_eq!(std::fs::read(ledger.root.join(&key)).unwrap(), before);

    assert_eq!(
        ws.commit_changes[1].files[0].original_path.as_deref(),
        Some("old\nname.txt")
    );
    ledger
        .update_task_run(
            UpdateTaskRunInput {
                task_id: r.task_id.clone(),
                run_id: r.run_id.clone(),
                expected_revision: r.revision,
                note: Some("保留提交文件".into()),
                agent_conversation: vec![CHAT.into()],
                ..Default::default()
            },
            actor(),
        )
        .unwrap();
    assert_eq!(
        ledger.get(&r.task_id).unwrap().runs[0]
            .agent_workspace
            .commit_changes[1]
            .commit,
        sha
    );
}

#[test]
fn finish_requires_an_explicit_owned_file_list_and_preserves_it_on_updates() {
    let d = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(d.path());
    let r = ledger.create_task(create(d.path()), actor()).unwrap();
    let mut missing = json!(finish(&r, &[0, 1], TestResult::Passed));
    missing.as_object_mut().unwrap().remove("changedFiles");
    assert!(serde_json::from_value::<FinishTaskRunInput>(missing).is_err());
    for paths in [
        vec!["../escape.rs".into()],
        vec!["/absolute.rs".into()],
        vec![" ".into()],
        vec!["same.rs".into(), "same.rs".into()],
    ] {
        let mut f = finish(&r, &[0, 1], TestResult::Passed);
        f.changed_files = paths;
        assert!(ledger.finish_task_run(f, actor()).is_err());
        assert_eq!(ledger.get(&r.task_id).unwrap().revision, 1);
    }
    let mut f = finish(&r, &[0, 1], TestResult::Passed);
    f.changed_files = vec!["src/removed.rs".into()];
    f.operation_id = Some("finish-owned-files".into());
    let done = ledger.finish_task_run(f.clone(), actor()).unwrap();
    f.changed_files = vec![];
    assert!(ledger
        .finish_task_run(f, actor())
        .unwrap_err()
        .contains("同一操作 ID"));
    ledger
        .update_task_run(
            UpdateTaskRunInput {
                task_id: r.task_id.clone(),
                run_id: r.run_id.clone(),
                expected_revision: done.revision,
                agent_conversation: vec![CHAT.into()],
                note: Some("后续进展".into()),
                ..Default::default()
            },
            actor(),
        )
        .unwrap();
    assert_eq!(
        ledger.get(&r.task_id).unwrap().runs[0]
            .agent_workspace
            .run_files,
        Some(vec!["src/removed.rs".into()])
    );
    let empty = ledger
        .finish_task_run(
            finish(
                &Receipt {
                    revision: done.revision + 1,
                    ..done
                },
                &[0, 1],
                TestResult::Passed,
            ),
            actor(),
        )
        .unwrap();
    assert_eq!(
        ledger.get(&empty.task_id).unwrap().runs[0]
            .agent_workspace
            .run_files,
        Some(vec![])
    );
}
