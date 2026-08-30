use crate::{HostError, HostResult};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub(crate) fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> HostResult<()> {
    write_json_atomically_with_mode(path, value, false)
}

pub(crate) fn write_secret_json_atomically<T: Serialize>(
    path: &Path,
    value: &T,
) -> HostResult<()> {
    write_json_atomically_with_mode(path, value, true)
}

pub(crate) fn protect_secret_file(path: &Path) -> HostResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| persistence_io("inspect secret file", path, error))?;
    if !metadata.file_type().is_file() {
        return Err(HostError::new(
            "invalid_secret_file_type",
            format!("secret persistence path is not a regular file: {}", path.display()),
        )
        .with_detail("path", path.display().to_string()));
    }
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| persistence_io("protect secret file", path, error))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_json_atomically_with_mode<T: Serialize>(
    path: &Path,
    value: &T,
    owner_only: bool,
) -> HostResult<()> {
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
    #[cfg(unix)]
    if owner_only {
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                persistence_io("protect temporary file", &temporary_path, error)
            })?;
    }
    #[cfg(not(unix))]
    let _ = owner_only;
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
    let persisted = temporary
        .persist(path)
        .map_err(|error| persistence_io("replace persistence file", path, error.error))?;
    persisted
        .sync_all()
        .map_err(|error| persistence_io("sync replaced file", path, error))?;
    sync_parent_after_replace(parent, path)?;
    Ok(())
}

fn sync_parent_after_replace(parent: &Path, path: &Path) -> HostResult<()> {
    #[cfg(unix)]
    {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| persistence_io("sync parent after replace", path, error))?;
    }
    #[cfg(not(unix))]
    {
        // Stable Rust has no portable Windows directory fsync. The replaced
        // file itself is synced above, which is the best available primitive.
        let _ = (parent, path);
    }
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
