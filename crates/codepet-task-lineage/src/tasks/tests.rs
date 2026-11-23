use super::*;
use std::process::Command;

struct Fixture {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    ledger: Ledger,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        git(&repo, &["config", "user.name", "Test"]);
        std::fs::write(repo.join("file"), "initial").unwrap();
        git(&repo, &["add", "file"]);
        git(&repo, &["commit", "-m", "initial"]);
        let ledger = Ledger::new(&dir.path().join("data"));
        Self {
            _dir: dir,
            repo,
            ledger,
        }
    }
    fn create(&self, key: &str) -> Receipt {
        self.ledger
            .save(
                SaveInput {
                    workspace: Some(self.repo.display().to_string()),
                    title: Some("修复菜单点击".into()),
                    objective: Some("点击桌宠能稳定打开菜单".into()),
                    operation_id: Some(key.into()),
                    plan: Some(plan()),
                    ..Default::default()
                },
                Actor::Agent,
            )
            .unwrap()
    }
}
fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn plan() -> PlanInput {
    PlanInput {
        summary: "恢复点击事件".into(),
        affected_areas: vec!["菜单与焦点".into()],
        criteria: vec![Criterion {
            id: String::new(),
            title: "切换窗口后点击".into(),
            precondition: "应用运行十分钟".into(),
            steps: "切换窗口再点击桌宠".into(),
            expected: "打开菜单".into(),
            scope: Scope::Both,
            required: true,
        }],
    }
}
fn update(r: &Receipt) -> SaveInput {
    SaveInput {
        task_id: Some(r.task_id.clone()),
        expected_revision: Some(r.revision),
        ..Default::default()
    }
}
fn run(r: &Receipt, result: TestResult, stage: Scope, target: &str) -> VerificationInput {
    VerificationInput {
        plan_revision: r.plan_revision.unwrap(),
        stage,
        target: target.into(),
        environment: "macOS".into(),
        results: vec![CheckResult {
            criterion_id: r.criterion_ids[0].clone(),
            result,
            evidence: "实际点击验证记录".into(),
        }],
    }
}

#[test]
fn milestones_are_idempotent_and_do_not_claim_head_commit_ownership() {
    let f = Fixture::new();
    let r = f.create("create");
    let again = f.create("create");
    assert_eq!(r.task_id, again.task_id);
    assert_eq!(f.ledger.list().unwrap().len(), 1);
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(task.rounds[0].code_refs[0].dirty, Some(false));
    assert!(task.rounds[0].code_refs[0].commits.is_empty());
    std::fs::write(f.repo.join("file"), "dirty").unwrap();
    let input = SaveInput {
        note: Some("完成本地实现".into()),
        ..update(&r)
    };
    let next = f.ledger.save(input.clone(), Actor::Agent).unwrap();
    let retry = f.ledger.save(input, Actor::Agent).unwrap();
    assert_eq!(next.revision, retry.revision);
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(task.rounds[0].code_refs.last().unwrap().dirty, Some(true));
    assert_eq!(task.rounds.len(), 1);
    assert!(serde_json::to_vec(&next).unwrap().len() < 1000);
}

#[test]
fn stale_revisions_and_reused_operation_ids_cannot_overwrite_manual_changes() {
    let f = Fixture::new();
    let r = f.create("create");
    f.ledger
        .save(
            SaveInput {
                title: Some("用户改名".into()),
                ..update(&r)
            },
            Actor::Human,
        )
        .unwrap();
    assert!(f
        .ledger
        .save(
            SaveInput {
                note: Some("旧状态".into()),
                ..update(&r)
            },
            Actor::Agent
        )
        .unwrap_err()
        .contains("revision=2"));
    assert!(f
        .ledger
        .save(
            SaveInput {
                workspace: Some(f.repo.display().to_string()),
                title: Some("其他标题".into()),
                objective: Some("目标".into()),
                operation_id: Some("create".into()),
                ..Default::default()
            },
            Actor::Agent
        )
        .unwrap_err()
        .contains("操作 ID"));
    assert_eq!(f.ledger.get(&r.task_id).unwrap().title, "用户改名");
}

