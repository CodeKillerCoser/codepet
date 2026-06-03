pub mod activity_actions;
pub mod agent_control;
pub mod agents;
pub mod app_log;
pub mod autostart;
pub mod cli;
pub mod claude_transcript;
pub mod codex_app_server;
pub mod codex_audit;
pub mod collector;
pub mod events;
pub mod hooks;
#[cfg(target_os = "macos")]
pub mod macos_window;
pub mod pets;
pub mod settings;
pub mod state;
pub mod subject_cutout;
pub mod theme_defaults;
pub mod title_resolver;
pub mod token_usage;

use agents::{AgentId, AgentView};
use base64::Engine;
use events::PetEvent;
use pets::PetLibraryView;
use settings::{load_app_settings, save_app_settings, AppSettings};
use state::{ApprovalBehavior, ApprovalDecision, SharedState, COLLECTOR_PORT};
use subject_cutout::SubjectCutoutResult;
use token_usage::TokenUsageSummary;
use std::str::FromStr;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const TRAY_MENU_OPEN: &str = "open-main";
const TRAY_MENU_QUIT: &str = "quit";

#[tauri::command]
fn list_agents(state: tauri::State<'_, SharedState>) -> Result<Vec<AgentView>, String> {
    let views = agent_control::list_agent_views().map_err(|error| error.to_string())?;
    state.set_agents(views.clone());
    Ok(views)
}

#[tauri::command]
fn set_agent_enabled(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
    agent_id: String,
    enabled: bool,
) -> Result<Vec<AgentView>, String> {
    let id = AgentId::from_str(&agent_id)?;
    let views = agent_control::set_agent_enabled(id, enabled).map_err(|error| error.to_string())?;
    state.set_agents(views.clone());
    if !enabled {
        state.remove_events_for_agent(id);
        let _ = app.emit("agent-disabled", id.as_str());
    }
    Ok(views)
}

#[tauri::command]
fn get_app_settings() -> Result<AppSettings, String> {
    load_app_settings().map_err(|error| error.to_string())
}

#[tauri::command]
fn update_app_settings(app: AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    let _ = app.emit("settings-updated", settings.clone());
    Ok(settings)
}

#[tauri::command]
fn get_launch_at_login_enabled(app: AppHandle) -> Result<bool, String> {
    autostart::launch_at_login_enabled(&app)
}

#[tauri::command]
fn set_launch_at_login_enabled(app: AppHandle, enabled: bool) -> Result<bool, String> {
    autostart::set_launch_at_login_enabled(&app, enabled)
}

#[tauri::command]
fn list_pets() -> Result<PetLibraryView, String> {
    pets::list_pet_library()
}

#[tauri::command]
fn select_pet(app: AppHandle, pet_id: String) -> Result<PetLibraryView, String> {
    let view = pets::switch_pet(pet_id)?;
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    Ok(view)
}

#[tauri::command]
fn delete_pet(app: AppHandle, pet_id: String) -> Result<PetLibraryView, String> {
    let view = pets::remove_pet_from_library(pet_id)?;
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    Ok(view)
}

#[tauri::command]
fn set_pet_data_directory(app: AppHandle, path: String) -> Result<PetLibraryView, String> {
    let view = pets::update_pet_data_directory(path)?;
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    Ok(view)
}

#[tauri::command]
fn import_pet_image(app: AppHandle, source_path: String, name: Option<String>, pixel_size: Option<u32>) -> Result<PetLibraryView, String> {
    let view = pets::import_pet_image(source_path, name, pixel_size)?;
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    Ok(view)
}

#[tauri::command]
fn update_pet_image_pixel_size(app: AppHandle, pixel_size: u32) -> Result<PetLibraryView, String> {
    let view = pets::update_active_image_pet_pixel_size(pixel_size)?;
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    Ok(view)
}

#[tauri::command]
fn cut_out_image_subject(source_path: String, output_path: Option<String>) -> Result<SubjectCutoutResult, String> {
    subject_cutout::cut_out_subject(source_path, output_path)
}

#[tauri::command]
fn recent_events(state: tauri::State<'_, SharedState>) -> Vec<PetEvent> {
    state.recent_events()
}

#[tauri::command]
fn token_usage_summary() -> Result<TokenUsageSummary, String> {
    token_usage::load_default_usage_summary().map_err(|error| error.to_string())
}

