use crate::{
    extraction::TaskExtractor,
    service::{self, Snapshot},
    sources::ConversationSource,
    store::Store,
    Result,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchConfig {
    pub revision: u64,
    pub enabled: bool,
    pub extract: bool,
    pub thread_id: Option<String>,
    pub model: String,
    pub remaining_jobs: u32,
    #[serde(default)]
    pub continuous: bool,
    pub last_error: Option<String>,
}
impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            revision: 0,
            enabled: false,
            extract: false,
            thread_id: None,
            model: "haiku".into(),
            remaining_jobs: 5,
            continuous: false,
            last_error: None,
        }
    }
}
pub fn read(store: &Store) -> Result<WatchConfig> {
    Ok(store.read("watch.json")?.unwrap_or_default())
}
pub fn configure(store: &Store, mut config: WatchConfig) -> Result<WatchConfig> {
    if config.remaining_jobs > 5 || config.model.trim().is_empty() || config.model.len() > 120 {
        return Err("Invalid watch budget/model".into());
    }
    if config.extract
        && !config.continuous
        && config.thread_id.as_ref().is_none_or(|id| id.is_empty())
    {
        return Err("Select a conversation before enabling extraction".into());
    }
    config.revision = read(store)?.revision + 1;
    config.last_error = None;
    store.write("watch.json", &config)?;
    Ok(config)
}
pub fn tick(
    store: &Store,
    source: &dyn ConversationSource,
    extractor: Option<&dyn TaskExtractor>,
) -> Result<Option<Snapshot>> {
    let mut config = read(store)?;
    if !config.enabled {
        return Ok(None);
    }
    let snapshot = service::scan(store, source)?;
    if !config.extract || (!config.continuous && config.remaining_jobs == 0) {
        return Ok(Some(snapshot));
    }
    if service::pending_for(store, &snapshot, config.thread_id.as_deref())? == 0 {
        return Ok(Some(snapshot));
    }
    let settings = crate::management::settings(store)?;
    // Legacy callers with no persisted settings retain their immediate first scan.
    let debounce = if store
        .read::<crate::management::ExtractionSettings>("extraction-settings.json")?
        .is_some()
    {
        settings.debounce_seconds
    } else {
        0
    };
    let Some(ready) =
        service::ready_thread(store, &snapshot, config.thread_id.as_deref(), debounce)?
    else {
        return Ok(Some(snapshot));
    };
    if crate::management::jobs(store)?
        .iter()
        .any(|job| matches!(job.state.as_str(), "queued" | "running"))
    {
        return Ok(Some(snapshot));
    }
    let Some(extractor) = extractor else {
        return Err("Configured Claude runtime is unavailable".into());
    };
    // Reserve before spending; a crash cannot silently replenish the user's budget.
    let request_id = format!(
        "scheduled-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let (mut job, _) = crate::management::enqueue(store, &request_id, Some(ready.clone()))?;
    job.state = "running".into();
    store.write(&crate::management::job_key(&job.id), &job)?;
    if !config.continuous {
        config.remaining_jobs -= 1;
    }
    config.revision += 1;
    store.write("watch.json", &config)?;
    let result = service::extract_thread(store, extractor, &ready);
    job.state = if result.is_ok() {
        "completed"
    } else {
        "failed"
    }
    .into();
    job.error = result.as_ref().err().cloned();
    job.finished_at = Some(chrono::Utc::now().timestamp_millis());
    store.write(&crate::management::job_key(&job.id), &job)?;
    if read(store)?.revision == config.revision {
        if let Err(error) = &result {
            config.last_error = Some(error.clone());
            config.extract = false;
        }
        if !config.continuous && config.remaining_jobs == 0 {
            config.extract = false;
        }
        store.write("watch.json", &config)?;
    }
    result.map(Some)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watch_is_opt_in_and_budget_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        assert!(!read(&store).unwrap().enabled);
        let config = configure(
            &store,
            WatchConfig {
                enabled: true,
                extract: true,
                thread_id: Some("thread".into()),
                remaining_jobs: 2,
                ..WatchConfig::default()
            },
        )
        .unwrap();
        assert_eq!(config.revision, 1);
        drop(store);
        assert_eq!(read(&Store::open(root.path()).unwrap()).unwrap(), config);
    }
}
