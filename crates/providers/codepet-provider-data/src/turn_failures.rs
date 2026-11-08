//! Bounded recovery of terminal failures received by the Host. Native histories
//! can omit failed attempts entirely, so these records are independent of items.
use crate::{db_error, ProtocolError, RoutedResourceId, TurnStatus, TurnTask};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

const LIMIT: usize = 100;
type Key = (String, String, String);
#[derive(Default)]
pub struct TurnFailureStore {
    path: Option<PathBuf>,
    memory: Mutex<BTreeMap<Key, TurnTask>>,
}
impl TurnFailureStore {
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }
    fn database(&self) -> Result<rusqlite::Connection, ProtocolError> {
        let db = crate::open(self.path.as_ref().expect("persistent store"))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS turn_failures (
            provider TEXT NOT NULL, conversation TEXT NOT NULL, turn TEXT NOT NULL,
            updated INTEGER NOT NULL, payload TEXT NOT NULL,
            PRIMARY KEY(provider,conversation,turn));
            CREATE INDEX IF NOT EXISTS turn_failures_recent ON turn_failures(provider,conversation,updated);")
            .map_err(db_error)?;
        Ok(db)
    }
    pub fn observe(&self, turn: &TurnTask) -> Result<(), ProtocolError> {
        if turn.status != TurnStatus::Failed {
            return Ok(());
        }
        let resource = &turn.conversation;
        let key = (
            resource.provider_id.clone(),
            resource.native_resource_id.clone(),
            turn.resource.native_resource_id.clone(),
        );
        if self.path.is_some() {
            let mut db = self.database()?;
            let tx = db.transaction().map_err(db_error)?;
            tx.execute("INSERT INTO turn_failures VALUES (?1,?2,?3,?4,?5)
                ON CONFLICT(provider,conversation,turn) DO UPDATE SET updated=excluded.updated,payload=excluded.payload
                WHERE excluded.updated > turn_failures.updated OR
                    (excluded.updated = turn_failures.updated AND (json_extract(excluded.payload,'$.error') IS NOT NULL OR json_extract(turn_failures.payload,'$.error') IS NULL))", rusqlite::params![key.0,key.1,key.2,time(turn),serde_json::to_string(turn).map_err(db_error)?]).map_err(db_error)?;
            tx.execute("DELETE FROM turn_failures WHERE provider=?1 AND conversation=?2 AND turn NOT IN
                (SELECT turn FROM turn_failures WHERE provider=?1 AND conversation=?2 ORDER BY updated DESC,turn DESC LIMIT ?3)", rusqlite::params![key.0,key.1,LIMIT as i64]).map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        } else {
            let mut rows = self.memory.lock().map_err(db_error)?;
            if rows.get(&key).is_none_or(|old| {
                time(turn) > time(old)
                    || (time(turn) == time(old) && (turn.error.is_some() || old.error.is_none()))
            }) {
                rows.insert(key, turn.clone());
            }
            let mut same = rows
                .iter()
                .filter(|((p, c, _), _)| {
                    p == &resource.provider_id && c == &resource.native_resource_id
                })
                .map(|(k, v)| (time(v), k.clone()))
                .collect::<Vec<_>>();
            same.sort();
            let excess = same.len().saturating_sub(LIMIT);
            for (_, key) in same.into_iter().take(excess) {
                rows.remove(&key);
            }
        }
        Ok(())
    }
    pub fn remove(&self, conversation: &RoutedResourceId) -> Result<(), ProtocolError> {
        if self.path.is_some() {
            self.database()?
                .execute(
                    "DELETE FROM turn_failures WHERE provider=?1 AND conversation=?2",
                    rusqlite::params![conversation.provider_id, conversation.native_resource_id],
                )
                .map_err(db_error)?;
        } else {
            self.memory
                .lock()
                .map_err(db_error)?
                .retain(|(p, c, _), _| {
                    p != &conversation.provider_id || c != &conversation.native_resource_id
                });
        }
        Ok(())
    }
    pub fn recent(&self, conversation: &RoutedResourceId) -> Result<Vec<TurnTask>, ProtocolError> {
        let mut turns = if self.path.is_some() {
            let db = self.database()?;
            let mut stmt = db.prepare("SELECT payload FROM turn_failures WHERE provider=?1 AND conversation=?2 ORDER BY updated,turn LIMIT ?3").map_err(db_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![
                        conversation.provider_id,
                        conversation.native_resource_id,
                        LIMIT as i64
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(db_error)?;
            rows.map(|row| serde_json::from_str(&row.map_err(db_error)?).map_err(db_error))
                .collect::<Result<Vec<TurnTask>, ProtocolError>>()?
        } else {
            self.memory
                .lock()
                .map_err(db_error)?
                .values()
                .filter(|turn| turn.conversation == *conversation)
                .cloned()
                .collect()
        };
        turns.sort_by(|a, b| {
            time(a).cmp(&time(b)).then(
                a.resource
                    .native_resource_id
                    .cmp(&b.resource.native_resource_id),
            )
        });
        Ok(turns)
    }
}
fn time(turn: &TurnTask) -> u64 {
    turn.completed_at
        .or(turn.updated_at)
        .or(turn.started_at)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn turn(id: &str, t: u64) -> TurnTask {
        serde_json::from_value(serde_json::json!({
        "resource":{"providerId":"p","nativeResourceId":id},"conversation":{"providerId":"p","nativeResourceId":"c"},
        "status":"failed","completedAt":t,"error":{"code":"quota_exceeded","message":"Limit reached","retryable":false}
    })).unwrap()
    }
    #[test]
    fn persists_bounded_deduplicated_failures_and_isolates_routes() {
        let path =
            std::env::temp_dir().join(format!("turn-failures-{}.sqlite", uuid::Uuid::new_v4()));
        let store = TurnFailureStore::at(path.clone());
        for n in 0..105 {
            store.observe(&turn(&format!("t{n}"), n)).unwrap();
        }
        let latest = turn("t104", 104);
        store.observe(&latest).unwrap();
        let mut stale = latest.clone();
        stale.completed_at = Some(104);
        stale.error = None;
        store.observe(&stale).unwrap();
        let mut running = latest.clone();
        running.status = TurnStatus::Running;
        store.observe(&running).unwrap();
        drop(store);
        let reopened = TurnFailureStore::at(path.clone());
        let rows = reopened.recent(&latest.conversation).unwrap();
        assert_eq!(rows.len(), 100);
        assert_eq!(rows.last(), Some(&latest));
        assert!(reopened
            .recent(&RoutedResourceId {
                provider_id: "other".into(),
                native_resource_id: "c".into()
            })
            .unwrap()
            .is_empty());
        reopened.remove(&latest.conversation).unwrap();
        assert!(reopened.recent(&latest.conversation).unwrap().is_empty());
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
