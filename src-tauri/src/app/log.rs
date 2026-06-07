use crate::settings::current_app_data_dir;
use chrono::{DateTime, Local, NaiveDate, SecondsFormat};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const LOG_FILE_NAME: &str = "code-pet.log";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const APP_NAME: &str = "Code Pet";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfEvent {
    pub name: String,
    pub duration_ms: f64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub error: Option<String>,
}

pub struct PerfSpan {
    name: &'static str,
    started_at: Instant,
}

impl PerfSpan {
    pub fn start(name: &'static str) -> Self {
        Self {
            name,
            started_at: Instant::now(),
        }
    }

    pub fn finish_ok(self, fields: &[(&str, String)]) {
        self.finish("ok", None, fields);
    }

    pub fn finish_error(self, error: &str, fields: &[(&str, String)]) {
        self.finish("error", Some(error.to_string()), fields);
    }

    fn finish(self, status: &str, error: Option<String>, fields: &[(&str, String)]) {
        record_perf_event(PerfEvent {
            name: self.name.to_string(),
            duration_ms: self.started_at.elapsed().as_secs_f64() * 1000.0,
            status: Some(status.to_string()),
            fields: fields
                .iter()
                .map(|(key, value)| ((*key).to_string(), Value::String(value.clone())))
                .collect(),
            error,
        });
    }
}

pub fn init_app_logging() -> io::Result<PathBuf> {
    let data_dir = current_app_data_dir();
    let path = log_file_path(&data_dir);
    let existed_before_rotation = path.exists();
    let rotated_by_date = rotate_log_file_for_date_if_needed(&path, log_file_modified_date(&path)?, Local::now().date_naive())?;
    let rotated_by_size = rotate_log_file_if_needed(&path, MAX_LOG_BYTES)?;
    let header_reason = if rotated_by_date {
        Some("rotated_by_date")
    } else if rotated_by_size {
        Some("rotated_by_size")
    } else if !existed_before_rotation {
        Some("created")
    } else {
        None
    };
    if LOG_FILE.get().is_none() {
        let mut file = open_log_file(&path)?;
        if let Some(reason) = header_reason {
            write_banner(&mut file, "CODE PET LOG FILE OPENED", &banner_fields(reason))?;
        }
        let _ = LOG_FILE.set(Mutex::new(file));
    }
    install_panic_hook();
    info("app", &format!("logging initialized path={}", path.display()));
    Ok(path)
}

pub fn log_app_start_banner() {
    write_global_banner("CODE PET APP START", &banner_fields("app_start"));
}

pub fn code_pet_data_dir(root: &Path) -> PathBuf {
    root.join("code-pet")
}

pub fn log_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join(LOG_FILE_NAME)
}

pub fn rotate_log_file_for_date_if_needed(
    path: &Path,
    active_date: Option<NaiveDate>,
    current_date: NaiveDate,
) -> io::Result<bool> {
    let Some(date) = active_date else {
        return Ok(false);
    };
    if !path.exists() || date >= current_date {
        return Ok(false);
    }

    fs::rename(path, next_available_archive_path(path, date))?;
    Ok(true)
}

pub fn rotate_log_file_if_needed(path: &Path, max_bytes: u64) -> io::Result<bool> {
    if !path.exists() || path.metadata()?.len() <= max_bytes {
        return Ok(false);
    }

    let rotated_path = path.with_extension("log.1");
    if rotated_path.exists() {
        fs::remove_file(&rotated_path)?;
    }
    fs::rename(path, rotated_path)?;
    Ok(true)
}

pub fn append_log_line_to(path: &Path, level: &str, target: &str, message: &str) -> io::Result<()> {
    let mut file = open_log_file(path)?;
    writeln!(file, "{}", format_log_line(level, target, message))?;
    file.flush()
}

pub fn append_app_start_banner_to(path: &Path) -> io::Result<()> {
    let mut file = open_log_file(path)?;
    write_banner(&mut file, "CODE PET APP START", &banner_fields("app_start"))
}