#[tauri::command]
fn record_perf_event(event: app_log::PerfEvent) {
    app_log::record_perf_event(event);
}

#[tauri::command]
fn activate_activity(state: tauri::State<'_, SharedState>, event_id: String) -> Result<(), String> {
    let event = state
        .event_by_id(&event_id)
        .ok_or_else(|| format!("activity not found: {event_id}"))?;
    activity_actions::activate_event(&event)
}

#[tauri::command]
fn send_activity_reply(
    state: tauri::State<'_, SharedState>,
    event_id: String,
    message: String,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("reply message is empty".to_string());
    }
    let event = state
        .event_by_id(&event_id)
        .ok_or_else(|| format!("activity not found: {event_id}"))?;
    activity_actions::send_reply_to_event(&event, message.trim())
}

#[tauri::command]
fn resolve_activity_approval(
    state: tauri::State<'_, SharedState>,
    event_id: String,
    behavior: ApprovalBehavior,
    message: Option<String>,
) -> Result<(), String> {
    let event = state
        .event_by_id(&event_id)
        .or_else(|| state.approval_event_by_id(&event_id))
        .ok_or_else(|| format!("approval not found: {event_id}"))?;
    activity_actions::resolve_approval_for_event(&state, &event, ApprovalDecision { behavior, message })
}

#[tauri::command]
fn collector_endpoint() -> String {
    format!("http://127.0.0.1:{COLLECTOR_PORT}/hook")
}

