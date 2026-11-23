//! System actions stay behind a platform-independent business interface.
use std::path::Path;
pub fn open_terminal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return status(
            std::process::Command::new("/usr/bin/open")
                .args(["-b", "com.apple.Terminal"])
                .arg(path),
        );
    }
    #[cfg(target_os = "windows")]
    {
        return status(
            std::process::Command::new("cmd")
                .arg("/C")
                .arg("start")
                .arg("")
                .arg("cmd")
                .current_dir(path),
        );
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err("当前平台暂不支持从工作台打开终端".into())
    }
}
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return status(
            std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(path),
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        open::that(path.parent().ok_or("文件目录不存在")?).map_err(|e| e.to_string())
    }
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn status(command: &mut std::process::Command) -> Result<(), String> {
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("系统操作失败：{status}"))
    }
}