#[test]
fn self_test_does_not_close_and_recurrence_preserves_old_round() {
    let f = Fixture::new();
    let r = f.create("create");
    let fail = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(&r, TestResult::Failed, Scope::Local, "dirty workspace")),
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    let pass = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(&fail, TestResult::Passed, Scope::Release, "1.2.0")),
                ..update(&fail)
            },
            Actor::Agent,
        )
        .unwrap();
    assert!(f
        .ledger
        .close(&r.task_id, pass.revision, "accepted", "自测通过")
        .is_err());
    let human = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(&pass, TestResult::Passed, Scope::Release, "1.2.0")),
                ..update(&pass)
            },
            Actor::Human,
        )
        .unwrap();
    let closed = f
        .ledger
        .close(
            &r.task_id,
            human.revision,
            "accepted",
            "已安装并实际验证 1.2.0",
        )
        .unwrap();
    let reopened = f
        .ledger
        .save(
            SaveInput {
                reopen_reason: Some("1.3.0 再次出现".into()),
                thread_id: Some(id()),
                ..update(&closed)
            },
            Actor::Agent,
        )
        .unwrap();
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(task.rounds.len(), 2);
    assert_eq!(task.rounds[0].closure.as_ref().unwrap().outcome, "accepted");
    assert_eq!(task.rounds[0].verifications.len(), 3);
    assert!(task.rounds[1].verifications.is_empty());
    assert!(task.rounds[1].closure.is_none());
    assert_ne!(closed.round_id, reopened.round_id);
}

#[test]
fn plan_revisions_keep_old_tests_and_target_change_invalidates_old_passes() {
    let f = Fixture::new();
    let r = f.create("create");
    let pass = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(&r, TestResult::Passed, Scope::Release, "1.2.0")),
                ..update(&r)
            },
            Actor::Human,
        )
        .unwrap();
    let changed = f
        .ledger
        .save(
            SaveInput {
                plan: Some(plan()),
                ..update(&pass)
            },
            Actor::Agent,
        )
        .unwrap();
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(task.rounds[0].plans.len(), 2);
    assert_eq!(task.rounds[0].verifications[0].input.plan_revision, 1);
    assert!(!human_acceptance_ready(&task.rounds[0]));
    let newer = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(
                    &changed,
                    TestResult::NotTested,
                    Scope::Release,
                    "1.3.0",
                )),
                ..update(&changed)
            },
            Actor::Human,
        )
        .unwrap();
    assert!(f
        .ledger
        .close(&r.task_id, newer.revision, "accepted", "不能沿用旧结果")
        .is_err());
    f.ledger
        .close(
            &r.task_id,
            newer.revision,
            "cancelled",
            "暂不继续，保留未测项",
        )
        .unwrap();
}

