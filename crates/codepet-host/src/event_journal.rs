//! Diagnostic events: one writer, rotating JSONL history and an in-memory live cursor.
mod history;
pub use history::HistoryCursor;
use history::HistoryStore;
use codepet_provider_sdk::ProtocolEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::VecDeque, fs, io, path::PathBuf,
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}, mpsc::{self, SyncSender}}, time::{SystemTime, UNIX_EPOCH}};

const FILE_BYTES: u64 = 5 * 1024 * 1024;
const ARCHIVES: usize = 5;
const LIVE_BYTES: usize = 8 * 1024 * 1024;
const LIVE_COUNT: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEvent {
    pub sequence: u64,
    pub received_at: u64,
    pub source: String,
    pub provider: String,
    pub payload: Value,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalQuery {
    pub before: Option<u64>,
    pub history_cursor: Option<HistoryCursor>,
    pub after: Option<u64>,
    pub keyword: Option<String>,
    pub source: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalPage {
    pub events: Vec<JournalEvent>,
    pub cursor: u64,
    pub next_before: Option<u64>,
    pub next_history_cursor: Option<HistoryCursor>,
    pub reset_reason: Option<String>,
    pub has_more: bool,
    pub reset_required: bool,
    pub dropped: u64,
    pub error: Option<String>,
}

enum Input {
    Hook(String, Value),
    Provider(String, ProtocolEvent),
}
struct State {
    history: HistoryStore,
    sequence: u64,
    live: VecDeque<(JournalEvent, String, usize)>,
    live_bytes: usize,
    error: Option<String>,
    day: u64,
    max_bytes: u64,
}

pub struct EventJournal {
    sender: SyncSender<(u64, Input)>,
    dropped: Arc<AtomicU64>,
    state: Arc<Mutex<State>>,
}

impl EventJournal {
    pub fn open(directory: PathBuf) -> io::Result<Arc<Self>> {
        Self::open_with_limit(directory, FILE_BYTES)
    }

    fn open_with_limit(directory: PathBuf, max_bytes: u64) -> io::Result<Arc<Self>> {
        fs::create_dir_all(&directory)?;
        let active = directory.join("events.jsonl");
        let day = fs::metadata(&active).and_then(|m| m.modified()).ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|t| t.as_secs() / 86400).unwrap_or(now() / 86_400_000);
        let history = HistoryStore::open(directory, ARCHIVES)?;
        let state = State { sequence: history.last_sequence(), history, live: VecDeque::new(), live_bytes: 0, error: None, day, max_bytes };
        let state = Arc::new(Mutex::new(state));
        let (sender, receiver) = mpsc::sync_channel(256);
        let writer = state.clone();
        let dropped = Arc::new(AtomicU64::new(0));
        let writer_dropped = dropped.clone();
        std::thread::Builder::new().name("event-journal".into()).spawn(move || {
            while let Ok((received_at, input)) = receiver.recv() {
                let (source, provider, payload) = match input {
                    Input::Hook(provider, payload) => ("hook", provider, payload),
                    Input::Provider(provider, event) => {
                        // Typed event already decoded for business processing; no wire decode here.
                        match serde_json::to_value(event) {
                            Ok(payload) => ("provider", provider, payload),
                            Err(error) => { writer_dropped.fetch_add(1, Ordering::Relaxed); writer.lock().unwrap().error = Some(error.to_string()); continue; }
                        }
                    }
                };
                let mut state = writer.lock().unwrap();
                if let Err(error) = state.append(received_at, source, provider, payload) {
                    writer_dropped.fetch_add(1, Ordering::Relaxed);
                    state.error = Some(error.to_string());
                }
            }
        })?;
        Ok(Arc::new(Self { sender, state, dropped }))
    }

    fn submit(&self, input: Input) {
        if self.sender.try_send((now(), input)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn hook(&self, provider: &str, payload: Value) {
        self.submit(Input::Hook(provider.to_owned(), payload));
    }

    pub fn provider(&self, provider: &str, event: &ProtocolEvent) {
        if let ProtocolEvent::EventNotification { params, .. } = event {
            // Observation's envelope preserves the original object, including arbitrary keys.
            if let Some(raw) = params.payload.get("codepet_observation").and_then(|v| v.get("raw")) {
                self.hook(provider, raw.clone());
            }
        }
        self.submit(Input::Provider(provider.to_owned(), event.clone()));
    }

    pub fn query(&self, query: JournalQuery) -> io::Result<JournalPage> {
        let state = self.state.lock().unwrap();
        let limit = query.limit.unwrap_or(100).clamp(1, 200);
        let keyword = query.keyword.unwrap_or_default().to_lowercase();
        let matches = |record: &JournalEvent, text: &str| {
            query.source.as_ref().filter(|s| !s.is_empty()).map_or(true, |s| s == &record.source)
                && (keyword.is_empty() || text.contains(&keyword))
        };
        let mut page = JournalPage { events: vec![], cursor: state.sequence, next_before: None, next_history_cursor: None, reset_reason: None,
            has_more: false, reset_required: false, dropped: self.dropped.load(Ordering::Relaxed), error: state.error.clone() };
        if let Some(after) = query.after {
            page.reset_required = after > state.sequence || (after < state.sequence && state.live.front().map_or(true, |(e, _, _)| after < e.sequence.saturating_sub(1)));
            if page.reset_required { page.reset_reason = Some("live_expired".into()); return Ok(page); }
            for (record, text, _) in &state.live {
                if record.sequence <= after { continue; }
                if matches(record, text) {
                    if page.events.len() == limit {
                        page.has_more = true;
                        page.cursor = page.events.last().unwrap().sequence;
                        break;
                    }
                    page.events.push(record.clone());
                }
            }
        } else {
            let snapshot = state.history.snapshot(query.history_cursor.as_ref(), query.before)?;
            drop(state);
            let Some(snapshot) = snapshot else {
                page.reset_required = true;
                page.reset_reason = Some("history_expired".into());
                return Ok(page);
            };
            let history = snapshot.read(limit, &keyword, query.source.as_deref())?;
            page.events = history.events;
            page.has_more = history.next.is_some();
            page.next_history_cursor = history.next;
            page.next_before = page.events.last().map(|e| e.sequence);
        }
        Ok(page)
    }
}

impl State {
    #[cfg(test)]
    fn path(&self, index: usize) -> PathBuf { self.history.path(index) }

    fn append(&mut self, received_at: u64, source: &str, provider: String, payload: Value) -> io::Result<()> {
        let record = JournalEvent { sequence: self.sequence + 1, received_at, source: source.into(), provider, payload };
        let line = serde_json::to_string(&record)?;
        let size = self.history.bytes;
        let day = received_at / 86_400_000;
        if size > 0 && (size + line.len() as u64 + 1 > self.max_bytes || day != self.day) {
            self.history.rotate()?;
        }
        self.history.append(record.sequence, &line)?;
        if let Some(error) = self.history.index_error.as_ref() { self.error = Some(error.clone()); }
        self.day = day;
        self.sequence = record.sequence;
        let searchable = line.to_lowercase();
        self.live_bytes += line.len() + searchable.len();
        self.live.push_back((record, searchable, line.len()));
        while self.live.len() > LIVE_COUNT || self.live_bytes > LIVE_BYTES {
            if let Some((_, text, bytes)) = self.live.pop_front() {
                self.live_bytes = self.live_bytes.saturating_sub(bytes + text.len());
            }
        }
        Ok(())
    }
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs::OpenOptions, io::Write};

    #[test]
    fn provider_writer_records_raw_hook_and_typed_event_without_a_remote() {
        let directory = tempfile::tempdir().unwrap();
        let journal = EventJournal::open(directory.path().into()).unwrap();
        let raw = json!({"hook_event_name":"Stop", "prompt":"中文 原始", "codepet_gap":"user data"});
        let event = ProtocolEvent::EventNotification { jsonrpc: "2.0".into(), params: codepet_provider_sdk::ProviderNotificationEvent {
            subscription_id: "host".into(), event_id: "native-1".into(), received_at: 5,
            payload: json!({"codepet_observation":{"raw":raw,"gap":true}}).as_object().unwrap().clone().into_iter().collect(),
        } };
        journal.provider("dev.codepet.claude", &event);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let page = journal.query(JournalQuery::default()).unwrap();
            if page.cursor == 2 {
                assert_eq!(page.events[0].source, "provider");
                assert_eq!(page.events[0].payload, serde_json::to_value(&event).unwrap());
                assert_eq!(page.events[1].source, "hook");
                assert_eq!(page.events[1].payload, raw);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn queue_overflow_never_waits_for_the_file_lock_and_reports_loss() {
        let directory = tempfile::tempdir().unwrap();
        let journal = EventJournal::open(directory.path().into()).unwrap();
        let state = journal.state.lock().unwrap();
        for _ in 0..300 { journal.hook("claude", json!({"prompt":"queued"})); }
        assert!(journal.dropped.load(Ordering::Relaxed) > 0);
        drop(state);
    }

    #[test]
    fn date_rotation_partial_tail_and_live_page_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let journal = EventJournal::open(directory.path().into()).unwrap();
        {
            let mut state = journal.state.lock().unwrap();
            state.append(86_400_000, "hook", "claude".into(), json!({"text":"one"})).unwrap();
            state.append(172_800_000, "hook", "claude".into(), json!({"text":"two"})).unwrap();
            assert!(state.path(1).exists());
        }
        let first = journal.query(JournalQuery { after: Some(0), limit: Some(1), ..Default::default() }).unwrap();
        assert!(first.has_more); assert_eq!(first.cursor, 1);
        let next = journal.query(JournalQuery { after: Some(first.cursor), ..Default::default() }).unwrap();
        assert_eq!(next.events[0].sequence, 2);
        drop(journal);
        OpenOptions::new().append(true).open(directory.path().join("events.jsonl")).unwrap().write_all(b"{broken").unwrap();
        let journal = EventJournal::open(directory.path().into()).unwrap();
        journal.state.lock().unwrap().append(172_800_001, "provider", "codex".into(), json!({"text":"three"})).unwrap();
        assert_eq!(journal.query(JournalQuery::default()).unwrap().events.len(), 3);
    }

    #[test]
    fn history_rotation_filter_live_gap_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let journal = EventJournal::open_with_limit(directory.path().into(), 180).unwrap();
        {
            let mut state = journal.state.lock().unwrap();
            for i in 0..8 { state.append(now(), "hook", "claude".into(), json!({"prompt": format!("原始 KEY {i}"), "nested": {"n": 2}})).unwrap(); }
        }
        let first = journal.query(JournalQuery { keyword: Some("key".into()), limit: Some(2), ..Default::default() }).unwrap();
        assert_eq!(first.events.iter().map(|e| e.sequence).collect::<Vec<_>>(), [8, 7]);
        assert!(first.has_more);
        let older = journal.query(JournalQuery { history_cursor: first.next_history_cursor, ..Default::default() }).unwrap();
        assert_eq!(older.events.iter().map(|e| e.sequence).collect::<Vec<_>>(), [6, 5, 4, 3]);
        let live = journal.query(JournalQuery { after: Some(6), ..Default::default() }).unwrap();
        assert_eq!(live.events.iter().map(|e| e.sequence).collect::<Vec<_>>(), [7, 8]);
        assert_eq!(live.events[1].payload["nested"], json!({"n":2}));
        let empty = journal.query(JournalQuery { after: Some(6), keyword: Some("missing".into()), ..Default::default() }).unwrap();
        assert!(empty.events.is_empty()); assert_eq!(empty.cursor, 8);
        drop(journal);
        let reopened = EventJournal::open(directory.path().into()).unwrap();
        assert_eq!(reopened.query(JournalQuery::default()).unwrap().cursor, 8);
        assert!(reopened.query(JournalQuery { after: Some(6), ..Default::default() }).unwrap().reset_required);
        assert_eq!(fs::read_dir(directory.path()).unwrap().filter_map(Result::ok).filter(|entry| entry.file_name().to_string_lossy().ends_with(".jsonl")).count(), ARCHIVES + 1);
    }

    #[test]
    fn expired_history_cursor_returns_an_explicit_reset_reason() {
        let directory = tempfile::tempdir().unwrap();
        let journal = EventJournal::open_with_limit(directory.path().into(), 1).unwrap();
        {
            let mut state = journal.state.lock().unwrap();
            for _ in 0..3 { state.append(now(), "hook", "claude".into(), json!({"text":"first"})).unwrap(); }
        }
        let page = journal.query(JournalQuery { limit: Some(1), ..Default::default() }).unwrap();
        let cursor = page.next_history_cursor.unwrap();
        {
            let mut state = journal.state.lock().unwrap();
            for _ in 0..6 { state.append(now(), "hook", "claude".into(), json!({"text":"later"})).unwrap(); }
        }
        let page = journal.query(JournalQuery { history_cursor: Some(cursor), ..Default::default() }).unwrap();
        assert!(page.reset_required);
        assert_eq!(page.reset_reason.as_deref(), Some("history_expired"));
        assert!(page.events.is_empty());
    }
}
