//! Read-only discovery evidence. Native objects remain owned by App Server.
use codepet_provider_sdk::ProtocolError;
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct DbEvidence {
    pub archived: bool,
    pub updated_ms: Option<u64>,
    pub legacy: bool,
}

fn unavailable(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError {
        code: "conversation_query_incomplete".into(),
        message: format!("Codex state DB discovery failed: {error}"),
        retryable: true,
        details: None,
    }
}

/// A single SELECT sees committed WAL data. The connection is dropped before RPCs.
/// No immutable URI, migrations, checkpoints, or writes to the native database.
pub(crate) fn read_evidence(home: &Path) -> Result<BTreeMap<String, DbEvidence>, ProtocolError> {
    let files = match std::fs::read_dir(home) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(unavailable(error)),
    };
    let mut databases = Vec::new();
    for entry in files {
        let entry = entry.map_err(unavailable)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(version) = name
            .strip_prefix("state_")
            .and_then(|name| name.strip_suffix(".sqlite"))
            .and_then(|version| version.parse::<u32>().ok())
        {
            databases.push((version, entry.path()));
        }
    }
    // Never fall back to an older database after a new schema fails validation.
    let Some((_, path)) = databases.into_iter().max_by_key(|(version, _)| *version) else {
        return Ok(BTreeMap::new());
    };
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(unavailable)?;
    connection
        .busy_timeout(Duration::from_millis(500))
        .map_err(unavailable)?;
    let columns = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(unavailable)?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(unavailable)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(unavailable)?;
    for required in ["id", "archived", "updated_at"] {
        if !columns.contains(required) {
            return Err(unavailable(format!(
                "unsupported threads schema: missing {required}"
            )));
        }
    }
    let updated = if columns.contains("updated_at_ms") {
        "COALESCE(updated_at_ms, updated_at * 1000)"
    } else {
        "updated_at * 1000"
    };
    let history = if columns.contains("history_mode") {
        "history_mode"
    } else {
        "'legacy'"
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, archived, {updated}, {history} FROM threads ORDER BY id"
        ))
        .map_err(unavailable)?;
    let entries = statement
        .query_map([], |row| {
            let updated: Option<i64> = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                DbEvidence {
                    archived: row.get::<_, bool>(1)?,
                    updated_ms: updated.and_then(|value| u64::try_from(value).ok()),
                    legacy: row.get::<_, Option<String>>(3)?.as_deref() == Some("legacy"),
                },
            ))
        })
        .map_err(unavailable)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(unavailable)?;
    Ok(entries)
}

/// Correct only the observed legacy read-at-creation regression, not arbitrary
/// timestamp corrections. List metadata and paginated history keep their times.
pub(crate) fn repair_time(row: &mut codepet_provider_sdk::Conversation, evidence: &DbEvidence) {
    if evidence.legacy && row.updated_at == row.created_at && evidence.updated_ms > row.updated_at {
        row.updated_at = evidence.updated_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_committed_wal_without_preview_filter_and_does_not_create_database() {
        let home = tempfile::tempdir().unwrap();
        assert!(read_evidence(home.path()).unwrap().is_empty());
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
        let writer = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE threads(id TEXT, archived INTEGER, updated_at INTEGER, updated_at_ms INTEGER, history_mode TEXT, preview TEXT);
            INSERT INTO threads VALUES ('omitted',0,123,123456,'legacy',''),('archived',1,124,NULL,'paginated','');").unwrap();
        let rows = read_evidence(home.path()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows["omitted"].updated_ms, Some(123456));
        assert!(rows["omitted"].legacy);
        assert!(rows["archived"].archived);
        writer
            .execute_batch(
                "BEGIN IMMEDIATE; UPDATE threads SET updated_at_ms=999 WHERE id='omitted';",
            )
            .unwrap();
        assert_eq!(
            read_evidence(home.path()).unwrap()["omitted"].updated_ms,
            Some(123456)
        );
        writer.execute_batch("COMMIT;").unwrap();
        assert_eq!(
            read_evidence(home.path()).unwrap()["omitted"].updated_ms,
            Some(999)
        );
    }

    #[test]
    fn incompatible_newest_database_does_not_silently_fall_back() {
        let home = tempfile::tempdir().unwrap();
        Connection::open(home.path().join("state_6.sqlite")).unwrap();
        assert_eq!(
            read_evidence(home.path()).unwrap_err().code,
            "conversation_query_incomplete"
        );
    }

    #[test]
    fn older_schema_without_millisecond_or_history_columns_is_supported() {
        let home = tempfile::tempdir().unwrap();
        let writer = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        writer
            .execute_batch(
                "CREATE TABLE threads(id TEXT, archived INTEGER, updated_at INTEGER);
            INSERT INTO threads VALUES ('legacy',0,123);",
            )
            .unwrap();
        let evidence = read_evidence(home.path()).unwrap();
        assert_eq!(evidence["legacy"].updated_ms, Some(123000));
        assert!(evidence["legacy"].legacy);
        writer.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        assert_eq!(
            read_evidence(home.path()).unwrap_err().code,
            "conversation_query_incomplete"
        );
        writer.execute_batch("ROLLBACK;").unwrap();
        assert_eq!(read_evidence(home.path()).unwrap().len(), 1);
    }
}
