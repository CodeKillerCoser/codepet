//! OS permission entry points. Opening settings never implies that access was granted.

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPermissionSettings {
    full_disk_access_supported: bool,
    app_path: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
fn app_bundle(executable: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_owned())
}

#[cfg(target_os = "macos")]
mod implementation {
    pub fn app_path() -> Option<std::path::PathBuf> {
        super::app_bundle(&std::env::current_exe().ok()?)
    }

    pub fn open_settings() -> Result<(), String> {
        open::that("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .map_err(|error| format!("无法打开完全磁盘访问权限设置：{error}"))
    }
}

#[cfg(not(target_os = "macos"))]
mod implementation {
    pub fn app_path() -> Option<std::path::PathBuf> { None }
    pub fn open_settings() -> Result<(), String> {
        Err("当前系统不支持 macOS 完全磁盘访问权限设置".into())
    }
}

#[tauri::command]
pub fn get_system_permission_settings() -> SystemPermissionSettings {
    SystemPermissionSettings {
        full_disk_access_supported: cfg!(target_os = "macos"),
        app_path: implementation::app_path().map(|path| path.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
pub async fn open_full_disk_access_settings(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" { return Err("权限设置仅可从主窗口打开".into()); }
    tauri::async_runtime::spawn_blocking(implementation::open_settings)
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn reveal_permission_app(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if window.label() != "main" { return Err("权限设置仅可从主窗口打开".into()); }
    let path = implementation::app_path().ok_or("请先安装并打开 Code Pet.app，再添加完全磁盘访问权限")?;
    app.opener().reveal_item_in_dir(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn permission_target_is_the_running_bundle_not_a_hardcoded_installation() {
        assert_eq!(app_bundle(Path::new("/Volumes/Preview/Code Pet.app/Contents/MacOS/code-pet")),
            Some(PathBuf::from("/Volumes/Preview/Code Pet.app")));
        assert_eq!(app_bundle(Path::new("/project/target/debug/code-pet")), None);
        assert_eq!(app_bundle(Path::new("/project/example.app/code-pet")), None);
    }

    #[test]
    fn capability_does_not_claim_authorization_status() {
        let value = serde_json::to_value(get_system_permission_settings()).unwrap();
        assert_eq!(value["fullDiskAccessSupported"], cfg!(target_os = "macos"));
        assert!(value.get("granted").is_none());
    }
}
