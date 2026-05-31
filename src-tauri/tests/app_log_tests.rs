use code_pet_lib::app_log::{
    append_log_line_to, code_pet_data_dir, log_file_path, rotate_log_file_for_date_if_needed,
    rotate_log_file_if_needed,
};
use chrono::NaiveDate;

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
