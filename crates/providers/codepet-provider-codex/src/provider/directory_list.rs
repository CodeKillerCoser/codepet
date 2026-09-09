//! Demand-driven union of native and DB pages. Only visited pages are frozen.
use super::*;
use crate::directory::{DbQuery, read_page, repair_time};
use codepet_provider_sdk::ConversationListQuery;
use codepet_provider_data::conversation_state::SharedConversationStateStore;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Default)]
pub(super) struct DirectoryPages(Mutex<HashMap<String, Arc<tokio::sync::Mutex<QueryState>>>>);

#[derive(Clone)]
struct QueryState {
    binding: String,
    created: Instant,
    assignments: CodexDesktopProjectAssignments,
    native_cursor: Option<String>,
    native_seen: HashSet<String>,
    native_done: bool,
    native_frontier: Option<u64>,
    db: DbQuery,
    db_done: bool,
    db_frontier: Option<u64>,
    pending: BTreeMap<String, Conversation>,
    returned: Vec<Conversation>,
    emitted: HashSet<String>,
    issued_offsets: HashSet<usize>,
}

fn invalid_cursor() -> ProtocolError {
    protocol_error("invalid_cursor", "Conversation cursor expired or does not match this query".into(), false)
}
fn project_filter(request: &ConversationListRequest) -> Option<Option<String>> {
    match &request.project_filter {
        ConversationProjectFilter::ConversationProjectFilterAll(_) => None,
        ConversationProjectFilter::ConversationProjectFilterStandalone(_) => Some(None),
        ConversationProjectFilter::ConversationProjectFilterProject(filter) => Some(Some(filter.project.native_resource_id.clone())),
    }
}
fn accepts(row: &Conversation, request: &ConversationListRequest, assignments: &CodexDesktopProjectAssignments) -> bool {
    let membership = assignments.membership_for(&row.resource.native_resource_id, row.project.as_ref().map(|p| p.native_resource_id.as_str()));
    let project = match project_filter(request) {
        None => true,
        Some(None) => membership == CodexConversationMembership::Standalone,
        Some(Some(id)) => membership == CodexConversationMembership::Project(id),
    };
    project && match &request.query {
        Some(ConversationListQuery::ConversationUpdatedAfterQuery(query)) => row.updated_at.is_some_and(|time| time >= query.updated_after),
        Some(ConversationListQuery::ConversationIdsQuery(query)) => query.ids.contains(&row.resource.native_resource_id),
        None => true,
    }
}

impl QueryState {
    fn insert(&mut self, row: Conversation, request: &ConversationListRequest) {
        let id = row.resource.native_resource_id.clone();
        if self.emitted.contains(&id) || !accepts(&row, request, &self.assignments) { return; }
        if let Some(previous) = self.pending.get_mut(&id) {
            previous.updated_at = previous.updated_at.max(row.updated_at);
            if previous.title == id { previous.title = row.title; }
            if previous.preview.is_none() { previous.preview = row.preview; }
        } else { self.pending.insert(id, row); }
    }
    fn sorted(&self) -> Vec<Conversation> {
        let mut rows = self.pending.values().cloned().collect::<Vec<_>>();
        conversation_atoms::sort_summaries(&mut rows);
        if self.db.ids.is_some() { rows.sort_by(|a,b| a.resource.native_resource_id.cmp(&b.resource.native_resource_id)); }
        rows
    }
}

