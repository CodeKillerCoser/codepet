//! Optional source verification checks existence only; conversations may span projects.
use crate::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{path::Path, time::Duration};

pub(super) fn verify(home: &Path, thread: &str) -> Result<bool> {
    let lookup = || -> rusqlite::Result<Option<String>> {
        let connection = Connection::open_with_flags(
            home.join("state_5.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(100))?;
        connection
            .query_row("SELECT id FROM threads WHERE id=?1", [thread], |r| r.get(0))
            .optional()
    };
    Ok(matches!(lookup(), Ok(Some(_))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_conversations_are_verified_regardless_of_project_or_deleted_workspace() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!verify(dir.path(), "chat").unwrap());
        let db = Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT, cwd TEXT);")
            .unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1,?2)",
            ["chat", "/another/deleted/project"],
        )
        .unwrap();
        assert!(verify(dir.path(), "chat").unwrap());
        assert!(!verify(dir.path(), "missing").unwrap());
    }
}
