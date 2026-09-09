use crate::{db_error, ProviderData};
use codepet_provider_sdk::{local_runtime::RuntimeSelectionStorage, ProtocolError};
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn initialize(db: &Connection) -> Result<(), ProtocolError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS runtime_selection (
        singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
        executable_path TEXT NOT NULL
    );").map_err(db_error)
}

impl RuntimeSelectionStorage for ProviderData {
    fn load_last_selected(&self) -> Result<Option<String>, ProtocolError> {
        // Protocol fixtures may initialize without Host-assigned directories.
        if !self.configured() { return Ok(None); }
        self.connection()?.query_row(
            "SELECT executable_path FROM runtime_selection WHERE singleton = 1",
            [], |row| row.get(0),
        ).optional().map_err(db_error)
    }

    fn save_last_selected(&self, path: Option<&str>) -> Result<(), ProtocolError> {
        if !self.configured() { return Ok(()); }
        let db = self.connection()?;
        match path {
            Some(path) => db.execute(
                "INSERT INTO runtime_selection(singleton, executable_path) VALUES(1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET executable_path = excluded.executable_path",
                [path],
            ),
            None => db.execute("DELETE FROM runtime_selection WHERE singleton = 1", []),
        }.map_err(db_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codepet_provider_sdk::ProviderDirectories;

    #[tokio::test]
    async fn scanner_reopens_last_selection_and_falls_back_when_installation_disappears() {
        use codepet_provider_sdk::{local_runtime::{candidate, RuntimeScanner}, RuntimeCandidateSource, RuntimeInstallation};
        use std::sync::Arc;
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let dirs = ProviderDirectories {
            data: data_dir.to_string_lossy().into_owned(),
            logs: temp.path().join("logs").to_string_lossy().into_owned(),
            database_path: data_dir.join("provider.sqlite").to_string_lossy().into_owned(),
        };
        let old = temp.path().join("old.exe");
        let latest = temp.path().join("latest.exe");
        std::fs::write(&old, b"fixture").unwrap();
        std::fs::write(&latest, b"fixture").unwrap();
        let old_path = std::fs::canonicalize(&old).unwrap().to_string_lossy().into_owned();
        let latest_path = std::fs::canonicalize(&latest).unwrap().to_string_lossy().into_owned();
        for round in 0..3 {
            if round == 2 { std::fs::remove_file(&old).unwrap(); }
            let data = Arc::new(ProviderData::default());
            data.initialize(Some(&dirs)).unwrap();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let scanner = RuntimeScanner::new(Arc::new(move |event| { tx.send(event).unwrap(); Ok(()) }));
            scanner.set_selection_storage(data.clone());
            let paths = [old.clone(), latest.clone()];
            scanner.start("agent", "missing", move || paths.into_iter().filter(|p| p.is_file())
                .map(|p| candidate(p, RuntimeCandidateSource::CurrentPath)).collect(),
                |candidate, _| Ok(RuntimeInstallation {
                    version: if candidate.executable_path.ends_with("old.exe") { "1.9.0" } else { "1.10.0" }.into(),
                    executable_path: candidate.executable_path, source: candidate.source,
                    minimum_version: None, incompatibility_reason: None,
                }));
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            let codepet_provider_sdk::ProtocolEvent::RuntimeInventoryChanged { params, .. } = event else { panic!("wrong event") };
            let expected = if round == 1 { &old_path } else { &latest_path };
            assert_eq!(&params.selected.as_ref().unwrap().executable_path, expected);
            assert!(params.harness_list.contains(params.selected.as_ref().unwrap()));
            assert_eq!(data.load_last_selected().unwrap().as_ref(), Some(expected));
            assert!(params.scan_error.is_none());
            let wire = serde_json::to_value(&params).unwrap();
            assert!(wire.get("harnessList").is_some());
            assert!(wire.get("installed").is_none());
            assert_eq!(scanner.snapshot(), params);
            if round == 0 {
                scanner.select(&codepet_provider_sdk::RuntimeCandidate {
                    executable_path: old_path.clone(), source: RuntimeCandidateSource::Configured,
                }).unwrap();
                assert_eq!(data.load_last_selected().unwrap().as_ref(), Some(&old_path));
            }
            scanner.stop();
        }
    }

    #[test]
    fn selection_survives_reopen_and_is_isolated_per_provider_database() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = |name: &str| {
            let data = temp.path().join(name);
            ProviderDirectories {
                data: data.to_string_lossy().into_owned(),
                logs: temp.path().join(format!("{name}-logs")).to_string_lossy().into_owned(),
                database_path: data.join("provider.sqlite").to_string_lossy().into_owned(),
            }
        };
        let first = ProviderData::default();
        first.initialize(Some(&dirs("codex"))).unwrap();
        assert_eq!(first.load_last_selected().unwrap(), None);
        first.save_last_selected(Some("/install/codex.exe")).unwrap();
        drop(first);
        let reopened = ProviderData::default();
        reopened.initialize(Some(&dirs("codex"))).unwrap();
        assert_eq!(reopened.load_last_selected().unwrap().as_deref(), Some("/install/codex.exe"));
        let other = ProviderData::default();
        other.initialize(Some(&dirs("claude"))).unwrap();
        assert_eq!(other.load_last_selected().unwrap(), None);
        reopened.save_last_selected(Some("/new/codex.exe")).unwrap();
        assert_eq!(reopened.load_last_selected().unwrap().as_deref(), Some("/new/codex.exe"));
        reopened.save_last_selected(None).unwrap();
        assert_eq!(reopened.load_last_selected().unwrap(), None);
    }
}
