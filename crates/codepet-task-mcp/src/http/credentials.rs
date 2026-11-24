use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub(super) fn token_path(data: &Path) -> PathBuf {
    data.join("tasks/mcp-http.token")
}

// Platform details stay behind the credential-file interface.
#[cfg(unix)]
fn open_private(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}
#[cfg(not(unix))]
fn open_private(path: &Path) -> io::Result<File> {
    // Inherit the user's application-data directory ACL on Windows.
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

pub(super) fn load_or_create(data: &Path) -> io::Result<String> {
    let path = token_path(data);
    std::fs::create_dir_all(path.parent().expect("token parent"))?;
    let mut file = open_private(&path)?;
    fs2::FileExt::lock_exclusive(&file)?;
    let mut token = String::new();
    Read::by_ref(&mut file)
        .take(65)
        .read_to_string(&mut token)?;
    if token.is_empty() {
        token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
    }
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "任务 MCP 本机凭据格式无效",
        ));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_is_stable_private_and_invalid_credentials_are_not_rotated() {
        let data = tempfile::tempdir().unwrap();
        let first = load_or_create(data.path()).unwrap();
        assert_eq!(first, load_or_create(data.path()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(token_path(data.path()))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::write(token_path(data.path()), "invalid").unwrap();
        assert!(load_or_create(data.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(token_path(data.path())).unwrap(),
            "invalid"
        );
    }
}
