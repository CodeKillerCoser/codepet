use tauri::AppHandle;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use tauri_plugin_autostart::ManagerExt;

pub fn launch_at_login_enabled(app: &AppHandle) -> Result<bool, String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        return app.autolaunch().is_enabled().map_err(|error| error.to_string());
    }

    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = app;
        Ok(false)
    }
}

pub fn set_launch_at_login_enabled(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        let manager = app.autolaunch();
        if enabled {
            manager.enable().map_err(|error| error.to_string())?;
        } else {
            manager.disable().map_err(|error| error.to_string())?;
        }
        return manager.is_enabled().map_err(|error| error.to_string());
    }

    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = app;
        let _ = enabled;
        Err("当前平台不支持开机启动".to_string())
    }
}
