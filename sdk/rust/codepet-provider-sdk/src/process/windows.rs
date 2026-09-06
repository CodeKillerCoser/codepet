use super::*;
pub(super) fn configure_std(command: &mut process_wrap::std::CommandWrap) {
    command.wrap(process_wrap::std::CreationFlags(
        windows::Win32::System::Threading::CREATE_NO_WINDOW,
    ));
    command.wrap(process_wrap::std::JobObject);
}
pub(super) fn configure_async(command: &mut process_wrap::tokio::CommandWrap) {
    command.wrap(process_wrap::tokio::CreationFlags(
        windows::Win32::System::Threading::CREATE_NO_WINDOW,
    ));
    command.wrap(process_wrap::tokio::JobObject);
}
pub(super) fn terminate(control: &ProcessControl) -> io::Result<()> {
    control.kill()
}
pub(super) fn interrupt(_control: &ProcessControl) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "console-free Windows process has no Unix interrupt signal",
    ))
}
// Keep a non-inheritable owner handle until process exit. Windows closes it even on a crash,
// killing the whole descendant tree, including descendants whose immediate parent has exited.
pub(super) fn prepare_owner() -> io::Result<()> {
    use std::{
        os::windows::io::{FromRawHandle, OwnedHandle},
        sync::OnceLock,
    };
    use windows::Win32::{
        Foundation::CloseHandle,
        System::{JobObjects::*, Threading::GetCurrentProcess},
    };
    static JOB: OnceLock<Result<OwnedHandle, String>> = OnceLock::new();
    JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).map_err(|e| e.to_string())?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
        .and_then(|_| AssignProcessToJobObject(job, GetCurrentProcess()));
        if let Err(e) = result {
            let _ = CloseHandle(job);
            return Err(e.to_string());
        }
        Ok(OwnedHandle::from_raw_handle(job.0))
    })
    .as_ref()
    .map(|_| ())
    .map_err(|e| io::Error::other(e.clone()))
}