#[tauri::command]
fn open_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::App("index.html".into()))
        .title("Code Pet")
        .inner_size(980.0, 700.0)
        .min_inner_size(820.0, 600.0)
        .resizable(true)
        .build()
        .map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn pet_asset_data_url(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let mime = match std::path::Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("webp") => "image/webp",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => return Err(format!("unsupported pet asset type: {path}")),
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

pub fn run() {
    if let Err(error) = app_log::init_app_logging() {
        eprintln!("failed to initialize Code Pet file logging: {error}");
    }
    app_log::log_app_start_banner();
    app_log::info("app", "tauri builder initializing");

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            crate::app_log::info("app", &format!("single instance requested args={} cwd={}", args.len(), cwd));
            raise_existing_windows(app);
            let _ = app.emit("single-instance", serde_json::json!({ "args": args, "cwd": cwd }));
        }));

    install_platform_plugins(builder)
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(SharedState::default())
        .setup(|app| {
            let setup_span = app_log::PerfSpan::start("startup.total");
            app_log::info("startup", "setup started");
            let handle = app.handle().clone();
            let state = app.state::<SharedState>().inner().clone();
            if let Err(error) = install_tray_icon(&handle) {
                app_log::error("startup", &format!("failed to create tray icon error={error}"));
                let _ = handle.emit("collector-error", error);
            }
            let overlay_span = app_log::PerfSpan::start("startup.configure_pet_overlay_window");
            configure_pet_overlay_window(&handle);
            overlay_span.finish_ok(&[]);
            app_log::info("startup", "pet overlay window configured");
            let agents_span = app_log::PerfSpan::start("startup.list_agent_views");
            match agent_control::list_agent_views() {
                Ok(views) => {
                    agents_span.finish_ok(&[("agents", views.len().to_string())]);
                    app_log::info("startup", &format!("agent views loaded count={}", views.len()));
                    state.set_agents(views);
                }
                Err(error) => {
                    agents_span.finish_error(&error.to_string(), &[]);
                    app_log::error("startup", &format!("failed to list agent views error={error}"));
                    let _ = handle.emit("collector-error", error.to_string());
                }
            }
            let spool_span = app_log::PerfSpan::start("startup.replay_spooled_events");
            match collector::replay_default_spooled_events(&state) {
                Ok(count) => {
                    spool_span.finish_ok(&[("events", count.to_string())]);
                    app_log::info("startup", &format!("spooled events replayed count={count}"));
                }
                Err(error) => {
                    spool_span.finish_error(&error.to_string(), &[]);
                    app_log::error("startup", &format!("failed to replay spooled events error={error}"));
                    let _ = handle.emit("collector-error", error.to_string());
                }
            }
            let usage_handle = handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let span = crate::app_log::PerfSpan::start("token_usage.refresh_default_usage_summary");
                match token_usage::refresh_default_usage_summary() {
                    Ok(summary) => {
                        span.finish_ok(&[("sessions", summary.sessions.len().to_string())]);
                        crate::app_log::info("token_usage", "default usage summary refreshed");
                        let _ = usage_handle.emit("token-usage-updated", summary);
                    }
                    Err(error) => {
                        span.finish_error(&error.to_string(), &[]);
                        crate::app_log::error("token_usage", &format!("failed to refresh default usage summary error={error}"));
                        let _ = usage_handle.emit("collector-error", error.to_string());
                    }
                }
            });
            let audit_handle = handle.clone();
            let audit_state = state.clone();
            tauri::async_runtime::spawn(async move {
                codex_audit::watch_default_codex_audit(audit_state, audit_handle).await;
            });
            let collector_handle = handle.clone();
            let collector_state = state.clone();
            tauri::async_runtime::spawn(async move {
                crate::app_log::info("collector", "collector starting");
                if let Err(error) = collector::run_collector(collector_state, collector_handle.clone()).await {
                    crate::app_log::error("collector", &format!("collector exited error={error}"));
                    let _ = collector_handle.emit("collector-error", error.to_string());
                    collector_handle.exit(1);
                }
            });
            app_log::info("startup", "setup finished");
            setup_span.finish_ok(&[]);
            let replay_handle = handle.clone();
            let replay_state = state.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let codex_span = crate::app_log::PerfSpan::start("startup.replay_codex_audit_events");
                match codex_audit::replay_default_codex_audit_events(&replay_state) {
                    Ok(count) => {
                        codex_span.finish_ok(&[("events", count.to_string())]);
                        crate::app_log::info("startup", &format!("codex audit events replayed count={count}"));
                    }
                    Err(error) => {
                        codex_span.finish_error(&error.to_string(), &[]);
                        crate::app_log::error("startup", &format!("failed to replay codex audit events error={error}"));
                        let _ = replay_handle.emit("collector-error", error.to_string());
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_agents,
            set_agent_enabled,
            get_app_settings,
            update_app_settings,
            get_launch_at_login_enabled,
            set_launch_at_login_enabled,
            list_pets,
            select_pet,
            delete_pet,
            set_pet_data_directory,
            import_pet_image,
            update_pet_image_pixel_size,
            cut_out_image_subject,
            recent_events,
            token_usage_summary,
            record_perf_event,
            activate_activity,
            send_activity_reply,
            resolve_activity_approval,
            collector_endpoint,
            open_main_window,
            pet_asset_data_url
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Code Pet")
        .run(handle_run_event);
}

fn install_tray_icon(app: &AppHandle) -> Result<(), String> {
    let open = MenuItem::with_id(app, TRAY_MENU_OPEN, "Open Code Pet", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "Quit Code Pet", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let menu = Menu::with_items(app, &[&open, &quit]).map_err(|error| error.to_string())?;
    let mut builder = TrayIconBuilder::with_id("code-pet")
        .tooltip("Code Pet")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id() == TRAY_MENU_OPEN {
                let _ = open_main_window(app.clone());
            } else if event.id() == TRAY_MENU_QUIT {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = open_main_window(tray.app_handle().clone());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app).map(|_| ()).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn install_platform_plugins<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.plugin(tauri_nspanel::init())
}

#[cfg(not(target_os = "macos"))]
fn install_platform_plugins<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
}

#[cfg(target_os = "macos")]
fn configure_pet_overlay_window(app: &AppHandle) {
    if let Err(error) = macos_window::configure_pet_overlay_window(app) {
        let _ = app.emit("collector-error", error);
    }
}

#[cfg(not(target_os = "macos"))]
fn configure_pet_overlay_window(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
fn handle_run_event(app: &AppHandle, event: tauri::RunEvent) {
    if let tauri::RunEvent::Reopen {
        has_visible_windows,
        ..
    } = event
    {
        if should_restore_main_on_reopen(has_visible_windows) {
            let _ = open_main_window(app.clone());
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn handle_run_event(_app: &AppHandle, _event: tauri::RunEvent) {}

#[cfg(any(target_os = "macos", test))]
fn should_restore_main_on_reopen(_has_visible_windows: bool) -> bool {
    true
}

fn raise_existing_windows(app: &AppHandle) {
    for label in ["main", "pet"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_restore_main_on_reopen;

    #[test]
    fn dock_reopen_restores_main_even_when_pet_window_is_visible() {
        assert!(should_restore_main_on_reopen(false));
        assert!(should_restore_main_on_reopen(true));
    }
}
