//! Rebuildable, caller-scoped summary snapshots. No transcript or read authority lives here.
use codepet_gateway_sdk as gateway;
use ring::hmac;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) const RECENT_WINDOW_MS: u64 = 14 * 24 * 60 * 60 * 1_000;
const SNAPSHOT_TTL_MS: u64 = 5 * 60 * 1_000;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ViewKey {
    pub provider_id: String,
    pub reader_scope: String,
    pub generation: u64,
}

pub(crate) type Identity = (String, String);

pub(crate) fn identity(conversation: &gateway::Conversation) -> Identity {
    resource_identity(&conversation.resource)
}

pub(crate) fn resource_identity(resource: &gateway::RoutedResourceId) -> Identity {
    (
        resource.provider_id.clone(),
        resource.native_resource_id.clone(),
    )
}

/// All three sources must have finished successfully before calling this function.
pub(crate) fn aggregate(
    summaries: BTreeMap<Identity, gateway::Conversation>,
    active: &BTreeSet<Identity>,
    unread: &BTreeMap<Identity, gateway::ConversationReadState>,
    now: u64,
) -> (Vec<gateway::Conversation>, Option<u64>) {
    let cutoff = now.saturating_sub(RECENT_WINDOW_MS);
    let mut expires_at = None;
    let mut rows = Vec::new();
    for (id, mut summary) in summaries {
        let is_active = active.contains(&id);
        let is_unread = unread.contains_key(&id);
        let is_recent = summary.updated_at.is_some_and(|updated| updated >= cutoff);
        if !is_active && !is_unread && !is_recent {
            continue;
        }
        if !is_active && !is_unread {
            // The boundary is inclusive: remove at updatedAt + 14 days + 1 ms.
            if let Some(expiry) = summary
                .updated_at
                .and_then(|t| t.checked_add(RECENT_WINDOW_MS + 1))
            {
                expires_at = Some(expires_at.map_or(expiry, |old: u64| old.min(expiry)));
            }
        }
        if let Some(read_state) = unread.get(&id) {
            summary.read_state = Some(read_state.clone());
        }
        let group = if is_active {
            0
        } else if is_unread {
            1
        } else {
            2
        };
        rows.push((group, id, summary));
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.2.updated_at.cmp(&a.2.updated_at))
            .then_with(|| a.1.cmp(&b.1))
    });
    (
        rows.into_iter().map(|(_, _, row)| row).collect(),
        expires_at,
    )
}

#[derive(Clone)]
pub(crate) struct RecentTail {
    pub cursor: Option<String>,
    pub cutoff: u64,
    pub excluded: BTreeSet<Identity>,
    pub seen_cursors: BTreeSet<String>,
}

struct Snapshot {
    tail: Option<RecentTail>,
    rows: Vec<gateway::Conversation>,
    revision: String,
    fence: gateway::EventCursor,
    expires_at: u64,
    nonce: String,
}

#[derive(Default)]
struct ProviderVersion {
    epoch: u64,
    revision: u64,
    pending: bool,
    boundary_at: Option<u64>,
}

pub(crate) struct RecentSnapshots {
    versions: BTreeMap<String, ProviderVersion>,
    snapshots: BTreeMap<ViewKey, Snapshot>,
    cursor_key: hmac::Key,
}

impl Default for RecentSnapshots {
    fn default() -> Self {
        let secret = [*Uuid::new_v4().as_bytes(), *Uuid::new_v4().as_bytes()].concat();
        Self {
            versions: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            cursor_key: hmac::Key::new(hmac::HMAC_SHA256, &secret),
        }
    }
}

pub(crate) struct Page {
    pub conversations: Vec<gateway::Conversation>,
    pub next_cursor: Option<String>,
    pub revision: String,
    pub snapshot_cursor: gateway::EventCursor,
}

impl RecentSnapshots {
    pub(crate) fn track(&mut self, provider_id: &str) {
        self.versions.entry(provider_id.to_owned()).or_default();
    }

    pub(crate) fn is_tracked(&self, provider_id: &str) -> bool {
        self.versions.contains_key(provider_id)
    }

    pub(crate) fn epoch(&self, provider_id: &str) -> u64 {
        self.versions.get(provider_id).map_or(0, |v| v.epoch)
    }

    pub(crate) fn invalidate(&mut self, provider_id: &str) {
        let version = self.versions.entry(provider_id.to_owned()).or_default();
        version.epoch = version.epoch.saturating_add(1);
        version.revision = version.revision.saturating_add(1);
        version.pending = true;
        version.boundary_at = None;
        self.snapshots
            .retain(|key, _| key.provider_id != provider_id);
    }

