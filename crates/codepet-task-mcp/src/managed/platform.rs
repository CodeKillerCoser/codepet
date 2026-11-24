//! Native process/permission details stay behind the managed launcher interface.
use std::{fs::File, io, process::Command};

#[cfg(unix)]
pub(super) fn make_executable(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
}
#[cfg(not(unix))]
pub(super) fn make_executable(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn launch(mut command: Command) -> io::Result<i32> {
    use std::os::unix::process::CommandExt;
    Err(command.exec())
}
#[cfg(not(unix))]
pub(super) fn launch(mut command: Command) -> io::Result<i32> {
    Ok(command.status()?.code().unwrap_or(1))
}
