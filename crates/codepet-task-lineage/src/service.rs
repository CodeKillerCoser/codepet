use crate::{
    domain::*,
    extraction::{recent_message, validate, TaskExtractor, LOOKBACK_HOURS},
    sources::ConversationSource,
    stable_id,
    store::Store,
    Result,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const CHUNK_SIZE: usize = 200;
const NORMALIZATION_VERSION: u32 = 3;
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanState {
    pub schema_version: u32,
    #[serde(default)]
    pub normalization_version: u32,
    pub cursor: Cursor,
    pub message_count: usize,
    pub extracted_count: usize,
    pub last_error: Option<String>,
    #[serde(default)]
    pub dirty_revision: u64,
    #[serde(default)]
    pub extracted_revision: u64,
    #[serde(default)]
    pub last_changed_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub threads: Vec<Thread>,
    pub tasks: Vec<Task>,
    pub pending_messages: usize,
    pub diagnostics: Vec<String>,
    pub last_extraction: Option<Value>,
}
fn event_key(thread_id: &str, generation: u64, chunk: usize) -> String {
    format!(
        "events/{}/{generation}/{chunk:08}.json",
        stable_id(thread_id)
    )
}
fn state(store: &Store, thread_id: &str) -> Result<ScanState> {
    Ok(store
        .read(&Store::thread_key(thread_id, "scan"))?
        .unwrap_or_default())
}
pub fn messages(store: &Store, thread_id: &str) -> Result<Vec<Message>> {
    let scan = state(store, thread_id)?;
    let mut messages = Vec::new();
    for index in 0..scan.message_count.div_ceil(CHUNK_SIZE) {
        let chunk: Vec<Message> = store
            .read(&event_key(thread_id, scan.cursor.generation, index))?
            .ok_or("Missing event chunk")?;
        messages.extend(chunk);
    }
    if messages.len() != scan.message_count {
        return Err("Event manifest/count mismatch".into());
    }
    Ok(messages)
}
/// UI history may include a proven fork prefix; inference always uses local messages().
pub fn conversation_messages(store: &Store, thread_id: &str) -> Result<Vec<Message>> {
    fn collect(store: &Store, id: &str, seen: &mut HashSet<String>) -> Result<Vec<Message>> {
        if !seen.insert(id.into()) || seen.len() > 64 {
            return Err("Cyclic or excessive fork ancestry".into());
        }
        let thread: Thread = store
            .read(&Store::thread_key(id, "thread"))?
            .ok_or("Thread metadata not found")?;
        let mut result = vec![];
        if thread.creation_kind == "fork" {
            if let (Some(parent), Some(end)) = (&thread.parent_id, thread.inherited_end_byte_offset)
            {
                if let Some(parent_thread) =
                    store.read::<Thread>(&Store::thread_key(parent, "thread"))?
                {
                    result = collect(store, parent, seen)?
                        .into_iter()
                        .filter(|message| {
                            message.evidence.file != parent_thread.source_file
                                || message.evidence.byte_offset < end
                        })
                        .collect();
                }
            }
        }
        result.extend(messages(store, id)?);
        Ok(result)
    }
    collect(store, thread_id, &mut HashSet::new())
}
pub fn snapshot(store: &Store) -> Result<Snapshot> {
    let threads: Vec<Thread> = store.read("conversation-tree.json")?.unwrap_or_default();
    let tasks = store.list("tasks")?;
    let mut pending = 0;
    let mut diagnostics = Vec::new();
    for thread in &threads {
        let scan = state(store, &thread.id)?;
        pending += recent_pending(store, &thread.id, scan.extracted_count)?;
        if let Some(error) = scan.last_error {
            diagnostics.push(format!("{}: {error}", thread.title));
        }
    }
    Ok(Snapshot {
        threads,
        tasks,
        pending_messages: pending,
        diagnostics,
        last_extraction: store.read("latest-extraction.json")?,
    })
}
pub fn scan(store: &Store, source: &dyn ConversationSource) -> Result<Snapshot> {
    let mut threads = source.discover()?;
    let mut all_links = Vec::new();
    for thread in &threads {
        let mut scan = state(store, &thread.id)?;
        let normalization_changed =
            scan.normalization_version != NORMALIZATION_VERSION && scan.cursor.offset > 0;
        let read_cursor = if normalization_changed {
            Cursor {
                generation: scan.cursor.generation + 1,
                ..Cursor::default()
            }
        } else {
            scan.cursor.clone()
        };
        match source.read(thread, &read_cursor) {
            Ok(mut batch) => {
                batch.reset |= normalization_changed;
                if batch.reset || !batch.messages.is_empty() {
                    scan.dirty_revision += 1;
                    scan.last_changed_at = chrono::Utc::now().timestamp_millis();
                }
                if !batch.reset
                    && batch.cursor == scan.cursor
                    && store
                        .read::<Thread>(&Store::thread_key(&thread.id, "thread"))?
                        .as_ref()
                        == Some(thread)
                {
                    all_links.extend(
                        store
                            .read::<Vec<LineageLink>>(&Store::thread_key(&thread.id, "links"))?
                            .unwrap_or_default(),
                    );
                    continue;
                }
                let mut writes = vec![];
                if batch.reset {
                    // Old extraction remains auditable. Remove only this thread's obsolete episodes.
                    for mut task in store.list::<Task>("tasks")? {
                        let before = task.episodes.len();
                        task.episodes.retain(|e| e.thread_id != thread.id);
                        if task.episodes.len() != before {
                            let ids: HashSet<_> =
                                task.episodes.iter().map(|e| e.id.clone()).collect();
                            task.edges
                                .retain(|edge| ids.contains(&edge.from) && ids.contains(&edge.to));
                            task.revision += 1;
                            writes.push((Store::task_key(&task.id), json!(task)));
                        }
                    }
                    scan.message_count = 0;
                    scan.extracted_count = 0;
                }
                let mut appended = batch.messages;
                let first_chunk = scan.message_count / CHUNK_SIZE;
                if scan.message_count % CHUNK_SIZE > 0 {
                    let mut tail: Vec<Message> = store
                        .read(&event_key(&thread.id, scan.cursor.generation, first_chunk))?
                        .ok_or("Missing tail chunk")?;
                    tail.append(&mut appended);
                    appended = tail;
                }
                let old_tail = scan.message_count % CHUNK_SIZE;
                scan.message_count += appended.len() - old_tail;
                for (index, chunk) in appended.chunks(CHUNK_SIZE).enumerate() {
                    writes.push((
                        event_key(&thread.id, batch.cursor.generation, first_chunk + index),
                        json!(chunk),
                    ));
                }
                if batch.reset || !batch.links.is_empty() {
                    let mut links = if batch.reset {
                        vec![]
                    } else {
                        store
                            .read::<Vec<LineageLink>>(&Store::thread_key(&thread.id, "links"))?
                            .unwrap_or_default()
                    };
                    links.extend(batch.links);
                    writes.push((Store::thread_key(&thread.id, "links"), json!(links)));
                }
                scan.cursor = batch.cursor;
                scan.schema_version = 1;
                scan.normalization_version = NORMALIZATION_VERSION;
                scan.last_error = None;
                writes.push((Store::thread_key(&thread.id, "scan"), json!(scan)));
                writes.push((Store::thread_key(&thread.id, "thread"), json!(thread)));
                store.commit(writes)?;
            }
            Err(error) => {
                scan.last_error = Some(error);
                store.write(&Store::thread_key(&thread.id, "scan"), &scan)?;
            }
        }
        all_links.extend(
            store
                .read::<Vec<LineageLink>>(&Store::thread_key(&thread.id, "links"))?
                .unwrap_or_default(),
        );
    }
    for link in &all_links {
        if link.kind == "message" {
            continue;
        }
        if let Some(thread) = threads.iter_mut().find(|t| t.id == link.child_id) {
            if thread.parent_id.is_none() {
                thread.parent_id = Some(link.parent_id.clone());
                thread.creation_kind = link.kind.clone();
                thread.created_by = "agent".into();
            }
        }
    }
    store.commit(vec![
        ("conversation-tree.json".into(), json!(threads)),
        ("lineage.json".into(), json!(all_links)),
    ])?;
    snapshot(store)
}
fn root(thread_id: &str, threads: &[Thread]) -> String {
    let mut current = thread_id.to_owned();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        let Some(parent) = threads
            .iter()
            .find(|t| t.id == current)
            .and_then(|t| t.parent_id.clone())
        else {
            break;
        };
        if !threads.iter().any(|t| t.id == parent) {
            break;
        }
        current = parent;
    }
    current
}
fn recent_pending(store: &Store, thread_id: &str, extracted: usize) -> Result<usize> {
    let now = chrono::Utc::now().timestamp_millis();
    Ok(messages(store, thread_id)?
        .iter()
        .skip(extracted)
        .filter(|message| recent_message(message, now))
        .count())
}