#[test]
fn linked_worktrees_share_project_but_distinct_repositories_do_not() {
    let f = Fixture::new();
    let a = f.create("a");
    let linked = f._dir.path().join("linked");
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-b",
            "codex/other",
            linked.to_str().unwrap(),
        ],
    );
    let b = f
        .ledger
        .save(
            SaveInput {
                workspace: Some(linked.display().to_string()),
                title: Some("另一个目标".into()),
                objective: Some("另一个功能".into()),
                ..Default::default()
            },
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(
        f.ledger.get(&a.task_id).unwrap().project_id,
        f.ledger.get(&b.task_id).unwrap().project_id
    );
    let other = Fixture::new();
    assert!(f
        .ledger
        .save(
            SaveInput {
                workspace: Some(other.repo.display().to_string()),
                ..update(&a)
            },
            Actor::Agent
        )
        .unwrap_err()
        .contains("不属于"));
    let scoped = f
        .ledger
        .find(FindInput {
            workspace: Some(linked.display().to_string()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(scoped["total"], 2);
}

#[test]
fn commits_are_explicit_and_verified_no_actor_spoofing() {
    let f = Fixture::new();
    let r = f.create("create");
    let sha = git(&f.repo, &["rev-parse", "HEAD"]);
    let r = f
        .ledger
        .save(
            SaveInput {
                commits: vec![sha[..8].into()],
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(task.rounds[0].code_refs.last().unwrap().commits[0].sha, sha);
    assert_eq!(
        task.rounds[0].code_refs.last().unwrap().commits[0].in_cached_upstream,
        None
    );
    assert!(f
        .ledger
        .save(
            SaveInput {
                commits: vec!["--help".into()],
                ..update(&r)
            },
            Actor::Agent
        )
        .is_err());
    assert!(serde_json::from_value::<SaveInput>(json!({"actor":"human"})).is_err());
    assert!(
        serde_json::from_value::<SaveInput>(json!({"closure":{"outcome":"accepted"}})).is_err()
    );
}

#[test]
fn concurrent_writers_keep_one_update_and_retries_survive_restart() {
    let f = Fixture::new();
    let r = f.create("create");
    let handles = (0..2)
        .map(|i| {
            let ledger = f.ledger.clone();
            let input = SaveInput {
                note: Some(format!("worker {i}")),
                ..update(&r)
            };
            std::thread::spawn(move || ledger.save(input, Actor::Agent))
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let reopened = Ledger::new(&f._dir.path().join("data"));
    assert_eq!(reopened.get(&r.task_id).unwrap().revision, 2);
    let same = f.create("create");
    assert_eq!(same.task_id, r.task_id);
    assert_eq!(same.revision, 1);
}

#[test]
fn linked_commits_follow_cached_upstream_without_inferring_task_ownership() {
    let f = Fixture::new();
    let remote = f._dir.path().join("remote.git");
    git(&f.repo, &["init", "--bare", remote.to_str().unwrap()]);
    git(
        &f.repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&f.repo, &["push", "-u", "origin", "main"]);
    let r = f.create("create");
    let first = git(&f.repo, &["rev-parse", "HEAD"]);
    let r = f
        .ledger
        .save(
            SaveInput {
                commits: vec![first],
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(
        f.ledger.get(&r.task_id).unwrap().rounds[0]
            .code_refs
            .last()
            .unwrap()
            .commits[0]
            .in_cached_upstream,
        Some(true)
    );
    std::fs::write(f.repo.join("file"), "second").unwrap();
    git(&f.repo, &["add", "file"]);
    git(&f.repo, &["commit", "-m", "second"]);
    let second = git(&f.repo, &["rev-parse", "HEAD"]);
    let r = f
        .ledger
        .save(
            SaveInput {
                commits: vec![second.clone()],
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(
        f.ledger.get(&r.task_id).unwrap().rounds[0]
            .code_refs
            .last()
            .unwrap()
            .commits[0]
            .in_cached_upstream,
        Some(false)
    );
    git(&f.repo, &["push"]);
    f.ledger
        .save(
            SaveInput {
                commits: vec![second],
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(
        task.rounds[0].code_refs.last().unwrap().commits[0].in_cached_upstream,
        Some(true)
    );
    assert!(task.rounds[0].code_refs[0].commits.is_empty());
}

#[test]
fn acceptance_cannot_combine_checks_from_different_installed_versions() {
    let f = Fixture::new();
    let r = f.create("create");
    let mut p = plan();
    let mut second = p.criteria[0].clone();
    second.title = "长时间运行后点击".into();
    p.criteria.push(second);
    let r = f
        .ledger
        .save(
            SaveInput {
                plan: Some(p),
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    let mut input = run(&r, TestResult::Passed, Scope::Release, "1.2.0");
    input.results.push(CheckResult {
        criterion_id: r.criterion_ids[1].clone(),
        result: TestResult::Passed,
        evidence: "实际验证长时间运行".into(),
    });
    let r = f
        .ledger
        .save(
            SaveInput {
                verification: Some(input),
                ..update(&r)
            },
            Actor::Human,
        )
        .unwrap();
    assert!(human_acceptance_ready(
        &f.ledger.get(&r.task_id).unwrap().rounds[0]
    ));
    let r = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(&r, TestResult::Passed, Scope::Release, "1.3.0")),
                ..update(&r)
            },
            Actor::Human,
        )
        .unwrap();
    assert!(!human_acceptance_ready(
        &f.ledger.get(&r.task_id).unwrap().rounds[0]
    ));
    assert!(f
        .ledger
        .close(&r.task_id, r.revision, "accepted", "不能拼接其他版本结果")
        .is_err());
}

#[test]
fn four_status_workflow_keeps_failed_results_and_reopen_history() {
    let f = Fixture::new();
    let r = f.create("simple-workflow");
    assert_eq!(r.status.as_deref(), Some("in_progress"));
    let ready = f
        .ledger
        .save(
            SaveInput {
                phase: Some(Phase::ReadyForDelivery),
                ..update(&r)
            },
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(ready.status.as_deref(), Some("ready_for_delivery"));
    assert!(f
        .ledger
        .save(
            SaveInput {
                phase: Some(Phase::Published),
                ..update(&ready)
            },
            Actor::Agent
        )
        .unwrap_err()
        .contains("发布版本"));
    let published = f
        .ledger
        .save(
            SaveInput {
                phase: Some(Phase::Published),
                delivery: Some("Release 1.3.0".into()),
                ..update(&ready)
            },
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(published.status.as_deref(), Some("published"));
    let failed = f
        .ledger
        .save(
            SaveInput {
                verification: Some(run(&published, TestResult::Failed, Scope::Release, "1.3.0")),
                ..update(&published)
            },
            Actor::Human,
        )
        .unwrap();
    assert_eq!(failed.status.as_deref(), Some("in_progress"));
    let closed = f
        .ledger
        .close(&r.task_id, failed.revision, "closed", "暂不继续处理")
        .unwrap();
    assert_eq!(closed.status.as_deref(), Some("closed"));
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(
        task.rounds[0].verifications[0].input.results[0].result,
        TestResult::Failed
    );
    assert_eq!(task.rounds[0].closure.as_ref().unwrap().outcome, "closed");
    let reopened = f
        .ledger
        .save(
            SaveInput {
                reopen_reason: Some("继续处理此前问题".into()),
                ..update(&closed)
            },
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(reopened.status.as_deref(), Some("in_progress"));
    let task = f.ledger.get(&r.task_id).unwrap();
    assert_eq!(task.rounds.len(), 2);
    assert!(task.rounds[0].closure.is_some());
    assert!(task.rounds[1].verifications.is_empty());
}

#[test]
fn legacy_phases_read_as_three_open_states_but_schema_only_advertises_new_values() {
    for legacy in [
        "investigating",
        "planning",
        "implementing",
        "validating_local",
    ] {
        let phase: Phase = serde_json::from_value(json!(legacy)).unwrap();
        assert_eq!(phase, Phase::InProgress);
        assert_eq!(serde_json::to_value(phase).unwrap(), json!("in_progress"));
    }
    for legacy in [
        "ready_to_publish",
        "ready_for_release",
        "awaiting_acceptance",
    ] {
        let phase: Phase = serde_json::from_value(json!(legacy)).unwrap();
        assert_eq!(phase, Phase::ReadyForDelivery);
    }
    assert_eq!(
        serde_json::from_value::<Phase>(json!("validating_release")).unwrap(),
        Phase::Published
    );
    let schema = serde_json::to_value(schemars::schema_for!(Phase)).unwrap();
    assert_eq!(
        schema["enum"],
        json!(["in_progress", "ready_for_delivery", "published"])
    );
    assert!(serde_json::from_value::<Phase>(json!("closed")).is_err());
}

#[test]
fn delivery_projection_scopes_shared_changes_and_uses_only_explicit_commits() {
    let f = Fixture::new();
    let r = f.create("delivery-facts");
    let mut task = f.ledger.get(&r.task_id).unwrap();
    let round = &mut task.rounds[0];
    assert_eq!(delivery_facts(round).summary, "提交待关联");
    let code = round.code_refs.last_mut().unwrap();
    code.dirty = Some(true);
    assert_eq!(delivery_facts(round).summary, "待提交（共享工作树）");
    round.code_refs.last_mut().unwrap().commits = vec![CommitRef {
        sha: "a".repeat(40),
        in_cached_upstream: Some(false),
    }];
    assert_eq!(
        delivery_facts(round).summary,
        "待提交（共享工作树） · 待推送"
    );
    let mut next = round.code_refs.last().unwrap().clone();
    next.dirty = Some(false);
    next.commits[0].in_cached_upstream = Some(true);
    round.code_refs.push(next);
    assert_eq!(delivery_facts(round).summary, "已推送");
    round.code_refs.last_mut().unwrap().commits[0].in_cached_upstream = None;
    assert_eq!(delivery_facts(round).summary, "推送待确认");
    round.code_refs.last_mut().unwrap().error = Some("Git unavailable".into());
    assert_eq!(delivery_facts(round).summary, "Git 状态待确认");
    let view = task_view(&task).unwrap();
    assert!(view["rounds"][0]["deliveryFacts"].is_object());
    assert!(
        serde_json::to_value(&task).unwrap()["rounds"][0]
            .get("deliveryFacts")
            .is_none(),
        "read projections must not become stored state"
    );
}
