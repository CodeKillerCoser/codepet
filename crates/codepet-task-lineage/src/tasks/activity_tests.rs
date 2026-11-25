use super::*;
use serde_json::json;

fn create(ledger: &Ledger, workspace: &Path) -> Receipt {
    ledger.create_task(serde_json::from_value(json!({"title":"共同目标","agentWorkspace":{"path":workspace},"initialRun":{"title":"协作执行","agentConversation":["claude-conversation"],"validationPlan":{"items":[{"title":"检查结果","steps":"检查","expected":"成功"}]}}})).unwrap(),Actor::agent("claude").unwrap()).unwrap()
}
fn comment(receipt: &Receipt, source: &str, message: &str) -> CommentTaskRunInput {
    serde_json::from_value(json!({"taskId":receipt.task_id,"runId":receipt.run_id,"comment":message,"agentConversation":[format!("{source}-conversation")],"operationId":id()})).unwrap()
}

#[test]
fn combined_update_keeps_comment_and_each_change_with_its_actual_author() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(root.path());
    let first = create(&ledger, root.path());
    let before = ledger.get(&first.task_id).unwrap();
    let criterion = &before.runs[0].validation_plan.items[0];
    let updated=ledger.update_task_run(serde_json::from_value(json!({"taskId":first.task_id,"runId":first.run_id,"expectedRevision":first.revision,
        "agentWorkspace":{"path":other.path()},"agentConversation":["cursor-conversation"],"solution":"跨工作区的新方案",
        "validationPlan":{"items":[{"id":criterion.id,"title":"更新后的检查","steps":"重新检查","expected":"成功"}]},
        "verification":{"planRevision":2,"stage":"local","target":"workspace B","environment":"isolated","results":[{"criterionId":criterion.id,"result":"passed","evidence":"实际检查通过"}]},
        "status":"published","delivery":"release-2","note":"请另一个 Agent 复核方案"})).unwrap(),Actor::agent("cursor").unwrap()).unwrap();
    let task = ledger.get(&first.task_id).unwrap();
    let run = &task.runs[0];
    let appended = &run.timeline[before.runs[0].timeline.len()..];
    for kind in [
        "content_updated",
        "workspace_changed",
        "plan_updated",
        "verification",
        "delivery",
        "status_changed",
        "comment",
        "conversation_linked",
    ] {
        assert!(
            appended.iter().any(|event| event.kind == kind),
            "missing {kind}"
        );
    }
    assert!(appended.iter().all(|entry| entry.actor == "cursor"
        && entry.agent_conversation.len() == 1
        && entry.agent_conversation[0].conversation_id == "cursor-conversation"));
    assert_eq!(run.created_by, "claude");
    assert_eq!(task.created_by, "claude");
    assert_eq!(json!(&run.timeline[..1]), json!(before.runs[0].timeline));
    let plan = appended
        .iter()
        .find(|entry| entry.kind == "plan_updated")
        .unwrap();
    assert_eq!(plan.changes[0].before["revision"], 1);
    assert_eq!(plan.changes[0].after["revision"], 2);
    assert_eq!(
        run.timeline.last().unwrap().summary,
        "请另一个 Agent 复核方案"
    );
    let failure=ledger.update_task_run(serde_json::from_value(json!({"taskId":first.task_id,"runId":first.run_id,"expectedRevision":updated.revision,
        "agentConversation":["other-conversation"],"verification":{"planRevision":2,"stage":"release","target":"release-2","environment":"isolated","results":[{"criterionId":criterion.id,"result":"failed","evidence":"复核发现失败"}]},"note":"复核未通过"})).unwrap(),Actor::agent("other-agent").unwrap()).unwrap();
    assert_eq!(failure.status, RunStatus::InProgress);
    let task = ledger.get(&first.task_id).unwrap();
    assert!(task.runs[0]
        .timeline
        .iter()
        .any(|entry| entry.kind == "status_changed" && entry.summary.contains("已发布 → 开发中")));
}

