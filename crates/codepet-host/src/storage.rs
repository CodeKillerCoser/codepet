use crate::{HostError, HostResult};
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn open(path: &Path) -> HostResult<Connection> {
    codepet_provider_data::app_state::open_app_state(path).map_err(HostError::from)
}
pub(crate) fn error(error: impl ToString) -> HostError {
    HostError::new("host_database_error", error.to_string())
}