impl CodexInstanceRuntime {
    pub(super) async fn list_directory_page(&self, generation: &str, request: &ConversationListRequest) -> Result<ConversationListResponse, ProtocolError> {
        let mut value = serde_json::to_value(request).map_err(|e| protocol_error("invalid_request", e.to_string(), false))?;
        value.as_object_mut().unwrap().remove("cursor");
        value.as_object_mut().unwrap().remove("limit");
        let binding = format!("{generation}:{value}");
        let (token, offset, state) = {
            let mut pages = lock(&self.directory_pages.0);
            pages.retain(|_, state| state.try_lock().map(|state| state.created.elapsed() < Duration::from_secs(120)).unwrap_or(true));
            if let Some(cursor) = &request.cursor {
                let (token, offset) = cursor.rsplit_once(':').ok_or_else(invalid_cursor)?;
                let offset = offset.parse::<usize>().map_err(|_| invalid_cursor())?;
                (token.to_string(), offset, pages.get(token).cloned().ok_or_else(invalid_cursor)?)
            } else {
                let mut random = [0u8; 24];
                SystemRandom::new().fill(&mut random).map_err(|_| protocol_error("internal_error", "Cannot generate list cursor".into(), true))?;
                let token = random.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
                if pages.len() >= 64 {
                    let oldest = pages.iter().filter_map(|(token, state)| state.try_lock().ok().map(|state| (token.clone(), state.created))).min_by_key(|(_, time)| *time).map(|(token,_)| token);
                    if let Some(oldest) = oldest { pages.remove(&oldest); }
                    else { return Err(protocol_error("conversation_query_busy", "Too many active list queries".into(), true)); }
                }
                let assignments = load_codex_desktop_project_assignments(self.settings.data_directory.as_deref())?;
                let db = DbQuery {
                    updated_after: match &request.query { Some(ConversationListQuery::ConversationUpdatedAfterQuery(query)) => Some(query.updated_after), _ => None },
                    ids: match &request.query { Some(ConversationListQuery::ConversationIdsQuery(query)) => Some(query.ids.clone()), _ => None },
                    project: project_filter(request), assignments: assignments.by_thread.clone().into_iter().collect(), limit: Some(20), ..Default::default()
                };
                let state = Arc::new(tokio::sync::Mutex::new(QueryState { binding: binding.clone(), created: Instant::now(), assignments, native_cursor: None, native_seen: HashSet::new(), native_done: db.ids.is_some(), native_frontier: None, db, db_done: false, db_frontier: None, pending: BTreeMap::new(), returned: Vec::new(), emitted: HashSet::new(), issued_offsets: HashSet::from([0]) }));
                pages.insert(token.clone(), state.clone());
                (token, 0, state)
            }
        };
        let mut saved = state.lock().await;
        if saved.binding != binding || !saved.issued_offsets.contains(&offset) || saved.created.elapsed() >= Duration::from_secs(120) { return Err(invalid_cursor()); }
        // Commit source progress only after a successful response; retries replay it.
        let mut next = saved.clone();
        let limit = request.limit.unwrap_or(20) as usize;
        let target = offset.checked_add(limit).ok_or_else(invalid_cursor)?;
        let mut budget = 128;
        while next.returned.len() < target {
            let needed = target - next.returned.len();
            let rows = next.sorted();
            let cutoff = rows.get(needed).map(|row| row.updated_at.unwrap_or_default() / 1000);
            let native_ready = next.native_done || cutoff.zip(next.native_frontier).is_some_and(|(cutoff, frontier)| frontier < cutoff);
            let db_ready = next.db_done || cutoff.zip(next.db_frontier).is_some_and(|(cutoff, frontier)| frontier < cutoff);
            if native_ready && db_ready {
                for row in rows.into_iter().take(needed) {
                    let id = row.resource.native_resource_id.clone();
                    next.pending.remove(&id);
                    next.emitted.insert(id);
                    next.returned.push(row);
                }
                break;
            }
            if budget == 0 { return Err(protocol_error("conversation_query_incomplete", "List page exceeded bounded source work; narrow the filter and retry".into(), true)); }
            budget -= 1;
            if !native_ready { self.read_native_page(&mut next, request).await?; }
            if !db_ready { self.read_db_page(&mut next, request).await?; }
        }
        if generation != self.query_generation()? { return Err(conversation_atoms::generation_changed()); }
        let end = target.min(next.returned.len());
        let mut rows = next.returned[offset..end].to_vec();
        for row in &mut rows { self.project_conversation(row); }
        if let Some(scope) = &request.reader_scope { SharedConversationStateStore::from_env()?.decorate_many(scope, &mut rows)?; }
        let more = end < next.returned.len() || !next.pending.is_empty() || !next.native_done || !next.db_done;
        if more { next.issued_offsets.insert(end); }
        *saved = next;
        Ok(ConversationListResponse { conversations: rows, page_info: PageInfo { next_cursor: more.then(|| format!("{token}:{end}")) } })
    }

