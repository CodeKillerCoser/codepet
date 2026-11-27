use super::*;
use crate::tasks::{Actor, ChangedFile, CommitChange, FindTaskRunsInput, Ledger};
use std::fs;

const CHAT: &str = "00000000-0000-4000-8000-000000000001";

fn create(ledger: &Ledger, path: &Path, text: &str) -> Task {
    let receipt = ledger
        .create_task(
            serde_json::from_value(json!({
                "title":"可移动账本",
                "description":text,
                "agentWorkspace":{"path":path},
                "initialRun":{"goal":text,"agentConversation":[CHAT],"validationPlan":{"items":[
                    {"title":"验证文档","steps":text,"expected":"原样还原"}
                ]}}
            }))
            .unwrap(),
            Actor::agent("codex").unwrap(),
        )
        .unwrap();
    ledger.get(&receipt.task_id).unwrap()
}

#[test]
fn legacy_inline_payloads_compact_on_write_and_survive_moving_the_ledger() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("original");
    let ledger = Ledger::new(&data);
    let text = "长段落与空行\n\n".repeat(180);
    let created = create(&ledger, temp.path(), &text);
    ledger.update_task_run(serde_json::from_value(json!({
        "taskId":created.id, "runId":created.runs[0].id, "expectedRevision":created.revision,
        "agentConversation":[CHAT],
        "verification":{"planRevision":created.runs[0].validation_plan.revision,
            "stage":"local", "target":"测试工作区", "environment":"本机测试",
            "results":[{"criterionId":created.runs[0].validation_plan.items[0].id,
                "result":"passed", "evidence":text}]}
    })).unwrap(), Actor::agent("codex").unwrap()).unwrap();
    let mut task = ledger.get(&created.id).unwrap();
    let run = &mut task.runs[0];
    let files = (0..160)
        .map(|index| ChangedFile {
            path: format!("src/components/component-{index}.rs"),
            change_type: "modified".into(),
            staged: false,
            original_path: None,
        })
        .collect::<Vec<_>>();
    run.agent_workspace.changed_files = files.clone();
    run.agent_workspace.commit_changes = vec![CommitChange {
        commit: "1234567".into(),
        files,
        error: None,
    }];
    run.timeline[0].agent_workspace = Some(run.agent_workspace.clone());
    let run_id = run.id.clone();
    let task_id = task.id.clone();
    let task_key = Ledger::key(&task.project_id, &task_id);
    // Simulate v6 data with duplicated payloads before the current writer touches it.
    ledger.store().unwrap().write(&task_key, &task).unwrap();
    let manifest = data.join("tasks/v4").join(&task_key);
    let old = fs::read(&manifest).unwrap();
    assert_eq!(json!(ledger.get(&task_id).unwrap()), json!(task));
    assert_eq!(fs::read(&manifest).unwrap(), old);

    let receipt = ledger
        .comment_task_run(
            serde_json::from_value(json!({
                "taskId":task_id,"runId":run_id,"agentConversation":[CHAT],
                "operationId":"relative-comment","comment":text
            }))
            .unwrap(),
            Actor::agent("codex").unwrap(),
        )
        .unwrap();
    let new = fs::read(&manifest).unwrap();
    let raw: Value = serde_json::from_slice(&new).unwrap();
    assert!(new.len() * 3 < old.len());
    assert_eq!(raw["schemaVersion"], REFERENCED_SCHEMA);
    let text_ref = raw["description"]["ref"].as_str().unwrap();
    assert_eq!(text_ref, key(Kind::Markdown, text.as_bytes()));
    assert_eq!(raw["runs"][0]["goal"]["ref"], text_ref);
    assert_eq!(
        raw["runs"][0]["validationPlan"]["items"][0]["steps"]["ref"],
        text_ref
    );
    assert_eq!(
        raw["runs"][0]["validationPlan"]["records"][0]["results"][0]["evidence"]["ref"],
        text_ref
    );
    let git_ref = raw["runs"][0]["agentWorkspace"]["changedFiles"]["ref"]
        .as_str()
        .unwrap();
    assert_eq!(
        raw["runs"][0]["agentWorkspace"]["commitChanges"][0]["files"]["ref"],
        git_ref
    );
    assert_eq!(
        raw["runs"][0]["timeline"][0]["agentWorkspace"]["changedFiles"]["ref"],
        git_ref
    );

    let moved = temp.path().join("relocated");
    fs::rename(&data, &moved).unwrap();
    let ledger = Ledger::new(&moved);
    let restored = ledger.get(&task_id).unwrap();
    assert_eq!(restored.revision, receipt.revision);
    assert_eq!(restored.description, text);
    assert_eq!(restored.runs[0].timeline.last().unwrap().summary, text);
    assert_eq!(
        json!(restored.runs[0].timeline[0]),
        json!(task.runs[0].timeline[0])
    );
    assert_eq!(
        restored.runs[0].agent_workspace,
        task.runs[0].agent_workspace
    );
    assert_eq!(ledger.list().unwrap()[0].id, task_id);

    // An exact summary can recover a task while its unrelated Git payload is unavailable.
    fs::remove_file(moved.join("tasks/v4").join(git_ref)).unwrap();
    assert_eq!(
        ledger
            .find_runs(FindTaskRunsInput {
                task_id: Some(task_id.clone()),
                ..Default::default()
            })
            .unwrap()["revision"],
        receipt.revision
    );
    assert!(ledger.get(&task_id).unwrap_err().contains("读取任务文档"));
}

#[test]
fn a_prepared_transaction_replays_after_document_publication() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(temp.path());
    let mut task = create(&ledger, temp.path(), "准备");
    let key = Ledger::key(&task.project_id, &task.id);
    let store = ledger.store().unwrap();
    task.revision += 1;
    task.description = "待恢复的正文\n".repeat(250);
    let stored = encode(&task, &store).unwrap();
    let path = stored["description"]["ref"].as_str().unwrap();
    assert!(store.root().join(path).is_file());
    let prepared = store.root().join("prepared.json");
    fs::write(&prepared, serde_json::to_vec(&vec![(key, stored)]).unwrap()).unwrap();
    drop(store);
    assert_eq!(json!(ledger.get(&task.id).unwrap()), json!(task));
    assert!(!prepared.exists());
}

#[test]
fn invalid_references_are_rejected_before_opening_files() {
    let temp = tempfile::tempdir().unwrap();
    for reference in [
        "../secret.md".to_owned(),
        "/tmp/secret.md".to_owned(),
        format!("documents/../{}.md", "a".repeat(64)),
        format!("documents\\{}.md", "a".repeat(64)),
        format!("documents/{}.json", "a".repeat(64)),
    ] {
        let error = open(temp.path(), Kind::Markdown, &reference).unwrap_err();
        assert!(error.contains("无效的任务文档相对引用"));
    }
}

#[cfg(unix)]
#[test]
fn a_document_symlink_cannot_redirect_the_ledger() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let ledger = Ledger::new(&root);
    let text = "长描述\n".repeat(300);
    let task = create(&ledger, temp.path(), &text);
    let document = root
        .join("tasks/v4")
        .join(key(Kind::Markdown, text.as_bytes()));
    let outside = temp.path().join("outside.md");
    fs::write(&outside, text).unwrap();
    fs::remove_file(&document).unwrap();
    std::os::unix::fs::symlink(&outside, document).unwrap();
    assert!(ledger
        .get(&task.id)
        .unwrap_err()
        .contains("Symlink in task store"));
}
