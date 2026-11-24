use super::*;

pub(super) fn prepare_conversation_workspace(
    workspace_root: Option<&str>,
    workspace_mode: Option<&str>,
) -> Result<Option<String>, ProtocolError> {
    let mode = workspace_mode.unwrap_or("main");
    if mode == "main" {
        return workspace_root
            .map(ensure_conversation_workspace)
            .transpose();
    }
    if mode != "worktree" {
        return Err(protocol_error(
            "unsupported_workspace_mode",
            format!("Codex Provider does not support workspace mode {mode}"),
            false,
        ));
    }
    let requested = workspace_root.ok_or_else(|| {
        protocol_error(
            "workspace_required",
            "Codex worktree mode requires workspaceRoot".to_string(),
            false,
        )
    })?;
    let requested = Path::new(requested);
    if !requested.is_absolute() || !requested.is_dir() {
        return Err(protocol_error(
            "invalid_workspace_root",
            "Codex worktree workspaceRoot must be an existing absolute directory".to_string(),
            false,
        ));
    }
    let workspace_root = codepet_paths::remote_workspace_for_user("codex").ok_or_else(|| {
        protocol_error(
            "worktree_create_failed",
            "cannot resolve CodePet workspace for managed worktrees".to_string(),
            false,
        )
    })?;
    create_managed_worktree(requested, Path::new(&workspace_root)).map(Some)
}

pub(super) fn codex_home() -> Option<PathBuf> {
    local_runtime::data_dir("CODEX_HOME", ".codex")
}

fn ensure_conversation_workspace(path: &str) -> Result<String, ProtocolError> {
    let workspace = Path::new(path);
    if !workspace.is_absolute() {
        return Err(protocol_error(
            "invalid_workspace_root",
            "Codex workspaceRoot must be an absolute path".to_string(),
            false,
        ));
    }
    std::fs::create_dir_all(workspace).map_err(|error| {
        protocol_error(
            "workspace_create_failed",
            format!("failed to create Codex workspaceRoot: {error}"),
            false,
        )
    })?;
    Ok(path.to_string())
}

fn create_managed_worktree(
    requested: &Path,
    workspace_root: &Path,
) -> Result<String, ProtocolError> {
    let requested = requested.canonicalize().map_err(|error| {
        protocol_error(
            "invalid_workspace_root",
            format!("failed to resolve workspaceRoot: {error}"),
            false,
        )
    })?;
    let output = codepet_provider_sdk::local_runtime::command("git")
        .arg("-C")
        .arg(&requested)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to inspect Git workspace: {error}"),
                true,
            )
        })?;
    if !output.status.success() {
        return Err(protocol_error(
            "workspace_not_git_repository",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
            false,
        ));
    }
    let repository_root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        .canonicalize()
        .map_err(|error| {
            protocol_error(
                "invalid_workspace_root",
                format!("failed to resolve Git repository root: {error}"),
                false,
            )
        })?;
    let relative_cwd = requested.strip_prefix(&repository_root).map_err(|_| {
        protocol_error(
            "invalid_workspace_root",
            "workspaceRoot is outside the discovered Git repository".to_string(),
            false,
        )
    })?;
    let unique = format!(
        "remote-{}-{}-{}",
        now_ms(),
        std::process::id(),
        NEXT_MANAGED_WORKTREE.fetch_add(1, Ordering::SeqCst),
    );
    let worktree_root = workspace_root.join("worktree").join(unique);
    if let Some(parent) = worktree_root.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to prepare managed worktree directory: {error}"),
                true,
            )
        })?;
    }
    let output = codepet_provider_sdk::local_runtime::command("git")
        .arg("-C")
        .arg(&repository_root)
        .args(["worktree", "add", "--detach"])
        .arg(&worktree_root)
        .arg("HEAD")
        .output()
        .map_err(|error| {
            protocol_error(
                "worktree_create_failed",
                format!("failed to start git worktree: {error}"),
                true,
            )
        })?;
    if !output.status.success() {
        return Err(protocol_error(
            "worktree_create_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
            false,
        ));
    }
    let worktree_cwd = worktree_root.join(relative_cwd);
    std::fs::create_dir_all(&worktree_cwd).map_err(|error| {
        protocol_error(
            "worktree_create_failed",
            format!("failed to prepare worktree working directory: {error}"),
            true,
        )
    })?;
    Ok(worktree_cwd.to_string_lossy().into_owned())
}

#[cfg(test)]
mod workspace_mode_tests {
    use super::{
        create_managed_worktree, ensure_conversation_workspace, prepare_conversation_workspace,
    };
    use std::fs;
    use std::path::Path;

    #[test]
    fn creates_a_missing_standalone_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("codex/task/task-1");

        let prepared = ensure_conversation_workspace(workspace.to_str().unwrap()).unwrap();

        assert_eq!(Path::new(&prepared), workspace);
        assert!(workspace.is_dir());
    }

    #[test]
    fn main_workspace_keeps_the_requested_project_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().to_str().unwrap();
        assert_eq!(
            prepare_conversation_workspace(Some(root), Some("main")).unwrap(),
            Some(root.to_string())
        );
    }

    #[test]
    fn creates_a_detached_managed_worktree_and_preserves_relative_cwd() {
        let fixture = tempfile::tempdir().unwrap();
        let repository = fixture.path().join("project");
        let nested = repository.join("packages/app");
        fs::create_dir_all(&nested).unwrap();
        git(&repository, &["init"]);
        git(&repository, &["config", "user.email", "test@codepet.dev"]);
        git(&repository, &["config", "user.name", "CodePet Test"]);
        fs::write(repository.join("README.md"), "fixture").unwrap();
        git(&repository, &["add", "README.md"]);
        git(&repository, &["commit", "-m", "fixture"]);

        let workspace_root = fixture.path().join(".codepet/remote_workspace/codex");
        let cwd = create_managed_worktree(&nested, &workspace_root).unwrap();
        let cwd = Path::new(&cwd);

        assert!(cwd.is_dir());
        assert!(cwd.starts_with(workspace_root.join("worktree")));
        let relative = cwd.strip_prefix(workspace_root.join("worktree")).unwrap();
        assert_eq!(relative.components().count(), 3);
        assert!(cwd.join("../../README.md").is_file());
        assert_eq!(
            cwd.file_name().and_then(|value| value.to_str()),
            Some("app")
        );
        let output = codepet_provider_sdk::local_runtime::command("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
        let detached = codepet_provider_sdk::local_runtime::command("git")
            .arg("-C")
            .arg(cwd)
            .args(["symbolic-ref", "-q", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(detached.status.code(), Some(1));
        let second = create_managed_worktree(&nested, &workspace_root).unwrap();
        assert_ne!(Path::new(&second), cwd);
    }

    fn git(repository: &Path, args: &[&str]) {
        let status = codepet_provider_sdk::local_runtime::command("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
}
