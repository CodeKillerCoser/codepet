use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn hook_script_posts_fallback_event_when_stdin_stays_open() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/hooks/code-pet-hook.mjs");
    let mut child = Command::new("node")
        .arg(script_path)
        .arg("--agent")
        .arg("codex")
        .arg("--event")
        .arg("Stop")
        .env("CODE_PET_COLLECTOR_URL", "http://127.0.0.1:9/hook")
        .env("CODE_PET_STDIN_WAIT_MS", "20")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _open_stdin = child.stdin.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("hook script should not hang when an agent leaves stdin open");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn hook_script_spools_to_custom_app_data_directory() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let custom_data = home.path().join("custom-data");
    let settings_path = home.path().join("settings.json");
    std::fs::write(
        &settings_path,
        serde_json::json!({
            "data": {
                "dataDirectory": custom_data
            }
        })
        .to_string(),
    )
    .unwrap();
    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/hooks/code-pet-hook.mjs");

    let output = Command::new("node")
        .arg(script_path)
        .arg("--agent")
        .arg("codex")
        .arg("--event")
        .arg("Stop")
        .env("CODE_PET_COLLECTOR_URL", "http://127.0.0.1:9/hook")
        .env("CODE_PET_SETTINGS_PATH", &settings_path)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(custom_data.join("spool").join("events.jsonl").exists());
    assert!(!home.path().join(".code-pet").join("spool").join("events.jsonl").exists());
}