pub fn pending_for(store: &Store, view: &Snapshot, selected: Option<&str>) -> Result<usize> {
    let mut count = 0;
    for thread in view.threads.iter().filter(|thread| {
        selected.is_none_or(|id| id == thread.id || root(&thread.id, &view.threads) == id)
    }) {
        let scan = state(store, &thread.id)?;
        count += recent_pending(store, &thread.id, scan.extracted_count)?;
    }
    Ok(count)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDirty {
    pub thread_id: String,
    pub state: String,
    pub revision: u64,
    pub extracted_revision: u64,
    pub pending_messages: usize,
    pub last_changed_at: i64,
    pub last_error: Option<String>,
}
pub fn dirty_states(store: &Store) -> Result<Vec<ConversationDirty>> {
    let threads: Vec<Thread> = store.read("conversation-tree.json")?.unwrap_or_default();
    let jobs = crate::management::jobs(store)?;
    threads
        .iter()
        .map(|thread| {
            let scan = state(store, &thread.id)?;
            let pending = recent_pending(store, &thread.id, scan.extracted_count)?;
            let active = jobs.iter().any(|job| {
                matches!(job.state.as_str(), "queued" | "running")
                    && job
                        .thread_id
                        .as_deref()
                        .is_none_or(|id| id == thread.id || root(&thread.id, &threads) == id)
            });
            Ok(ConversationDirty {
                thread_id: thread.id.clone(),
                state: if active {
                    "extracting"
                } else if scan.last_error.is_some() {
                    "error"
                } else if pending > 0 {
                    "dirty"
                } else {
                    "clean"
                }
                .into(),
                revision: scan.dirty_revision,
                extracted_revision: scan.extracted_revision,
                pending_messages: pending,
                last_changed_at: scan.last_changed_at,
                last_error: scan.last_error,
            })
        })
        .collect()
}
pub fn ready_thread(
    store: &Store,
    view: &Snapshot,
    selected: Option<&str>,
    debounce_seconds: u64,
) -> Result<Option<String>> {
    let now = chrono::Utc::now().timestamp_millis();
    for thread in &view.threads {
        if !selected.is_none_or(|id| id == thread.id || root(&thread.id, &view.threads) == id) {
            continue;
        }
        let scan = state(store, &thread.id)?;
        if now - scan.last_changed_at >= debounce_seconds as i64 * 1000
            && recent_pending(store, &thread.id, scan.extracted_count)? > 0
        {
            return Ok(Some(thread.id.clone()));
        }
    }
    Ok(None)
}

/// Executes one bounded job. A failure does not advance extracted_count.
pub fn extract_next(
    store: &Store,
    extractor: &dyn TaskExtractor,
    selected_thread: Option<&str>,
) -> Result<Snapshot> {
    extract_scoped(store, extractor, selected_thread, true)
}
pub fn extract_thread(
    store: &Store,
    extractor: &dyn TaskExtractor,
    thread: &str,
) -> Result<Snapshot> {
    extract_scoped(store, extractor, Some(thread), false)
}
fn extract_scoped(
    store: &Store,
    extractor: &dyn TaskExtractor,
    selected_thread: Option<&str>,
    descendants: bool,
) -> Result<Snapshot> {
    let _lease = store.extraction_lease()?;
    let view = snapshot(store)?;
    if selected_thread.is_some_and(|id| !view.threads.iter().any(|thread| thread.id == id)) {
        return Err("抽取范围对应的会话不存在，请重新扫描并选择会话".into());
    }
    let mut ordered = view.threads.clone();
    ordered.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    for thread in ordered.iter().filter(|t| {
        selected_thread
            .is_none_or(|id| id == t.id || (descendants && root(&t.id, &view.threads) == id))
    }) {
        let mut scan = state(store, &thread.id)?;
        if scan.extracted_count >= scan.message_count {
            continue;
        }
        let all = messages(store, &thread.id)?;
        let input_revision = scan.dirty_revision;
        let input_message_count = scan.message_count;
        let now = chrono::Utc::now().timestamp_millis();
        let eligible: Vec<usize> = all
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                matches!(message.role.as_str(), "user" | "assistant")
                    && recent_message(message, now)
            })
            .map(|(index, _)| index)
            .collect();
        let Some(first_new) = eligible
            .iter()
            .position(|index| *index >= scan.extracted_count)
        else {
            scan.extracted_count = all.len();
            scan.extracted_revision = input_revision;
            scan.last_error = None;
            store.write(&Store::thread_key(&thread.id, "scan"), &scan)?;
            continue;
        };
        // Only recent messages may supply overlap; old history never fills the context.
        let start_position = first_new.saturating_sub(1);
        let start = eligible[start_position];
        let mut end = scan.extracted_count;
        let mut chars = 0;
        let mut input = vec![];
        for index in eligible.into_iter().skip(start_position) {
            let n = all[index].text.chars().count().min(2500);
            if index >= scan.extracted_count
                && !input.is_empty()
                && end > scan.extracted_count
                && (input.len() >= 6 || chars + n > 5000)
            {
                break;
            }
            chars += n;
            end = index + 1;
            input.push(all[index].clone());
        }
        for message in &mut input {
            if message.text.chars().count() > 2500 {
                message.text = message.text.chars().take(2500).collect::<String>()
                    + "\n[Input excerpt ends here; remaining text was not supplied.]";
            }
        }
        let root_id = root(&thread.id, &view.threads);
        let mut candidates: Vec<_> = view
            .tasks
            .iter()
            .filter(|t| t.root_thread_id == root_id)
            .cloned()
            .collect();
        let job_id = stable_id(format!(
            "{}:{}:{start}:{end}:{}:{}",
            thread.id,
            scan.cursor.generation,
            extractor.version(),
            stable_id(serde_json::to_vec(&input).map_err(|e| e.to_string())?)
        ));
        let result = match store.without_writer_lock(|| extractor.extract(&input, &candidates))? {
            Ok(value) => value,
            Err(error) => {
                let mut current = state(store, &thread.id)?;
                current.last_error = Some(error.clone());
                store.write(&Store::thread_key(&thread.id, "scan"), &current)?;
                return Err(error);
            }
        };
        let current_scan = state(store, &thread.id)?;
        if current_scan.cursor.generation != scan.cursor.generation
            || current_scan.extracted_count != scan.extracted_count
        {
            return Err(
                "Source or extraction cursor changed; result retained as pending for retry".into(),
            );
        }
        let current_tasks: Vec<Task> = store.list("tasks")?;
        if candidates.iter().any(|task| {
            current_tasks
                .iter()
                .find(|current| current.id == task.id)
                .is_none_or(|current| current.revision != task.revision)
        }) || current_tasks.iter().any(|current| {
            current.root_thread_id == root_id
                && !candidates.iter().any(|task| task.id == current.id)
        }) {
            return Err("Task state changed during extraction; retry with current state".into());
        }
        scan = current_scan;
        validate(&result.extraction, &input, &candidates)?;
        let source: HashMap<_, _> = input
            .iter()
            .map(|m| (m.evidence.event_id.as_str(), m))
            .collect();
        let incoming: Vec<u64> = store
            .read::<Vec<LineageLink>>(&Store::thread_key(&thread.id, "links"))?
            .unwrap_or_default()
            .iter()
            .filter(|link| link.kind == "message")
            .map(|link| link.evidence.byte_offset)
            .collect();
        for delta in &result.extraction.tasks {
            let evidence_ids: HashSet<_> = delta
                .episodes
                .iter()
                .flat_map(|e| e.evidence_ids.iter())
                .collect();
            let evidence_owner = candidates
                .iter()
                .find(|t| {
                    t.title.trim() == delta.title.trim()
                        && t.episodes
                            .iter()
                            .any(|e| e.evidence_ids.iter().any(|id| evidence_ids.contains(id)))
                })
                .map(|t| t.id.clone());
            let task_id = delta
                .existing_task_id
                .clone()
                .or(evidence_owner)
                .unwrap_or_else(|| {
                    stable_id(format!(
                        "task:{root_id}:{}:{}",
                        delta.episodes[0].evidence_ids[0],
                        delta.title.trim()
                    ))
                });
            let index = if let Some(index) = candidates.iter().position(|t| t.id == task_id) {
                index
            } else {
                candidates.push(Task {
                    schema_version: 1,
                    revision: 0,
                    id: task_id.clone(),
                    root_thread_id: root_id.clone(),
                    title: delta.title.clone(),
                    detail: delta.detail.clone(),
                    episodes: vec![],
                    edges: vec![],
                    manual_completion: false,
                    completion_watermarks: HashMap::new(),
                });
                candidates.len() - 1
            };
            let task = &mut candidates[index];
            let mut split_episodes = vec![];
            for episode in &delta.episodes {
                let mut groups = std::collections::BTreeMap::<usize, Vec<String>>::new();
                for id in &episode.evidence_ids {
                    let offset = source[id.as_str()].evidence.byte_offset;
                    groups
                        .entry(
                            incoming
                                .iter()
                                .filter(|boundary| **boundary <= offset)
                                .count(),
                        )
                        .or_default()
                        .push(id.clone());
                }
                for evidence_ids in groups.into_values() {
                    split_episodes.push(crate::extraction::EpisodeDelta {
                        title: episode.title.clone(),
                        evidence_ids,
                    });
                }
            }
            for episode in &split_episodes {
                let mut evidence: Vec<_> = episode
                    .evidence_ids
                    .iter()
                    .filter(|id| !task.episodes.iter().any(|e| e.evidence_ids.contains(id)))
                    .cloned()
                    .collect();
                evidence.sort_by_key(|id| source[id.as_str()].evidence.byte_offset);
                if evidence.is_empty() {
                    continue;
                }
                if task.manual_completion
                    && evidence.iter().any(|id| {
                        let message = source[id.as_str()];
                        message.role == "user"
                            && task
                                .completion_watermarks
                                .get(&thread.id)
                                .is_some_and(|cursor| {
                                    cursor.generation == message.evidence.generation
                                        && message.evidence.byte_offset >= cursor.offset
                                })
                    })
                {
                    task.manual_completion = false;
                }
                let first = source[evidence[0].as_str()];
                let last = source[evidence.last().unwrap().as_str()];
                let id = stable_id(format!("episode:{task_id}:{}", evidence[0]));
                // Sequential messages in the same turn extend an episode; a stopped/restarted turn gets a new node.
                let prior = task.episodes.last_mut().filter(|e| {
                    e.thread_id == thread.id
                        && e.evidence_ids
                            .last()
                            .and_then(|id| all.iter().find(|m| m.evidence.event_id == *id))
                            .is_some_and(|m| {
                                m.turn_id.is_some()
                                    && m.turn_id == first.turn_id
                                    && !incoming.iter().any(|offset| {
                                        *offset > m.evidence.byte_offset
                                            && *offset <= first.evidence.byte_offset
                                    })
                            })
                });
                if let Some(prior) = prior {
                    prior.evidence_ids.extend(evidence);
                    prior.ended_at = last.timestamp.clone();
                } else {
                    if let Some(previous) =
                        task.episodes.last().filter(|e| e.thread_id == thread.id)
                    {
                        task.edges.push(Edge {
                            from: previous.id.clone(),
                            to: id.clone(),
                            evidence_ids: vec![evidence[0].clone()],
                        });
                    }
                    task.episodes.push(Episode {
                        id,
                        thread_id: thread.id.clone(),
                        title: episode.title.clone(),
                        evidence_ids: evidence,
                        started_at: first.timestamp.clone(),
                        ended_at: last.timestamp.clone(),
                    });
                }
            }
            task.revision += 1;
        }
        let links: Vec<LineageLink> = store.read("lineage.json")?.unwrap_or_default();
        let mut event_offsets = HashMap::new();
        for id in candidates
            .iter()
            .flat_map(|task| {
                task.episodes
                    .iter()
                    .map(|episode| episode.thread_id.clone())
            })
            .collect::<HashSet<_>>()
        {
            for message in messages(store, &id)? {
                event_offsets.insert(message.evidence.event_id, message.evidence.byte_offset);
            }
        }
        for task in &mut candidates {
            for link in &links {
                let target = task
                    .episodes
                    .iter()
                    .filter(|episode| episode.thread_id == link.child_id)
                    .filter_map(|episode| {
                        episode
                            .evidence_ids
                            .iter()
                            .filter_map(|id| event_offsets.get(id))
                            .filter(|offset| **offset >= link.evidence.byte_offset)
                            .min()
                            .map(|offset| (episode, *offset))
                    })
                    .min_by_key(|(_, offset)| *offset)
                    .map(|(episode, _)| episode);
                let sources: Vec<_> = task
                    .episodes
                    .iter()
                    .filter(|episode| episode.thread_id == link.parent_id)
                    .collect();
                let origin = if let Some(timestamp) = &link.timestamp {
                    sources
                        .iter()
                        .copied()
                        .filter(|episode| {
                            episode
                                .started_at
                                .as_ref()
                                .is_some_and(|start| start <= timestamp)
                        })
                        .max_by_key(|episode| episode.started_at.clone())
                } else if sources.len() == 1 {
                    Some(sources[0])
                } else {
                    None
                };
                if let (Some(origin), Some(target)) = (origin, target) {
                    if origin.id != target.id
                        && !task
                            .edges
                            .iter()
                            .any(|edge| edge.from == origin.id && edge.to == target.id)
                    {
                        task.edges.push(Edge {
                            from: origin.id.clone(),
                            to: target.id.clone(),
                            evidence_ids: vec![link.evidence.event_id.clone()],
                        });
                    }
                }
            }
        }
        let mut writes: Vec<(String, Value)> = candidates
            .iter()
            .map(|t| (Store::task_key(&t.id), json!(t)))
            .collect();
        scan.extracted_count = end;
        if end >= input_message_count {
            scan.extracted_revision = input_revision;
        }
        scan.last_error = None;
        writes.push((Store::thread_key(&thread.id, "scan"), json!(scan)));
        writes.push((format!("extraction-revisions/{job_id}.json"),json!({"schemaVersion":1,"jobId":job_id,"input":input,"result":result,"version":extractor.version(),"lookbackHours":LOOKBACK_HOURS,"windowEndMs":now})));
        writes.push(("latest-extraction.json".into(),json!({"requestedModel":result.requested_model,"modelUsage":result.model_usage,"reportedCostUsd":result.reported_cost_usd})));
        store.commit(writes)?;
        return snapshot(store);
    }
    Ok(view)
}
pub fn set_completion(
    store: &Store,
    id: &str,
    expected_revision: u64,
    completed: bool,
) -> Result<Task> {
    let mut task: Task = store.read(&Store::task_key(id))?.ok_or("Task not found")?;
    if task.revision != expected_revision {
        return Err("Task changed; reload before confirming".into());
    }
    task.manual_completion = completed;
    if completed {
        for episode in &task.episodes {
            task.completion_watermarks.insert(
                episode.thread_id.clone(),
                state(store, &episode.thread_id)?.cursor,
            );
        }
    }
    task.revision += 1;
    store.write(&Store::task_key(id), &task)?;
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        extraction::{EpisodeDelta, Extraction, ExtractionResult, TaskDelta},
        sources::codex::CodexSource,
    };
    struct Extractor {
        fail: bool,
    }
    impl TaskExtractor for Extractor {
        fn version(&self) -> String {
            "fixture-v1".into()
        }
        fn extract(&self, messages: &[Message], candidates: &[Task]) -> Result<ExtractionResult> {
            if self.fail {
                return Err("offline".into());
            }
            Ok(ExtractionResult {
                requested_model: "fixture".into(),
                model_usage: json!({}),
                reported_cost_usd: None,
                extraction: Extraction {
                    tasks: vec![TaskDelta {
                        existing_task_id: candidates.first().map(|t| t.id.clone()),
                        title: "修复焦点".into(),
                        detail: "键盘能进入登录框".into(),
                        episodes: vec![EpisodeDelta {
                            title: "实现".into(),
                            evidence_ids: messages
                                .iter()
                                .map(|m| m.evidence.event_id.clone())
                                .collect(),
                        }],
                    }],
                },
            })
        }
    }
    fn fixture() -> (tempfile::TempDir, Store, CodexSource) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        let rows = [
            json!({"type":"session_meta","payload":{"id":"thread-a","cwd":"/repo","thread_source":"user"}}),
            json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"修复焦点"}]}}),
        ];
        std::fs::write(
            home.join("sessions/a.jsonl"),
            rows.iter()
                .map(|r| r.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        (dir, store, CodexSource { home })
    }
    #[test]
    fn failed_job_can_retry_and_rescans_preserve_manual_completion() {
        let (_dir, store, source) = fixture();
        assert_eq!(scan(&store, &source).unwrap().pending_messages, 1);
        assert!(extract_next(&store, &Extractor { fail: true }, None).is_err());
        assert_eq!(snapshot(&store).unwrap().pending_messages, 1);
        let view = extract_next(&store, &Extractor { fail: false }, None).unwrap();
        assert_eq!(view.pending_messages, 0);
        assert_eq!(view.tasks.len(), 1);
        let task = &view.tasks[0];
        set_completion(&store, &task.id, task.revision, true).unwrap();
        assert!(set_completion(&store, &task.id, task.revision, false).is_err());
        scan(&store, &source).unwrap();
        let view = extract_next(&store, &Extractor { fail: false }, None).unwrap();
        assert_eq!(view.tasks.len(), 1);
        assert!(view.tasks[0].manual_completion);
        assert_eq!(view.tasks[0].episodes.len(), 1);
    }
    #[test]
    fn manifest_chunks_handle_append_across_two_hundred_messages() {
        let (_dir, store, source) = fixture();
        let path = source.home.join("sessions/a.jsonl");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        for index in 0..201 {
            writeln!(file,"{}",json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":format!("message {index}")}]}})).unwrap();
        }
        scan(&store, &source).unwrap();
        assert_eq!(messages(&store, "thread-a").unwrap().len(), 202);
        writeln!(file,"{}",json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"next"}]}})).unwrap();
        scan(&store, &source).unwrap();
        let result = messages(&store, "thread-a").unwrap();
        assert_eq!(result.len(), 203);
        assert_eq!(result.last().unwrap().text, "next");
    }
    #[test]
    fn tools_do_not_dirty_and_debounce_does_not_spend_budget() {
        let (_dir, store, source) = fixture();
        scan(&store, &source).unwrap();
        let revision = state(&store, "thread-a").unwrap().dirty_revision;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(source.home.join("sessions/a.jsonl"))
            .unwrap();
        writeln!(file,"{}",json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"function_call","name":"exec","arguments":"do not extract"}})).unwrap();
        scan(&store, &source).unwrap();
        assert_eq!(state(&store, "thread-a").unwrap().dirty_revision, revision);
        store
            .write(
                "extraction-settings.json",
                &crate::management::ExtractionSettings::default(),
            )
            .unwrap();
        crate::watch::configure(
            &store,
            crate::watch::WatchConfig {
                enabled: true,
                extract: true,
                thread_id: Some("thread-a".into()),
                ..Default::default()
            },
        )
        .unwrap();
        crate::watch::tick(&store, &source, Some(&Extractor { fail: false })).unwrap();
        assert_eq!(crate::watch::read(&store).unwrap().remaining_jobs, 5);
        assert!(crate::management::jobs(&store).unwrap().is_empty());
        let mut s = state(&store, "thread-a").unwrap();
        s.last_changed_at -= 21000;
        store
            .write(&Store::thread_key("thread-a", "scan"), &s)
            .unwrap();
        crate::watch::tick(&store, &source, Some(&Extractor { fail: false })).unwrap();
        assert_eq!(crate::watch::read(&store).unwrap().remaining_jobs, 4);
        assert_eq!(
            crate::management::jobs(&store).unwrap()[0].state,
            "completed"
        );
        let dirty = dirty_states(&store).unwrap();
        assert_eq!(dirty[0].state, "clean");
        assert_eq!(dirty[0].revision, dirty[0].extracted_revision);
    }
    #[test]
    fn messages_arriving_during_inference_remain_dirty() {
        let (dir, store, source) = fixture();
        scan(&store, &source).unwrap();
        struct Append {
            root: std::path::PathBuf,
            home: std::path::PathBuf,
        }
        impl TaskExtractor for Append {
            fn version(&self) -> String {
                "append".into()
            }
            fn extract(&self, input: &[Message], tasks: &[Task]) -> Result<ExtractionResult> {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(self.home.join("sessions/a.jsonl"))
                    .unwrap();
                writeln!(file,"{}",json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"继续修复"}]}})).unwrap();
                let writer = Store::open(&self.root)?;
                scan(
                    &writer,
                    &CodexSource {
                        home: self.home.clone(),
                    },
                )?;
                Extractor { fail: false }.extract(input, tasks)
            }
        }
        extract_next(
            &store,
            &Append {
                root: dir.path().join("store"),
                home: source.home,
            },
            None,
        )
        .unwrap();
        let dirty = dirty_states(&store).unwrap();
        assert_eq!(dirty[0].state, "dirty");
        assert_eq!(dirty[0].pending_messages, 1);
        assert!(dirty[0].revision > dirty[0].extracted_revision);
    }
    #[test]
    fn scheduled_agent_continues_past_five_batches_and_pauses_on_failure() {
        let (dir, store, source) = fixture();
        let layout = crate::management::Layout::initialize(&dir.path().join("data")).unwrap();
        crate::management::save_settings(
            &store,
            crate::management::ExtractionSettings {
                automatic: true,
                debounce_seconds: 0,
                reasoning_effort: "medium".into(),
                ..Default::default()
            },
            &layout,
        )
        .unwrap();
        use std::io::Write;
        for index in 0..7 {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(source.home.join("sessions/a.jsonl"))
                .unwrap();
            writeln!(file,"{}",json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":format!("继续验证 {index}")}]}})).unwrap();
            let result = crate::watch::tick(&store, &source, Some(&Extractor { fail: index == 6 }));
            if index < 6 {
                result.unwrap();
                assert!(crate::management::settings(&store).unwrap().automatic);
            } else {
                assert!(result.is_err());
            }
        }
        assert_eq!(crate::management::jobs(&store).unwrap().len(), 7);
        assert!(!crate::management::settings(&store).unwrap().automatic);
        assert!(crate::watch::read(&store).unwrap().last_error.is_some());
        assert_eq!(
            crate::management::settings(&store)
                .unwrap()
                .reasoning_effort,
            "medium"
        );
    }
    #[test]
    fn scheduled_root_does_not_extract_a_child_still_in_debounce() {
        let (_dir, store, source) = fixture();
        let rows = [
            json!({"type":"session_meta","payload":{"id":"child","cwd":"/repo","source":{"subagent":{"thread_spawn":{"parent_thread_id":"thread-a"}}}}}),
            json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"验证焦点修复"}]}}),
        ];
        std::fs::write(
            source.home.join("sessions/child.jsonl"),
            rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        scan(&store, &source).unwrap();
        let mut tree: Vec<Thread> = store.read("conversation-tree.json").unwrap().unwrap();
        for thread in &mut tree {
            thread.timestamp = Some(
                if thread.id == "child" {
                    "2020-01-01"
                } else {
                    "2021-01-01"
                }
                .into(),
            );
        }
        store.write("conversation-tree.json", &tree).unwrap();
        let mut parent = state(&store, "thread-a").unwrap();
        parent.last_changed_at -= 21000;
        store
            .write(&Store::thread_key("thread-a", "scan"), &parent)
            .unwrap();
        let ready = ready_thread(&store, &snapshot(&store).unwrap(), Some("thread-a"), 20)
            .unwrap()
            .unwrap();
        assert_eq!(ready, "thread-a");
        extract_thread(&store, &Extractor { fail: false }, &ready).unwrap();
        assert_eq!(state(&store, "child").unwrap().extracted_count, 0);
        assert_eq!(state(&store, "thread-a").unwrap().extracted_count, 1);
        assert!(extract_thread(&store, &Extractor { fail: false }, "absent").is_err());
    }
    #[test]
    fn model_wait_does_not_lock_readers_and_stale_results_cannot_override_acceptance() {
        let (dir, store, source) = fixture();
        scan(&store, &source).unwrap();
        let view = extract_next(&store, &Extractor { fail: false }, None).unwrap();
        let mut current = state(&store, "thread-a").unwrap();
        current.extracted_count = 0;
        store
            .write(&Store::thread_key("thread-a", "scan"), &current)
            .unwrap();
        struct ConcurrentAcceptance {
            root: std::path::PathBuf,
            task: Task,
        }
        impl TaskExtractor for ConcurrentAcceptance {
            fn version(&self) -> String {
                "concurrent".into()
            }
            fn extract(
                &self,
                messages: &[Message],
                candidates: &[Task],
            ) -> Result<ExtractionResult> {
                let reader = Store::read_only(&self.root)?;
                assert_eq!(snapshot(&reader)?.tasks.len(), 1);
                drop(reader);
                let writer = Store::open(&self.root)?;
                assert!(writer.extraction_lease().is_err());
                set_completion(&writer, &self.task.id, self.task.revision, true)?;
                drop(writer);
                Extractor { fail: false }.extract(messages, candidates)
            }
        }
        let result = extract_next(
            &store,
            &ConcurrentAcceptance {
                root: dir.path().join("store"),
                task: view.tasks[0].clone(),
            },
            None,
        );
        assert!(result.unwrap_err().contains("changed"));
        let view = snapshot(&store).unwrap();
        assert!(view.tasks[0].manual_completion);
        assert_eq!(view.pending_messages, 1);
    }
}
