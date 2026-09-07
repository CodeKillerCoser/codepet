//! Product-neutral, immutable enumeration snapshots shared by provider adapters.
use crate::ProtocolError;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};
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
/// Tokens bind a random snapshot identity and authenticated offset; reader scopes stay private.
/// The binding must include the operation, instance generation, query and scope.
pub struct SnapshotPager<T> { snapshots: Mutex<BTreeMap<String, Snapshot<T>>>, secret: OnceLock<Result<[u8; 24], ProtocolError>> }
impl<T> Default for SnapshotPager<T> {
    fn default() -> Self { Self { snapshots: Mutex::new(BTreeMap::new()), secret: OnceLock::new() } }
}
#[derive(Debug)]
pub struct SnapshotPage<T> {
    pub rows: Vec<T>, pub next_cursor: Option<String>, pub revision: String,
}
impl<T: Clone> SnapshotPager<T> {
    fn key(&self) -> Result<&[u8; 24], ProtocolError> {
        self.secret.get_or_init(|| {
            let mut key = [0; 24];
            SystemRandom::new().fill(&mut key).map_err(|_| query_error("conversation_state_unavailable", "cannot create cursor key"))?;
            Ok(key)
        }).as_ref().map_err(Clone::clone)
    }

    pub fn page(&self, binding: &str, cursor: &str, limit: Option<u64>) -> Result<SnapshotPage<T>, ProtocolError> {
        let (unsigned, signature) = cursor.rsplit_once(':').ok_or_else(|| query_error("invalid_cursor", "invalid enumeration signature"))?;
        let (revision, offset) = unsigned.rsplit_once(':').ok_or_else(|| query_error("invalid_cursor", "invalid enumeration cursor"))?;
        let offset = offset.parse::<usize>().map_err(|_| query_error("invalid_cursor", "invalid enumeration offset"))?;
        let signature = (0..signature.len()).step_by(2).map(|i| signature.get(i..i+2).and_then(|v| u8::from_str_radix(v, 16).ok())).collect::<Option<Vec<_>>>()
            .ok_or_else(|| query_error("invalid_cursor", "invalid enumeration signature"))?;
        hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, self.key()?), unsigned.as_bytes(), &signature)
            .map_err(|_| query_error("invalid_cursor", "enumeration cursor was modified"))?;
        let snapshots = self.snapshots.lock().map_err(|_| query_error("conversation_state_unavailable", "snapshot lock poisoned"))?;
        let snapshot = snapshots.get(revision).ok_or_else(|| query_error("conversation_cursor_expired", "enumeration snapshot expired"))?;
        if snapshot.binding != binding { return Err(query_error("invalid_cursor", "cursor belongs to another query, scope or instance generation")); }
        if snapshot.created.elapsed() >= SNAPSHOT_TTL { return Err(query_error("conversation_cursor_expired", "enumeration snapshot expired")); }
        self.slice(snapshot, revision, offset, limit)
    }
    pub fn start(&self, binding: String, rows: Vec<T>, limit: Option<u64>) -> Result<SnapshotPage<T>, ProtocolError> {
        let mut random = [0_u8; 24];
        SystemRandom::new().fill(&mut random).map_err(|_| query_error("conversation_state_unavailable", "cannot create snapshot token"))?;
        let revision: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let snapshot = Snapshot { binding, rows, created: Instant::now() };
        let page = self.slice(&snapshot, &revision, 0, limit)?;
        let mut snapshots = self.snapshots.lock().map_err(|_| query_error("conversation_state_unavailable", "snapshot lock poisoned"))?;
        snapshots.retain(|_, value| value.created.elapsed() < SNAPSHOT_TTL);
        // Bound retained snapshots, never the number of rows in an enumeration.
        if snapshots.len() >= 64 {
            return Err(query_error("conversation_query_busy", "too many live enumeration snapshots"));
        }
        if page.next_cursor.is_some() { snapshots.insert(revision, snapshot); }
        Ok(page)
    }
    fn slice(&self, snapshot: &Snapshot<T>, revision: &str, offset: usize, limit: Option<u64>) -> Result<SnapshotPage<T>, ProtocolError> {
        let limit = limit.unwrap_or(20);
        if limit == 0 || limit > 100 { return Err(query_error("invalid_request", "enumeration limit must be between 1 and 100")); }
        if offset > snapshot.rows.len() { return Err(query_error("invalid_cursor", "enumeration offset is outside the snapshot")); }
        let end = offset.saturating_add(limit as usize).min(snapshot.rows.len());
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.key()?);
        Ok(SnapshotPage { rows: snapshot.rows[offset..end].to_vec(), next_cursor: (end < snapshot.rows.len()).then(|| {
            let unsigned = format!("{revision}:{end}");
            let signature: String = hmac::sign(&key, unsigned.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect();
            format!("{unsigned}:{signature}")
        }), revision: revision.into() })
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
    fn rejects_tampered_tokens_before_lookup_but_recognizes_expired_signed_tokens() {
        let pager = SnapshotPager::default();
        let page = pager.start("instance/generation/scope".into(), (0..30).collect(), Some(20)).unwrap();
        let cursor = page.next_cursor.unwrap();
        let parts = cursor.split(':').collect::<Vec<_>>();
        for forged in [format!("missing:{}:{}", parts[1], parts[2]), format!("{}:21:{}", parts[0], parts[2]), format!("{}:{}:00", parts[0], parts[1])] {
            assert_eq!(pager.page("instance/generation/scope", &forged, Some(20)).unwrap_err().code, "invalid_cursor");
        }
        pager.snapshots.lock().unwrap().get_mut(parts[0]).unwrap().created = Instant::now() - SNAPSHOT_TTL;
        assert_eq!(pager.page("instance/generation/scope", &cursor, Some(20)).unwrap_err().code, "conversation_cursor_expired");
        pager.snapshots.lock().unwrap().clear();
        assert_eq!(pager.page("instance/generation/scope", &cursor, Some(20)).unwrap_err().code, "conversation_cursor_expired");
    }

    #[test]
    fn rejects_upstream_cursor_cycle() {
        let mut progress = EnumerationProgress::default();
        progress.advance(Some("a".into())).unwrap();
        progress.advance(Some("b".into())).unwrap();
        assert!(progress.advance(Some("a".into())).is_err());
    }
}
