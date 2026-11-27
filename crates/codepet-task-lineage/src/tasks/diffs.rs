//! Local, immutable diff evidence. Kept outside task/MCP payloads and independent of Git GC.
use super::{documents, model::*, now, workspace, Ledger, Task};
use crate::{stable_id, store::Store, workbench, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};
mod batch;

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
    #[serde(default)]
    patch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patch_ref: Option<PatchRef>,
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct PatchRef {
    #[serde(rename = "ref")]
    reference: String,
    offset: u64,
    length: usize,
    sha256: String,
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
        let fields = file.as_object_mut().unwrap();
        fields.remove("patch");
        fields.remove("patchRef");
    }
    value
}
fn stored_version(version: &Version) -> Value {
    let mut value = json!(version);
    for file in value["files"].as_array_mut().unwrap() {
        if file.get("patchRef").is_some() {
            file.as_object_mut().unwrap().remove("patch");
        }
    }
    value
}

fn persist_patches(store: &Store, version: &mut Version) -> Result<()> {
    let mut document = Vec::new();
    let mut ranges = Vec::new();
    for (index, file) in version.files.iter_mut().enumerate() {
        if file.patch.is_empty() {
            continue;
        }
        let bytes = std::mem::take(&mut file.patch).into_bytes();
        let offset = document.len() as u64;
        let length = bytes.len();
        let sha256 = stable_id(&bytes);
        ranges.push((index, offset, length, sha256));
        document.extend(bytes);
    }
    if !document.is_empty() {
        let reference = documents::key(documents::Kind::Diff, &document);
        store.publish_document(&reference, &document)?;
        for (index, offset, length, sha256) in ranges {
            version.files[index].patch_ref = Some(PatchRef {
                reference: reference.clone(),
                offset,
                length,
                sha256,
            });
        }
    }
    Ok(())
}

fn read_patch(store: &Store, patch: PatchRef) -> Result<String> {
    if patch.length > CAPTURE_LIMIT {
        return Err("差异文档范围超过快照上限".into());
    }
    let mut doc = documents::open(store.root(), documents::Kind::Diff, &patch.reference)?;
    let size = doc.metadata().map_err(|error| error.to_string())?.len();
    if patch
        .offset
        .checked_add(patch.length as u64)
        .is_none_or(|end| end > size)
    {
        return Err("差异文档范围越界".into());
    }
    doc.seek(SeekFrom::Start(patch.offset))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0; patch.length];
    doc.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    if stable_id(&bytes) != patch.sha256 {
        return Err("差异文档摘要不匹配".into());
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn git_patch(
    root: &Path,
    file: &ChangedFile,
    read_as_added: bool,
    patch: Result<&[u8]>,
) -> Result<(String, Option<u64>, Option<u64>)> {
    workspace::validate_run_files(std::slice::from_ref(&file.path))?;
    if let Some(original) = &file.original_path {
        workspace::validate_run_files(std::slice::from_ref(original))?;
    }
    if read_as_added {
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
    let bytes = patch?;
    if bytes.len() > FILE_LIMIT {
        return Err("差异超过 256 KiB 预览上限".into());
    }
    let patch = std::str::from_utf8(bytes)
        .map_err(|_| "差异不是 UTF-8 文本")?
        .to_owned();
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
    let files = &files[..files.len().min(300)];
    let unborn = commit.is_none()
        && files.iter().any(|file| file.change_type == "added")
        && workbench::text_git(root, &["rev-parse", "--verify", "HEAD"]).is_err();
    capture_batch(root, files, commit, unborn, budget)
}

fn capture_batch(
    root: &Path,
    files: &[ChangedFile],
    commit: Option<&str>,
    unborn: bool,
    budget: &mut usize,
) -> Vec<FileDiff> {
    let output = if *budget < FILE_LIMIT || unborn {
        Ok(vec![])
    } else {
        batch::read(root, files, commit)
    };
    if files.len() > 1
        && output
            .as_ref()
            .is_err_and(|error| error == workbench::GIT_OUTPUT_LIMIT_ERROR)
    {
        // Keep unrelated small patches when a combined Git reply hits the output cap.
        let (first, second) = files.split_at(files.len() / 2);
        let mut captured = capture_batch(root, first, commit, unborn, budget);
        captured.extend(capture_batch(root, second, commit, unborn, budget));
        return captured;
    }
    let patches = output
        .as_deref()
        .map_err(Clone::clone)
        .and_then(batch::patches);
    files
        .iter()
        .map(|file| {
            let result = if *budget < FILE_LIMIT {
                Err("本次快照达到 8 MiB 上限，未保存此文件差异".into())
            } else {
                let patch =
                    patches.as_ref().map_err(Clone::clone).and_then(|patches| {
                        match patches.get(file.path.as_bytes()) {
                            Some(patch) => Ok(*patch),
                            None if commit.is_none() => Ok(&[][..]),
                            None => Err("Git 未返回该文件差异".into()),
                        }
                    });
                let read_as_added = commit.is_none()
                    && (file.change_type == "untracked" || (file.change_type == "added" && unborn));
                git_patch(root, file, read_as_added, patch)
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
                patch_ref: None,
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
                && (capture_workspace
                    || v["error"].is_null()
                    || v["files"].as_array().is_some_and(|files| !files.is_empty()))
        }) {
            continue;
        }
        let changes = run
            .agent_workspace
            .commit_changes
            .iter()
            .find(|change| change.commit == *sha && change.error.is_none())
            .cloned()
            .unwrap_or_else(|| workspace::commit_changes(Path::new(&repository), sha));
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
        persist_patches(store, &mut version)?;
        index.retain(|v| v["id"] != id);
        index.push(summary(&version));
        writes.push((version_key(&prefix, &id), stored_version(&version)));
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
                    let mut version = Version {
                        id: id.clone(),
                        commit: None,
                        captured_at: now(),
                        observed_at: run.agent_workspace.observed_at,
                        files: captured,
                        error: None,
                    };
                    persist_patches(store, &mut version)?;
                    index.push(summary(&version));
                    writes.push((version_key(&prefix, &id), stored_version(&version)));
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
        let task = Self::load_with_documents(&store, task_id, documents::Resolve::TextOnly)?;
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
        let mut file = saved
            .files
            .into_iter()
            .find(|f| f.path == path)
            .ok_or("文件不属于此快照")?;
        let patch = match file.patch_ref.take() {
            Some(reference) => read_patch(&store, reference)?,
            None => std::mem::take(&mut file.patch),
        };
        let mut output = json!(file);
        output["patch"] = json!(patch);
        Ok(output)
    }
}
#[cfg(test)]
mod tests;
