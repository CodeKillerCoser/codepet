//! Optional source verification is a single read-only lookup, never a history scan.
use crate::{workbench, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{path::Path, time::Duration};

pub(super) fn verify(home: &Path, thread: &str, repository: &str) -> Result<bool> {
    let lookup = || -> rusqlite::Result<Option<String>> {
        let connection = Connection::open_with_flags(
            home.join("state_5.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(100))?;
        connection
            .query_row("SELECT cwd FROM threads WHERE id=?1", [thread], |r| {
                r.get(0)
            })
            .optional()
    };
    let Ok(Some(cwd)) = lookup() else {
        return Ok(false);
    };
    let Ok(root) = Path::new(&cwd).canonicalize() else {
        return Ok(false);
    };
    let common = workbench::text_git(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()
    .and_then(|s| Path::new(&s).canonicalize().ok())
    .unwrap_or(root);
    if common != Path::new(repository) {
        return Err("对话属于其他项目，不能关联到当前任务".into());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_sources_stay_pending_and_wrong_projects_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        assert!(!verify(
            dir.path(),
            "chat",
            repo.path().canonicalize().unwrap().to_str().unwrap()
        )
        .unwrap());
        let db = Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT, cwd TEXT);")
            .unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1,?2)",
            [
                "chat",
                repo.path().canonicalize().unwrap().to_str().unwrap(),
            ],
        )
        .unwrap();
        assert!(verify(
            dir.path(),
            "chat",
            repo.path().canonicalize().unwrap().to_str().unwrap()
        )
        .unwrap());
        assert!(!verify(
            dir.path(),
            "missing",
            repo.path().canonicalize().unwrap().to_str().unwrap()
        )
        .unwrap());
        assert!(verify(dir.path(), "chat", dir.path().to_str().unwrap())
            .unwrap_err()
            .contains("其他项目"));
    }
}
