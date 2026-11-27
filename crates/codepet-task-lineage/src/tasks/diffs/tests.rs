use super::*;
use crate::tasks::{Actor, CreateTaskInput, FinishTaskRunInput, TestResult};
use std::{fs, path::PathBuf};
fn git(root: &Path, args: &[&str]) -> String {
    workbench::text_git(root, args).unwrap()
}
fn setup() -> (tempfile::TempDir, Ledger, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "QA"]);
    git(&root, &["config", "user.email", "qa@example.invalid"]);
    fs::write(root.join("a.txt"), "old\nunchanged\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    let ledger = Ledger::new(&tmp.path().join("data"));
    (tmp, ledger, root)
}
fn create(ledger: &Ledger, root: &Path, commits: Vec<String>) -> super::super::Receipt {
    let input: CreateTaskInput = serde_json::from_value(json!({"title":"Retained diff","agentWorkspace":{"path":root,"relatedCommits":commits},"initialRun":{"agentConversation":["00000000-0000-4000-8000-000000000002"],"validationPlan":{"items":[{"title":"Diff","steps":"Read","expected":"Retained","scope":"local"}]}}})).unwrap();
    ledger
        .create_task(input, Actor::agent("codex").unwrap())
        .unwrap()
}
#[test]
fn committed_diff_survives_clean_worktree_repository_removal_and_no_revision_changes() {
    let (_tmp, ledger, root) = setup();
    fs::write(root.join("a.txt"), "new\nunchanged\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "change"]);
    let sha = git(&root, &["rev-parse", "HEAD"]);
    let r = create(&ledger, &root, vec![sha]);
    fs::remove_dir_all(&root).unwrap();
    let versions = ledger.change_diffs(&r.task_id, &r.run_id).unwrap();
    assert_eq!(versions.as_array().unwrap().len(), 1);
    let version = versions[0]["id"].as_str().unwrap();
    assert!(versions[0]["files"][0].get("patch").is_none());
    let diff = ledger
        .file_diff(&r.task_id, &r.run_id, version, "a.txt")
        .unwrap();
    assert!(diff["patch"].as_str().unwrap().contains("-old\n+new"));
    assert_eq!(diff["added"], 1);
    assert_eq!(diff["deleted"], 1);
    assert_eq!(ledger.get(&r.task_id).unwrap().revision, r.revision);
    assert!(ledger
        .file_diff(&r.task_id, &r.run_id, "../index", "a.txt")
        .is_err());
    assert!(ledger
        .file_diff(&r.task_id, &r.run_id, version, "../outside")
        .is_err());
}
#[test]
fn finish_captures_only_declared_files_and_retains_dirty_diff_after_cleanup() {
    let (_tmp, ledger, root) = setup();
    let r = create(&ledger, &root, vec![]);
    fs::write(root.join("a.txt"), "new\n").unwrap();
    fs::write(root.join("untracked.txt"), "added\n").unwrap();
    fs::write(root.join("private.txt"), "unrelated\n").unwrap();
    let input: FinishTaskRunInput=serde_json::from_value(json!({"taskId":r.task_id,"runId":r.run_id,"expectedRevision":r.revision,"conclusion":"Done","agentConversation":["00000000-0000-4000-8000-000000000002"],"changedFiles":["a.txt","untracked.txt"],"verification":{"planRevision":r.plan_revision,"stage":"local","target":"test","environment":"test","results":[{"criterionId":r.criterion_ids[0],"result":TestResult::Passed,"evidence":"read"}]}})).unwrap();
    ledger
        .finish_task_run(input, Actor::agent("codex").unwrap())
        .unwrap();
    git(&root, &["reset", "--hard"]);
    fs::remove_file(root.join("untracked.txt")).unwrap();
    let versions = ledger.change_diffs(&r.task_id, &r.run_id).unwrap();
    assert_eq!(versions[0]["files"].as_array().unwrap().len(), 2);
    let diff = ledger
        .file_diff(
            &r.task_id,
            &r.run_id,
            versions[0]["id"].as_str().unwrap(),
            "untracked.txt",
        )
        .unwrap();
    assert!(diff["patch"].as_str().unwrap().contains("+added"));
    assert!(ledger
        .file_diff(
            &r.task_id,
            &r.run_id,
            versions[0]["id"].as_str().unwrap(),
            "private.txt"
        )
        .is_err());
}
#[test]
fn rename_deletion_binary_and_limits_are_explicit() {
    let (_tmp, _ledger, root) = setup();
    git(&root, &["mv", "a.txt", "renamed.txt"]);
    fs::write(root.join("binary.bin"), [0, 1, 2]).unwrap();
    fs::write(root.join("large.txt"), vec![b'x'; FILE_LIMIT + 1]).unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "rename and binaries"]);
    let sha = git(&root, &["rev-parse", "HEAD"]);
    let changes = workspace::commit_changes(&root, &sha);
    let mut budget = CAPTURE_LIMIT;
    let files = capture(&root, &changes.files, Some(&sha), &mut budget);
    assert!(files
        .iter()
        .find(|f| f.path == "renamed.txt")
        .unwrap()
        .patch
        .contains("rename from a.txt"));
    assert!(files
        .iter()
        .find(|f| f.path == "binary.bin")
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("二进制"));
    assert!(files
        .iter()
        .find(|f| f.path == "large.txt")
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("上限"));
    fs::remove_file(root.join("renamed.txt")).unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "delete"]);
    let sha = git(&root, &["rev-parse", "HEAD"]);
    let file = workspace::commit_changes(&root, &sha).files.remove(0);
    let patch = capture(&root, &[file], Some(&sha), &mut { CAPTURE_LIMIT }).remove(0);
    assert_eq!(patch.deleted, Some(2));
}
#[cfg(unix)]
#[test]
fn untracked_symlink_cannot_capture_outside_workspace() {
    let (tmp, _ledger, root) = setup();
    let outside = tmp.path().join("outside");
    fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(outside, root.join("link")).unwrap();
    let file = ChangedFile {
        path: "link".into(),
        change_type: "untracked".into(),
        staged: false,
        original_path: None,
    };
    let patch = capture(&root, &[file], None, &mut { CAPTURE_LIMIT }).remove(0);
    assert!(patch.error.is_some());
}

