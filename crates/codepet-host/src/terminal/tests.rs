use super::*;

fn open(manager: &TerminalManager, owner: &str, key: &str) -> gateway::TerminalSession {
    manager
        .open(
            owner,
            gateway::TerminalOpenRequest {
                session_key: key.into(),
                cwd: None,
                columns: 80,
                rows: 24,
            },
        )
        .unwrap()
        .session
}

#[test]
fn bounded_output_preserves_bytes_and_rejects_gaps() {
    let mut output = Output::default();
    let bytes = b"\x1b[31m\xe4\xb8\xad\r\x1b[2Kred\x1b[0m";
    output.append(&bytes[..8]);
    output.append(&bytes[8..]);
    let first = output.read(0).unwrap();
    assert_eq!(BASE64.decode(&first.data).unwrap(), bytes);
    assert_eq!(first.next_offset, bytes.len() as u64);
    assert_eq!(
        output.read(0).unwrap(),
        first,
        "read retries must be lossless"
    );
    assert_eq!(
        output.read(first.next_offset + 1).unwrap_err().code,
        "terminal_invalid_offset"
    );
    output.append(&vec![b'x'; output::OUTPUT_CAPACITY + 10]);
    assert_eq!(output.read(0).unwrap_err().code, "terminal_output_expired");
}

#[test]
fn sessions_are_idempotent_scoped_and_resize_the_real_pty() {
    let manager = TerminalManager::new();
    let first = open(&manager, "phone-a", "conversation-1");
    assert_eq!(
        first.terminal_id,
        open(&manager, "phone-a", "conversation-1").terminal_id
    );
    let other = open(&manager, "phone-b", "conversation-1");
    assert_ne!(first.terminal_id, other.terminal_id);
    assert_eq!(
        manager
            .read(
                "phone-b",
                gateway::TerminalReadRequest {
                    terminal_id: first.terminal_id.clone(),
                    offset: 0,
                }
            )
            .unwrap_err()
            .code,
        "terminal_not_found"
    );
    manager
        .resize(
            "phone-a",
            gateway::TerminalResizeRequest {
                terminal_id: first.terminal_id.clone(),
                columns: 107,
                rows: 37,
            },
        )
        .unwrap();
    let resized = open(&manager, "phone-a", "conversation-1");
    assert_eq!((resized.columns, resized.rows), (107, 37));
    manager
        .close(
            "phone-b",
            gateway::TerminalCloseRequest {
                terminal_id: first.terminal_id.clone(),
            },
        )
        .unwrap();
    assert!(manager.get("phone-a", &first.terminal_id).is_ok());
    manager
        .close(
            "phone-a",
            gateway::TerminalCloseRequest {
                terminal_id: first.terminal_id.clone(),
            },
        )
        .unwrap();
    manager
        .close(
            "phone-a",
            gateway::TerminalCloseRequest {
                terminal_id: first.terminal_id.clone(),
            },
        )
        .unwrap();
    assert!(manager.get("phone-a", &first.terminal_id).is_err());
    manager.shutdown();
}

#[cfg(unix)]
#[test]
fn real_shell_preserves_ansi_utf8_input_order_and_exit() {
    let manager = TerminalManager::new();
    let session = open(&manager, "phone", "interactive");
    // Avoid startup config changes. These are explicit input bytes to a real shell.
    let bytes = "printf '\\033[31m电脑输出\\033[0m\\r\\n'; stty size; exit\r".as_bytes();
    let request = gateway::TerminalWriteRequest {
        terminal_id: session.terminal_id.clone(),
        sequence: 1,
        data: BASE64.encode(bytes),
    };
    manager.write("phone", request.clone()).unwrap();
    manager.write("phone", request.clone()).unwrap();
    let mut conflicting = request;
    conflicting.data = BASE64.encode(b"different");
    assert_eq!(
        manager.write("phone", conflicting).unwrap_err().code,
        "terminal_input_sequence"
    );
    let mut offset = 0;
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let output = manager
            .read(
                "phone",
                gateway::TerminalReadRequest {
                    terminal_id: session.terminal_id.clone(),
                    offset,
                },
            )
            .unwrap();
        bytes.extend(BASE64.decode(output.data).unwrap());
        offset = output.next_offset;
        if output.exited {
            break;
        }
        assert!(Instant::now() < deadline, "real shell did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = String::from_utf8_lossy(&bytes);
    assert!(result.contains("\x1b[31m电脑输出\x1b[0m"), "{result:?}");
    assert!(
        result.contains("24 80"),
        "PTY window size must reach stty: {result:?}"
    );
    // The monitor reaps the process and clears the handle before close/drop.
    assert!(manager
        .get("phone", &session.terminal_id)
        .unwrap()
        .child
        .lock()
        .unwrap()
        .is_none());
    manager.shutdown();
}

#[test]
fn invalid_directory_and_dimensions_do_not_poison_other_sessions() {
    let manager = TerminalManager::new();
    assert!(manager
        .open(
            "phone",
            gateway::TerminalOpenRequest {
                session_key: "bad".into(),
                cwd: Some("relative/path".into()),
                columns: 80,
                rows: 24,
            }
        )
        .is_err());
    assert!(manager
        .open(
            "phone",
            gateway::TerminalOpenRequest {
                session_key: "bad".into(),
                cwd: None,
                columns: 0,
                rows: 24,
            }
        )
        .is_err());
    let healthy = open(&manager, "phone", "healthy");
    assert!(!healthy.exited);
    manager.shutdown();
}

#[test]
fn client_capacity_is_independent_and_shutdown_cleans_sessions() {
    let manager = TerminalManager::new();
    for n in 0..MAX_CLIENT_SESSIONS {
        open(&manager, "a", &n.to_string());
    }
    assert_eq!(
        manager
            .open(
                "a",
                gateway::TerminalOpenRequest {
                    session_key: "excess".into(),
                    cwd: None,
                    columns: 80,
                    rows: 24,
                }
            )
            .unwrap_err()
            .code,
        "terminal_capacity"
    );
    open(&manager, "b", "healthy");
    let processes: Vec<_> = manager
        .sessions
        .lock()
        .unwrap()
        .values()
        .map(|s| s.child.clone())
        .collect();
    manager.shutdown();
    assert!(manager.sessions.lock().unwrap().is_empty());
    let deadline = Instant::now() + Duration::from_secs(5);
    while processes.iter().any(|p| p.lock().unwrap().is_some()) {
        assert!(
            Instant::now() < deadline,
            "PTY child was not reaped after shutdown"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn reaping_finished_sessions_does_not_kill_an_offline_running_shell() {
    let manager = TerminalManager::new();
    let live = open(&manager, "phone", "live");
    let finished = open(&manager, "phone", "finished");
    manager
        .write(
            "phone",
            gateway::TerminalWriteRequest {
                terminal_id: finished.terminal_id.clone(),
                sequence: 1,
                data: BASE64.encode(b"exit\r"),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !manager
        .get("phone", &finished.terminal_id)
        .unwrap()
        .output
        .lock()
        .unwrap()
        .finished()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let old = Instant::now() - FINISHED_TTL - Duration::from_secs(1);
    for session in manager.sessions.lock().unwrap().values() {
        *session.touched.lock().unwrap() = old;
    }
    TerminalManager::reap_finished(&manager.sessions);
    assert!(manager.get("phone", &live.terminal_id).is_ok());
    assert!(manager.get("phone", &finished.terminal_id).is_err());
    manager.shutdown();
}
