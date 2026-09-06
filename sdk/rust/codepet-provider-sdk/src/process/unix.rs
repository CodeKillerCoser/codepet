use super::*;
pub(super) fn prepare_owner() -> io::Result<()> {
    Ok(())
}
pub(super) fn configure_std(command: &mut process_wrap::std::CommandWrap) {
    command.wrap(process_wrap::std::ProcessGroup::leader());
}
pub(super) fn configure_async(command: &mut process_wrap::tokio::CommandWrap) {
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
}
pub(super) fn terminate(control: &ProcessControl) -> io::Result<()> {
    signal(control, libc::SIGTERM)
}
pub(super) fn interrupt(control: &ProcessControl) -> io::Result<()> {
    signal(control, libc::SIGINT)
}
fn signal(control: &ProcessControl, signal: libc::c_int) -> io::Result<()> {
    let mut tree = control.0.lock().unwrap();
    if tree.poll()?.is_some() {
        return Ok(());
    }
    let group = i32::try_from(tree.inner.id()).map_err(io::Error::other)?;
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}
