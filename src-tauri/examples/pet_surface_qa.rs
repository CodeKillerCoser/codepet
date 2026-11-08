//! Isolated native UI smoke test: production surface/platform code + sample data.
//! Does not start Provider Host or touch the user's settings, tasks or surface position.
//! Run after npm run build:webcontent:
//! TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo run --manifest-path src-tauri/Cargo.toml --example pet_surface_qa --features custom-protocol
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[cfg(target_os = "macos")]
#[path = "../src/platform/pet_menu.rs"]
pub mod qa_menu;
#[cfg(target_os = "macos")]
#[path = "../src/platform/pet_surface.rs"]
pub mod qa_platform;
#[cfg(target_os = "macos")]
#[path = "../src/pet/surface/mod.rs"]
pub mod qa_surface;
#[cfg(target_os = "macos")]
pub mod pet {
    pub use crate::qa_surface as surface;
}
#[cfg(target_os = "macos")]
pub mod platform {
    pub use crate::{qa_menu as pet_menu, qa_platform as pet_surface};
}
static QA_DATA: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
pub mod settings {
    pub fn load_app_settings() -> Result<(), String> {
        Ok(())
    }
}
pub mod paths {
    pub fn data(_: &()) -> std::path::PathBuf {
        crate::QA_DATA.get().unwrap().clone()
    }
}
pub mod app_log {
    pub fn error(scope: &str, message: &str) {
        eprintln!("{scope}: {message}");
    }
}

#[tauri::command]
fn get_app_settings() -> serde_json::Value {
    let mut value = serde_json::to_value(code_pet_lib::settings::AppSettings::default()).unwrap();
    value["pet"]["selectedPetId"] = "builtin:panda".into();
    value
}
#[tauri::command]
fn provider_connection_status() -> serde_json::Value {
    serde_json::json!([
        {"providerId":"codex","displayName":"Codex","icon":"https://avatars.githubusercontent.com/u/14957082?s=200&v=4","connectionStatus":"online"},
        {"providerId":"claude","displayName":"Claude Code","icon":"https://cdn.prod.website-files.com/6889473510b50328dbb70ae6/68c33859cc6cd903686c66a2_apple-touch-icon.png","connectionStatus":"online"}
    ])
}
#[tauri::command]
fn pet_gateway_request(mut request: serde_json::Value) -> serde_json::Value {
    static NOW: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let now = *NOW.get_or_init(|| chrono::Utc::now().timestamp_millis());
    request["response"] = serde_json::json!({"status":"ok","result":{"snapshot":{"sources":[],"tasks":[
        {"id":"layout","title":"调整顶部面板布局","summary":"已完成菜单栏入口与任务列表\n正在检查拖出悬浮与自动避让","status":"running","providerId":"codex","updatedAt":now},
        {"id":"review","title":"检查角色导入兼容性","summary":"角色导入已完成，等待你确认\n确认后继续检查动画状态","status":"waiting-input","providerId":"claude","updatedAt":now-60000},
        {"id":"done","title":"完成窗口尺寸回归检查","summary":"内容自适应和位置恢复检查通过\n小屏与大屏布局已完成检查","status":"completed","providerId":"codex","updatedAt":now-120000}
    ]}}});
    request
}
#[tauri::command]
fn qa_report(value: serde_json::Value) {
    eprintln!("DOM: {value}");
}
#[tauri::command]
fn open_main_window() {}

#[cfg(target_os = "macos")]
async fn native_smoke(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let w = app.get_webview_window("pet").ok_or("missing pet window")?;
    for (open, expected) in [(true, true), (false, false), (true, true)] {
        qa_surface::pet_surface_panel(app.clone(), open).await?;
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        if w.is_visible().map_err(|e| e.to_string())? != expected {
            return Err(format!("panel visible != {expected}"));
        }
    }
    qa_surface::pet_surface_mode(app.clone(), qa_surface::Mode::Floating).await?;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    if !w.is_visible().map_err(|e| e.to_string())? {
        return Err("floating window hidden".into());
    }
    qa_surface::pet_surface_mode(app.clone(), qa_surface::Mode::Docked).await?;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    if w.is_visible().map_err(|e| e.to_string())? {
        return Err("collapsed menu left a visible window".into());
    }
    qa_surface::pet_surface_panel(app.clone(), true).await?;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let state = qa_surface::pet_surface_state(app.clone()).await?;
    eprintln!(
        "Final native state: {state:?}; window size: {:?}",
        w.inner_size()
    );
    // Check actual AppKit window coordinates, not just WebView content dimensions.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            let env = qa_platform::read(&handle)?;
            let menu = qa_menu::anchor(&handle).ok_or("menu anchor unavailable")?;
            let display = qa_surface::geometry::display_for(menu.center(), &env.displays)
                .ok_or("menu display unavailable")?;
            let area = qa_surface::geometry::menu_area(display, menu);
            eprintln!(
                "Native placement: menu={menu:?}, work={:?}, actual={:?}, expectedTop={}",
                display.work, env.frame, area.y
            );
            if (env.frame.y - area.y).abs() > 1. || (env.frame.y - menu.bottom()).abs() > 1. {
                return Err("native panel must touch the bottom of its menu item".into());
            }
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())??;
    let _ = w.eval("window.__TAURI_INTERNALS__.invoke('qa_report',{value:{rect:document.querySelector('.pet-content')?.getBoundingClientRect().toJSON(),expanded:!!document.querySelector('.notch-details')}})");
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() {
    let data = tempfile::Builder::new()
        .prefix("codepet-native-surface-qa-")
        .tempdir()
        .unwrap();
    QA_DATA.set(data.path().to_owned()).unwrap();
    std::fs::create_dir_all(data.path().join("config")).unwrap();
    std::fs::write(
        data.path().join("config/pet-surface.json"),
        r#"{"mode":"docked","displayId":"","xRatio":0.5,"yRatio":0.5}"#,
    )
    .unwrap();
    eprintln!("Isolated QA data: {}", data.path().display());
    tauri::Builder::default()
        .plugin(tauri_nspanel::init())
        .manage(qa_surface::SurfaceState::default())
        .setup(|app| {
            code_pet_lib::platform::macos_window::configure_pet_overlay_window(app.handle())?;
            qa_surface::start(app.handle().clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let result = native_smoke(handle.clone()).await;
                eprintln!("Native smoke: {result:?}");
                if std::env::args().any(|arg| arg == "--smoke") {
                    handle.exit(if result.is_ok() { 0 } else { 1 });
                }
            });
            // Log changes without writing to the live application's logs.
            use tauri::Listener;
            app.listen_any("pet-surface-changed", |e| {
                eprintln!("surface: {}", e.payload())
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_settings,
            provider_connection_status,
            pet_gateway_request,
            open_main_window,
            qa_report,
            qa_surface::pet_surface_state,
            qa_surface::pet_surface_layout,
            qa_surface::pet_surface_mode,
            qa_surface::pet_surface_drag,
            qa_surface::pet_surface_pointer,
            qa_surface::pet_surface_panel,
            qa_surface::pet_menu_summary
        ])
        .run(tauri::generate_context!(
            "examples/pet-surface-qa/tauri.conf.json"
        ))
        .expect("native pet UI QA failed");
    drop(data);
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Native menu QA requires macOS; use the browser UI QA on this platform.");
}