pub fn append_log_file_header_to(path: &Path, reason: &str) -> io::Result<()> {
    let mut file = open_log_file(path)?;
    write_banner(&mut file, "CODE PET LOG FILE OPENED", &banner_fields(reason))
}

pub fn append_perf_event_to(path: &Path, event: &PerfEvent) -> io::Result<()> {
    append_log_line_to(path, "INFO", "perf", &format_perf_event(event))
}

pub fn record_perf_event(event: PerfEvent) {
    write_global("INFO", "perf", &format_perf_event(&event));
}

pub fn info(target: &str, message: &str) {
    write_global("INFO", target, message);
}

pub fn warn(target: &str, message: &str) {
    write_global("WARN", target, message);
}

pub fn error(target: &str, message: &str) {
    write_global("ERROR", target, message);
}

fn log_file_modified_date(path: &Path) -> io::Result<Option<NaiveDate>> {
    if !path.exists() {
        return Ok(None);
    }
    let modified = path.metadata()?.modified()?;
    Ok(Some(DateTime::<Local>::from(modified).date_naive()))
}

fn next_available_archive_path(path: &Path, date: NaiveDate) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or(LOG_FILE_NAME);
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("log");
    let base = parent.join(format!("{stem}.{date}.{extension}"));
    if !base.exists() {
        return base;
    }

    for index in 1.. {
        let candidate = parent.join(format!("{stem}.{date}.{index}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("date archive index loop should always find a candidate")
}

fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

fn write_global(level: &str, target: &str, message: &str) {
    let Some(file) = LOG_FILE.get() else {
        return;
    };
    let Ok(mut file) = file.lock() else {
        return;
    };
    let _ = writeln!(file, "{}", format_log_line(level, target, message));
    let _ = file.flush();
}

fn write_global_banner(title: &str, fields: &[(&str, String)]) {
    let Some(file) = LOG_FILE.get() else {
        return;
    };
    let Ok(mut file) = file.lock() else {
        return;
    };
    let _ = write_banner(&mut file, title, fields);
}

fn write_banner(file: &mut File, title: &str, fields: &[(&str, String)]) -> io::Result<()> {
    writeln!(file)?;
    writeln!(file, "==================== {title} ====================")?;
    for (key, value) in fields {
        writeln!(file, "{key}={value}")?;
    }
    writeln!(file, "==================================================")?;
    file.flush()
}

fn banner_fields(reason: &str) -> Vec<(&'static str, String)> {
    vec![
        ("time", Local::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        ("app", APP_NAME.to_string()),
        ("version", APP_VERSION.to_string()),
        ("pid", std::process::id().to_string()),
        ("reason", reason.to_string()),
    ]
}

fn format_log_line(level: &str, target: &str, message: &str) -> String {
    let timestamp = Local::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    format!(
        "{timestamp} {level:<5} [{target}] {}",
        message.replace('\n', "\\n")
    )
}

fn format_perf_event(event: &PerfEvent) -> String {
    let mut parts = vec![
        format!("name={}", perf_value(&Value::String(event.name.clone()))),
        format!("status={}", event.status.as_deref().unwrap_or("ok")),
        format!("duration_ms={:.0}", event.duration_ms.max(0.0)),
    ];

    for (key, value) in &event.fields {
        if is_safe_key(key) {
            parts.push(format!("{key}={}", perf_value(value)));
        }
    }

    if let Some(error) = &event.error {
        parts.push(format!("error={}", perf_value(&Value::String(error.clone()))));
    }
    parts.join(" ")
}

fn perf_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) if value.chars().all(is_safe_value_char) && !value.is_empty() => value.clone(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"<invalid>\"".to_string()),
        other => serde_json::to_string(other).unwrap_or_else(|_| "\"<invalid>\"".to_string()),
    }
}

fn is_safe_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '.')
}

fn is_safe_value_char(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(character, '_' | '-' | '.' | '/' | ':' | '@')
}

fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        error("panic", &panic_info.to_string());
        previous(panic_info);
    }));
}
