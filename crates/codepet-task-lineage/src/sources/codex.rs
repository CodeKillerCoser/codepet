use super::ConversationSource;
use crate::{domain::*, stable_id, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub struct CodexSource {
    pub home: PathBuf,
}

pub fn data_directory(settings: &Value) -> Result<PathBuf> {
    if let Some(path) = settings
        .get("dataDirectory")
        .or_else(|| settings.get("codexHome"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("Codex data directory must be absolute".into());
        }
        return Ok(path);
    }
    codepet_provider_sdk::local_runtime::data_dir("CODEX_HOME", ".codex")
        .ok_or("Codex home unavailable".into())
}

fn walk(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            walk(&entry.path(), files)?;
        } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn tagged<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    text.split_once(&start)?
        .1
        .split_once(&end)
        .map(|(value, _)| value)
}

fn metadata(path: &Path) -> Result<Thread> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    for line in BufReader::new(file).lines().take(32) {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if row["type"] != "session_meta" {
            continue;
        }
        let payload = &row["payload"];
        let id = string(payload, "id").ok_or("session_meta missing id")?;
        let fork = string(payload, "forked_from_id");
        let parent = fork.clone().or_else(|| {
            payload
                .pointer("/source/subagent/thread_spawn/parent_thread_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let created_by = if payload["thread_source"] == "user" {
            "human"
        } else if parent.is_some() || payload["thread_source"] == "agent_created_thread" {
            "agent"
        } else {
            "unknown"
        };
        let kind = if fork.is_some() {
            "fork"
        } else if parent.is_some() {
            "subagent"
        } else if created_by == "human" {
            "main"
        } else {
            "unknown"
        };
        return Ok(Thread {
            title: id.clone(),
            id,
            workspace: string(payload, "cwd").unwrap_or_default(),
            created_by: created_by.into(),
            creation_kind: kind.into(),
            parent_id: parent,
            source_file: path.to_string_lossy().into_owned(),
            timestamp: string(payload, "timestamp"),
            inherited_end_byte_offset: payload
                .pointer("/history_base/end_byte_offset")
                .and_then(Value::as_u64),
        });
    }
    Err("No session metadata".into())
}

fn prefix_hasher(file: &mut File, length: u64) -> Result<Sha256> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut reader = file.take(length);
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher)
}
fn hash_prefix(file: &mut File, length: u64) -> Result<String> {
    Ok(format!("{:x}", prefix_hasher(file, length)?.finalize()))
}

