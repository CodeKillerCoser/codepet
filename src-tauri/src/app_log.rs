use chrono::{DateTime, Local, NaiveDate, SecondsFormat};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const LOG_FILE_NAME: &str = "code-pet.log";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn init_app_logging() -> io::Result<PathBuf> {
    let data_dir = code_pet_data_dir(&default_data_root());
    let path = log_file_path(&data_dir);
    rotate_log_file_for_date_if_needed(&path, log_file_modified_date(&path)?, Local::now().date_naive())?;
    rotate_log_file_if_needed(&path, MAX_LOG_BYTES)?;
    if LOG_FILE.get().is_none() {
        let file = open_log_file(&path)?;
        let _ = LOG_FILE.set(Mutex::new(file));
    }
    install_panic_hook();
    info("app", &format!("logging initialized path={}", path.display()));
    Ok(path)
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
) -> io::Result<()> {
    let Some(date) = active_date else {
        return Ok(());
    };
    if !path.exists() || date >= current_date {
        return Ok(());
    }

    fs::rename(path, next_available_archive_path(path, date))
}

pub fn rotate_log_file_if_needed(path: &Path, max_bytes: u64) -> io::Result<()> {
    if !path.exists() || path.metadata()?.len() <= max_bytes {
        return Ok(());
    }

    let rotated_path = path.with_extension("log.1");
    if rotated_path.exists() {
        fs::remove_file(&rotated_path)?;
    }
    fs::rename(path, rotated_path)
}

pub fn append_log_line_to(path: &Path, level: &str, target: &str, message: &str) -> io::Result<()> {
    let mut file = open_log_file(path)?;
    writeln!(file, "{}", format_log_line(level, target, message))?;
    file.flush()
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

fn default_data_root() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
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

fn format_log_line(level: &str, target: &str, message: &str) -> String {
    let timestamp = Local::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    format!(
        "{timestamp} {level:<5} [{target}] {}",
        message.replace('\n', "\\n")
    )
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
