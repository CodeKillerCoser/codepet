//! Append-only JSONL segments and a disposable, checksummed byte-offset index.
//! Queries pin open files briefly under the writer lock, then perform all scans outside it.
use super::JournalEvent;
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};

const MAGIC: &[u8; 8] = b"CPJIDX01";
const INDEX_BYTES: usize = 32;
const READ_BYTES: u64 = 64 * 1024;
const INDEX_FLUSH_ROWS: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCursor {
    pub segment_id: String,
    /// Exclusive upper byte boundary. Only complete records ending here or earlier are read.
    pub before_offset: u64,
}

#[derive(Clone, Copy, Debug)]
struct Offset {
    sequence: u64,
    start: u64,
    end: u64,
}

struct SegmentLease {
    path: Mutex<PathBuf>,
    retired: AtomicBool,
}
impl Drop for SegmentLease {
    fn drop(&mut self) {
        if self.retired.load(Ordering::Relaxed) {
            let path = self.path.get_mut().unwrap();
            // Open query handles are dropped before the last lease on every platform.
            // Failed cleanup is retried by retention discovery on the next startup.
            let _ = fs::remove_file(&*path);
            let _ = fs::remove_file(index_path(path));
        }
    }
}

struct Segment {
    id: String,
    path: PathBuf,
    offsets: Arc<Vec<Offset>>,
    lease: Arc<SegmentLease>,
}

pub(super) struct HistoryStore {
    directory: PathBuf,
    active: Segment,
    archives: Vec<Segment>, // newest first
    file: Option<File>,
    index: Option<BufWriter<File>>,
    pub bytes: u64,
    pub index_error: Option<String>,
    retained: usize,
}

impl HistoryStore {
    pub fn open(directory: PathBuf, retained: usize) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let active_path = directory.join("events.jsonl");
        let mut options = OpenOptions::new(); options.create(true).read(true).append(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        drop(options.open(&active_path)?);
        let mut active = Segment::load(active_path, None)?;
        let mut archives = vec![];
        let entries = fs::read_dir(&directory)?.collect::<io::Result<Vec<_>>>()?;
        for entry in entries {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if let Some(id) = name.strip_prefix("events.").and_then(|n| n.strip_suffix(".jsonl")) {
                if uuid::Uuid::parse_str(id).is_ok() { archives.push(Segment::load(path.clone(), Some(id))?); }
            } else if name.strip_prefix("events.jsonl.").and_then(|n| n.parse::<usize>().ok()).is_some() {
                // One-time migration from numbered archives; no log content is rewritten.
                let mut segment = Segment::load(path, None)?;
                let destination = directory.join(format!("events.{}.jsonl", segment.id));
                fs::rename(&segment.path, &destination)?;
                let _ = fs::rename(index_path(&segment.path), index_path(&destination));
                segment.path = destination.clone();
                *segment.lease.path.lock().unwrap() = destination;
                archives.push(segment);
            }
        }
        if archives.iter().any(|segment| segment.id == active.id) {
            active.id = uuid::Uuid::new_v4().to_string();
            active.persist_index()?;
        }
        archives.sort_by_key(|segment| std::cmp::Reverse(segment.last_sequence()));
        let bytes = fs::metadata(&active.path)?.len();
        let mut store = Self { directory, active, archives, file: None, index: None, bytes, index_error: None, retained };
        store.prune();
        store.open_writer()?;
        store.open_index_writer();
        Ok(store)
    }

    pub fn last_sequence(&self) -> u64 {
        std::iter::once(&self.active).chain(self.archives.iter()).map(Segment::last_sequence).max().unwrap_or(0)
    }

    #[cfg(test)]
    pub fn path(&self, index: usize) -> PathBuf {
        if index == 0 { self.active.path.clone() } else { self.archives[index - 1].path.clone() }
    }

    fn open_writer(&mut self) -> io::Result<()> {
        let mut options = OpenOptions::new(); options.create(true).read(true).append(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = options.open(&self.active.path)?;
        if file.metadata()?.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0]; file.read_exact(&mut last)?;
            if last[0] != b'\n' { file.write_all(b"\n")?; }
        }
        self.bytes = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    fn open_index_writer(&mut self) {
        match OpenOptions::new().append(true).open(index_path(&self.active.path)) {
            Ok(file) => self.index = Some(BufWriter::new(file)),
            Err(error) => self.index_error = Some(format!("offset index: {error}")),
        }
    }

