//! Single writer and replayable atomic JSON transactions; projections may be rebuilt.
use crate::{stable_id, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Component, Path, PathBuf},
};

pub struct Store {
    root: PathBuf,
    _lock: File,
    writable: bool,
}
// Own the lock lifetime explicitly. A duplicated descriptor (including a concurrent
// child process between spawn and exec) must not extend a finished operation's lease.
pub(crate) struct ExtractionLease(File);
impl Drop for ExtractionLease {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self._lock);
    }
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err("Task data directory must be absolute".into());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("writer.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "Task lineage is busy in another operation")?;
        let store = Self {
            root: root.to_owned(),
            _lock: lock,
            writable: true,
        };
        store.recover()?;
        Ok(store)
    }
    pub fn read_only(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err("Task data directory must be absolute".into());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("writer.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_shared(&lock).map_err(|_| "Task lineage is updating")?;
        if root.join("prepared.json").exists() {
            drop(lock);
            return Self::open(root);
        }
        Ok(Self {
            root: root.to_owned(),
            _lock: lock,
            writable: false,
        })
    }
    fn path(&self, relative: &str) -> Result<PathBuf> {
        Self::checked_path(&self.root, relative)
    }
    fn checked_path(root: &Path, relative: &str) -> Result<PathBuf> {
        let path = Path::new(relative);
        if path.as_os_str().is_empty()
            || !path.components().all(|c| matches!(c, Component::Normal(_)))
        {
            return Err("Invalid store key".into());
        }
        let result = root.join(path);
        // Store directories are owned by this service; reject external symlink redirection.
        let mut current = root.to_path_buf();
        for component in path.components() {
            current.push(component);
            if fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err("Symlink in task store".into());
            }
        }
        Ok(result)
    }
    pub fn read<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Self::read_published(&self.root, key)
    }
    // Only use without a Store for immutable files that are atomically published.
    pub(crate) fn read_published<T: DeserializeOwned>(root: &Path, key: &str) -> Result<Option<T>> {
        if !root.is_absolute() {
            return Err("Task data directory must be absolute".into());
        }
        let path = Self::checked_path(root, key)?;
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data)
                .map(Some)
                .map_err(|e| format!("Invalid task data {key}: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn list<T: DeserializeOwned>(&self, directory: &str) -> Result<Vec<T>> {
        self.list_bytes(directory)?
            .iter()
            .map(|bytes| serde_json::from_slice(bytes).map_err(|error| error.to_string()))
            .collect()
    }

    pub(crate) fn list_bytes(&self, directory: &str) -> Result<Vec<Vec<u8>>> {
        let directory = self.path(directory)?;
        if !directory.exists() {
            return Ok(vec![]);
        }
        let mut entries = fs::read_dir(directory)
            .map_err(|e| e.to_string())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.path());
        entries
            .into_iter()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|entry| {
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    return Err("Unexpected store entry".into());
                }
                fs::read(entry.path()).map_err(|error| error.to_string())
            })
            .collect()
    }
    pub(crate) fn publish_document(&self, key: &str, bytes: &[u8]) -> Result<()> {
        if !self.writable {
            return Err("Read-only task store".into());
        }
        let path = self.path(key)?;
        match fs::metadata(&path) {
            Ok(meta) if meta.is_file() => {
                let existing = fs::read(path).map_err(|error| error.to_string())?;
                if existing == bytes {
                    Ok(())
                } else {
                    Err(format!("任务文档摘要不匹配：{key}"))
                }
            }
            Ok(_) => Err("Invalid task document".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.atomic_file(key, |file| file.write_all(bytes).map_err(|e| e.to_string()))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    pub(crate) fn open_document(root: &Path, key: &str) -> Result<File> {
        if !root.is_absolute() {
            return Err("Task data directory must be absolute".into());
        }
        let path = Self::checked_path(root, key)?;
        let meta = fs::metadata(&path).map_err(|e| format!("读取任务文档 {key} 失败：{e}"))?;
        if !meta.is_file() {
            return Err("Invalid task document".into());
        }
        File::open(path).map_err(|e| format!("读取任务文档 {key} 失败：{e}"))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn atomic(&self, key: &str, value: &Value) -> Result<()> {
        self.atomic_file(key, |file| {
            let mut output = BufWriter::new(file);
            serde_json::to_writer(&mut output, value).map_err(|e| e.to_string())?;
            output.write_all(b"\n").map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())
        })
    }

    fn atomic_file(&self, key: &str, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
        let path = self.path(key)?;
        let parent = path.parent().ok_or("No parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        write(temporary.as_file_mut())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn recover(&self) -> Result<()> {
        if let Some(writes) = self.read::<Vec<(String, Value)>>("prepared.json")? {
            for (key, value) in &writes {
                self.atomic(key, value)?;
            }
            fs::remove_file(self.root.join("prepared.json")).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn commit(&self, writes: Vec<(String, Value)>) -> Result<()> {
        if !self.writable {
            return Err("Read-only task store".into());
        }
        for (key, _) in &writes {
            self.path(key)?;
            if key == "prepared.json" || key == "writer.lock" {
                return Err("Reserved store key".into());
            }
        }
        self.atomic(
            "prepared.json",
            &serde_json::to_value(&writes).map_err(|e| e.to_string())?,
        )?;
        self.recover()
    }
    pub fn write<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.commit(vec![(
            key.into(),
            serde_json::to_value(value).map_err(|e| e.to_string())?,
        )])
    }
    pub(crate) fn extraction_lease(&self) -> Result<ExtractionLease> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("inference.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "Another extraction job is already running")?;
        Ok(ExtractionLease(lock))
    }
    /// Long-running inference does not block readers or ingestion. Callers must revalidate revisions afterward.
    pub(crate) fn without_writer_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<Result<T>> {
        if !self.writable {
            return Err("Inference requires writer ownership".into());
        }
        fs2::FileExt::unlock(&self._lock).map_err(|e| e.to_string())?;
        let result = operation();
        let started = std::time::Instant::now();
        while fs2::FileExt::try_lock_exclusive(&self._lock).is_err() {
            if started.elapsed() > std::time::Duration::from_secs(3) {
                return Err("Task state is busy; extraction result not applied".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.recover()?;
        Ok(result)
    }
    pub fn thread_key(thread_id: &str, kind: &str) -> String {
        format!("threads/{}/{kind}.json", stable_id(thread_id))
    }
    pub fn task_key(task_id: &str) -> String {
        format!("tasks/{}.json", stable_id(task_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn operation_releases_locks_even_when_a_duplicate_descriptor_is_retained() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let duplicate_writer = store._lock.try_clone().unwrap();
        let lease = store.extraction_lease().unwrap();
        let duplicate_lease = lease.0.try_clone().unwrap();
        drop(lease);
        assert!(store.extraction_lease().is_ok());
        drop(store);
        assert!(Store::open(dir.path()).is_ok());
        drop((duplicate_writer, duplicate_lease));
    }
    #[test]
    fn prepared_transaction_replays_after_restart_and_lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(Store::open(dir.path()).is_err());
        store
            .atomic(
                "prepared.json",
                &json!([["tasks/a.json",{"revision":2}],["threads/b.json",{"offset":42}]]),
            )
            .unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.read::<Value>("tasks/a.json").unwrap().unwrap()["revision"],
            2
        );
        assert_eq!(
            store.read::<Value>("threads/b.json").unwrap().unwrap()["offset"],
            42
        );
        assert!(!dir.path().join("prepared.json").exists());
    }
    #[test]
    fn bad_json_is_not_silently_replaced_and_paths_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(store.read::<Value>("bad.json").is_err());
        assert!(store.write("../outside.json", &json!(1)).is_err());
        store.write("valid.json", &json!(1)).unwrap();
        store.write("valid.json", &json!(2)).unwrap();
        assert_eq!(store.read::<Value>("valid.json").unwrap(), Some(json!(2)));
    }
}
