//! Use the same bounded, read-only Git command and NUL parser as the workbench.
use super::{model::*, now};
use crate::{stable_id, workbench, Result};
use std::path::Path;

pub(super) struct Context {
    pub key: String,
    pub name: String,
    pub repository: String,
    pub observation: AgentWorkspace,
}
pub(super) fn inspect(input: &WorkspaceInput) -> Result<Context> {
    let path = Path::new(&input.path);
    if !path.is_absolute() {
        return Err("工作目录必须是绝对路径".into());
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("工作目录不可用：{e}"))?;
    if !path.is_dir() {
        return Err("工作目录不是文件夹".into());
    }
    let common = workbench::text_git(
        &path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok();
    let repository = common
        .as_ref()
        .and_then(|p| Path::new(p).canonicalize().ok())
        .unwrap_or(path.clone());
    let display = if repository.file_name().is_some_and(|n| n == ".git") {
        repository.parent().unwrap_or(&repository)
    } else {
        &repository
    };
    let mut observation = AgentWorkspace {
        path: path.display().to_string(),
        branch: None,
        head: None,
        upstream: None,
        change_status: ChangeStatus {
            has_uncommitted_changes: None,
            has_unpushed_commits: None,
        },
        commit_changes: vec![],
        run_files: None,
        changed_files: vec![],
        related_commits: vec![],
        observed_at: now(),
        error: None,
    };
    if common.is_some() {
        match workbench::git(
            &path,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ],
        ) {
            Ok(bytes) => {
                let parsed = workbench::parse_status(&bytes);
                observation.branch = parsed["branch"]
                    .as_str()
                    .filter(|s| !s.is_empty() && *s != "(detached)")
                    .map(str::to_owned);
                observation.head = parsed["head"]
                    .as_str()
                    .filter(|s| !s.starts_with('(') && !s.is_empty())
                    .map(str::to_owned);
                observation.upstream =
                    workbench::text_git(&path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).ok();
                for file in parsed["changes"].as_array().into_iter().flatten() {
                    let status = file["status"].as_str().unwrap_or("??");
                    let kind = if file["conflict"] == true {
                        "conflicted"
                    } else if status == "??" {
                        "untracked"
                    } else if status.contains('R') {
                        "renamed"
                    } else if status.contains('D') {
                        "deleted"
                    } else if status.contains('A') {
                        "added"
                    } else {
                        "modified"
                    };
                    observation.changed_files.push(ChangedFile {
                        path: file["path"].as_str().unwrap_or_default().into(),
                        change_type: kind.into(),
                        staged: kind != "conflicted" && file["staged"].as_bool().unwrap_or(false),
                        original_path: file["original"].as_str().map(str::to_owned),
                    });
                }
                observation.change_status.has_uncommitted_changes =
                    Some(!observation.changed_files.is_empty());
                if let Some(upstream) = &observation.upstream {
                    observation.change_status.has_unpushed_commits = workbench::text_git(
                        &path,
                        &["rev-list", "--count", &format!("{upstream}..HEAD")],
                    )
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|n| n > 0);
                }
            }
            Err(e) => observation.error = Some(e),
        }
    } else {
        observation.error = Some("目录不是可读取的 Git 仓库，代码状态未知".into());
    }
    if input.related_commits.len() > 30 {
        return Err("一次最多登记 30 个关联提交".into());
    }
    for sha in &input.related_commits {
        if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("关联提交必须是 SHA".into());
        }
        let full = workbench::text_git(
            &path,
            &["rev-parse", "--verify", &format!("{sha}^{{commit}}")],
        )
        .map_err(|_| format!("提交 {sha} 在当前仓库中不存在"))?;
        if !observation.related_commits.contains(&full) {
            observation
                .commit_changes
                .push(commit_changes(&path, &full));
            observation.related_commits.push(full);
        }
    }
    Ok(Context {
        key: stable_id(repository.to_string_lossy().as_bytes()),
        name: display
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        repository: repository.display().to_string(),
        observation,
    })
}

pub(super) fn commit_changes(path: &Path, commit: &str) -> CommitChange {
    let result = workbench::git(
        path,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-status",
            "-r",
            "--first-parent",
            "-m",
            "-z",
            "-M",
            commit,
            "--",
        ],
    )
    .and_then(|bytes| parse_commit_files(&bytes));
    match result {
        Ok(files) => CommitChange {
            commit: commit.into(),
            files,
            error: None,
        },
        Err(error) => CommitChange {
            commit: commit.into(),
            files: vec![],
            error: Some(error),
        },
    }
}
fn parse_commit_files(bytes: &[u8]) -> Result<Vec<ChangedFile>> {
    let mut parts = bytes.split(|b| *b == 0).filter(|b| !b.is_empty());
    let mut files = vec![];
    while let Some(status) = parts.next() {
        let first = parts.next().ok_or("提交文件列表不完整")?;
        let rename = matches!(status.first(), Some(b'R' | b'C'));
        let (name, original) = if rename {
            (
                parts.next().ok_or("提交重命名列表不完整")?,
                Some(String::from_utf8_lossy(first).into_owned()),
            )
        } else {
            (first, None)
        };
        let kind = match status.first() {
            Some(b'A') => "added",
            Some(b'D') => "deleted",
            Some(b'R') => "renamed",
            Some(b'C') => "added",
            _ => "modified",
        };
        files.push(ChangedFile {
            path: String::from_utf8_lossy(name).into_owned(),
            change_type: kind.into(),
            staged: false,
            original_path: original,
        });
    }
    Ok(files)
}

/// Agent-owned run scope is distinct from observed status and explicit commit facts.
pub(super) fn validate_run_files(files: &[String]) -> Result<()> {
    use std::{collections::HashSet, path::Component};
    if files.len() > 300 {
        return Err("本次变更列表最多 300 个文件".into());
    }
    let mut used = HashSet::new();
    for file in files {
        super::text(file, "变更文件路径", 1024)?;
        if file.contains('\\')
            || file.contains(':')
            || file.contains('\0')
            || file
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p == ".git")
            || Path::new(file)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("变更文件必须是仓库内的相对文件路径".into());
        }
        if !used.insert(file) {
            return Err("变更文件列表有重复路径".into());
        }
    }
    Ok(())
}