    pub fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        if let Some(mut index) = self.index.take() {
            if let Err(error) = index.flush() { self.index_error = Some(format!("offset index: {error}")); }
        }
        let destination = self.directory.join(format!("events.{}.jsonl", self.active.id));
        fs::rename(&self.active.path, &destination)?;
        // The JSONL is authoritative. A missing/stale sidecar is rebuilt on startup.
        let _ = fs::rename(index_path(&self.active.path), index_path(&destination));
        self.active.path = destination.clone();
        *self.active.lease.path.lock().unwrap() = destination;
        let new_active = Segment::empty(self.directory.join("events.jsonl"));
        // Commit the identity transition immediately after rename. Even if opening the new
        // file fails, later retries must never append to the sealed archive.
        let archived = std::mem::replace(&mut self.active, new_active);
        self.archives.insert(0, archived);
        self.bytes = 0;
        self.prune();
        // Never inherit an old sidecar if its rename failed.
        match self.active.persist_index() {
            Ok(()) => self.open_index_writer(),
            Err(error) => self.index_error = Some(format!("offset index: {error}")),
        }
        self.open_writer()
    }

    fn prune(&mut self) {
        for segment in self.archives.drain(self.retained.min(self.archives.len())..) {
            segment.lease.retired.store(true, Ordering::Relaxed);
        }
    }

    pub fn append(&mut self, sequence: u64, line: &str) -> io::Result<()> {
        if self.file.is_none() { self.open_writer()?; }
        let file = self.file.as_mut().unwrap();
        if let Err(error) = file.write_all(format!("{line}\n").as_bytes()).and_then(|_| file.flush()) {
            self.file.take();
            return Err(error);
        }
        let offset = Offset { sequence, start: self.bytes, end: self.bytes + line.len() as u64 + 1 };
        self.bytes = offset.end;
        Arc::make_mut(&mut self.active.offsets).push(offset);
        if let Some(index) = self.index.as_mut() {
            let result = index.write_all(&encode_offset(offset)).and_then(|_| {
                if self.active.offsets.len() % INDEX_FLUSH_ROWS == 0 { index.flush() } else { Ok(()) }
            });
            if let Err(error) = result {
                self.index_error = Some(format!("offset index: {error}; will rebuild from JSONL"));
                self.index.take();
            }
        }
        Ok(())
    }

    /// Only opens the cursor's segment and older segments. No log contents are read here.
    pub fn snapshot(&self, cursor: Option<&HistoryCursor>, before: Option<u64>) -> io::Result<Option<HistorySnapshot>> {
        let segments: Vec<_> = std::iter::once(&self.active).chain(self.archives.iter()).collect();
        let start = match cursor {
            Some(cursor) => match segments.iter().position(|s| s.id == cursor.segment_id) {
                Some(start) => start,
                None => return Ok(None),
            },
            None => segments.iter().position(|s| s.offsets.first().is_some_and(|e| before.map_or(true, |before| e.sequence < before))).unwrap_or(segments.len()),
        };
        let mut readers = vec![];
        for (index, segment) in segments.iter().enumerate().skip(start) {
            let end = if let Some(cursor) = cursor.filter(|_| index == start) {
                let end = segment.offsets.partition_point(|entry| entry.end <= cursor.before_offset);
                if cursor.before_offset != 0 && !end.checked_sub(1).is_some_and(|i| segment.offsets[i].end == cursor.before_offset) && !segment.offsets.get(end).is_some_and(|entry| entry.start == cursor.before_offset) {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "History cursor is not a record boundary"));
                }
                end
            } else {
                segment.offsets.partition_point(|entry| before.map_or(true, |before| entry.sequence < before))
            };
            if end == 0 { continue; }
            readers.push(ReadSegment { file: File::open(&segment.path)?, offsets: segment.offsets.clone(), end, id: segment.id.clone(), _lease: segment.lease.clone() });
        }
        Ok(Some(HistorySnapshot { segments: readers }))
    }
}

impl Segment {
    fn empty(path: PathBuf) -> Self {
        Self { id: uuid::Uuid::new_v4().to_string(), path: path.clone(), offsets: Arc::new(vec![]),
            lease: Arc::new(SegmentLease { path: Mutex::new(path), retired: AtomicBool::new(false) }) }
    }

    fn last_sequence(&self) -> u64 { self.offsets.last().map_or(0, |e| e.sequence) }