    async fn read_native_page(&self, state: &mut QueryState, request: &ConversationListRequest) -> Result<(), ProtocolError> {
        let server = self.ready_server()?;
        let cursor = state.native_cursor.clone();
        let project_id = project_filter(request);
        let page = tokio::task::spawn_blocking(move || server.thread_list(CodexThreadListRequest { cursor, limit: Some(20), project_id, workspace_root: None, search_term: None })).await.map_err(provider_task_error)?.map_err(CodexProtocolMapper::error)?;
        if let Some(cursor) = &page.next_cursor {
            if !state.native_seen.insert(cursor.clone()) { return Err(protocol_error("conversation_query_incomplete", "Native list cursor repeated".into(), true)); }
        }
        state.native_done = page.next_cursor.is_none();
        state.native_cursor = page.next_cursor;
        let home = self.settings.data_directory.clone().or_else(codex_home).ok_or_else(|| protocol_error("conversation_query_incomplete", "Cannot resolve Codex data directory".into(), true))?;
        let ids = page.data.iter().map(|snapshot| snapshot.thread.id.clone()).collect::<Vec<_>>();
        let evidence = tokio::task::spawn_blocking(move || read_page(&home, &DbQuery { ids: Some(ids), limit: Some(100), ..Default::default() })).await.map_err(provider_task_error)??.into_iter().collect::<BTreeMap<_,_>>();
        for mut snapshot in page.data {
            state.assignments.decorate(&mut snapshot);
            let mut row = lock(&self.mapper).conversation(&snapshot);
            state.native_frontier = Some(row.updated_at.unwrap_or_default() / 1000);
            if state.db.updated_after.is_some_and(|since| row.updated_at.unwrap_or_default() / 1000 < since / 1000) { state.native_done = true; }
            if let Some(fact) = evidence.get(&snapshot.thread.id) {
                if fact.archived { continue; }
                repair_time(&mut row, fact);
            }
            if !snapshot.thread.ephemeral { state.insert(row, request); }
        }
        Ok(())
    }

    async fn read_db_page(&self, state: &mut QueryState, request: &ConversationListRequest) -> Result<(), ProtocolError> {
        let home = self.settings.data_directory.clone().or_else(codex_home).ok_or_else(|| protocol_error("conversation_query_incomplete", "Cannot resolve Codex data directory".into(), true))?;
        let mut query = state.db.clone();
        if query.ids.is_some() { query.limit = Some(100); }
        let evidence = tokio::task::spawn_blocking(move || read_page(&home, &query)).await.map_err(provider_task_error)??;
        state.db_done = state.db.ids.is_some() || evidence.len() < 20;
        if let Some((id, fact)) = evidence.last() {
            state.db.after = Some((fact.updated_ms.unwrap_or_default(), id.clone()));
            state.db_frontier = Some(fact.updated_ms.unwrap_or_default() / 1000);
        }
        // Explicit IDs can include just-created tasks not yet committed to SQLite.
        let mut candidates = evidence.into_iter().map(|(id, fact)| (id, Some(fact))).collect::<Vec<_>>();
        if let Some(ids) = &state.db.ids {
            for id in ids { if !candidates.iter().any(|(known,_)| known == id) { candidates.push((id.clone(), None)); } }
        }
        for (id, fact) in candidates {
            if state.emitted.contains(&id) { continue; }
            if let Some(row) = state.pending.get_mut(&id) {
                if let Some(fact) = fact { repair_time(row, &fact); }
                continue;
            }
            let server = self.ready_server()?;
            let requested = id.clone();
            let mut snapshot = match tokio::task::spawn_blocking(move || server.thread_read_metadata(&requested)).await.map_err(provider_task_error)? {
                Ok(snapshot) => snapshot,
                Err(error) if state.db.ids.is_some() && error.is_thread_not_loaded(&id) => continue,
                Err(error) => return Err(CodexProtocolMapper::error(error)),
            };
            if snapshot.thread.ephemeral { continue; }
            state.assignments.decorate(&mut snapshot);
            let mut row = lock(&self.mapper).conversation(&snapshot);
            if let Some(fact) = fact { repair_time(&mut row, &fact); }
            state.insert(row, request);
        }
        Ok(())
    }
}
