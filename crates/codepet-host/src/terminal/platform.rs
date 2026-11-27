//! Shell selection is a platform concern; terminal callers use one PTY API.
use std::path::PathBuf;

#[cfg(unix)]
pub(super) fn shell() -> PathBuf {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

#[cfg(windows)]
pub(super) fn shell() -> PathBuf {
    std::env::var_os("COMSPEC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"))
}

pub(super) fn home() -> Option<PathBuf> {
    #[cfg(unix)]
    let value = std::env::var_os("HOME");
    #[cfg(windows)]
    let value = std::env::var_os("USERPROFILE");
    value.map(PathBuf::from)
}

pub(super) fn command() -> portable_pty::CommandBuilder {
    let mut command = portable_pty::CommandBuilder::new(shell());
    #[cfg(unix)]
    command.arg("-l");
    command.env("TERM", "xterm-256color");
    command
}

#[cfg(unix)]
pub(super) fn is_eof_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::EIO)
}

#[cfg(windows)]
pub(super) fn is_eof_error(_: &std::io::Error) -> bool {
    false
}
