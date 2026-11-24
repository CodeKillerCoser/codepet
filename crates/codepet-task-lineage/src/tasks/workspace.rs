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
