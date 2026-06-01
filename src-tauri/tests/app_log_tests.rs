use code_pet_lib::app_log::{
    append_app_start_banner_to, append_log_file_header_to, append_log_line_to,
    append_perf_event_to, code_pet_data_dir, log_file_path, rotate_log_file_for_date_if_needed,
    rotate_log_file_if_needed, PerfEvent,
};
use chrono::NaiveDate;
use std::collections::BTreeMap;

#[test]
fn log_file_lives_under_code_pet_logs_directory() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = code_pet_data_dir(root.path());

    assert_eq!(data_dir, root.path().join("code-pet"));
    assert_eq!(log_file_path(&data_dir), root.path().join("code-pet").join("logs").join("code-pet.log"));
}

#[test]
fn appends_structured_log_lines_to_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");

    append_log_line_to(&path, "INFO", "test", "hello logging").unwrap();

    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("INFO"));
    assert!(text.contains("[test]"));
    assert!(text.contains("hello logging"));
}

#[test]
fn rotates_large_log_file_before_appending() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "abcdef").unwrap();

    rotate_log_file_if_needed(&path, 5).unwrap();

    assert!(!path.exists());
    assert_eq!(std::fs::read_to_string(path.with_extension("log.1")).unwrap(), "abcdef");
}

#[test]
fn rotates_stale_log_file_to_date_archive() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "old log").unwrap();

    rotate_log_file_for_date_if_needed(
        &path,
        Some(NaiveDate::from_ymd_opt(2026, 5, 30).unwrap()),
        NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(),
    )
    .unwrap();

    assert!(!path.exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("logs").join("code-pet.2026-05-30.log")).unwrap(),
        "old log"
    );
}

#[test]
fn keeps_current_day_log_file_active() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "today log").unwrap();

    rotate_log_file_for_date_if_needed(
        &path,
        Some(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()),
        NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(path).unwrap(), "today log");
}

#[test]
fn does_not_overwrite_existing_date_archives() {
    let root = tempfile::tempdir().unwrap();
    let log_dir = root.path().join("logs");
    let path = log_dir.join("code-pet.log");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("code-pet.2026-05-30.log"), "first archive").unwrap();
    std::fs::write(&path, "second archive").unwrap();

    rotate_log_file_for_date_if_needed(
        &path,
        Some(NaiveDate::from_ymd_opt(2026, 5, 30).unwrap()),
        NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(log_dir.join("code-pet.2026-05-30.log")).unwrap(), "first archive");
    assert_eq!(std::fs::read_to_string(log_dir.join("code-pet.2026-05-30.1.log")).unwrap(), "second archive");
}

#[test]
fn appends_perf_events_as_parseable_key_value_lines() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");
    let mut fields = BTreeMap::new();
    fields.insert("events".to_string(), "42".into());
    fields.insert("audit_bytes".to_string(), "27946907".into());

    append_perf_event_to(
        &path,
        &PerfEvent {
            name: "startup.codex_audit_replay".to_string(),
            duration_ms: 1384.4,
            status: Some("ok".to_string()),
            fields,
            error: None,
        },
    )
    .unwrap();

    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("[perf]"));
    assert!(text.contains("name=startup.codex_audit_replay"));
    assert!(text.contains("status=ok"));
    assert!(text.contains("duration_ms=1384"));
    assert!(text.contains("events=42"));
    assert!(text.contains("audit_bytes=27946907"));
}

#[test]
fn app_start_banner_is_visually_distinct_from_log_file_header() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");

    append_app_start_banner_to(&path).unwrap();

    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("==================== CODE PET APP START ===================="));
    assert!(text.contains("version="));
    assert!(text.contains("pid="));
    assert!(!text.contains("CODE PET LOG FILE OPENED"));
}

#[test]
fn log_file_header_marks_new_files_without_looking_like_app_start() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("logs").join("code-pet.log");

    append_log_file_header_to(&path, "rotated_by_date").unwrap();

    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("==================== CODE PET LOG FILE OPENED ===================="));
    assert!(text.contains("reason=rotated_by_date"));
    assert!(text.contains("version="));
    assert!(text.contains("pid="));
    assert!(!text.contains("CODE PET APP START"));
}
