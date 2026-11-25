use super::*;

fn create(ledger: &Ledger, workspace: &Path, title: &str) -> Receipt {
    ledger.create_task(serde_json::from_value(json!({
        "title":title, "description":"持续追踪的主题背景",
        "agentWorkspace":{"path":workspace},
        "initialRun":{"title":"最初排查", "kind":"bugfix", "goal":"修复菜单",
            "solution":"只在旧执行出现的线索", "agentConversation":["00000000-0000-4000-8000-000000000001"]}
    })).unwrap(), Actor::agent("test").unwrap()).unwrap()
}

fn next_run(ledger: &Ledger, receipt: &Receipt) -> Receipt {
    let published = ledger
        .update_task_run(
            serde_json::from_value(json!({
                "taskId":receipt.task_id,"runId":receipt.run_id,"expectedRevision":receipt.revision,
                "status":"published","delivery":"版本 1.0",
                "agentConversation":["00000000-0000-4000-8000-000000000001"]
            }))
            .unwrap(),
            Actor::agent("test").unwrap(),
        )
        .unwrap();
    ledger
        .create_task_run(
            serde_json::from_value(json!({
                "taskId":published.task_id,"expectedRevision":published.revision,
                "title":"发布后另一个问题","goal":"检查新问题","solution":"新的处理方法",
                "agentConversation":["00000000-0000-4000-8000-000000000001"]
            }))
            .unwrap(),
            Actor::agent("test").unwrap(),
        )
        .unwrap()
}

#[test]
fn a_historical_match_returns_that_run_not_the_current_run() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(&dir.path().join("data"));
    let old = create(&ledger, dir.path(), "长期主题");
    let current = next_run(&ledger, &old);
    let result = ledger
        .find_runs(FindTaskRunsInput {
            query: Some("  只在旧执行出现的线索  ".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result["total"], 1);
    assert!(result.get("tasks").is_none());
    assert_eq!(result["runs"][0]["runId"], old.run_id);
    assert_ne!(result["runs"][0]["runId"], current.run_id);
    assert_eq!(result["runs"][0]["title"], "最初排查");
    assert_eq!(result["runs"][0]["taskTitle"], "长期主题");
    assert_eq!(result["runs"][0]["isCurrentRun"], false);
}

#[test]
fn a_topic_match_returns_each_run_and_exact_run_id_needs_no_task_id() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(&dir.path().join("data"));
    let old = create(&ledger, dir.path(), "长期主题");
    let current = next_run(&ledger, &old);
    let result = ledger
        .find_runs(FindTaskRunsInput {
            query: Some("长期主题".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result["total"], 2);
    assert_eq!(result["runs"].as_array().unwrap().len(), 2);
    let detail = ledger
        .find_runs(FindTaskRunsInput {
            run_id: Some(old.run_id.clone()),
            workspace: Some("/removed/unrelated/workspace".into()),
            history: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(detail["runId"], old.run_id);
    assert_eq!(detail["run"]["id"], old.run_id);
    assert_eq!(detail["taskId"], old.task_id);
    assert_eq!(detail["taskTitle"], "长期主题");
    assert_eq!(detail["isCurrentRun"], false);
    assert_eq!(detail["history"].as_array().unwrap().len(), 2);
    let latest = ledger
        .find_runs(FindTaskRunsInput {
            task_id: Some(old.task_id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(latest["runId"], current.run_id);
    assert_eq!(latest["isCurrentRun"], true);
}

#[test]
fn workspace_prioritizes_without_hiding_cross_workspace_matches() {
    let preferred = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(data.path());
    let first = create(&ledger, preferred.path(), "共同主题 A");
    let other = create(&ledger, elsewhere.path(), "共同主题 B");
    let result = ledger
        .find_runs(FindTaskRunsInput {
            workspace: Some(preferred.path().to_string_lossy().into()),
            query: Some("共同主题".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result["total"], 2);
    assert_eq!(result["runs"][0]["runId"], first.run_id);
    assert_eq!(result["runs"][1]["runId"], other.run_id);
    let foreign_detail = ledger
        .find_runs(FindTaskRunsInput {
            task_id: Some(other.task_id),
            workspace: Some(preferred.path().to_string_lossy().into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(foreign_detail["runId"], other.run_id);
}

#[test]
fn a_run_id_from_a_different_topic_does_not_leak_into_the_requested_topic() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(&dir.path().join("data"));
    let first = create(&ledger, dir.path(), "主题 A");
    let second = create(&ledger, dir.path(), "主题 B");
    assert!(ledger
        .find_runs(FindTaskRunsInput {
            task_id: Some(first.task_id),
            run_id: Some(second.run_id),
            ..Default::default()
        })
        .is_err());
    assert!(ledger
        .find_runs(FindTaskRunsInput {
            run_id: Some("invalid".into()),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn total_and_truncation_count_runs_instead_of_topics() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(&dir.path().join("data"));
    for index in 0..11 {
        let first = create(&ledger, dir.path(), &format!("共同主题 {index}"));
        next_run(&ledger, &first);
    }
    let result = ledger
        .find_runs(FindTaskRunsInput {
            query: Some("共同主题".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result["total"], 22);
    assert_eq!(result["runs"].as_array().unwrap().len(), 20);
    assert_eq!(result["truncated"], true);
}
