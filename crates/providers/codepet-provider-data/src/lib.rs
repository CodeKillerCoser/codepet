//! Provider business persistence. This crate is deliberately outside the transport runtime.
pub mod app_state;
mod collection;
mod usage;
mod usage_snapshot;
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
    metadata: Mutex<Option<(PathBuf, String)>>,
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
        let plugin_id = data.file_name().and_then(|v| v.to_str()).ok_or_else(|| error("invalid_provider_directories", "Missing plugin directory name"))?.to_owned();
        let metadata_path = std::env::var_os(app_state::APP_STATE_DATABASE_ENV).map(PathBuf::from)
            .unwrap_or_else(|| data.join(app_state::APP_STATE_DATABASE));
        let metadata = app_state::open_app_state(&metadata_path)?;
        let now = chrono::Utc::now().timestamp_millis();
        metadata.execute("INSERT OR IGNORE INTO providers(id,display_name,created_at,updated_at) VALUES(?1,?1,?2,?2)", rusqlite::params![plugin_id,now]).map_err(db_error)?;
        *self.metadata.lock().map_err(db_error)? = Some((metadata_path, plugin_id));
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
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(db_error)?; }
    // Create private before SQLite creates WAL/SHM companions. Never follow a symlink.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    match options.open(path) {
        Ok(_) => {},
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if !std::fs::symlink_metadata(path).map_err(db_error)?.file_type().is_file() {
                return Err(error("invalid_secret_file_type", "Database path must be a regular file"));
            }
        },
        Err(e) => return Err(db_error(e)),
    }
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(db_error)?; }
    let db = Connection::open(path).map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(db_error)?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(db_error)?;
    db.pragma_update(None, "synchronous", "FULL").map_err(db_error)?;
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
