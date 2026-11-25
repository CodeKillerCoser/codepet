use super::*;
use crate::workbench;
use serde_json::json;
use std::fs;
fn repository(root: &Path, name: &str) -> (PathBuf, String) {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    for args in [
        vec!["init"],
        vec!["config", "user.name", "QA"],
        vec!["config", "user.email", "qa@example.invalid"],
    ] {
        workbench::text_git(&path, &args).unwrap();
    }
    fs::write(path.join(format!("{name}.txt")), name).unwrap();
    workbench::text_git(&path, &["add", "."]).unwrap();
    workbench::text_git(&path, &["commit", "-m", name]).unwrap();
    let sha = workbench::text_git(&path, &["rev-parse", "HEAD"]).unwrap();
    (path.canonicalize().unwrap(), sha)
}
fn actor() -> Actor {
    Actor::agent("claude").unwrap()
}
fn create(ledger: &Ledger, path: &Path, sha: &str) -> Receipt {
    ledger.create_task(serde_json::from_value(json!({"title":"Cross-project goal", "agentWorkspace":{"path":path,"relatedCommits":[sha]},"initialRun":{"title":"First implementation","agentConversation":["conversation-one"],"validationPlan":{"items":[{"title":"Cross-project result","steps":"Inspect the new project","expected":"Correct files","scope":"local"}]}}})).unwrap(), actor()).unwrap()
}
#[test]
fn another_project_run_preserves_task_history_checklist_and_its_own_diffs() {
    let root = tempfile::tempdir().unwrap();
    let (a, a_sha) = repository(root.path(), "first");
    let (b, b_sha) = repository(root.path(), "second");
    let ledger = Ledger::new(&root.path().join("ledger"));
    let first = create(&ledger, &a, &a_sha);
    let published = ledger.update_task_run(serde_json::from_value(json!({"taskId":first.task_id,"runId":first.run_id,"expectedRevision":first.revision,"agentConversation":["conversation-one"],"status":"published","delivery":"1.0"})).unwrap(),actor()).unwrap();
    let before = ledger.get(&first.task_id).unwrap();
    let next = ledger.create_task_run(serde_json::from_value(json!({"taskId":first.task_id,"expectedRevision":published.revision,"title":"Continue in another repository","agentConversation":["conversation-two"],"agentWorkspace":{"path":b,"relatedCommits":[b_sha]}})).unwrap(),actor()).unwrap();
    let task = ledger.get(&next.task_id).unwrap();
    assert_eq!(task.project_id, before.project_id);
    assert_eq!(json!(task.runs[0]), json!(before.runs[0]));
    assert_eq!(
        task.runs[1].validation_plan.items[0].title,
        task.runs[0].validation_plan.items[0].title
    );
    assert_ne!(
        task.runs[1].validation_plan.items[0].id,
        task.runs[0].validation_plan.items[0].id
    );
    assert!(task.runs[1].validation_plan.records.is_empty());
    assert_eq!(
        ledger.commit_changes(&task.id, &next.run_id).unwrap()[0].files[0].path,
        "second.txt"
    );
    fs::remove_dir_all(&b).unwrap();
    let versions = ledger.change_diffs(&task.id, &next.run_id).unwrap();
    let patch = ledger
        .file_diff(
            &task.id,
            &next.run_id,
            versions[0]["id"].as_str().unwrap(),
            "second.txt",
        )
        .unwrap();
    assert!(patch["patch"].as_str().unwrap().contains("+second"));
    let original = ledger.change_diffs(&task.id, &first.run_id).unwrap();
    assert_eq!(original[0]["files"][0]["path"], "first.txt");
}
#[test]
fn continuing_from_another_project_keeps_conversations_without_carrying_unrelated_git_facts() {
    let root = tempfile::tempdir().unwrap();
    let (a, a_sha) = repository(root.path(), "first");
    let (b, b_sha) = repository(root.path(), "second");
    let ledger = Ledger::new(&root.path().join("ledger"));
    let r = create(&ledger, &a, &a_sha);
    let updated=ledger.update_task_run(serde_json::from_value(json!({"taskId":r.task_id,"runId":r.run_id,"expectedRevision":r.revision,"agentConversation":["conversation-two"],"agentWorkspace":{"path":b,"relatedCommits":[b_sha]},"note":"Continue from another project"})).unwrap(),actor()).unwrap();
    let task = ledger.get(&r.task_id).unwrap();
    let run = task.runs.last().unwrap();
    assert_eq!(run.agent_conversation.len(), 2);
    assert_eq!(run.agent_workspace.related_commits, vec![b_sha]);
    assert_ne!(run.agent_workspace.related_commits[0], a_sha);
    assert_eq!(
        run.timeline[0].agent_workspace.as_ref().unwrap().path,
        a.display().to_string()
    );
    assert_eq!(
        run.timeline
            .last()
            .unwrap()
            .agent_workspace
            .as_ref()
            .unwrap()
            .path,
        b.display().to_string()
    );
    assert_eq!(
        ledger
            .find_runs(FindTaskRunsInput {
                workspace: Some(b.display().to_string()),
                ..Default::default()
            })
            .unwrap()["total"],
        1
    );
    let found = ledger
        .find_runs(FindTaskRunsInput {
            task_id: Some(r.task_id),
            workspace: Some("/removed/or/unrelated/project".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(found["revision"], updated.revision);
    assert_eq!(
        ledger.commit_changes(&task.id, &r.run_id).unwrap()[0].files[0].path,
        "second.txt"
    );
}

#[test]
fn changing_a_removed_worktree_in_the_same_repository_keeps_explicit_commits() {
    let root = tempfile::tempdir().unwrap();
    let (main, sha) = repository(root.path(), "main");
    let linked = root.path().join("linked");
    workbench::text_git(
        &main,
        &[
            "worktree",
            "add",
            "-b",
            "codex/linked",
            linked.to_str().unwrap(),
        ],
    )
    .unwrap();
    let ledger = Ledger::new(&root.path().join("ledger"));
    let r = create(&ledger, &linked, &sha);
    workbench::text_git(&main, &["worktree", "remove", linked.to_str().unwrap()]).unwrap();
    ledger.update_task_run(serde_json::from_value(json!({"taskId":r.task_id,"runId":r.run_id,"expectedRevision":r.revision,"agentConversation":["conversation-two"],"agentWorkspace":{"path":main}})).unwrap(),actor()).unwrap();
    assert_eq!(
        ledger.get(&r.task_id).unwrap().runs[0]
            .agent_workspace
            .related_commits,
        vec![sha]
    );
}
