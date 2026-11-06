use serde::{Deserialize, Serialize};
use std::panic::Location;
use std::time::{SystemTime, UNIX_EPOCH};

/// Event origin, captured before handing diagnostics to another thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogOrigin {
    pub timestamp_unix_ms: u64,
    pub pid: u32,
    pub thread_id: Option<u64>,
    pub thread_name: String,
    pub source_file: String,
    pub source_line: u32,
}

impl LogOrigin {
    #[track_caller]
    pub fn capture() -> Self {
        let caller = Location::caller();
        Self {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            pid: std::process::id(),
            thread_id: current_thread_id(),
            thread_name: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string(),
            source_file: caller.file().to_string(),
            source_line: caller.line(),
        }
    }
}

pub fn format_text_log(origin: &LogOrigin, level: &str, target: &str, message: &str) -> String {
    let time =
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(origin.timestamp_unix_ms as i64)
            .map(|value| value.format("%Y-%m-%d %H:%M:%S%.3f+00:00").to_string())
            .unwrap_or_else(|| "unknown-time".to_string());
    format!(
        "{time} pid={} thread={} {level:<5} {}:{} [{}] {} | threadName={}",
        origin.pid,
        origin
            .thread_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".into()),
        clean(&origin.source_file),
        origin.source_line,
        clean(target),
        clean(message),
        clean(&origin.thread_name)
    )
}

fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Decode the in-memory queue envelope; only this text is persisted or printed.
pub fn format_rtc_log(envelope: &str) -> Option<String> {
    let record: serde_json::Value = serde_json::from_str(envelope).ok()?;
    let origin: LogOrigin = serde_json::from_value(record.get("origin")?.clone()).ok()?;
    let event = record.get("event")?.as_str()?;
    let mut message = event.to_string();
    fn append(out: &mut String, key: &str, value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(values) => {
                for (k, v) in values {
                    append(out, &format!("{key}.{k}"), v);
                }
            }
            serde_json::Value::Array(values) => {
                out.push_str(&format!(" | {key}.count={}", values.len()));
                for (i, v) in values.iter().enumerate() {
                    append(out, &format!("{key}[{i}]"), v);
                }
            }
            serde_json::Value::String(text) => out.push_str(&format!(" | {key}={text}")),
            other => out.push_str(&format!(" | {key}={other}")),
        }
    }
    for (key, value) in record.as_object()? {
        if !["origin", "event", "timestampUnixMs", "schema"].contains(&key.as_str()) {
            append(&mut message, key, value);
        }
    }
    let level = if event.ends_with(".failed") || record["fields"]["outcome"] == "failed" {
        "WARN"
    } else {
        "INFO"
    };
    Some(format_text_log(&origin, level, "rtc", &message))
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn current_thread_id() -> Option<u64> {
    let mut id = 0;
    // Zero requests the calling thread. The output pointer lives for the call.
    (unsafe { libc::pthread_threadid_np(0, &mut id) } == 0).then_some(id)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn current_thread_id() -> Option<u64> {
    let id = unsafe { libc::syscall(libc::SYS_gettid) };
    (id > 0).then_some(id as u64)
}

#[cfg(windows)]
fn current_thread_id() -> Option<u64> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    Some(unsafe { GetCurrentThreadId() } as u64)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn current_thread_id() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rtc_writer_formats_original_origin_and_flat_fields() {
        let origin = LogOrigin {
            timestamp_unix_ms: 1793970672216,
            pid: 321,
            thread_id: Some(654),
            thread_name: "rtc-worker".into(),
            source_file: "transport.rs".into(),
            source_line: 72,
        };
        let envelope = serde_json::json!({"origin":origin, "event":"connect.failed",
            "offerId":"abc", "fields":{"stats":[{"state":"failed"}]}})
        .to_string();
        let line = std::thread::spawn(move || format_rtc_log(&envelope).unwrap())
            .join()
            .unwrap();
        assert!(line.contains("pid=321 thread=654 WARN  transport.rs:72 [rtc] connect.failed"));
        assert!(line.contains("fields.stats[0].state=failed"));
        assert!(line.contains("offerId=abc"));
        assert!(!line.contains('{'));
        assert!(line.starts_with("2026-11-06 "));
    }
    #[test]
    fn producer_origin_survives_a_writer_thread() {
        let line = line!() + 1;
        let origin = LogOrigin::capture();
        assert_eq!(origin.source_line, line);
        assert_eq!(origin.source_file, file!());
        assert_eq!(origin.pid, std::process::id());
        let producer_thread = origin.thread_id;
        let received = std::thread::spawn(move || {
            let writer = LogOrigin::capture();
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "linux",
                target_os = "android",
                windows
            ))]
            assert_ne!(writer.thread_id, origin.thread_id);
            origin
        })
        .join()
        .unwrap();
        assert_eq!(received.thread_id, producer_thread);
        assert_eq!(received.source_line, line);
    }
}
