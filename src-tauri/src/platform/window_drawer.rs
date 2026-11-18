//! Window identities and original frames stay in the native process.
use serde::Deserialize;
use serde_json::Value;
use tauri::WebviewWindow;

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    max_width: f64,
    max_height: f64,
}

#[cfg(target_os = "macos")]
fn connect(window: &WebviewWindow) -> Result<(), String> {
    use std::sync::OnceLock;
    use tauri::{AppHandle, Emitter, Manager};
    static APP: OnceLock<AppHandle> = OnceLock::new();
    extern "C" fn changed(cover: bool, dx: f64, dy: f64) {
        if let Some(app) = APP.get() {
            let _ = app.emit_to("pet", "pet-drawer-changed", serde_json::json!({ "cover": cover, "dx": dx, "dy": dy }));
        }
    }
    unsafe extern "C" { fn cp_drawer_connect(host: *mut std::ffi::c_void, notify: extern "C" fn(bool, f64, f64)); }
    let panel = window.clone();
    APP.get_or_init(|| window.app_handle().clone());
    window.run_on_main_thread(move || {
        if let Ok(host) = panel.ns_window() { unsafe { cp_drawer_connect(host, changed) }; }
    })
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn call(operation: &str, id: &str, region: Region) -> Result<Value, String> {
    use std::ffi::{c_char, CStr, CString};
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    extern "C" {
        fn cp_drawer_call(
            op: *const c_char,
            id: *const c_char,
            x: f64,
            y: f64,
            w: f64,
            h: f64,
            max_w: f64,
            max_h: f64,
        ) -> *mut c_char;
    }
    let op = CString::new(operation).map_err(|e| e.to_string())?;
    let key = CString::new(id).map_err(|e| e.to_string())?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let raw = unsafe {
        cp_drawer_call(
            op.as_ptr(),
            key.as_ptr(),
            region.x,
            region.y,
            region.width,
            region.height,
            region.max_width.floor(),
            region.max_height.floor(),
        )
    };
    if raw.is_null() {
        return Err("窗口服务不可用".into());
    }
    let reply = unsafe { serde_json::from_slice::<Value>(CStr::from_ptr(raw).to_bytes()) };
    unsafe { libc::free(raw.cast()) };
    let reply = reply.map_err(|e| e.to_string())?;
    if let Some(diagnostic) = reply.get("diagnostic").and_then(Value::as_str) {
        crate::app_log::info("pet.window_drawer", diagnostic);
    }
    if operation == "drop" {
        // Stage only. Do not persist window titles or application content.
        if let Some(reason) = reply.get("ignored").and_then(Value::as_str) {
            crate::app_log::info("pet.window_drawer", &format!("drop skipped: {reason}"));
            return Ok(Value::Null);
        }
        if let Some(error) = reply.get("error").and_then(Value::as_str) {
            crate::app_log::warn("pet.window_drawer", error);
        } else if reply.get("id").is_some() {
            crate::app_log::info("pet.window_drawer", "drop matched window");
        }
    }
    match reply.get("error").and_then(Value::as_str) {
        Some(error) => Err(error.into()),
        None => Ok(reply),
    }
}

#[cfg(target_os = "macos")]
pub fn restore_all() {
    if let Err(error) = call("reset", "", Region::default()) {
        crate::app_log::error("pet.window_drawer", &error);
    }
}

#[tauri::command]
pub async fn pet_window_drawer(
    window: WebviewWindow,
    operation: String,
    id: Option<String>,
    region: Option<Region>,
) -> Result<Value, String> {
    if window.label() != "pet" {
        return Err("仅桌宠窗口可以收纳应用".into());
    }
    if operation == "supported" {
        #[cfg(target_os = "macos")]
        connect(&window)?;
        return Ok(Value::Bool(cfg!(target_os = "macos")));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, region);
        Err("当前平台不支持窗口收纳".into())
    }
    #[cfg(target_os = "macos")]
    {
        if operation == "settings" {
            open::that(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            )
            .map_err(|e| e.to_string())?;
            return Ok(Value::Null);
        }
        let mut target = region.unwrap_or_default();
        if operation == "show" || operation == "activate" {
            use tauri::Manager;
            let app = window.app_handle().clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            window
                .run_on_main_thread(move || {
                    let _ = tx.send(super::pet_surface::drawer_frame(&app));
                })
                .map_err(|e| e.to_string())?;
            let frame = rx.await.map_err(|e| e.to_string())??;
            if !(target.x >= 0.
                && target.y >= 0.
                && target.width >= 30.
                && target.height >= 30.
                && target.x + target.width <= frame.width + 1.
                && target.y + target.height <= frame.height + 1.)
            {
                // DOM geometry can reach the next layout before AppKit commits its frame.
                return Ok(serde_json::json!({ "pending": true }));
            }
            target.x += frame.x;
            target.y += frame.y;
            target.x = target.x.round();
            target.y = target.y.round();
            target.width = target.width.round();
            target.height = target.height.round();
        }
        tauri::async_runtime::spawn_blocking(move || {
            call(&operation, id.as_deref().unwrap_or(""), target)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}
