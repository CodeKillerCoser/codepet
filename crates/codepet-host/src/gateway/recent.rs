use super::*;
use crate::recent_conversations::{self as feed, Identity, ViewKey};
use codepet_provider_sdk::conversation_query::EnumerationProgress;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Active = BTreeMap<Identity, (gateway::ConversationStatus, String)>;
type Unread = BTreeMap<Identity, gateway::ConversationReadState>;

impl ProviderGatewayService {
    pub(super) async fn mark_read_for_scope(
        &self,
        scope: &str,
        request: gateway::ConversationMarkReadRequest,
    ) -> Result<gateway::ConversationMarkReadResponse, gateway::ProtocolError> {
        validate_gateway_resource(&request.conversation)?;
        let provider_id = request.conversation.provider_id.clone();
        let runtime = self
            .gateway_providers(None)
            .await?
            .into_iter()
            .find(|runtime| runtime.summary.id == provider_id);
        let read_state = if self.conversation_state.path().is_some()
            && runtime.is_some_and(|runtime| runtime.provider_mark_read)
        {
            let conversation = self.resolve_resource(request.conversation).await?;
            self.manager
                .conversation_mark_read(provider::ConversationMarkReadRequest {
                    conversation,
                    reader_scope: scope.to_owned(),
                    observed_activity_version: request.observed_activity_version,
                })
                .await
                .map_err(gateway_error)?
                .read_state
        } else {
            self.conversation_state
                .mark_read(
                    scope,
                    &request.conversation,
                    &request.observed_activity_version,
                )
                .map_err(gateway_error)?
        };
        self.invalidate_recent(&provider_id)?;
        Ok(gateway::ConversationMarkReadResponse { read_state })
    }

    pub(super) fn invalidate_recent(
        &self,
        provider_id: &str,
    ) -> Result<(), gateway::ProtocolError> {
        let mut snapshots = self.recent.lock().map_err(|_| gateway_state_error())?;
        if snapshots.is_tracked(provider_id) {
            snapshots.invalidate(provider_id);
        }
        Ok(())
    }

    pub(super) fn publish_recent_invalidations(&self) -> Result<(), gateway::ProtocolError> {
        let changes = self
            .recent
            .lock()
            .map_err(|_| gateway_state_error())?
            .take_invalidations(now_ms());
        for (provider_id, revision) in changes {
            self.events
                .publish(gateway::ProtocolEvent::ConversationRecentChanged {
                    jsonrpc: "2.0".into(),
                    params: gateway::ProtocolEventParams {
                        event_cursor: event_cursor(0),
                        payload: gateway::ConversationRecentChangedEvent {
                            provider_id,
                            revision,
                        },
                    },
                })?;
        }
        Ok(())
    }

