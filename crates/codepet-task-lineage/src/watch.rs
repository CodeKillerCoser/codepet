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
    if config.extract && config.thread_id.as_ref().is_none_or(|id| id.is_empty()) {
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
    if !config.extract || config.remaining_jobs == 0 {
        return Ok(Some(snapshot));
    }
    if service::pending_for(store, &snapshot, config.thread_id.as_deref())? == 0 {
        return Ok(Some(snapshot));
    }
    let Some(extractor) = extractor else {
        return Err("Configured Claude runtime is unavailable".into());
    };
    // Reserve before spending; a crash cannot silently replenish the user's budget.
    config.remaining_jobs -= 1;
    config.revision += 1;
    store.write("watch.json", &config)?;
    let result = service::extract_next(store, extractor, config.thread_id.as_deref());
    if read(store)?.revision == config.revision {
        if let Err(error) = &result {
            config.last_error = Some(error.clone());
            config.extract = false;
        }
        if config.remaining_jobs == 0 {
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