    /// Broad hints carry no reader scope, IDs, or read state; safe for the shared replay bus.
    pub(crate) fn take_invalidations(&mut self, now: u64) -> Vec<(String, String)> {
        // Keep the time fence after cursor TTL eviction: a connected Remote can keep
        // displaying its first page longer than the Host retains its cursor snapshot.
        let expired: Vec<_> = self
            .versions
            .iter()
            .filter(|(_, version)| version.boundary_at.is_some_and(|expiry| now >= expiry))
            .map(|(provider, _)| provider.clone())
            .collect();
        for provider in expired {
            self.invalidate(&provider);
        }
        self.snapshots.retain(|_, s| now < s.expires_at);
        self.versions
            .iter_mut()
            .filter_map(|(provider, version)| {
                if !version.pending {
                    return None;
                }
                version.pending = false;
                Some((provider.clone(), revision(version.revision)))
            })
            .collect()
    }

    pub(crate) fn install(
        &mut self,
        key: ViewKey,
        expected_epoch: u64,
        rows: Vec<gateway::Conversation>,
        boundary_at: Option<u64>,
        fence: gateway::EventCursor,
        now: u64,
    ) -> Result<(), gateway::ProtocolError> {
        if self.epoch(&key.provider_id) != expected_epoch
            || boundary_at.is_some_and(|expiry| now >= expiry)
        {
            return Err(error(
                "recent_snapshot_changed",
                "Recent candidates changed during collection",
                true,
            ));
        }
        let version = self.versions.entry(key.provider_id.clone()).or_default();
        if let Some(boundary) = boundary_at {
            version.boundary_at = Some(
                version
                    .boundary_at
                    .map_or(boundary, |old| old.min(boundary)),
            );
        }
        self.snapshots.insert(
            key,
            Snapshot {
                tail: None,
                rows,
                revision: revision(version.revision),
                fence,
                expires_at: now.saturating_add(SNAPSHOT_TTL_MS),
                nonce: Uuid::new_v4().to_string(),
            },
        );
        Ok(())
    }

    pub(crate) fn set_tail(&mut self, key: &ViewKey, tail: Option<RecentTail>) {
        if let Some(snapshot) = self.snapshots.get_mut(key) { snapshot.tail = tail; }
    }

    pub(crate) fn pending_tail(&self, key: &ViewKey, cursor: Option<&str>, limit: Option<u64>) -> Result<Option<(RecentTail, usize)>, gateway::ProtocolError> {
        let Some(snapshot) = self.snapshots.get(key) else { return Ok(None); };
        let offset = match cursor {
            Some(cursor) => {
                let (nonce, offset, _) = decode_cursor(&self.cursor_key, key, cursor)?;
                if nonce != snapshot.nonce { return Err(expired_cursor()); }
                offset
            }
            None => 0,
        };
        Ok(snapshot.tail.clone().map(|tail| (tail, offset.saturating_add(limit.unwrap_or(20) as usize).saturating_sub(snapshot.rows.len()))))
    }

    pub(crate) fn append(&mut self, key: &ViewKey, epoch: u64, rows: Vec<gateway::Conversation>, tail: Option<RecentTail>, now: u64) -> Result<(), gateway::ProtocolError> {
        if self.epoch(&key.provider_id) != epoch { return Err(expired_cursor()); }
        let snapshot = self.snapshots.get_mut(key).ok_or_else(expired_cursor)?;
        if now >= snapshot.expires_at { return Err(expired_cursor()); }
        if let Some(boundary) = rows.iter().filter_map(|row| row.updated_at.and_then(|time| time.checked_add(RECENT_WINDOW_MS + 1))).min() {
            if now >= boundary { return Err(expired_cursor()); }
            let version = self.versions.entry(key.provider_id.clone()).or_default();
            version.boundary_at = Some(version.boundary_at.map_or(boundary, |old| old.min(boundary)));
        }
        snapshot.rows.extend(rows);
        snapshot.tail = tail;
        Ok(())
    }

    pub(crate) fn page(
        &mut self,
        key: &ViewKey,
        cursor: Option<&str>,
        limit: Option<u64>,
        now: u64,
    ) -> Result<Option<Page>, gateway::ProtocolError> {
        self.page_with_fence(key, cursor, limit, now, None)
    }

