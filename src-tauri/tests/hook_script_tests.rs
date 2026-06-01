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
