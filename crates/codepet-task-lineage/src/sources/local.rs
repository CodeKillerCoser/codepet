//! Local history discovery is independent of a running Provider.
use super::codex;
use crate::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const CODEX_ID: &str = "local:codex";

pub fn contexts() -> Vec<(String, String, Value)> {
    vec![(CODEX_ID.into(), "Codex（本地）".into(), json!({}))]
}

pub fn directory(id: &str) -> Result<PathBuf> {
    if id != CODEX_ID {
        return Err("不支持的本地 Agent".into());
    }
    codex::data_directory(&json!({}))
}

/// Only an unambiguous connection to the same existing directory may resume history.
pub fn matching_provider(home: &Path, contexts: &[(String, String, Value)]) -> Option<String> {
    let home = home.canonicalize().ok()?;
    let mut matches = contexts.iter().filter(|(_, _, settings)| {
        codex::data_directory(settings)
            .ok()
            .and_then(|path| path.canonicalize().ok())
            .is_some_and(|path| path == home)
    });
    let first = matches.next()?;
    matches.next().is_none().then(|| first.0.clone())
}

/// Preserve a unique legacy store with an atomic pointer, without moving user files.
pub fn adopt_legacy_store(
    data: &Path,
    home: &Path,
    connections: &[(String, String, Value)],
) -> Result<()> {
    let pointer = data.join("local-codex-store.json");
    if pointer.exists() || data.join("v1").join(crate::stable_id(CODEX_ID)).exists() {
        return Ok(());
    }
    let Some(id) = matching_provider(home, connections) else {
        return Ok(());
    };
    if !data.join("v1").join(crate::stable_id(&id)).is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(data).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&id).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    match file.persist_noclobber(pointer) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_source_exists_without_connections() {
        let sources = contexts();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].0, CODEX_ID);
        assert_eq!(sources[0].2, json!({}));
        assert!(directory("remote:codex").is_err());
    }

    #[test]
    fn resume_requires_a_unique_matching_directory() {
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let entry = |id: &str, path: &Path| (id.into(), id.into(), json!({"dataDirectory": path}));
        assert_eq!(matching_provider(home.path(), &[]), None);
        assert_eq!(
            matching_provider(home.path(), &[entry("other", other.path())]),
            None
        );
        assert_eq!(
            matching_provider(
                home.path(),
                &[entry("local", home.path()), entry("other", other.path())]
            ),
            Some("local".into())
        );
        assert_eq!(
            matching_provider(
                home.path(),
                &[entry("a", home.path()), entry("b", home.path())]
            ),
            None
        );
    }

    #[test]
    fn upgrade_keeps_existing_store_and_never_replaces_a_binding() {
        let data = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let old = data.path().join("v1").join(crate::stable_id("old"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("extraction-settings.json"), "preserved").unwrap();
        let entry = |id: &str| (id.into(), id.into(), json!({"codexHome":home.path()}));
        adopt_legacy_store(data.path(), home.path(), &[entry("old"), entry("other")]).unwrap();
        let pointer = data.path().join("local-codex-store.json");
        assert!(!pointer.exists());
        adopt_legacy_store(data.path(), home.path(), &[entry("old")]).unwrap();
        assert_eq!(std::fs::read_to_string(&pointer).unwrap(), "\"old\"");
        adopt_legacy_store(data.path(), home.path(), &[entry("other")]).unwrap();
        assert_eq!(std::fs::read_to_string(pointer).unwrap(), "\"old\"");
        assert_eq!(
            std::fs::read_to_string(old.join("extraction-settings.json")).unwrap(),
            "preserved"
        );
        let fresh = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(fresh.path().join("v1").join(crate::stable_id(CODEX_ID))).unwrap();
        std::fs::create_dir_all(fresh.path().join("v1").join(crate::stable_id("old"))).unwrap();
        adopt_legacy_store(fresh.path(), home.path(), &[entry("old")]).unwrap();
        assert!(!fresh.path().join("local-codex-store.json").exists());
    }
}
