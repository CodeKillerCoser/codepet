//! Desktop shutdown is selected by executable provenance and ancestry, never by name alone.
use std::collections::HashSet;
use std::time::{Duration, Instant};
#[cfg(not(windows))]
use sysinfo::Pid;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessIdentity {
    pid: u32,
    parent: Option<u32>,
    exe: String,
    started: u64,
    user: Option<String>,
}

fn normalized(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn desktop_root(path: &str) -> bool {
    let path = normalized(path);
    // The Store Codex application currently ships its Electron main as ChatGPT.exe.
    let parts: Vec<_> = path.split('/').collect();
    let store = parts.len() >= 4
        && parts[parts.len() - 4..].windows(4).any(|p| {
            p[0] == "windowsapps"
                && p[1].starts_with("openai.codex_")
                && p[2] == "app"
                && matches!(p[3], "codex.exe" | "chatgpt.exe")
        });
    (store && matches!(parts.last(), Some(&"codex.exe") | Some(&"chatgpt.exe")))
        || path.ends_with("/codex.app/contents/macos/codex")
}

fn codepet_process(path: &str) -> bool {
    let path = normalized(path);
    let name = path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".exe");
    matches!(name, "code-pet" | "codepet" | "hanging-metal")
        || name.starts_with("codepet-provider-")
}

fn descendants(processes: &[ProcessIdentity], roots: &HashSet<u32>) -> HashSet<u32> {
    let mut result = roots.clone();
    loop {
        let size = result.len();
        for p in processes {
            if p.parent.is_some_and(|parent| result.contains(&parent)) {
                result.insert(p.pid);
            }
        }
        if result.len() == size {
            return result;
        }
    }
}

fn plan(processes: &[ProcessIdentity], own_pid: u32) -> Result<Vec<ProcessIdentity>, String> {
    if !processes.iter().any(|p| p.pid == own_pid) {
        return Err("Cannot identify the Codepet provider process".into());
    }
    let user = processes
        .iter()
        .find(|p| p.pid == own_pid)
        .and_then(|p| p.user.as_ref())
        .ok_or("Cannot identify the Codepet process user")?;
    let mut protected_roots: HashSet<_> = processes
        .iter()
        .filter(|p| codepet_process(&p.exe))
        .map(|p| p.pid)
        .collect();
    protected_roots.insert(own_pid);
    let mut protected = descendants(processes, &protected_roots);
    // Do not kill the host, shells or desktop which launched this provider either.
    let mut ancestor = Some(own_pid);
    let mut seen = HashSet::new();
    while let Some(pid) = ancestor {
        if !seen.insert(pid) {
            break;
        }
        protected.insert(pid);
        ancestor = processes
            .iter()
            .find(|p| p.pid == pid)
            .and_then(|p| p.parent);
    }
    let roots: HashSet<_> = processes
        .iter()
        .filter(|p| p.user.as_ref() == Some(user) && desktop_root(&p.exe))
        .map(|p| p.pid)
        .collect();
    if roots.is_empty() {
        return Err("No supported Codex desktop process could be identified".into());
    }
    if roots.iter().any(|pid| protected.contains(pid)) {
        return Err(
            "Codex desktop is an ancestor of Codepet; refusing to terminate its host".into(),
        );
    }
    let targets = descendants(processes, &roots);
    let mut result: Vec<_> = processes
        .iter()
        .filter(|p| targets.contains(&p.pid) && !protected.contains(&p.pid))
        .cloned()
        .collect();
    if result
        .iter()
        .any(|p| p.exe.is_empty() || p.started == 0 || p.user.as_ref() != Some(user))
    {
        return Err("Cannot verify every desktop descendant's executable and creation time".into());
    }
    // Close main first to prevent it from respawning the app-server.
    result.sort_by_key(|p| (!roots.contains(&p.pid), p.pid));
    Ok(result)
}

