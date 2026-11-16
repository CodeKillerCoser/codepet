use codepet_host::event_journal::{EventJournal, JournalPage, JournalQuery};
use std::sync::{Arc, OnceLock};

static JOURNAL: OnceLock<Result<Arc<EventJournal>, String>> = OnceLock::new();

pub fn journal() -> Result<&'static Arc<EventJournal>, String> {
    JOURNAL.get_or_init(|| crate::paths::current().and_then(|paths| EventJournal::open(paths.logs()))
        .map_err(|error| error.to_string())).as_ref().map_err(Clone::clone)
}

#[tauri::command]
pub async fn query_event_journal(query: JournalQuery) -> Result<JournalPage, String> {
    let journal = journal()?.clone();
    tauri::async_runtime::spawn_blocking(move || journal.query(query).map_err(|error| error.to_string()))
        .await.map_err(|error| error.to_string())?
}
