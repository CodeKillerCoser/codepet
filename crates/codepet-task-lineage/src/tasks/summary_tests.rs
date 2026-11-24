use super::*;

fn setup() -> (tempfile::TempDir, Ledger, Task) {
    let data = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(data.path());
    let input = serde_json::from_value(json!({"title":"汇总测试","agentWorkspace":{"path":data.path()},"initialRun":{"agentConversation":[]}})).unwrap();
    let receipt = ledger.create_task(input, Actor::human()).unwrap();
    let task = ledger.get(&receipt.task_id).unwrap();
    (data, ledger, task)
}

#[test]
fn all_run_combinations_are_order_independent_and_counted_exactly_once() {
    let (_data, _ledger, template) = setup();
    let states = [
        RunStatus::InProgress,
        RunStatus::ReadyForDelivery,
        RunStatus::Published,
        RunStatus::Closed,
    ];
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                let mut task = template.clone();
                task.runs = [a, b, c]
                    .map(|i| {
                        let mut run = template.runs[0].clone();
                        run.status = states[i].clone();
                        run
                    })
                    .to_vec();
                let summary = task.summary();
                assert_eq!(summary.status, states[*[a, b, c].iter().min().unwrap()]);
                assert_eq!(summary.run_counts.total, 3);
                assert_eq!(
                    summary.run_counts.in_progress,
                    [a, b, c].iter().filter(|i| **i == 0).count()
                );
                assert_eq!(
                    summary.run_counts.ready_for_delivery,
                    [a, b, c].iter().filter(|i| **i == 1).count()
                );
                assert_eq!(
                    summary.run_counts.published,
                    [a, b, c].iter().filter(|i| **i == 2).count()
                );
                assert_eq!(
                    summary.run_counts.closed,
                    [a, b, c].iter().filter(|i| **i == 3).count()
                );
                task.runs.reverse();
                assert_eq!(json!(summary), json!(task.summary()));
            }
        }
    }
}

#[test]
fn closing_one_run_keeps_other_pending_runs_visible_and_find_agrees_with_app() {
    let (_data, ledger, mut task) = setup();
    task.runs[0].status = RunStatus::ReadyForDelivery;
    let mut next = task.runs[0].clone();
    next.id = id();
    task.runs.push(next);
    ledger
        .store()
        .unwrap()
        .commit(vec![(Ledger::key(&task.project_id, &task.id), json!(task))])
        .unwrap();
    let r = ledger
        .close(
            &task.id,
            &task.runs[1].id,
            task.revision,
            false,
            "确认这一轮",
        )
        .unwrap();
    let saved = ledger.get(&task.id).unwrap();
    assert_eq!(saved.summary().status, RunStatus::ReadyForDelivery);
    assert_eq!(saved.summary().run_counts.ready_for_delivery, 1);
    assert_eq!(saved.summary().run_counts.closed, 1);
    assert!(!saved.summary().manually_completed);
    assert_eq!(
        ledger.view(&saved).unwrap()["summary"]["status"],
        "ready_for_delivery"
    );
    let found = ledger.find(FindTasksInput::default()).unwrap();
    assert_eq!(found["tasks"][0]["status"], "ready_for_delivery");
    assert_eq!(found["tasks"][0]["revision"], r.revision);
}

#[test]
fn manual_task_completion_overrides_without_changing_run_statuses_and_blocks_agents() {
    let (_data, ledger, task) = setup();
    let completed = ledger
        .complete_task(&task.id, task.revision, "人工完成整个任务")
        .unwrap();
    let closed = ledger.get(&task.id).unwrap();
    let persisted: Value = ledger
        .store()
        .unwrap()
        .read(&Ledger::key(&closed.project_id, &closed.id))
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted["schemaVersion"], 5,
        "Old v4 MCP writers must reject the record instead of dropping its new lock"
    );
    assert_eq!(closed.runs[0].status, RunStatus::InProgress);
    assert_eq!(closed.summary().status, RunStatus::Closed);
    assert!(closed.summary().manually_completed);
    assert_eq!(closed.summary().run_counts.in_progress, 1);
    assert_eq!(
        json!(closed.runs[0].validation_plan),
        json!(task.runs[0].validation_plan)
    );
    let update = UpdateTaskRunInput {
        task_id: task.id.clone(),
        run_id: task.runs[0].id.clone(),
        expected_revision: completed.revision,
        agent_conversation: vec![id()],
        ..Default::default()
    };
    assert!(ledger
        .update_task_run(update, Actor::agent("codex").unwrap())
        .unwrap_err()
        .contains("锁定"));
    let input = || {
        serde_json::from_value(json!({"taskId":task.id,"expectedRevision":completed.revision,"title":"人工重新开启","agentConversation":[]})).unwrap()
    };
    assert!(ledger
        .create_task_run(input(), Actor::agent("codex").unwrap())
        .unwrap_err()
        .contains("锁定"));
    assert!(ledger
        .complete_task(&task.id, task.revision, "过期操作")
        .is_err());
    let historical = json!(closed.runs);
    let reopened = ledger.create_task_run(input(), Actor::human()).unwrap();
    let opened = ledger.get(&task.id).unwrap();
    assert_eq!(opened.summary().status, RunStatus::InProgress);
    assert!(!opened.summary().manually_completed);
    assert_eq!(opened.summary().run_counts.in_progress, 2);
    assert_eq!(json!(opened.runs[..1]), historical);
    assert_eq!(opened.revision, reopened.revision);
}

#[test]
fn legacy_manual_completion_survives_read_without_rewriting_storage_or_evidence() {
    let (data, ledger, mut task) = setup();
    let mut last = task.runs[0].clone();
    last.id = id();
    last.status = RunStatus::Closed;
    last.closure = Some(Closure {
        accepted: false,
        reason: "原有人工完成".into(),
        closed_at: 123,
    });
    task.runs.push(last);
    let mut value = json!(task);
    value["schemaVersion"] = json!(4);
    value.as_object_mut().unwrap().remove("completion");
    value
        .as_object_mut()
        .unwrap()
        .remove("statusAggregationVersion");
    let key = Ledger::key(&task.project_id, &task.id);
    ledger
        .store()
        .unwrap()
        .commit(vec![(key.clone(), value)])
        .unwrap();
    let path = data.path().join("tasks/v4").join(key);
    let before = std::fs::read(&path).unwrap();
    let read = ledger.get(&task.id).unwrap();
    assert!(read.summary().manually_completed);
    assert_eq!(read.summary().status, RunStatus::Closed);
    assert_eq!(read.summary().run_counts.in_progress, 1);
    assert_eq!(json!(read.runs), json!(task.runs));
    assert_eq!(
        ledger.list().unwrap()[0].summary().status,
        RunStatus::Closed
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}
