//! Current repository facts, never inferred historical episode outcomes.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::Path,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFacts {
    pub workspace: String,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub main_workspace: Option<String>,
    pub work_mode: String,
    pub commit_state: String,
    pub sync_state: String,
    pub diagnostic: Option<String>,
}
fn run(directory: &Path, args: &[&str]) -> Result<String> {
    let mut command = codepet_provider_sdk::local_runtime::command("git");
    command
        .arg("--no-optional-locks")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("Git stdout unavailable")?;
    let output = thread::spawn(move || {
        let mut data = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Git inspection timed out".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let data = output
        .join()
        .map_err(|_| "Git reader failed")?
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Git could not verify this fact".into());
    }
    if data.len() > 1024 * 1024 {
        return Err("Git output exceeds inspection limit".into());
    }
    String::from_utf8(data)
        .map(|s| s.trim_end().to_owned())
        .map_err(|e| e.to_string())
}
pub fn inspect(workspace: &str) -> WorkspaceFacts {
    let mut facts = WorkspaceFacts {
        workspace: workspace.into(),
        repository: None,
        branch: None,
        head: None,
        main_workspace: None,
        work_mode: "unknown".into(),
        commit_state: "unknown".into(),
        sync_state: "unknown".into(),
        diagnostic: None,
    };
    let path = Path::new(workspace);
    if !path.is_absolute() || !path.is_dir() {
        facts.diagnostic = Some("工作区不存在或路径无效".into());
        return facts;
    }
    let result = (|| -> Result<()> {
        let repository = run(path, &["rev-parse", "--show-toplevel"])?;
        let head = run(path, &["rev-parse", "--verify", "HEAD"])?;
        facts.repository = Some(repository.clone());
        facts.head = Some(head.clone());
        facts.branch = run(path, &["symbolic-ref", "--short", "HEAD"]).ok();
        let status = run(
            path,
            &["status", "--porcelain=v1", "--untracked-files=normal"],
        )?;
        facts.commit_state = if status.is_empty() {
            "clean"
        } else {
            "uncommitted"
        }
        .into();
        let list = run(path, &["worktree", "list", "--porcelain"])?;
        let main = list
            .lines()
            .find_map(|line| line.strip_prefix("worktree "))
            .ok_or("Main workspace unavailable")?;
        facts.main_workspace = Some(main.into());
        let same = Path::new(main).canonicalize().map_err(|e| e.to_string())?
            == Path::new(&repository)
                .canonicalize()
                .map_err(|e| e.to_string())?;
        facts.work_mode = if same { "main" } else { "worktree" }.into();
        if same {
            facts.sync_state = "notRequired".into();
            return Ok(());
        }
        let main_head = run(Path::new(main), &["rev-parse", "--verify", "HEAD"])?;
        // Only ancestry is proven. Squash/cherry-pick equivalence is intentionally unknown.
        facts.sync_state =
            if run(path, &["merge-base", "--is-ancestor", &head, &main_head]).is_ok()
                && status.is_empty()
            {
                "synced"
            } else {
                "unknown"
            }
            .into();
        Ok(())
    })();
    if let Err(error) = result {
        facts.diagnostic = Some(error);
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_worktree_ancestry_and_dirty_state() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("main");
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init"]).unwrap();
        run(&root, &["config", "user.email", "fixture@example.invalid"]).unwrap();
        run(&root, &["config", "user.name", "Fixture"]).unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        run(&root, &["add", "a.txt"]).unwrap();
        run(&root, &["commit", "-m", "initial"]).unwrap();
        let clean = inspect(root.to_str().unwrap());
        assert_eq!(clean.commit_state, "clean");
        assert_eq!(clean.work_mode, "main");
        let derived = temp.path().join("derived");
        run(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "derived",
                derived.to_str().unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(inspect(derived.to_str().unwrap()).sync_state, "synced");
        std::fs::write(derived.join("b.txt"), "b").unwrap();
        assert_eq!(
            inspect(derived.to_str().unwrap()).commit_state,
            "uncommitted"
        );
        run(&derived, &["add", "b.txt"]).unwrap();
        run(&derived, &["commit", "-m", "derived"]).unwrap();
        assert_eq!(inspect(derived.to_str().unwrap()).sync_state, "unknown");
        run(&root, &["merge", "--ff-only", "derived"]).unwrap();
        assert_eq!(inspect(derived.to_str().unwrap()).sync_state, "synced");
    }
}
