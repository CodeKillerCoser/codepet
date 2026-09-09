//! Provider business persistence. This crate is deliberately outside the transport runtime.
mod collection;
mod usage;
mod runtime_selection;
pub use codepet_provider_sdk::*;
pub use collection::UsageSink;
pub use usage::*;
pub mod conversation_atoms;
pub mod conversation_state;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub fn error(code: &str, message: impl ToString) -> ProtocolError {
    ProtocolError {
        code: code.into(),
        message: message.to_string(),
        retryable: false,
        details: None,
    }
}
pub(crate) fn db_error(e: impl ToString) -> ProtocolError {
    error("provider_database_error", e)
}

pub fn with_usage_capabilities(
    mut capabilities: codepet_provider_sdk::ProviderCapabilities,
) -> codepet_provider_sdk::ProviderCapabilities {
    if !capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::UsageQuery)
    {
        capabilities
            .methods
            .push(codepet_provider_sdk::ProviderCapability::UsageQuery);
    }
    conversation_atoms::with_capabilities(capabilities)
}

#[derive(Default)]
pub struct ProviderData {
    path: Mutex<Option<PathBuf>>,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    wake: std::sync::Arc<tokio::sync::Notify>,
}

impl ProviderData {
    pub fn initialize(&self, dirs: Option<&ProviderDirectories>) -> Result<(), ProtocolError> {
        let Some(dirs) = dirs else { return Ok(()) };
        let data = Path::new(&dirs.data);
        let logs = Path::new(&dirs.logs);
        let database = Path::new(&dirs.database_path);
        if !data.is_absolute()
            || !logs.is_absolute()
            || !database.is_absolute()
            || database.parent() != Some(data)
        {
            return Err(error(
                "invalid_provider_directories",
                "Expected absolute Provider paths and database directly inside data",
            ));
        }
        let mut path = self.path.lock().map_err(db_error)?;
        if let Some(old) = path.as_ref() {
            if old != database {
                return Err(error(
                    "provider_already_initialized",
                    "Provider database cannot be rebound",
                ));
            }
            return Ok(());
        }
        std::fs::create_dir_all(data).map_err(db_error)?;
        std::fs::create_dir_all(logs).map_err(db_error)?;
        let connection = open(database)?;
        usage::initialize(&connection)?;
        runtime_selection::initialize(&connection)?;
        *path = Some(database.to_owned());
        Ok(())
    }
    pub fn connection(&self) -> Result<Connection, ProtocolError> {
        let path = self.path.lock().map_err(db_error)?;
        open(path.as_ref().ok_or_else(|| {
            error(
                "usage_unavailable",
                "Host has not assigned a Provider database",
            )
        })?)
    }
    pub fn configured(&self) -> bool {
        self.path.lock().map(|p| p.is_some()).unwrap_or(false)
    }
}

fn open(path: &Path) -> Result<Connection, ProtocolError> {
    let db = Connection::open(path).map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(db_error)?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(db_error)?;
    db.pragma_update(None, "foreign_keys", "ON")
        .map_err(db_error)?;
    Ok(db)
}

impl Drop for ProviderData {
    fn drop(&mut self) {
        if let Ok(worker) = self.worker.get_mut() {
            if let Some(task) = worker.take() {
                task.abort();
            }
        }
    }
}