    pub(crate) fn page_with_fence(
        &mut self,
        key: &ViewKey,
        cursor: Option<&str>,
        limit: Option<u64>,
        now: u64,
        first_page_fence: Option<gateway::EventCursor>,
    ) -> Result<Option<Page>, gateway::ProtocolError> {
        let limit = limit.unwrap_or(20);
        if !(1..=100).contains(&limit) {
            return Err(error(
                "invalid_request",
                "Recent limit must be between 1 and 100",
                false,
            ));
        }
        let decoded = cursor
            .map(|cursor| decode_cursor(&self.cursor_key, key, cursor))
            .transpose()?;
        let boundary_expired = self
            .versions
            .get(&key.provider_id)
            .is_some_and(|version| version.boundary_at.is_some_and(|expiry| now >= expiry));
        if boundary_expired {
            self.invalidate(&key.provider_id);
        }
        if self.snapshots.get(key).is_some_and(|s| now >= s.expires_at) {
            self.snapshots.remove(key);
        }
        let Some(snapshot) = self.snapshots.get_mut(key) else {
            return if cursor.is_some() {
                Err(expired_cursor())
            } else {
                Ok(None)
            };
        };
        // Authentication binds the complete key. The random snapshot nonce binds revision,
        // cutoff and lifetime without exposing the reader scope in the cursor.
        let (offset, fence) = match decoded {
            None => (
                0,
                first_page_fence.unwrap_or_else(|| snapshot.fence.clone()),
            ),
            Some((nonce, offset, fence)) if nonce == snapshot.nonce => (offset, fence),
            Some(_) => return Err(expired_cursor()),
        };
        if offset > snapshot.rows.len() {
            return Err(invalid_cursor());
        }
        if snapshot.tail.is_some() && offset.saturating_add(limit as usize) > snapshot.rows.len() {
            return Ok(None);
        }
        let end = offset
            .saturating_add(limit as usize)
            .min(snapshot.rows.len());
        let next_cursor = (end < snapshot.rows.len() || snapshot.tail.is_some())
            .then(|| encode_cursor(&self.cursor_key, key, &snapshot.nonce, end, &fence));
        Ok(Some(Page {
            conversations: snapshot.rows[offset..end].to_vec(),
            next_cursor,
            revision: snapshot.revision.clone(),
            snapshot_cursor: fence,
        }))
    }
}

fn cursor_input(key: &ViewKey, nonce: &str, offset: usize, fence: &str) -> Vec<u8> {
    // JSON tuple avoids delimiter collisions in opaque IDs/scopes.
    serde_json::to_vec(&(
        &key.provider_id,
        &key.reader_scope,
        key.generation,
        nonce,
        offset,
        fence,
    ))
    .expect("cursor tuple is serializable")
}

fn encode_cursor(
    secret: &hmac::Key,
    key: &ViewKey,
    nonce: &str,
    offset: usize,
    fence: &str,
) -> String {
    let signature = hmac::sign(secret, &cursor_input(key, nonce, offset, fence));
    let signature: String = signature
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("recent.{nonce}.{offset}.{fence}.{signature}")
}

fn decode_cursor(
    secret: &hmac::Key,
    key: &ViewKey,
    cursor: &str,
) -> Result<(String, usize, String), gateway::ProtocolError> {
    let parts: Vec<_> = cursor.split('.').collect();
    if parts.len() != 5 || parts[0] != "recent" || parts[4].len() != 64 {
        return Err(invalid_cursor());
    }
    let offset = parts[2].parse::<usize>().map_err(|_| invalid_cursor())?;
    let signature = parts[4]
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hex = std::str::from_utf8(pair).map_err(|_| invalid_cursor())?;
            u8::from_str_radix(hex, 16).map_err(|_| invalid_cursor())
        })
        .collect::<Result<Vec<_>, _>>()?;
    hmac::verify(
        secret,
        &cursor_input(key, parts[1], offset, parts[3]),
        &signature,
    )
    .map_err(|_| invalid_cursor())?;
    Ok((parts[1].to_owned(), offset, parts[3].to_owned()))
}

fn invalid_cursor() -> gateway::ProtocolError {
    error(
        "invalid_cursor",
        "Recent cursor does not belong to this view",
        false,
    )
}

fn revision(value: u64) -> String {
    format!("recent-{value}")
}

fn expired_cursor() -> gateway::ProtocolError {
    error(
        "recent_cursor_expired",
        "Recent cursor is unavailable; fetch a new first page",
        false,
    )
}

pub(crate) fn error(code: &str, message: &str, retryable: bool) -> gateway::ProtocolError {
    gateway::ProtocolError {
        code: code.into(),
        message: message.into(),
        retryable,
        details: None,
    }
}

#[cfg(test)]
mod tests;