impl ConversationSource for CodexSource {
    fn discover(&self) -> Result<Vec<Thread>> {
        if !self.home.is_absolute() {
            return Err("Codex home must be absolute".into());
        }
        if !self.home.is_dir() {
            return Err("Codex data directory does not exist".into());
        }
        let mut files = Vec::new();
        walk(&self.home.join("sessions"), &mut files)?;
        walk(&self.home.join("archived_sessions"), &mut files)?;
        files.sort();
        let mut titles = HashMap::new();
        if let Ok(file) = File::open(self.home.join("session_index.jsonl")) {
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|e| e.to_string())?;
                if let Ok(row) = serde_json::from_str::<Value>(&line) {
                    if let (Some(id), Some(title)) =
                        (string(&row, "id"), string(&row, "thread_name"))
                    {
                        titles.insert(id, title);
                    }
                }
            }
        }
        let mut threads = HashMap::new();
        for path in files {
            // Empty/incomplete files can be concurrently created by the source.
            if fs::metadata(&path).map_err(|e| e.to_string())?.len() == 0 {
                continue;
            }
            let mut thread = metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if let Some(title) = titles.get(&thread.id) {
                thread.title = title.clone();
            }
            threads.insert(thread.id.clone(), thread);
        }
        let mut threads: Vec<_> = threads.into_values().collect();
        threads.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(a.id.cmp(&b.id)));
        Ok(threads)
    }

    fn read(&self, thread: &Thread, cursor: &Cursor) -> Result<SourceBatch> {
        let path = Path::new(&thread.source_file);
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        let home = self.home.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(&home) {
            return Err("Source file outside Codex home".into());
        }
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let length = metadata.len();
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        if cursor.offset > 0
            && modified_nanos != 0
            && modified_nanos == cursor.modified_nanos
            && length == cursor.observed_length
        {
            return Ok(SourceBatch {
                thread: thread.clone(),
                messages: vec![],
                links: vec![],
                cursor: cursor.clone(),
                reset: false,
                last_runtime_event: None,
            });
        }
        let reset = cursor.offset > 0
            && (length < cursor.offset
                || hash_prefix(&mut file, cursor.offset)? != cursor.prefix_hash);
        let generation = cursor.generation + u64::from(reset);
        let mut offset = if reset { 0 } else { cursor.offset };
        let mut scanned_hash = prefix_hasher(&mut file, offset)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(file);
        let mut messages = Vec::new();
        let mut links = Vec::new();
        let mut turn_id = if reset {
            None
        } else {
            cursor.current_turn_id.clone()
        };
        let mut runtime = None;
        loop {
            let start = offset;
            let mut bytes = Vec::new();
            let n = reader
                .read_until(b'\n', &mut bytes)
                .map_err(|e| e.to_string())?;
            if n == 0 || bytes.last() != Some(&b'\n') {
                break;
            }
            let row: Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid source record at {start}: {e}"))?;
            offset += n as u64;
            scanned_hash.update(&bytes);
            let evidence = Evidence {
                event_id: stable_id(format!(
                    "codex:{}:{generation}:{start}:{}",
                    thread.id,
                    stable_id(&bytes)
                )),
                file: thread.source_file.clone(),
                byte_offset: start,
                generation,
            };
            let payload = &row["payload"];
            if row["type"] == "session_meta" {
                if let Some(parent_id) = &thread.parent_id {
                    links.push(LineageLink {
                        parent_id: parent_id.clone(),
                        child_id: thread.id.clone(),
                        kind: thread.creation_kind.clone(),
                        evidence,
                        timestamp: string(&row, "timestamp"),
                    });
                }
            } else if row["type"] == "turn_context" {
                turn_id = string(payload, "turn_id");
            } else if row["type"] == "event_msg" {
                match payload["type"].as_str() {
                    Some("task_started") => {
                        runtime = Some("running".into());
                        turn_id = string(payload, "turn_id");
                    }
                    Some("task_complete" | "task_completed" | "turn_aborted") => {
                        runtime = Some("stopped".into())
                    }
                    _ => {}
                }
            } else if row["type"] == "response_item"
                && payload["type"] == "function_call_output"
                && matches!(
                    payload["name"].as_str(),
                    Some("create_thread" | "send_message_to_thread")
                )
            {
                // Codex records native delegation on the CHILD thread. This is not a child ID returned to the parent.
                if let Some(output) = payload["output"]
                    .as_str()
                    .filter(|s| s.trim_start().starts_with("<codex_delegation>"))
                {
                    if let Some(parent) = tagged(output, "source_thread_id").filter(|id| {
                        id.len() == 36
                            && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                            && *id != thread.id
                    }) {
                        links.push(LineageLink {
                            parent_id: parent.into(),
                            child_id: thread.id.clone(),
                            kind: if payload["name"] == "create_thread" {
                                "newThread"
                            } else {
                                "message"
                            }
                            .into(),
                            evidence: evidence.clone(),
                            timestamp: string(&row, "timestamp"),
                        });
                    }
                }
            } else if row["type"] == "response_item" && payload["type"] == "message" {
                let role = payload["role"].as_str().unwrap_or("");
                if !matches!(role, "user" | "assistant") {
                    continue;
                }
                let text = payload["content"]
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| {
                                if matches!(
                                    part["type"].as_str(),
                                    Some("input_text" | "output_text" | "text")
                                ) {
                                    part["text"].as_str()
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                if text.is_empty() {
                    continue;
                }
                messages.push(Message {
                    evidence,
                    thread_id: thread.id.clone(),
                    role: role.into(),
                    text,
                    timestamp: string(&row, "timestamp"),
                    turn_id: turn_id.clone(),
                });
            }
        }
        let mut file = reader.into_inner();
        let prefix_hash = format!("{:x}", scanned_hash.finalize());
        if hash_prefix(&mut file, offset)? != prefix_hash {
            return Err("Source changed while being read; retry scan".into());
        }
        Ok(SourceBatch {
            thread: thread.clone(),
            messages,
            links,
            reset,
            last_runtime_event: runtime,
            cursor: Cursor {
                offset,
                generation,
                prefix_hash,
                prefix_length: 0,
                current_turn_id: turn_id,
                observed_length: length,
                modified_nanos,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture() -> (tempfile::TempDir, CodexSource, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        let path = sessions.join("a.jsonl");
        fs::write(&path,"{\"type\":\"session_meta\",\"payload\":{\"id\":\"a\",\"thread_source\":\"user\",\"cwd\":\"/repo\"}}\n").unwrap();
        let source = CodexSource {
            home: dir.path().to_owned(),
        };
        (dir, source, path)
    }
    const MESSAGE: &str="{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"修复焦点\"}]}}";
    #[test]
    fn partial_record_is_retried_and_append_is_idempotent() {
        let (_dir, source, path) = fixture();
        let thread = source.discover().unwrap().remove(0);
        let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(MESSAGE.as_bytes()).unwrap();
        let first = source.read(&thread, &Cursor::default()).unwrap();
        assert!(first.messages.is_empty());
        writer.write_all(b"\n").unwrap();
        let second = source.read(&thread, &first.cursor).unwrap();
        assert_eq!(second.messages.len(), 1);
        assert_eq!(second.messages[0].text, "修复焦点");
        assert!(source
            .read(&thread, &second.cursor)
            .unwrap()
            .messages
            .is_empty());
    }
    #[test]
    fn same_length_rewrite_changes_generation() {
        let (_dir, source, path) = fixture();
        let original = fs::read_to_string(&path).unwrap() + MESSAGE + "\n";
        fs::write(&path, &original).unwrap();
        let thread = source.discover().unwrap().remove(0);
        let first = source.read(&thread, &Cursor::default()).unwrap();
        fs::write(&path, original.replace("修复焦点", "修复按钮")).unwrap();
        let second = source.read(&thread, &first.cursor).unwrap();
        assert!(second.reset);
        assert_eq!(second.cursor.generation, 1);
        assert_ne!(
            first.messages[0].evidence.event_id,
            second.messages[0].evidence.event_id
        );
    }
    #[test]
    fn only_user_and_assistant_text_are_extraction_messages() {
        let (_dir, source, path) = fixture();
        let thread = source.discover().unwrap().remove(0);
        let records = [
            serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"TOOL_ARGUMENT_SECRET"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","output":"TOOL_RESULT_SECRET"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"reasoning","summary":[{"text":"REASONING_SECRET"}]}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","name":"create_thread","output":"<codex_delegation><source_thread_id>00000000-0000-0000-0000-000000000001</source_thread_id><input>DELEGATION_TOOL_SECRET</input></codex_delegation>"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"AI 正文"},{"type":"reasoning_text","text":"HIDDEN"}]}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"agent_message","message":"DUPLICATE_EVENT_TEXT"}}),
        ];
        let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(writer, "{MESSAGE}").unwrap();
        for record in records {
            writeln!(writer, "{record}").unwrap();
        }
        let batch = source.read(&thread, &Cursor::default()).unwrap();
        assert_eq!(
            batch
                .messages
                .iter()
                .map(|m| m.text.as_str())
                .collect::<Vec<_>>(),
            vec!["修复焦点", "AI 正文"]
        );
        assert_eq!(batch.links.len(), 1);
        assert_eq!(
            batch.links[0].parent_id,
            "00000000-0000-0000-0000-000000000001"
        );
    }
}