    fn load(path: PathBuf, expected_id: Option<&str>) -> io::Result<Self> {
        let mut segment = Self::empty(path);
        if let Some(id) = expected_id { segment.id = id.to_string(); }
        let bytes = fs::metadata(&segment.path)?.len();
        let saved = fs::read(index_path(&segment.path)).ok();
        let mut valid = false;
        if let Some(saved) = saved.as_ref().filter(|saved| saved.len() >= INDEX_BYTES) {
            let header = &saved[..INDEX_BYTES];
            if &header[..8] == MAGIC && checksum(&header[..24]) == header[24..] {
                let id = uuid::Uuid::from_slice(&header[8..24]).unwrap().to_string();
                if expected_id.map_or(true, |expected| expected == id) {
                    segment.id = id;
                    valid = true;
                    for row in saved[INDEX_BYTES..].chunks(INDEX_BYTES) {
                        let Some(offset) = decode_offset(row) else { valid = false; break; };
                        if offset.start >= offset.end || offset.end > bytes || segment.offsets.last().is_some_and(|last| last.sequence >= offset.sequence || last.end > offset.start) {
                            valid = false; break;
                        }
                        Arc::make_mut(&mut segment.offsets).push(offset);
                    }
                }
            }
        }
        if !valid { Arc::make_mut(&mut segment.offsets).clear(); }
        let scanned_from = segment.offsets.last().map_or(0, |e| e.end);
        let mut reader = BufReader::new(File::open(&segment.path)?);
        reader.seek(SeekFrom::Start(scanned_from))?;
        let mut start = scanned_from;
        let mut line = vec![];
        loop {
            line.clear();
            let length = reader.read_until(b'\n', &mut line)?;
            if length == 0 { break; }
            let end = start + length as u64;
            if let Ok(record) = serde_json::from_slice::<JournalEvent>(&line) {
                if record.sequence > segment.last_sequence() { Arc::make_mut(&mut segment.offsets).push(Offset { sequence: record.sequence, start, end }); }
            }
            start = end;
        }
        if !valid || scanned_from < bytes { segment.persist_index()?; }
        Ok(segment)
    }

    fn persist_index(&self) -> io::Result<()> {
        let mut temporary = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
        let mut header = [0; INDEX_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8..24].copy_from_slice(uuid::Uuid::parse_str(&self.id).unwrap().as_bytes());
        let digest = checksum(&header[..24]); header[24..].copy_from_slice(&digest);
        temporary.write_all(&header)?;
        for entry in self.offsets.iter() { temporary.write_all(&encode_offset(*entry))?; }
        temporary.flush()?;
        temporary.persist(index_path(&self.path)).map_err(|error| error.error)?;
        Ok(())
    }
}

fn index_path(path: &Path) -> PathBuf { path.with_file_name(format!("{}.idx", path.file_name().unwrap().to_string_lossy())) }

fn checksum(bytes: &[u8]) -> [u8; 8] { ring::digest::digest(&ring::digest::SHA256, bytes).as_ref()[..8].try_into().unwrap() }
fn encode_offset(offset: Offset) -> [u8; INDEX_BYTES] {
    let mut row = [0; INDEX_BYTES];
    row[..8].copy_from_slice(&offset.sequence.to_le_bytes());
    row[8..16].copy_from_slice(&offset.start.to_le_bytes());
    row[16..24].copy_from_slice(&offset.end.to_le_bytes());
    let digest = checksum(&row[..24]); row[24..].copy_from_slice(&digest);
    row
}
fn decode_offset(row: &[u8]) -> Option<Offset> {
    if row.len() != INDEX_BYTES || checksum(&row[..24]) != row[24..] { return None; }
    Some(Offset { sequence: u64::from_le_bytes(row[..8].try_into().ok()?), start: u64::from_le_bytes(row[8..16].try_into().ok()?), end: u64::from_le_bytes(row[16..24].try_into().ok()?) })
}

struct ReadSegment {
    // Field order matters on Windows: close the file before releasing its retention lease.
    file: File,
    offsets: Arc<Vec<Offset>>,
    end: usize,
    id: String,
    _lease: Arc<SegmentLease>,
}
pub(super) struct HistorySnapshot { segments: Vec<ReadSegment> }

#[derive(Default)]
pub(super) struct ReadStats { pub bytes: u64, pub records: usize, pub files: usize }
pub(super) struct HistoryPage {
    pub events: Vec<JournalEvent>,
    pub next: Option<HistoryCursor>,
    pub stats: ReadStats,
}

