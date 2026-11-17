//! Rank explicit candidates before hydrating the requested page. SQLite supplies
//! only verified paginated timestamps; formal objects still come from App Server.
use super::*;
use crate::directory::{read_page, repair_time, DbEvidence, DbQuery};
use codepet_provider_data::conversation_state::SharedConversationStateStore;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Pages(Mutex<HashMap<String, Arc<tokio::sync::Mutex<View>>>>);

#[derive(Clone)]
struct View {
    binding: String,
    created: Instant,
    epoch: u64,
    assignments: CodexDesktopProjectAssignments,
    evidence: BTreeMap<String, DbEvidence>,
    ranks: Vec<(String, Option<u64>)>,
    objects: BTreeMap<String, Conversation>,
    returned: Vec<Conversation>,
    issued: BTreeSet<usize>,
    position: usize,
    prepared: bool,
    metadata_candidates: usize,
}

fn invalid_cursor() -> ProtocolError {
    protocol_error(
        "invalid_cursor",
        "Ordered IDs cursor expired or no longer matches its source".into(),
        false,
    )
}

impl CodexInstanceRuntime {
    pub(super) async fn list_ordered_ids(
        &self,
        generation: &str,
        request: &ConversationListRequest,
    ) -> Result<ConversationListResponse, ProtocolError> {
        let started = Instant::now();
        let Some(codepet_provider_sdk::ConversationListQuery::ConversationIdsQuery(query)) =
            &request.query
        else {
            unreachable!()
        };
        let mut value = serde_json::to_value(request)
            .map_err(|error| protocol_error("invalid_request", error.to_string(), false))?;
        value.as_object_mut().unwrap().remove("cursor");
        value.as_object_mut().unwrap().remove("limit");
        let binding = format!("{generation}:{value}");
        let (token, offset, saved) = {
            let mut pages = lock(&self.ordered_ids.0);
            pages.retain(|_, view| {
                view.try_lock()
                    .map(|view| view.created.elapsed() < Duration::from_secs(120))
                    .unwrap_or(true)
            });
            if let Some(cursor) = &request.cursor {
                let (token, offset) = cursor.rsplit_once(':').ok_or_else(invalid_cursor)?;
                (
                    token.to_owned(),
                    offset.parse::<usize>().map_err(|_| invalid_cursor())?,
                    pages.get(token).cloned().ok_or_else(invalid_cursor)?,
                )
            } else {
                if pages.len() >= 64 {
                    let oldest = pages
                        .iter()
                        .filter_map(|(token, view)| {
                            view.try_lock()
                                .ok()
                                .map(|view| (token.clone(), view.created))
                        })
                        .min_by_key(|(_, created)| *created)
                        .map(|(token, _)| token)
                        .ok_or_else(|| {
                            protocol_error(
                                "conversation_query_busy",
                                "Too many ordered ID views".into(),
                                true,
                            )
                        })?;
                    pages.remove(&oldest);
                }
                let mut random = [0u8; 24];
                ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random)
                    .map_err(|_| {
                        protocol_error(
                            "internal_error",
                            "Cannot generate ordered IDs cursor".into(),
                            true,
                        )
                    })?;
                let token = random
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let view = Arc::new(tokio::sync::Mutex::new(View {
                    binding: binding.clone(),
                    created: Instant::now(),
                    epoch: self.atoms.event_epoch(),
                    assignments: load_codex_desktop_project_assignments(
                        self.settings.data_directory.as_deref(),
                    )?,
                    evidence: BTreeMap::new(),
                    ranks: vec![],
                    objects: BTreeMap::new(),
                    returned: vec![],
                    issued: BTreeSet::from([0]),
                    position: 0,
                    prepared: false,
                    metadata_candidates: 0,
                }));
                pages.insert(token.clone(), view.clone());
                (token, 0, view)
            }
        };
        let mut saved = saved.lock().await;
        if saved.binding != binding
            || !saved.issued.contains(&offset)
            || saved.created.elapsed() >= Duration::from_secs(120)
        {
            return Err(invalid_cursor());
        }
        // An already visited page is replayable, including after source updates.
        let target = offset
            .checked_add(request.limit.unwrap_or(20) as usize)
            .ok_or_else(invalid_cursor)?;
        let mut next = saved.clone();
        let previous_candidates = next.metadata_candidates;
        let extending =
            target > next.returned.len() && (!next.prepared || next.position < next.ranks.len());
        if extending && next.epoch != self.atoms.event_epoch() {
            return Err(invalid_cursor());
        }
        if !next.prepared {
            let home = self
                .settings
                .data_directory
                .clone()
                .or_else(codex_home)
                .ok_or_else(|| {
                    protocol_error(
                        "conversation_query_incomplete",
                        "Cannot resolve Codex home".into(),
                        true,
                    )
                })?;
            let ids = query.ids.clone();
            next.evidence = tokio::task::spawn_blocking(move || {
                read_page(
                    &home,
                    &DbQuery {
                        ids: Some(ids),
                        ..Default::default()
                    },
                )
            })
            .await
            .map_err(provider_task_error)??
            .into_iter()
            .collect();
            let version = self.ready_server()?.harness_version().unwrap_or_default();
            let sqlite_ranks = version.starts_with("0.166.") || version.starts_with("0.167.");
            let uncertain: Vec<_> = query
                .ids
                .iter()
                .filter(|id| {
                    !sqlite_ranks
                        || !next
                            .evidence
                            .get(*id)
                            .is_some_and(|fact| fact.paginated && fact.updated_ms.is_some())
                })
                .cloned()
                .collect();
            next.metadata_candidates += uncertain.len();
            next.objects = self.ordered_objects(generation, &next, uncertain).await?;
            let projection = lock(&self.hook_activity);
            next.ranks = query
                .ids
                .iter()
                .filter_map(|id| {
                    let time = match next.objects.get(id) {
                        Some(row) => row.updated_at,
                        None if sqlite_ranks
                            && next.evidence.get(id).is_some_and(|fact| {
                                fact.paginated && fact.updated_ms.is_some()
                            }) =>
                        {
                            projection.projected_time(id, next.evidence[id].updated_ms)
                        }
                        None => return None, // Only an exact native thread-not-loaded was omitted.
                    };
                    Some((id.clone(), time))
                })
                .collect();
            next.ranks
                .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            next.prepared = true;
        }
        while next.returned.len() < target && next.position < next.ranks.len() {
            let end = (next.position + target - next.returned.len()).min(next.ranks.len());
            let ids: Vec<_> = next.ranks[next.position..end]
                .iter()
                .filter(|(id, _)| !next.objects.contains_key(id))
                .map(|(id, _)| id.clone())
                .collect();
            next.metadata_candidates += ids.len();
            next.objects
                .extend(self.ordered_objects(generation, &next, ids).await?);
            let mismatch = next.ranks[next.position..end].iter().any(|(id, time)| {
                next.objects
                    .get(id)
                    .is_some_and(|row| &row.updated_at != time)
            });
            if mismatch {
                if !next.returned.is_empty() {
                    return Err(invalid_cursor());
                }
                // Unknown schema/runtime behavior or a concurrent edit: certify
                // every candidate natively rather than guessing a global order.
                next.metadata_candidates += query.ids.len();
                next.objects = self
                    .ordered_objects(generation, &next, query.ids.clone())
                    .await?;
                next.ranks = next
                    .objects
                    .iter()
                    .map(|(id, row)| (id.clone(), row.updated_at))
                    .collect();
                next.ranks
                    .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                next.position = 0;
                continue;
            }
            for (id, _) in &next.ranks[next.position..end] {
                if let Some(row) = next.objects.get(id) {
                    next.returned.push(row.clone());
                }
            }
            next.position = end;
        }
        if generation != self.query_generation()? {
            return Err(conversation_atoms::generation_changed());
        }
        if next.epoch != self.atoms.event_epoch() && extending {
            return Err(invalid_cursor());
        }
        let end = target.min(next.returned.len());
        let mut rows = next.returned[offset..end].to_vec();
        if let Some(scope) = &request.reader_scope {
            SharedConversationStateStore::from_env()?.decorate_many(scope, &mut rows)?;
        }
        let more = end < next.returned.len() || next.position < next.ranks.len();
        if more {
            next.issued.insert(end);
        }
        eprintln!(
            "{}",
            json!({"name":"provider.conversation.orderedIds", "candidateCount":query.ids.len(),
            "offset":offset,"returned":rows.len(),"metadataCandidates":next.metadata_candidates - previous_candidates,
            "elapsedMs":started.elapsed().as_millis()})
        );
        // Only visited pages retain formal objects. Legacy certification may
        // inspect many candidates, but must not pin every unvisited object for
        // the whole cursor lifetime. Future pages revalidate their objects.
        next.objects.clear();
        *saved = next;
        Ok(ConversationListResponse {
            conversations: rows,
            page_info: PageInfo {
                next_cursor: more.then(|| format!("{token}:{end}")),
            },
        })
    }

    async fn ordered_objects(
        &self,
        generation: &str,
        view: &View,
        ids: Vec<String>,
    ) -> Result<BTreeMap<String, Conversation>, ProtocolError> {
        let mut jobs = tokio::task::JoinSet::new();
        // Queue at most a protocol-sized batch; the native semaphore is shared
        // with ordinary directory reads and survives caller cancellation.
        let mut result = BTreeMap::new();
        for batch in ids.chunks(100) {
            for id in batch {
                let id = id.clone();
                let reader = self.directory_pages.metadata.clone();
                let generation = generation.to_owned();
                let epoch = view.epoch;
                let server = self.ready_server()?;
                jobs.spawn(async move {
                    let value = reader.read(generation, epoch, server, id.clone()).await;
                    (id, value)
                });
            }
            while let Some(value) = jobs.join_next().await {
                let (id, value) = value.map_err(provider_task_error)?;
                let mut snapshot = match value {
                    Ok(snapshot) => snapshot,
                    Err(error) if error.is_thread_not_loaded(&id) => continue,
                    Err(error) => return Err(CodexProtocolMapper::error(error)),
                };
                if snapshot.thread.ephemeral {
                    continue;
                }
                view.assignments.decorate(&mut snapshot);
                let mut row = lock(&self.mapper).conversation(&snapshot);
                if let Some(evidence) = view.evidence.get(&id) {
                    repair_time(&mut row, evidence);
                }
                self.project_conversation(&mut row);
                result.insert(id, row);
            }
        }
        Ok(result)
    }
}
