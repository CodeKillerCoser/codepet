//! Read-only discovery evidence. Native objects remain owned by App Server.
use codepet_provider_sdk::ProtocolError;
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug)]
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
#[cfg(test)]
pub(crate) fn read_evidence(home: &Path) -> Result<BTreeMap<String, DbEvidence>, ProtocolError> {
    Ok(read_page(home, &DbQuery::default())?.into_iter().collect())
}

#[derive(Clone, Default)]
pub(crate) struct DbQuery {
    pub after: Option<(u64, String)>,
    pub updated_after: Option<u64>,
    pub ids: Option<Vec<String>>,
    // None = all; Some(None) = standalone.
    pub project: Option<Option<String>>,
    pub assignments: BTreeMap<String, Option<String>>,
    pub limit: Option<usize>,
}

pub(crate) fn read_page(home: &Path, query: &DbQuery) -> Result<Vec<(String, DbEvidence)>, ProtocolError> {
    let files = match std::fs::read_dir(home) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
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
        return Ok(Vec::new());
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
    let native_project = if columns.contains("project_id") { "NULLIF(project_id, '')" } else { "NULL" };
    let mut predicates = Vec::new();
    let mut parameters = Vec::<rusqlite::types::Value>::new();
    if query.limit.is_some() && query.ids.is_none() { predicates.push("archived = 0".to_string()); }
    if let Some(since) = query.updated_after {
        predicates.push(format!("{updated} >= ?"));
        parameters.push((since.min(i64::MAX as u64) as i64).into());
    }
    if let Some((time, id)) = &query.after {
        predicates.push(format!("({updated} <= ? AND ({updated} < ? OR ({updated} = ? AND id > ?)))"));
        parameters.extend([(*time as i64).into(), (*time as i64).into(), (*time as i64).into(), id.clone().into()]);
    }
    if let Some(ids) = &query.ids {
        predicates.push("id IN (SELECT value FROM json_each(?))".into());
        parameters.push(serde_json::to_string(ids).map_err(unavailable)?.into());
    }
    if let Some(project) = &query.project {
        let assignments = serde_json::to_string(&query.assignments).map_err(unavailable)?;
        match project {
            None => {
                predicates.push(format!("({native_project} IS NULL AND NOT EXISTS (SELECT 1 FROM json_each(?) a WHERE a.key = threads.id))"));
                parameters.push(assignments.into());
            }
            Some(project) => {
                predicates.push(format!("({native_project} = ? OR ({native_project} IS NULL AND EXISTS (SELECT 1 FROM json_each(?) a WHERE a.key = threads.id AND a.value = ?)))"));
                parameters.extend([project.clone().into(), assignments.into(), project.clone().into()]);
            }
        }
    }
    let predicate = if predicates.is_empty() { String::new() } else { format!(" WHERE {}", predicates.join(" AND ")) };
    let limit = query.limit.map(|limit| format!(" LIMIT {limit}")).unwrap_or_default();
    let has_time_index = |name: &str| -> bool {
        connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?)", [name], |row| row.get(0)).unwrap_or(false)
    };
    let sql = if query.limit.is_some() && query.ids.is_none() && columns.contains("updated_at_ms") {
        // Keep the common millisecond path indexable. COALESCE in ORDER BY
        // would sort the entire filtered table before applying LIMIT.
        let index = if has_time_index("idx_threads_updated_at_ms") { " INDEXED BY idx_threads_updated_at_ms" } else { "" };
        let condition = |extra: &str| if predicate.is_empty() { format!(" WHERE {extra}") } else { format!("{predicate} AND {extra}") };
        let modern = condition("updated_at_ms IS NOT NULL").replace(updated, "updated_at_ms");
        let legacy = condition("updated_at_ms IS NULL").replace(updated, "updated_at * 1000");
        parameters.extend(parameters.clone());
        format!("SELECT * FROM (SELECT id, archived, updated_at_ms AS time, {history} AS history FROM threads{index}{modern} ORDER BY updated_at_ms DESC, id ASC{limit}) UNION ALL SELECT * FROM (SELECT id, archived, updated_at * 1000 AS time, {history} AS history FROM threads{index}{legacy} ORDER BY updated_at DESC, id ASC{limit}) ORDER BY time DESC, id ASC{limit}")
    } else {
        let index = if query.ids.is_none() && has_time_index("idx_threads_updated_at") { " INDEXED BY idx_threads_updated_at" } else { "" };
        format!("SELECT id, archived, {updated}, {history} FROM threads{index}{predicate} ORDER BY {updated} DESC, id ASC{limit}")
    };
    let mut statement = connection.prepare(&sql).map_err(unavailable)?;
    let entries = statement
        .query_map(rusqlite::params_from_iter(parameters), |row| {
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
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    Ok(entries)
}

/// Restore DB millisecond precision within the same native second, and correct
/// the observed legacy read-at-creation regression. Do not replace unrelated
/// native timestamp changes with older DB observations.
pub(crate) fn repair_time(row: &mut codepet_provider_sdk::Conversation, evidence: &DbEvidence) {
    if row.updated_at.zip(evidence.updated_ms).is_some_and(|(native, db)| native % 1000 == 0 && native / 1000 == db / 1000)
        || (evidence.legacy && row.updated_at == row.created_at && evidence.updated_ms > row.updated_at) {
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
    fn db_precision_preserves_subsecond_filter_boundaries() {
        let mut row: codepet_provider_sdk::Conversation = serde_json::from_value(serde_json::json!({
            "resource":{"providerId":"i","nativeResourceId":"t"},
            "title":"t","status":"idle","createdAt":9000,"updatedAt":10000
        })).unwrap();
        let evidence = DbEvidence { archived: false, updated_ms: Some(10500), legacy: false };
        repair_time(&mut row, &evidence);
        assert_eq!(row.updated_at, Some(10500));
        row.updated_at = Some(11000);
        repair_time(&mut row, &evidence);
        assert_eq!(row.updated_at, Some(11000));
    }

    #[test]
    fn page_filters_and_keysets_include_empty_previews_and_legacy_null_times() {
        let home = tempfile::tempdir().unwrap();
        let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER, updated_at INTEGER, updated_at_ms INTEGER, project_id TEXT, preview TEXT);
            CREATE INDEX idx_threads_updated_at_ms ON threads(updated_at_ms DESC, id DESC);
            INSERT INTO threads VALUES ('a',0,30,30001,NULL,''),('b',0,30,NULL,NULL,''),('c',0,20,20000,'p',''),('d',0,10,10000,NULL,''),('archived',1,90,90000,NULL,'');").unwrap();
        let mut query = DbQuery { limit: Some(1), updated_after: Some(20000), project: Some(None), ..Default::default() };
        let first = read_page(home.path(), &query).unwrap();
        assert_eq!(first[0].0, "a");
        query.after = Some((30001, "a".into()));
        let second = read_page(home.path(), &query).unwrap();
        assert_eq!(second[0].0, "b");
        assert_eq!(second[0].1.updated_ms, Some(30000));
        query.after = Some((30000, "b".into()));
        assert!(read_page(home.path(), &query).unwrap().is_empty());
        query.after = None;
        query.project = Some(Some("p".into()));
        query.assignments.insert("b".into(), Some("p".into()));
        query.limit = Some(20);
        assert_eq!(read_page(home.path(), &query).unwrap().into_iter().map(|(id,_)| id).collect::<Vec<_>>(), ["b", "c"]);
        query.ids = Some(vec!["archived".into()]); query.project = None;
        assert!(read_page(home.path(), &query).unwrap()[0].1.archived);
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
