//! Local, immutable diff evidence. Kept outside task/MCP payloads and independent of Git GC.
use super::{model::*, now, workspace, Ledger, Task};
use crate::{stable_id, store::Store, workbench, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::Read, path::Path};
const FILE_LIMIT: usize = 256 * 1024;
const CAPTURE_LIMIT: usize = 8 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDiff {
    path: String,
    original_path: Option<String>,
    change_type: String,
    added: Option<u64>,
    deleted: Option<u64>,
    patch: String,
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    id: String,
    commit: Option<String>,
    captured_at: i64,
    observed_at: i64,
    files: Vec<FileDiff>,
    error: Option<String>,
}
fn prefix(task: &Task, run: &TaskRun) -> String {
    format!("diffs/{}/{}", task.id, run.id)
}
fn version_key(prefix: &str, id: &str) -> String {
    format!("{prefix}/{id}.json")
}
fn summary(version: &Version) -> Value {
    let mut value = json!(version);
    for file in value["files"].as_array_mut().unwrap() {
        file.as_object_mut().unwrap().remove("patch");
    }
    value
}
fn git_patch(
    root: &Path,
    file: &ChangedFile,
    commit: Option<&str>,
) -> Result<(String, Option<u64>, Option<u64>)> {
    workspace::validate_run_files(std::slice::from_ref(&file.path))?;
    if let Some(original) = &file.original_path {
        workspace::validate_run_files(std::slice::from_ref(original))?;
    }
    if commit.is_none()
        && (file.change_type == "untracked"
            || (file.change_type == "added"
                && workbench::text_git(root, &["rev-parse", "--verify", "HEAD"]).is_err()))
    {
        let target = workbench::change_path(root, &json!({"path":file.path}))?;
        let mut bytes = vec![];
        std::fs::File::open(target)
            .map_err(|e| e.to_string())?
            .take(FILE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > FILE_LIMIT {
            return Err("文件超过 256 KiB 差异上限".into());
        }
        if bytes.contains(&0) {
            return Err("二进制文件，不提供文本差异".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "文件不是 UTF-8 文本")?;
        let count = text.lines().count();
        let mut patch = format!(
            "--- /dev/null\n+++ b/{}\n@@ -0,0 +1,{} @@\n",
            file.path, count
        );
        for line in text.split_inclusive('\n') {
            patch.push('+');
            patch.push_str(line);
        }
        if !text.is_empty() && !text.ends_with('\n') {
            patch.push_str("\n\\ No newline at end of file\n");
        }
        return Ok((patch, Some(count as u64), Some(0)));
    }
    if file.change_type == "conflicted" {
        return Err("存在合并冲突，请在工作区解决后记录差异".into());
    }
    let base;
    let mut args = if let Some(commit) = commit {
        vec!["show", "--format=", "--first-parent", "--root", commit]
    } else {
        base = workbench::text_git(root, &["rev-parse", "--verify", "HEAD"])?;
        vec!["diff", base.as_str()]
    };
    args.extend([
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--binary",
        "--find-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--unified=3",
        "--",
    ]);
    let literal = format!(":(literal){}", file.path);
    let original = file
        .original_path
        .as_ref()
        .map(|p| format!(":(literal){p}"));
    args.push(&literal);
    if let Some(original) = &original {
        args.push(original);
    }
    let bytes = workbench::git(root, &args)?;
    if bytes.len() > FILE_LIMIT {
        return Err("差异超过 256 KiB 预览上限".into());
    }
    let patch = String::from_utf8(bytes).map_err(|_| "差异不是 UTF-8 文本")?;
    if patch
        .lines()
        .any(|l| l.starts_with("Binary files ") || l == "GIT binary patch")
    {
        return Err("二进制文件，不提供文本差异".into());
    }
    let mut added = 0;
    let mut deleted = 0;
    let mut hunk = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            hunk = false;
        } else if line.starts_with("@@ ") {
            hunk = true;
        } else if hunk && line.starts_with('+') {
            added += 1;
        } else if hunk && line.starts_with('-') {
            deleted += 1;
        }
    }
    Ok((patch, Some(added), Some(deleted)))
}
fn capture(
    root: &Path,
    files: &[ChangedFile],
    commit: Option<&str>,
    budget: &mut usize,
) -> Vec<FileDiff> {
    files
        .iter()
        .take(300)
        .map(|file| {
            let result = if *budget < FILE_LIMIT {
                Err("本次快照达到 8 MiB 上限，未保存此文件差异".into())
            } else {
                git_patch(root, file, commit)
            };
            let (patch, added, deleted, error) = match result {
                Ok((patch, added, deleted)) => {
                    *budget = budget.saturating_sub(patch.len());
                    (patch, added, deleted, None)
                }
                Err(error) => (String::new(), None, None, Some(error)),
            };
            FileDiff {
                path: file.path.clone(),
                original_path: file.original_path.clone(),
                change_type: file.change_type.clone(),
                added,
                deleted,
                patch,
                error,
            }
        })
        .collect()
}
/// Called in the task transaction. Only explicitly linked commits or declared run files are captured.
pub(super) fn prepare(
    store: &Store,
    task: &Task,
    repository: &str,
    capture_workspace: bool,
) -> Result<Vec<(String, Value)>> {
    let run = task.runs.last().ok_or("执行不存在")?;
    prepare_run(store, task, run, repository, capture_workspace)
}
fn prepare_run(
    store: &Store,
    task: &Task,
    run: &TaskRun,
    repository: &str,
    capture_workspace: bool,
) -> Result<Vec<(String, Value)>> {
    let repository = workspace::evidence_repository(&run.agent_workspace, repository);
    let prefix = prefix(task, run);
    let index_key = format!("{prefix}/index.json");
    let mut index: Vec<Value> = store.read(&index_key)?.unwrap_or_default();
    let mut writes = vec![];
    let mut budget = CAPTURE_LIMIT;
    for sha in &run.agent_workspace.related_commits {
        if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let id = format!("commit-{sha}");
        if index.iter().any(|v| {
            v["id"] == id
                && v["error"].is_null()
                && v["files"]
                    .as_array()
                    .is_some_and(|f| f.iter().all(|f| f["error"].is_null()))
        }) {
            continue;
        }
        let changes = workspace::commit_changes(Path::new(&repository), sha);
        let version = Version {
            id: id.clone(),
            commit: Some(sha.clone()),
            captured_at: now(),
            observed_at: run.agent_workspace.observed_at,
            files: capture(
                Path::new(&repository),
                &changes.files,
                Some(sha),
                &mut budget,
            ),
            error: changes.error.or_else(|| {
                (changes.files.len() > 300)
                    .then(|| "提交超过 300 个文件，仅保留前 300 个文件差异".into())
            }),
        };
        // An unsuccessful retry must never replace successfully retained file evidence.
        let old: Option<Version> = store.read(&version_key(&prefix, &id))?;
        let mut version = version;
        if let Some(old) = old {
            for file in &mut version.files {
                if file.error.is_some() {
                    if let Some(saved) = old
                        .files
                        .iter()
                        .find(|f| f.path == file.path && f.error.is_none())
                    {
                        *file = saved.clone();
                    }
                }
            }
            if version.error.is_some() && old.error.is_none() {
                version = old;
            }
        }
        index.retain(|v| v["id"] != id);
        index.push(summary(&version));
        writes.push((version_key(&prefix, &id), json!(version)));
    }
    if capture_workspace && run.agent_workspace.error.is_none() {
        if let Some(scope) = &run.agent_workspace.run_files {
            let files: Vec<_> = run
                .agent_workspace
                .changed_files
                .iter()
                .filter(|f| scope.contains(&f.path))
                .cloned()
                .collect();
            if !files.is_empty() {
                let captured = capture(
                    Path::new(&run.agent_workspace.path),
                    &files,
                    None,
                    &mut budget,
                );
                let fingerprint = stable_id(
                    serde_json::to_string(&captured)
                        .map_err(|e| e.to_string())?
                        .as_bytes(),
                );
                let id = format!("workspace-{fingerprint}");
                if !index.iter().any(|v| v["id"] == id) {
                    let version = Version {
                        id: id.clone(),
                        commit: None,
                        captured_at: now(),
                        observed_at: run.agent_workspace.observed_at,
                        files: captured,
                        error: None,
                    };
                    index.push(summary(&version));
                    writes.push((version_key(&prefix, &id), json!(version)));
                }
            }
        }
    }
    if !writes.is_empty() {
        writes.push((index_key, json!(index)));
    }
    Ok(writes)
}
impl Ledger {
    /// Lazy backfill of immutable commit evidence; never reads a historical worktree as if current.
    pub fn change_diffs(&self, task_id: &str, run_id: &str) -> Result<Value> {
        let store = self.store()?;
        let task = Self::load(&store, task_id)?;
        let run = task
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .ok_or("执行不存在")?;
        let repository = Self::projects(&store)?
            .into_iter()
            .find(|p| p.id == task.project_id)
            .ok_or("项目不存在")?
            .path;
        let writes = prepare_run(&store, &task, run, &repository, false)?;
        if !writes.is_empty() {
            store.commit(writes)?;
        }
        Ok(json!(store
            .read::<Vec<Value>>(&format!("{}/index.json", prefix(&task, run)))?
            .unwrap_or_default()))
    }
    pub fn file_diff(
        &self,
        task_id: &str,
        run_id: &str,
        version: &str,
        path: &str,
    ) -> Result<Value> {
        let store = self.store()?;
        let task = Self::load(&store, task_id)?;
        let run = task
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .ok_or("执行不存在")?;
        let prefix = prefix(&task, run);
        let index: Vec<Value> = store
            .read(&format!("{prefix}/index.json"))?
            .unwrap_or_default();
        if !index.iter().any(|v| v["id"] == version) {
            return Err("差异快照不存在".into());
        }
        let saved: Version = store
            .read(&version_key(&prefix, version))?
            .ok_or("差异快照不可用")?;
        Ok(json!(saved
            .files
            .into_iter()
            .find(|f| f.path == path)
            .ok_or("文件不属于此快照")?))
    }
}
#[cfg(test)]
mod tests;