fn snapshot() -> (System, Vec<ProcessIdentity>) {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    let rows = system
        .processes()
        .values()
        .map(|p| ProcessIdentity {
            pid: p.pid().as_u32(),
            parent: p.parent().map(|p| p.as_u32()),
            exe: p
                .exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            started: p.start_time(),
            user: p.user_id().map(|user| format!("{user:?}")),
        })
        .collect();
    (system, rows)
}

/// Revalidate the entire plan before any side effect. Tests inject both IO operations.
fn execute(
    planned: &[ProcessIdentity],
    fresh: &[ProcessIdentity],
    own_pid: u32,
    mut terminate: impl FnMut(&ProcessIdentity) -> Result<(), String>,
) -> Result<(), String> {
    let allowed = plan(fresh, own_pid)?;
    for target in planned {
        if let Some(current) = fresh.iter().find(|p| p.pid == target.pid) {
            if current != target || !allowed.contains(current) {
                return Err(
                    "Desktop process identity or protection changed; retry takeover".into(),
                );
            }
        }
    }
    if allowed.iter().any(|p| !planned.contains(p)) {
        return Err("Desktop process tree changed; retry takeover".into());
    }
    for target in planned {
        if fresh.contains(target) {
            terminate(target)?;
        }
    }
    Ok(())
}

