//! Product-neutral, immutable enumeration snapshots shared by provider adapters.
use crate::ProtocolError;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const INTERNAL_PAGE_SIZE: u64 = 100;
const SNAPSHOT_TTL: Duration = Duration::from_secs(120);

pub fn query_error(code: &str, message: impl Into<String>) -> ProtocolError {
    ProtocolError { code: code.into(), message: message.into(), retryable: false, details: None }
}

/// Detect upstream cursor cycles instead of silently returning a partial set.
#[derive(Default)]
pub struct EnumerationProgress { seen: BTreeSet<String> }
impl EnumerationProgress {
    pub fn advance(&mut self, cursor: Option<String>) -> Result<Option<String>, ProtocolError> {
        if let Some(value) = &cursor {
            if value.is_empty() || !self.seen.insert(value.clone()) {
                return Err(query_error("conversation_query_incomplete", "upstream pagination did not advance"));
            }
        }
        Ok(cursor)
    }
}

struct Snapshot<T> { binding: String, rows: Vec<T>, created: Instant }
/// Tokens are random lookup keys; neither offsets nor reader scopes are exposed.
/// The binding must include the operation, instance generation, query and scope.
pub struct SnapshotPager<T> { snapshots: Mutex<BTreeMap<String, Snapshot<T>>> }
impl<T> Default for SnapshotPager<T> {
    fn default() -> Self { Self { snapshots: Mutex::new(BTreeMap::new()) } }
}
#[derive(Debug)]
pub struct SnapshotPage<T> {
    pub rows: Vec<T>, pub next_cursor: Option<String>, pub revision: String,
}
impl<T: Clone> SnapshotPager<T> {
    pub fn page(&self, binding: &str, cursor: &str, limit: Option<u64>) -> Result<SnapshotPage<T>, ProtocolError> {
        let (revision, offset) = cursor.rsplit_once(':').ok_or_else(|| query_error("invalid_cursor", "invalid enumeration cursor"))?;
        let offset = offset.parse::<usize>().map_err(|_| query_error("invalid_cursor", "invalid enumeration offset"))?;
        let snapshots = self.snapshots.lock().map_err(|_| query_error("conversation_state_unavailable", "snapshot lock poisoned"))?;
        let snapshot = snapshots.get(revision).ok_or_else(|| query_error("conversation_cursor_expired", "enumeration snapshot expired"))?;
        if snapshot.binding != binding { return Err(query_error("invalid_cursor", "cursor belongs to another query, scope or instance generation")); }
        if snapshot.created.elapsed() >= SNAPSHOT_TTL { return Err(query_error("conversation_cursor_expired", "enumeration snapshot expired")); }
        Self::slice(snapshot, revision, offset, limit)
    }
    pub fn start(&self, binding: String, rows: Vec<T>, limit: Option<u64>) -> Result<SnapshotPage<T>, ProtocolError> {
        let mut random = [0_u8; 24];
        SystemRandom::new().fill(&mut random).map_err(|_| query_error("conversation_state_unavailable", "cannot create snapshot token"))?;
        let revision: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let snapshot = Snapshot { binding, rows, created: Instant::now() };
        let page = Self::slice(&snapshot, &revision, 0, limit)?;
        let mut snapshots = self.snapshots.lock().map_err(|_| query_error("conversation_state_unavailable", "snapshot lock poisoned"))?;
        snapshots.retain(|_, value| value.created.elapsed() < SNAPSHOT_TTL);
        // Bound retained snapshots, never the number of rows in an enumeration.
        if snapshots.len() >= 64 {
            return Err(query_error("conversation_query_busy", "too many live enumeration snapshots"));
        }
        if page.next_cursor.is_some() { snapshots.insert(revision, snapshot); }
        Ok(page)
    }
    fn slice(snapshot: &Snapshot<T>, revision: &str, offset: usize, limit: Option<u64>) -> Result<SnapshotPage<T>, ProtocolError> {
        let limit = limit.unwrap_or(20);
        if limit == 0 || limit > 100 { return Err(query_error("invalid_request", "enumeration limit must be between 1 and 100")); }
        if offset > snapshot.rows.len() { return Err(query_error("invalid_cursor", "enumeration offset is outside the snapshot")); }
        let end = offset.saturating_add(limit as usize).min(snapshot.rows.len());
        Ok(SnapshotPage { rows: snapshot.rows[offset..end].to_vec(), next_cursor: (end < snapshot.rows.len()).then(|| format!("{revision}:{end}")), revision: revision.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enumerates_more_than_one_hundred_without_truncation_and_isolates_scope() {
        let pager = SnapshotPager::default();
        let first = pager.start("instance/generation/mobile".into(), (0..237).collect(), Some(100)).unwrap();
        let cursor = first.next_cursor.unwrap();
        assert_eq!(pager.page("instance/generation/desktop", &cursor, Some(100)).unwrap_err().code, "invalid_cursor");
        let second = pager.page("instance/generation/mobile", &cursor, Some(100)).unwrap();
        let third = pager.page("instance/generation/mobile", &second.next_cursor.unwrap(), Some(100)).unwrap();
        assert_eq!(first.revision, second.revision);
        assert_eq!(second.revision, third.revision);
        assert_eq!(third.rows, (200..237).collect::<Vec<_>>());
        assert!(third.next_cursor.is_none());
    }
    #[test]
    fn rejects_upstream_cursor_cycle() {
        let mut progress = EnumerationProgress::default();
        progress.advance(Some("a".into())).unwrap();
        progress.advance(Some("b".into())).unwrap();
        assert!(progress.advance(Some("a".into())).is_err());
    }
}