impl HistorySnapshot {
    pub fn read(mut self, limit: usize, keyword: &str, source: Option<&str>) -> io::Result<HistoryPage> {
        let mut page = HistoryPage { events: vec![], next: None, stats: ReadStats::default() };
        for segment in &mut self.segments {
            page.stats.files += 1;
            let mut reader = WindowReader { file: &mut segment.file, start: 0, buffer: vec![], bytes: 0 };
            for offset in segment.offsets[..segment.end].iter().rev() {
                let line = reader.read(*offset)?;
                page.stats.records += 1;
                let Ok(record) = serde_json::from_slice::<JournalEvent>(line) else { continue; };
                if record.sequence != offset.sequence { return Err(io::Error::new(io::ErrorKind::InvalidData, "Event log no longer matches its offset index")); }
                if source.filter(|s| !s.is_empty()).is_some_and(|s| s != record.source) { continue; }
                if !keyword.is_empty() && !String::from_utf8_lossy(line).to_lowercase().contains(keyword) { continue; }
                if page.events.len() == limit {
                    // Resume at this lookahead record, skipping already-scanned nonmatches.
                    page.next = Some(HistoryCursor { segment_id: segment.id.clone(), before_offset: offset.end });
                    page.stats.bytes += reader.bytes;
                    return Ok(page);
                }
                page.events.push(record);
            }
            page.stats.bytes += reader.bytes;
        }
        Ok(page)
    }
}