pub(crate) fn close_desktop(
    mut check_inactive: impl FnMut() -> Result<(), codepet_provider_sdk::ProtocolError>,
) -> Result<(), codepet_provider_sdk::ProtocolError> {
    use super::provider::protocol_error;
    let failure = |message| protocol_error("force_takeover_failed", message, true);
    if !cfg!(any(windows, target_os = "macos")) {
        return Err(failure(
            "Desktop takeover is not supported on this platform".into(),
        ));
    }
    let (_, rows) = snapshot();
    let targets = plan(&rows, std::process::id()).map_err(failure)?;
    check_inactive()?;
    let (system, fresh) = snapshot();
    execute(&targets, &fresh, std::process::id(), |target| {
        terminate(&system, target)
    })
    .map_err(failure)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let (_, remaining) = snapshot();
        if !remaining.iter().any(|p| {
            (desktop_root(&p.exe) && p.user == targets[0].user)
                || targets
                    .iter()
                    .any(|t| t.pid == p.pid && t.started == p.started)
        }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(failure(
                "Codex desktop did not exit, or restarted; retry takeover".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn terminate(system: &System, target: &ProcessIdentity) -> Result<(), String> {
    // Refresh identity immediately before signalling on Unix.
    let (_, current) = snapshot();
    let Some(current) = current.iter().find(|p| p.pid == target.pid) else {
        return Ok(());
    };
    // Unix reparents surviving children when their desktop root exits.
    if current.exe != target.exe || current.started != target.started || current.user != target.user {
        return Err("Desktop process changed before termination".into());
    }
    if system
        .process(Pid::from_u32(target.pid))
        .is_some_and(|p| p.kill())
    {
        Ok(())
    } else {
        Err(format!(
            "Failed to terminate Codex desktop process {}",
            target.pid
        ))
    }
}

#[cfg(windows)]
fn terminate(_: &System, target: &ProcessIdentity) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FILETIME},
        System::Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
            TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        },
    };
    // Query and terminate the same kernel object, avoiding taskkill PID reuse and /T.
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            0,
            target.pid,
        );
        if handle.is_null() {
            if GetLastError() == ERROR_INVALID_PARAMETER {
                return Ok(());
            }
            return Err(format!("Cannot open desktop process {}", target.pid));
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let mut path = vec![0u16; 32768];
        let mut len = path.len() as u32;
        let valid = GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) != 0
            && QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) != 0;
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        let started = ticks.saturating_sub(116444736000000000) / 10000000;
        let matches = valid
            && started == target.started
            && normalized(&String::from_utf16_lossy(&path[..len as usize]))
                == normalized(&target.exe);
        let mut exit_code = 259;
        let exited = GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code != 259;
        let result = exited || (matches && TerminateProcess(handle, 1) != 0);
        CloseHandle(handle);
        if result {
            Ok(())
        } else {
            Err(format!(
                "Cannot safely terminate desktop process {}",
                target.pid
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(pid: u32, parent: u32, exe: &str) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            parent: Some(parent),
            exe: exe.into(),
            started: 123,
            user: Some("user".into()),
        }
    }
    fn fixture() -> Vec<ProcessIdentity> {
        vec![
            p(1, 0, "C:/Windows/explorer.exe"),
            p(
                10,
                1,
                "C:/Program Files/WindowsApps/OpenAI.Codex_1/app/ChatGPT.exe",
            ),
            p(
                11,
                10,
                "C:/Users/me/AppData/Local/OpenAI/Codex/bin/version/codex.exe",
            ),
            p(20, 1, "C:/apps/code-pet.exe"),
            p(21, 20, "C:/apps/codepet-provider-codex.exe"),
            p(
                22,
                21,
                "C:/Users/me/AppData/Local/OpenAI/Codex/bin/version/codex.exe",
            ),
            p(30, 1, "C:/tools/codex.exe"),
            p(40, 1, "C:/ChatGPT.app/Contents/MacOS/ChatGPT"),
        ]
    }
    #[test]
    fn selects_only_desktop_tree_preserving_shared_binary_and_standalone_cli() {
        let rows = fixture();
        let targets = plan(&rows, 21).unwrap();
        let mut killed = Vec::new();
        execute(&targets, &rows, 21, |p| {
            killed.push(p.pid);
            Ok(())
        })
        .unwrap();
        assert_eq!(killed, vec![10, 11]);
    }
    #[test]
    fn preserves_codepet_subtrees_even_beneath_desktop() {
        let mut rows = fixture();
        rows.push(p(50, 10, "C:/apps/codepet-provider-other.exe"));
        rows.push(p(51, 50, "C:/tools/codex.exe"));
        assert_eq!(
            plan(&rows, 21)
                .unwrap()
                .iter()
                .map(|p| p.pid)
                .collect::<Vec<_>>(),
            vec![10, 11]
        );
    }
    #[test]
    fn refuses_desktop_ancestor_and_unknown_roots() {
        let mut rows = fixture();
        rows[3].parent = Some(10);
        assert!(plan(&rows, 21).is_err());
        assert!(plan(&[p(21, 0, "C:/tools/codex.exe")], 21).is_err());
        assert!(desktop_root("/Applications/Codex.app/Contents/MacOS/Codex"));
        assert!(!desktop_root(
            "/Applications/Codex.app/Contents/Resources/codex"
        ));
        assert!(!desktop_root(
            "C:/Program Files/WindowsApps/OpenAI.ChatGPT_1/app/ChatGPT.exe"
        ));
    }
    #[test]
    fn changed_pid_or_new_children_abort_before_any_kill() {
        let rows = fixture();
        let targets = plan(&rows, 21).unwrap();
        for mutation in 0..3 {
            let mut fresh = rows.clone();
            match mutation {
                0 => fresh[2].started += 1,
                1 => fresh[2].parent = Some(21),
                _ => fresh.push(p(12, 10, "C:/child.exe")),
            }
            assert!(execute(&targets, &fresh, 21, |_| panic!("must not terminate")).is_err());
        }
    }
    #[test]
    fn termination_failure_stops_remaining_actions() {
        let rows = fixture();
        let targets = plan(&rows, 21).unwrap();
        let mut calls = 0;
        assert!(execute(&targets, &rows, 21, |_| {
            calls += 1;
            Err("permission denied".into())
        })
        .is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn excludes_other_users_and_refuses_unverifiable_descendants() {
        let mut rows = fixture();
        let mut other = p(60, 1, "/Applications/Codex.app/Contents/MacOS/Codex");
        other.user = Some("someone-else".into());
        rows.push(other);
        assert_eq!(plan(&rows, 21).unwrap().len(), 2);
        rows.push(p(12, 10, ""));
        assert!(plan(&rows, 21).is_err());
    }
}