    pub(super) async fn conversation_recent_for_scope(
        &self,
        scope: &str,
        request: gateway::ConversationRecentRequest,
    ) -> Result<gateway::ConversationRecentResponse, gateway::ProtocolError> {
        if scope.trim().is_empty() {
            return Err(feed::error(
                "invalid_caller_scope",
                "Recent requires an authenticated reader scope",
                false,
            ));
        }
        // A memory-only Host and a child process cannot share one read authority.
        if self.conversation_state.path().is_none() {
            return Err(feed::error(
                "unsupported",
                "Recent requires shared persistent reading state",
                false,
            ));
        }
        let runtime = self
            .gateway_providers(None)
            .await?
            .into_iter()
            .find(|runtime| runtime.summary.id == request.provider_id)
            .ok_or_else(|| feed::error("unknown_provider", "Unknown recent Provider", false))?;
        if !runtime
            .capabilities
            .methods
            .contains(&gateway::GatewayCapability::ConversationRecent)
        {
            return Err(feed::error(
                "unsupported",
                "Provider cannot enumerate complete recent candidates",
                false,
            ));
        }
        let key = ViewKey {
            provider_id: request.provider_id.clone(),
            reader_scope: scope.to_owned(),
            generation: runtime.summary.runtime.generation.unwrap_or(0),
        };
        // Start observing before any Provider query, including an empty or failed
        // initial feed. Legacy clients that never request recent gain no new traffic.
        self.recent
            .lock()
            .map_err(|_| gateway_state_error())?
            .track(&key.provider_id);
        if let Some(page) = self
            .recent
            .lock()
            .map_err(|_| gateway_state_error())?
            .page_with_fence(
                &key,
                request.cursor.as_deref(),
                request.limit,
                now_ms(),
                Some(self.current_event_cursor()),
            )?
        {
            return Ok(response(page));
        }
        let build_lock = {
            let mut builds = self.recent_builds.lock().await;
            builds.retain(|_, lock| lock.strong_count() > 0);
            match builds.get(&key).and_then(std::sync::Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(AsyncMutex::new(()));
                    builds.insert(key.clone(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        let _building = build_lock.lock().await;
        if let Some(page) = self
            .recent
            .lock()
            .map_err(|_| gateway_state_error())?
            .page_with_fence(
                &key,
                request.cursor.as_deref(),
                request.limit,
                now_ms(),
                Some(self.current_event_cursor()),
            )?
        {
            return Ok(response(page));
        }
        self.conversation_state
            .ensure_client(scope)
            .map_err(gateway_error)?;
        let _capacity = self
            .recent_concurrency
            .acquire()
            .await
            .map_err(|_| gateway_state_error())?;
        let pending = self.recent.lock().map_err(|_| gateway_state_error())?.pending_tail(&key, request.cursor.as_deref(), request.limit)?;
        if let Some((tail, needed)) = pending {
            let epoch = self.recent.lock().map_err(|_| gateway_state_error())?.epoch(&key.provider_id);
            let (rows, tail) = match self.collect_recent_tail(&runtime.route, scope, tail, needed).await {
                Ok(page) => page,
                Err(error) if error.code == "invalid_cursor" => {
                    self.recent.lock().map_err(|_| gateway_state_error())?.invalidate(&key.provider_id);
                    return Err(feed::error("recent_cursor_expired", "Provider list cursor expired; fetch a new recent first page", false));
                }
                Err(error) => return Err(error),
            };
            let mut snapshots = self.recent.lock().map_err(|_| gateway_state_error())?;
            snapshots.append(&key, epoch, rows, tail, now_ms())?;
            return snapshots.page_with_fence(&key, request.cursor.as_deref(), request.limit, now_ms(), Some(self.current_event_cursor()))?.map(response).ok_or_else(changed);
        }
        // Bounded internal concurrency: one sequential enumeration per build, one build
        // per view. Dropping this future cancels the outstanding RPC and no snapshot installs.
        for attempt in 0..3 {
            let started = now_ms();
            let epoch = self
                .recent
                .lock()
                .map_err(|_| gateway_state_error())?
                .epoch(&key.provider_id);
            let fence = self.current_event_cursor();
            let collected = self.collect_recent(&runtime.route, scope, started, request.limit.unwrap_or(20) as usize).await;
            let (rows, boundary_at, tail) = match collected {
                Ok(rows) => rows,
                Err(error) => {
                    // A failed completeness check also disqualifies older cached views.
                    // Do not evict a newer view already installed after an observed event.
                    let mut snapshots = self.recent.lock().map_err(|_| gateway_state_error())?;
                    if snapshots.epoch(&key.provider_id) == epoch {
                        snapshots.invalidate(&key.provider_id);
                    }
                    if error.code == "recent_snapshot_changed" && attempt < 2 {
                        continue;
                    }
                    return Err(error);
                }
            };
            let current = self
                .gateway_providers(None)
                .await?
                .into_iter()
                .find(|runtime| runtime.summary.id == key.provider_id);
            if !current.is_some_and(|runtime| {
                runtime.summary.runtime.generation == Some(key.generation)
                    && runtime
                        .capabilities
                        .methods
                        .contains(&gateway::GatewayCapability::ConversationRecent)
            }) {
                let mut snapshots = self.recent.lock().map_err(|_| gateway_state_error())?;
                if snapshots.epoch(&key.provider_id) == epoch {
                    snapshots.invalidate(&key.provider_id);
                }
                return Err(feed::error(
                    "recent_snapshot_changed",
                    "Provider generation or capabilities changed",
                    true,
                ));
            }
            let mut snapshots = self.recent.lock().map_err(|_| gateway_state_error())?;
            match snapshots.install(
                key.clone(),
                epoch,
                rows,
                boundary_at,
                fence.clone(),
                now_ms(),
            ) {
                Ok(()) => {
                    snapshots.set_tail(&key, tail);
                    return snapshots
                        .page_with_fence(&key, None, request.limit, now_ms(), Some(fence))?
                        .map(response)
                        .ok_or_else(changed)
                }
                Err(error) if error.code == "recent_snapshot_changed" && attempt < 2 => continue,
                Err(error) => return Err(error),
            }
        }
        Err(changed())
    }

    async fn collect_recent(
        &self,
        route: &provider::ProviderInstanceRoute,
        scope: &str,
        now: u64,
        limit: usize,
    ) -> Result<(Vec<gateway::Conversation>, Option<u64>, Option<feed::RecentTail>), gateway::ProtocolError> {
        let active = self.collect_active(route).await?;
        let unread = self.collect_unread(route, scope).await?;
        let mut summaries = BTreeMap::new();
        let missing: BTreeSet<_> = active
            .keys()
            .chain(unread.keys())
            .filter(|id| !summaries.contains_key(*id))
            .cloned()
            .collect();
        let missing: Vec<_> = missing.into_iter().map(|(_, native)| native).collect();
        for batch in missing.chunks(100) {
            let fetched = self
                .collect_summaries(
                    route,
                    scope,
                    provider::ConversationListQuery::ConversationIdsQuery(
                        provider::ConversationIdsQuery {
                            kind: provider::ConversationIdsQueryKind::Ids,
                            ids: batch.to_vec(),
                        },
                    ),
                )
                .await?;
            summaries.extend(fetched);
        }
        let state = self.conversation_state.clone();
        let snapshots = self.recent.clone();
        let owned_scope = scope.to_owned();
        let provider_id = route.provider_instance_id.clone();
        let mut rows: Vec<_> = summaries.into_values().collect();
        // File lock/JSON/atomic replacement stays off the async control-message workers.
        // Invalidate inside the operation even if its awaiting client disconnects.
        let rows = tokio::task::spawn_blocking(move || -> Result<_, gateway::ProtocolError> {
            if state
                .observe_and_decorate_summaries(&owned_scope, &mut rows)
                .map_err(gateway_error)?
            {
                snapshots
                    .lock()
                    .map_err(|_| gateway_state_error())?
                    .invalidate(&provider_id);
            }
            Ok(rows)
        })
        .await
        .map_err(|_| gateway_state_error())??;
        let mut summaries: BTreeMap<_, _> = rows
            .into_iter()
            .map(|row| (feed::identity(&row), row))
            .collect();
        // Revision is stable within each enumeration, but may be a fresh nonce for
        // each new snapshot. Compare complete facts across enumerations, not nonce equality.
        if active != self.collect_active(route).await?
            || unread != self.collect_unread(route, scope).await?
        {
            return Err(changed());
        }
        for (id, (status, _)) in &active {
            if let Some(summary) = summaries.get_mut(id) {
                summary.status = *status;
            }
        }
        let active_ids = active.keys().cloned().collect();
        let excluded = summaries.keys().cloned().collect();
        let (mut rows, mut boundary) = feed::aggregate(summaries, &active_ids, &unread, now);
        let tail = feed::RecentTail { cursor: None, cutoff: now.saturating_sub(feed::RECENT_WINDOW_MS), excluded, seen_cursors: BTreeSet::new() };
        let (recent, tail) = self.collect_recent_tail(route, scope, tail, limit.saturating_sub(rows.len())).await?;
        for row in &recent {
            if let Some(expiry) = row.updated_at.and_then(|time| time.checked_add(feed::RECENT_WINDOW_MS + 1)) { boundary = Some(boundary.map_or(expiry, |old| old.min(expiry))); }
        }
        rows.extend(recent);
        Ok((rows, boundary, tail))
    }

    async fn collect_recent_tail(&self, route: &provider::ProviderInstanceRoute, scope: &str, tail: feed::RecentTail, needed: usize) -> Result<(Vec<gateway::Conversation>, Option<feed::RecentTail>), gateway::ProtocolError> {
        let (mut rows, tail) = self.read_recent_tail(route, scope, tail, needed).await?;
        let state = self.conversation_state.clone();
        let snapshots = self.recent.clone();
        let scope = scope.to_owned();
        let provider_id = route.provider_instance_id.clone();
        let rows = tokio::task::spawn_blocking(move || -> Result<_, gateway::ProtocolError> {
            if state.observe_page_summaries(&scope, &mut rows).map_err(gateway_error)? {
                snapshots.lock().map_err(|_| gateway_state_error())?.invalidate(&provider_id);
            }
            Ok(rows)
        }).await.map_err(|_| gateway_state_error())??;
        Ok((rows, tail))
    }

    async fn read_recent_tail(&self, route: &provider::ProviderInstanceRoute, scope: &str, mut tail: feed::RecentTail, needed: usize) -> Result<(Vec<gateway::Conversation>, Option<feed::RecentTail>), gateway::ProtocolError> {
        let mut rows = Vec::new();
        // This enumerates only the requested time-window page; active and unread
        // IDs have already been fetched separately and retain their higher rank.
        for _ in 0..128 {
            if rows.len() >= needed { return Ok((rows, Some(tail))); }
            let page = self.manager.conversation_list(provider::ConversationListRequest {
                route: route.clone(), cursor: tail.cursor.clone(), limit: Some((needed - rows.len()).clamp(1, 100) as u64),
                project_filter: provider::ConversationProjectFilter::ConversationProjectFilterAll(provider::ConversationProjectFilterAll { kind: provider::ConversationProjectFilterAllKind::All }),
                query: Some(provider::ConversationListQuery::ConversationUpdatedAfterQuery(provider::ConversationUpdatedAfterQuery { kind: provider::ConversationUpdatedAfterQueryKind::UpdatedAfter, updated_after: tail.cutoff })),
                reader_scope: Some(scope.to_owned()),
            }).await.map_err(gateway_error)?;
            for row in page.conversations {
                if !row.updated_at.is_some_and(|time| time >= tail.cutoff) { return Err(feed::error("conversation_query_incomplete", "Provider returned an out-of-query summary", false)); }
                if tail.excluded.insert(feed::identity(&row)) { rows.push(row); }
            }
            tail.cursor = page.page_info.next_cursor;
            match &tail.cursor {
                None => return Ok((rows, None)),
                Some(cursor) if !tail.seen_cursors.insert(cursor.clone()) => return Err(feed::error("conversation_query_incomplete", "Provider list cursor repeated", true)),
                _ => {}
            }
        }
        Err(feed::error("conversation_query_incomplete", "Recent page exceeded bounded source work", true))
    }

    async fn collect_active(
        &self,
        route: &provider::ProviderInstanceRoute,
    ) -> Result<Active, gateway::ProtocolError> {
        let mut result = BTreeMap::new();
        let mut progress = EnumerationProgress::default();
        let mut cursor = None;
        let mut revision = None;
        loop {
            let page = self
                .manager
                .conversation_active_list(provider::ConversationActiveListRequest {
                    route: route.clone(),
                    cursor,
                    limit: Some(100),
                })
                .await
                .map_err(gateway_error)?;
            check_revision(&mut revision, &page.revision)?;
            for row in page.conversations {
                let id = (
                    row.conversation.provider_instance_id,
                    row.conversation.native_resource_id,
                );
                let value = (row.status, row.activity_version);
                if result
                    .insert(id, value.clone())
                    .is_some_and(|old| old != value)
                {
                    return Err(changed());
                }
            }
            cursor = progress.advance(page.page_info.next_cursor)?;
            if cursor.is_none() {
                return Ok(result);
            }
            tokio::task::yield_now().await;
        }
    }

    async fn collect_unread(
        &self,
        route: &provider::ProviderInstanceRoute,
        scope: &str,
    ) -> Result<Unread, gateway::ProtocolError> {
        let mut result = BTreeMap::new();
        let mut progress = EnumerationProgress::default();
        let mut cursor = None;
        let mut revision = None;
        loop {
            let page = self
                .manager
                .conversation_unread_list(provider::ConversationUnreadListRequest {
                    route: route.clone(),
                    reader_scope: scope.to_owned(),
                    cursor,
                    limit: Some(100),
                })
                .await
                .map_err(gateway_error)?;
            check_revision(&mut revision, &page.revision)?;
            for row in page.conversations {
                if !row.read_state.unread {
                    return Err(feed::error(
                        "conversation_query_incomplete",
                        "Unread enumeration included a read resource",
                        false,
                    ));
                }
                let id = (
                    row.conversation.provider_instance_id,
                    row.conversation.native_resource_id,
                );
                if result
                    .insert(id, row.read_state.clone())
                    .is_some_and(|old| old != row.read_state)
                {
                    return Err(changed());
                }
            }
            cursor = progress.advance(page.page_info.next_cursor)?;
            if cursor.is_none() {
                return Ok(result);
            }
            tokio::task::yield_now().await;
        }
    }

    async fn collect_summaries(
        &self,
        route: &provider::ProviderInstanceRoute,
        scope: &str,
        query: provider::ConversationListQuery,
    ) -> Result<BTreeMap<Identity, gateway::Conversation>, gateway::ProtocolError> {
        let mut summaries = BTreeMap::new();
        let mut progress = EnumerationProgress::default();
        let mut cursor = None;
        loop {
            let page = self
                .manager
                .conversation_list(provider::ConversationListRequest {
                    route: route.clone(),
                    cursor,
                    limit: Some(100),
                    project_filter:
                        provider::ConversationProjectFilter::ConversationProjectFilterAll(
                            provider::ConversationProjectFilterAll {
                                kind: provider::ConversationProjectFilterAllKind::All,
                            },
                        ),
                    query: Some(query.clone()),
                    reader_scope: Some(scope.to_owned()),
                })
                .await
                .map_err(gateway_error)?;
            for summary in page.conversations {
                let allowed = match &query {
                    provider::ConversationListQuery::ConversationUpdatedAfterQuery(query) => {
                        summary
                            .updated_at
                            .is_some_and(|updated| updated >= query.updated_after)
                    }
                    provider::ConversationListQuery::ConversationIdsQuery(query) => {
                        query.ids.contains(&summary.resource.native_resource_id)
                    }
                };
                if !allowed {
                    return Err(feed::error(
                        "conversation_query_incomplete",
                        "Provider returned an out-of-query summary",
                        false,
                    ));
                }
                if summaries
                    .insert(feed::identity(&summary), summary.clone())
                    .is_some_and(|old| old != summary)
                {
                    return Err(changed());
                }
            }
            cursor = progress.advance(page.page_info.next_cursor)?;
            if cursor.is_none() {
                return Ok(summaries);
            }
            tokio::task::yield_now().await;
        }
    }
}

fn check_revision(
    revision: &mut Option<String>,
    received: &str,
) -> Result<(), gateway::ProtocolError> {
    if received.is_empty()
        || revision
            .as_deref()
            .is_some_and(|revision| revision != received)
    {
        return Err(changed());
    }
    *revision = Some(received.to_owned());
    Ok(())
}

fn response(page: feed::Page) -> gateway::ConversationRecentResponse {
    gateway::ConversationRecentResponse {
        conversations: page.conversations,
        page_info: gateway::PageInfo {
            next_cursor: page.next_cursor,
        },
        revision: page.revision,
        snapshot_cursor: page.snapshot_cursor,
    }
}

fn changed() -> gateway::ProtocolError {
    feed::error(
        "recent_snapshot_changed",
        "Recent candidates changed during collection",
        true,
    )
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_advertisement_requires_every_complete_atom() {
        let capabilities: provider::ProviderCapabilities = serde_json::from_value(serde_json::json!({
            "revision":"native", "extensions":[],
            "methods":["conversation.list","conversation.active.list","conversation.unread.list","conversation.markRead"],
            "conversationListQuery":{"updatedAfter":true,"ids":true},
        })).unwrap();
        let supported = |capabilities: &provider::ProviderCapabilities| {
            map_capabilities(capabilities)
                .methods
                .contains(&gateway::GatewayCapability::ConversationRecent)
        };
        assert!(supported(&capabilities));
        for required in &capabilities.methods {
            let mut missing = capabilities.clone();
            missing.methods.retain(|method| method != required);
            assert!(!supported(&missing));
        }
        for query in [
            None,
            Some(provider::ConversationListQueryCapabilities {
                updated_after: false,
                ids: true,
            }),
            Some(provider::ConversationListQueryCapabilities {
                updated_after: true,
                ids: false,
            }),
        ] {
            let mut missing = capabilities.clone();
            missing.conversation_list_query = query;
            assert!(!supported(&missing));
        }
    }

    #[test]
    fn enumeration_revision_must_be_nonempty_and_constant_within_pages() {
        let mut revision = None;
        assert!(check_revision(&mut revision, "").is_err());
        check_revision(&mut revision, "one-snapshot").unwrap();
        check_revision(&mut revision, "one-snapshot").unwrap();
        assert_eq!(
            check_revision(&mut revision, "another-snapshot")
                .unwrap_err()
                .code,
            "recent_snapshot_changed"
        );
    }
}
