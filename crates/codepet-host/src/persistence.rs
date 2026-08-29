use crate::{HostError, HostResult};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;

pub(crate) fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> HostResult<()> {
    let parent = path.parent().ok_or_else(|| {
        HostError::new(
            "invalid_persistence_path",
            format!("persistence path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| persistence_io("create parent", path, error))?;
    let payload = serde_json::to_vec_pretty(value).map_err(HostError::from)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| persistence_io("create temporary file", parent, error))?;
    let temporary_path = temporary.path().to_path_buf();
    temporary
        .write_all(&payload)
        .map_err(|error| persistence_io("write temporary file", &temporary_path, error))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| persistence_io("finish temporary file", &temporary_path, error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| persistence_io("sync temporary file", &temporary_path, error))?;
    temporary
        .persist(path)
        .map_err(|error| persistence_io("replace persistence file", path, error.error))?;
    Ok(())
}

pub(crate) fn persistence_io(action: &str, path: &Path, error: std::io::Error) -> HostError {
    HostError::new(
        "host_persistence_error",
        format!("{action} {}: {error}", path.display()),
    )
    .retryable(true)
    .with_detail("path", path.display().to_string())
}
