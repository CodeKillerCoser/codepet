//! Shared desktop metadata. JSON configuration and external Harness files are separate.
use crate::{db_error, error, ProtocolError};
use rusqlite::Connection;
use std::path::Path;

pub const APP_STATE_DATABASE: &str = "app-state.sqlite";
pub const APP_STATE_DATABASE_ENV: &str = "CODEPET_APP_STATE_DATABASE";

pub fn open_app_state(path: &Path) -> Result<Connection, ProtocolError> {
    let db = crate::open(path)?;
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(db_error)?;
    if version != 0 && version != 1 {
        return Err(error(
            "unsupported_database_version",
            "Unsupported application database schema",
        ));
    }
    if version == 0 {
        db.execute_batch("BEGIN IMMEDIATE").map_err(db_error)?;
        db.execute_batch(include_str!("app_state.sql"))
            .map_err(db_error)?;
        db.execute_batch("PRAGMA user_version=1; COMMIT")
            .map_err(db_error)?;
    }
    Ok(db)
}