#[test]
fn older_linked_worktree_uses_common_repository_for_lazy_commit_recovery() {
    let (tmp, ledger, root) = setup();
    let sha = git(&root, &["rev-parse", "HEAD"]);
    let linked = tmp.path().join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            &sha,
        ],
    );
    let r = create(&ledger, &linked, vec![sha]);
    // Simulate a legacy record which predates diff capture.
    fs::remove_dir_all(ledger.root.join("diffs")).unwrap();
    git(&root, &["worktree", "remove", linked.to_str().unwrap()]);
    let versions = ledger.change_diffs(&r.task_id, &r.run_id).unwrap();
    assert!(versions[0]["error"].is_null());
    let diff = ledger
        .file_diff(
            &r.task_id,
            &r.run_id,
            versions[0]["id"].as_str().unwrap(),
            "a.txt",
        )
        .unwrap();
    assert!(diff["patch"].as_str().unwrap().contains("+old"));
}

#[test]
fn filenames_are_literal_pathspecs_not_globs() {
    let (_tmp, _ledger, root) = setup();
    fs::write(root.join("a[1].txt"), "literal old\n").unwrap();
    fs::write(root.join("a1.txt"), "other old\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base"]);
    fs::write(root.join("a[1].txt"), "literal new\n").unwrap();
    fs::write(root.join("a1.txt"), "unrelated secret\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "change"]);
    let sha = git(&root, &["rev-parse", "HEAD"]);
    let file = ChangedFile {
        path: "a[1].txt".into(),
        change_type: "modified".into(),
        staged: false,
        original_path: None,
    };
    let patch = capture(&root, &[file], Some(&sha), &mut { CAPTURE_LIMIT }).remove(0);
    assert!(patch.error.is_none(), "{:?}", patch.error);
    assert!(patch.patch.contains("+literal new"));
    assert!(!patch.patch.contains("unrelated secret"));
}

#[test]
fn patch_references_are_portable_and_legacy_inline_patches_remain_readable() {
    let (tmp, ledger, root) = setup();
    let sha = git(&root, &["rev-parse", "HEAD"]);
    let receipt = create(&ledger, &root, vec![sha.clone()]);
    let version = format!("commit-{sha}");
    let diff = ledger
        .file_diff(&receipt.task_id, &receipt.run_id, &version, "a.txt")
        .unwrap();
    let key = format!(
        "diffs/{}/{}/{version}.json",
        receipt.task_id, receipt.run_id
    );
    let store = ledger.store().unwrap();
    let stored: Value = store.read(&key).unwrap().unwrap();
    let patch_ref = &stored["files"][0]["patchRef"];
    assert!(stored["files"][0].get("patch").is_none());
    assert!(patch_ref["ref"].as_str().unwrap().starts_with("documents/"));
    assert_eq!(patch_ref["length"], diff["patch"].as_str().unwrap().len());
    drop(store);

    let moved = tmp.path().join("relocated-data");
    fs::rename(ledger.data_directory(), &moved).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let ledger = Ledger::new(&moved);
    let read = || ledger.file_diff(&receipt.task_id, &receipt.run_id, &version, "a.txt");
    assert_eq!(read().unwrap(), diff);
    assert!(diff.get("patchRef").is_none());

    let mut invalid = stored.clone();
    invalid["files"][0]["patchRef"]["offset"] = json!(u64::MAX);
    ledger.store().unwrap().write(&key, &invalid).unwrap();
    assert!(read().unwrap_err().contains("范围越界"));

    let mut legacy = stored;
    let file = legacy["files"][0].as_object_mut().unwrap();
    file.remove("patchRef");
    file.insert("patch".into(), diff["patch"].clone());
    ledger.store().unwrap().write(&key, &legacy).unwrap();
    assert_eq!(read().unwrap(), diff);
}