#[test]
fn concurrent_agents_append_without_revision_conflicts_and_retries_do_not_duplicate() {
    let root = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(root.path());
    let receipt = create(&ledger, root.path());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let writers = (0..4)
        .map(|index| {
            let ledger = ledger.clone();
            let receipt = receipt.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let source = format!("agent-{index}");
                let input = comment(&receipt, &source, &format!("评论 {index}"));
                barrier.wait();
                let first = ledger
                    .comment_task_run(input.clone(), Actor::agent(&source).unwrap())
                    .unwrap();
                assert_eq!(
                    ledger
                        .comment_task_run(input, Actor::agent(&source).unwrap())
                        .unwrap()
                        .activity_id,
                    first.activity_id
                );
                first
            })
        })
        .collect::<Vec<_>>();
    let receipts = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect::<Vec<_>>();
    let task = ledger.get(&receipt.task_id).unwrap();
    assert_eq!(task.revision, receipt.revision + 4);
    assert_eq!(task.runs[0].timeline.len(), 5);
    assert_eq!(
        receipts
            .iter()
            .filter_map(|r| r.activity_id.as_ref())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    assert!(task.runs[0].timeline[1..].iter().all(
        |entry| entry.agent_workspace.is_none() && entry.actor_role.as_deref() == Some("agent")
    ));
}

#[test]
fn historical_closed_run_accepts_replies_without_reopening_or_mutating_other_runs() {
    let root = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(root.path());
    let first = create(&ledger, root.path());
    let closed = ledger
        .close(
            &first.task_id,
            &first.run_id,
            first.revision,
            false,
            "人工结束",
        )
        .unwrap();
    let next=ledger.create_task_run(serde_json::from_value(json!({"taskId":first.task_id,"expectedRevision":closed.revision,"title":"人工恢复开发","agentConversation":[]})).unwrap(),Actor::human()).unwrap();
    ledger
        .complete_task(&first.task_id, next.revision, "暂时完成")
        .unwrap();
    let before = ledger.get(&first.task_id).unwrap();
    let first_comment = ledger
        .comment_task_run(
            comment(&first, "claude", "补充历史上下文"),
            Actor::agent("claude").unwrap(),
        )
        .unwrap();
    let mut reply = comment(&first, "cursor", "已读并回复");
    reply.reply_to = first_comment.activity_id;
    let receipt = ledger
        .comment_task_run(reply, Actor::agent("cursor").unwrap())
        .unwrap();
    assert_eq!(receipt.run_id, first.run_id);
    let after = ledger.get(&first.task_id).unwrap();
    assert_eq!(after.runs[0].status, RunStatus::Closed);
    assert_eq!(json!(after.completion), json!(before.completion));
    assert_eq!(json!(after.runs[1]), json!(before.runs[1]));
    assert_eq!(
        json!(&after.runs[0].timeline[..before.runs[0].timeline.len()]),
        json!(before.runs[0].timeline)
    );
    let mut invalid = comment(&first, "cursor", "无效回复");
    invalid.reply_to = Some("missing-event".into());
    assert!(ledger
        .comment_task_run(invalid, Actor::agent("cursor").unwrap())
        .is_err());
}

#[test]
fn old_event_prefix_is_immutable_and_activity_pages_are_stable() {
    let root = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(root.path());
    let first = create(&ledger, root.path());
    for index in 0..5 {
        ledger
            .comment_task_run(
                comment(&first, "claude", &format!("追加 {index}")),
                Actor::agent("claude").unwrap(),
            )
            .unwrap();
    }
    let before = ledger.get(&first.task_id).unwrap();
    for mode in 0..3 {
        let result = ledger.transact(
            format!("receipts/invalid-{mode}.json"),
            format!("invalid-{mode}"),
            |store, _| {
                let mut task = Ledger::load(store, &first.task_id)?;
                match mode {
                    0 => task.runs[0].timeline[0].summary = "改写".into(),
                    1 => {
                        task.runs[0].timeline.remove(0);
                    }
                    _ => task.runs[0].timeline.swap(0, 1),
                }
                Ok(task)
            },
        );
        assert!(result.unwrap_err().contains("只允许追加"));
        assert_eq!(json!(ledger.get(&first.task_id).unwrap()), json!(before));
    }
    let latest = ledger
        .find_runs(FindTaskRunsInput {
            run_id: Some(first.run_id.clone()),
            workspace: Some("/unrelated".into()),
            activity_limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(latest["activity"]["items"][0]["summary"], "追加 3");
    assert_eq!(latest["activity"]["hasEarlier"], true);
    let older = ledger
        .find_runs(FindTaskRunsInput {
            run_id: Some(first.run_id.clone()),
            activity_before: Some(latest["activity"]["firstCursor"].as_str().unwrap().into()),
            activity_limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(older["activity"]["items"][0]["summary"], "追加 1");
    let forward = ledger
        .find_runs(FindTaskRunsInput {
            run_id: Some(first.run_id.clone()),
            activity_after: Some(older["activity"]["nextCursor"].as_str().unwrap().into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(forward["activity"]["items"][0]["summary"], "追加 3");
}
