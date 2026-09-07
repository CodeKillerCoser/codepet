use codepet_host::event_journal::{EventJournal, JournalPage, JournalQuery};
use std::sync::{Arc, OnceLock};

static JOURNAL: OnceLock<Result<Arc<EventJournal>, String>> = OnceLock::new();

pub fn journal() -> Result<&'static Arc<EventJournal>, String> {
    JOURNAL.get_or_init(|| EventJournal::open(crate::settings::current_app_data_dir().join("logs"))
        .map_err(|error| error.to_string())).as_ref().map_err(Clone::clone)
}

pub fn record_hook(provider: &str, payload: serde_json::Value) {
    let Some(journal) = JOURNAL.get() else { return; };
    match journal {
        Ok(journal) => journal.hook(provider, payload),
        Err(error) => crate::app_log::error("event-journal", &error),
    }
}

#[tauri::command]
pub async fn query_event_journal(query: JournalQuery) -> Result<JournalPage, String> {
    let journal = journal()?.clone();
    tauri::async_runtime::spawn_blocking(move || journal.query(query).map_err(|error| error.to_string()))
        .await.map_err(|error| error.to_string())?
}