struct WindowReader<'a> { file: &'a mut File, start: u64, buffer: Vec<u8>, bytes: u64 }
impl WindowReader<'_> {
    fn read(&mut self, offset: Offset) -> io::Result<&[u8]> {
        if offset.start < self.start || offset.end > self.start + self.buffer.len() as u64 {
            self.start = offset.end.saturating_sub(READ_BYTES.max(offset.end - offset.start));
            let length = usize::try_from(offset.end - self.start).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Event is too large for this platform"))?;
            self.buffer.resize(length, 0);
            self.file.seek(SeekFrom::Start(self.start))?;
            self.file.read_exact(&mut self.buffer)?;
            self.bytes += length as u64;
        }
        Ok(&self.buffer[(offset.start - self.start) as usize..(offset.end - self.start) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn append(store: &mut HistoryStore, sequence: u64, text: &str) {
        let record = JournalEvent { sequence, received_at: sequence, source: "hook".into(), provider: "claude".into(), payload: json!({"text":text}) };
        store.append(sequence, &serde_json::to_string(&record).unwrap()).unwrap();
    }

    #[test]
    fn continuation_and_sequence_seek_decode_only_the_requested_page() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(directory.path().into(), 5).unwrap();
        for sequence in 1..=5000 { append(&mut store, sequence, if sequence % 1000 == 0 { "needle 中文" } else { "other" }); }
        let first = store.snapshot(None, None).unwrap().unwrap().read(10, "", None).unwrap();
        assert_eq!(first.events[0].sequence, 5000);
        assert_eq!(first.stats.records, 11);
        assert_eq!(first.stats.bytes, READ_BYTES);
        let second = store.snapshot(first.next.as_ref(), None).unwrap().unwrap().read(10, "", None).unwrap();
        assert_eq!(second.events[0].sequence, 4990);
        assert_eq!(second.stats.records, 11);
        assert_eq!(second.stats.bytes, READ_BYTES);
        let deep = store.snapshot(None, Some(2500)).unwrap().unwrap().read(10, "", None).unwrap();
        assert_eq!(deep.events[0].sequence, 2499);
        assert_eq!(deep.stats.records, 11);
        assert_eq!(deep.stats.bytes, READ_BYTES);

        let first = store.snapshot(None, None).unwrap().unwrap().read(1, "needle", Some("hook")).unwrap();
        let second = store.snapshot(first.next.as_ref(), None).unwrap().unwrap().read(1, "needle", Some("hook")).unwrap();
        assert_eq!(first.events[0].sequence, 5000);
        assert_eq!(second.events[0].sequence, 4000);
        assert_eq!(first.stats.records, 1001);
        assert_eq!(second.stats.records, 1001); // No repeated scan of 4999..4001.
    }

    #[test]
    fn cursor_survives_append_rotation_and_restart_but_expires_after_retention() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(directory.path().into(), 1).unwrap();
        for sequence in 1..=6 { append(&mut store, sequence, "raw"); }
        let first = store.snapshot(None, None).unwrap().unwrap().read(2, "", None).unwrap();
        let cursor = first.next.unwrap();
        append(&mut store, 7, "new");
        store.rotate().unwrap();
        append(&mut store, 8, "new segment");
        let second = store.snapshot(Some(&cursor), None).unwrap().unwrap().read(2, "", None).unwrap();
        assert_eq!(second.events.iter().map(|e| e.sequence).collect::<Vec<_>>(), [4, 3]);
        assert_eq!(second.stats.files, 1);
        drop(store);
        let mut store = HistoryStore::open(directory.path().into(), 1).unwrap();
        let second = store.snapshot(Some(&cursor), None).unwrap().unwrap().read(2, "", None).unwrap();
        assert_eq!(second.events[0].sequence, 4);
        store.rotate().unwrap();
        assert!(store.snapshot(Some(&cursor), None).unwrap().is_none());
    }

    #[test]
    fn in_flight_reader_pins_retired_files_and_never_sees_later_appends() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(directory.path().into(), 0).unwrap();
        append(&mut store, 1, "before snapshot");
        let snapshot = store.snapshot(None, None).unwrap().unwrap();
        let archived = directory.path().join(format!("events.{}.jsonl", store.active.id));
        append(&mut store, 2, "after snapshot");
        store.rotate().unwrap();
        assert!(archived.exists());
        append(&mut store, 3, "next file");
        let page = snapshot.read(10, "", None).unwrap();
        assert_eq!(page.events.iter().map(|e| e.sequence).collect::<Vec<_>>(), [1]);
        assert!(!archived.exists());
        assert!(!index_path(&archived).exists());
    }

    #[test]
    fn stale_and_corrupt_indexes_rebuild_without_changing_valid_segment_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(directory.path().into(), 5).unwrap();
        for sequence in 1..=100 { append(&mut store, sequence, "persistent"); }
        let id = store.active.id.clone();
        let path = index_path(&store.active.path);
        drop(store);
        OpenOptions::new().write(true).open(&path).unwrap().set_len((INDEX_BYTES * 3) as u64).unwrap();
        let store = HistoryStore::open(directory.path().into(), 5).unwrap();
        assert_eq!(store.active.id, id);
        assert_eq!(store.active.offsets.len(), 100);
        drop(store);
        let mut corrupt = fs::read(&path).unwrap(); corrupt[INDEX_BYTES + 10] ^= 1;
        fs::write(&path, corrupt).unwrap();
        let store = HistoryStore::open(directory.path().into(), 5).unwrap();
        assert_eq!(store.active.id, id);
        assert_eq!(store.last_sequence(), 100);
        assert_eq!(store.active.offsets.len(), 100);
        drop(store);
        // A missing archive sidecar retains identity via the stable filename.
        let mut store = HistoryStore::open(directory.path().into(), 5).unwrap();
        store.rotate().unwrap();
        let path = index_path(&store.archives[0].path);
        drop(store);
        fs::remove_file(path).unwrap();
        let store = HistoryStore::open(directory.path().into(), 5).unwrap();
        assert_eq!(store.archives[0].id, id);
        assert_eq!(store.archives[0].offsets.len(), 100);
    }

    #[test]
    fn numbered_archives_migrate_and_large_utf8_records_cross_read_blocks() {
        let directory = tempfile::tempdir().unwrap();
        let raw = "原始🦀".repeat(20000);
        let record = JournalEvent { sequence: 1, received_at: 1, source: "hook".into(), provider: "claude".into(), payload: json!({"text":raw}) };
        fs::write(directory.path().join("events.jsonl.1"), format!("{}\n{{partial", serde_json::to_string(&record).unwrap())).unwrap();
        let mut store = HistoryStore::open(directory.path().into(), 5).unwrap();
        append(&mut store, 2, "latest");
        let page = store.snapshot(None, None).unwrap().unwrap().read(10, "原始", None).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].payload["text"], raw);
        assert!(!directory.path().join("events.jsonl.1").exists());
        assert!(store.archives[0].path.exists());
        assert!(index_path(&store.archives[0].path).exists());
    }

    #[test]
    fn invalid_offset_and_torn_rotation_identity_are_handled_explicitly() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(directory.path().into(), 5).unwrap();
        append(&mut store, 1, "one");
        let id = store.active.id.clone();
        assert!(store.snapshot(Some(&HistoryCursor { segment_id: id.clone(), before_offset: 1 }), None).is_err());
        drop(store);
        // Simulate a crash after the data rename, before the sidecar rename.
        fs::rename(directory.path().join("events.jsonl"), directory.path().join(format!("events.{id}.jsonl"))).unwrap();
        let store = HistoryStore::open(directory.path().into(), 5).unwrap();
        assert_ne!(store.active.id, id);
        assert_eq!(store.archives[0].id, id);
        assert_eq!(store.last_sequence(), 1);
    }
}
