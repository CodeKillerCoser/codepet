pub mod activity;
pub mod agent;
pub mod app;
pub mod pet;
pub mod platform;
pub mod runtime_gateway;

pub use activity::collector;
pub use activity::events;
pub use activity::title_resolver;
pub use activity::token_usage;
pub use agent::actions as activity_actions;
pub use agent::claude_transcript;
pub use agent::codex_desktop_ipc;
pub use agent::control as agent_control;
pub use agent::hooks;
pub use agent::registry as agents;
pub use agent::runtime as agent_runtime;
pub use app::autostart;
pub use app::cli;
pub use app::log as app_log;
pub use app::notifications;
pub use app::settings;
pub use app::state;
pub use app::updates;
pub use pet::library as pets;
pub use pet::subject_cutout;
pub use pet::theme_defaults;
#[cfg(target_os = "macos")]
pub use platform::macos_window;

use agents::{AgentId, AgentView};
use agent_runtime::{
    AgentRuntime, AgentRuntimeCandidate, AgentRuntimeService, AgentRuntimeSource,
};
use base64::Engine;
use events::PetEvent;
use pets::PetLibraryView;
use settings::{
    app_data_directory_target_status as read_app_data_directory_target_status, configured_app_data_dir,
    load_app_settings, save_app_settings, update_app_data_directory, AppDataDirectoryTargetStatus,
    AppSettings,
};
use state::{ApprovalBehavior, ApprovalDecision, SharedState, COLLECTOR_PORT};
use subject_cutout::SubjectCutoutResult;
use token_usage::TokenUsageSummary;
use updates::PendingAppUpdate;
use runtime_gateway::tauri_bridge::{
    codex_desktop_companion_replay, codex_desktop_companion_request,
    codex_desktop_companion_snapshot,
    runtime_gateway_replay, runtime_gateway_request,
    start_codex_desktop_companion_event_bridge, start_runtime_gateway_event_bridge,
    CodexDesktopCompanionState, ProviderHostState, RuntimeGatewayState,
};
use runtime_gateway::remote_access::{
    cancel_remote_pairing, copy_remote_pairing_json, get_remote_pairing_status, list_remote_clients,
    list_remote_pairing_requests, remote_access_status, resolve_remote_pairing_request,
    retry_remote_access, revoke_remote_credential, start_remote_pairing, RemoteAccessRuntime,
};
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
fn set_agent_hook_events(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
    agent_id: String,
    hook_events: Vec<String>,
) -> Result<Vec<AgentView>, String> {
    let id = AgentId::from_str(&agent_id)?;
    let views = agent_control::set_agent_hook_events(id, hook_events)
        .map_err(|error| error.to_string())?;
    state.set_agents(views.clone());
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    Ok(views)
}

#[tauri::command]
async fn list_agent_runtimes(
    _service: tauri::State<'_, AgentRuntimeService>,
    provider_host: tauri::State<'_, ProviderHostState>,
) -> Result<Vec<AgentRuntime>, String> {
    Ok(provider_host.runtime_views().await)
}

#[tauri::command]
async fn detect_agent_runtime(
    service: tauri::State<'_, AgentRuntimeService>,
    provider_host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
) -> Result<AgentRuntime, String> {
    let _ = service;
    provider_host.runtime_view(&provider_id).await
}

#[tauri::command]
async fn refresh_agent_runtimes(
    app: AppHandle,
    service: tauri::State<'_, AgentRuntimeService>,
    provider_host: tauri::State<'_, ProviderHostState>,
) -> Result<Vec<AgentRuntime>, String> {
    let _ = service;
    let runtimes = provider_host.runtime_views().await;
    let _ = app.emit("agent-runtimes-updated", runtimes.clone());
    Ok(runtimes)
}

#[tauri::command]
async fn set_agent_runtime_executable(
    app: AppHandle,
    service: tauri::State<'_, AgentRuntimeService>,
    provider_host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
    executable: String,
) -> Result<AgentRuntime, String> {
    let selected = provider_host.select_runtime(&provider_id, AgentRuntimeCandidate {
        executable_path: executable,
        source: AgentRuntimeSource::Configured,
    }).await?;
    service.save_provider_selection(&provider_id, &selected.executable_path)
        .map_err(|error| error.to_string())?;
    let runtime = provider_host.runtime_view(&provider_id).await?;
    emit_runtime_settings(&app, &runtime);
    Ok(runtime)
}

#[tauri::command]
async fn clear_agent_runtime_executable(
    app: AppHandle,
    service: tauri::State<'_, AgentRuntimeService>,
    provider_host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
) -> Result<AgentRuntime, String> {
    let current = provider_host.runtime_view(&provider_id).await?;
    let automatic = current.installed.first().cloned()
        .ok_or_else(|| format!("{} Provider did not find a default runtime", current.display_name))?;
    provider_host.select_runtime(&provider_id, AgentRuntimeCandidate {
        executable_path: automatic.executable_path,
        source: automatic.source,
    }).await?;
    service.clear_provider_selection(&provider_id).map_err(|error| error.to_string())?;
    let runtime = provider_host.runtime_view(&provider_id).await?;
    emit_runtime_settings(&app, &runtime);
    Ok(runtime)
}


fn emit_runtime_settings(app: &AppHandle, runtime: &AgentRuntime) {
    if let Ok(settings) = load_app_settings() {
        let _ = app.emit("settings-updated", settings);
    }
    let _ = app.emit("agent-runtime-updated", runtime.clone());
}

#[tauri::command]
fn get_app_settings() -> Result<AppSettings, String> {
    load_app_settings().map_err(|error| error.to_string())
}

#[tauri::command]
fn update_app_settings(
    app: AppHandle,
    settings: serde_json::Value,
) -> Result<AppSettings, String> {
    let current = load_app_settings().map_err(|error| error.to_string())?;
    let settings = merge_app_settings_update(settings, current)?;
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    let _ = app.emit("settings-updated", settings.clone());
    Ok(settings)
}

fn merge_app_settings_update(
    value: serde_json::Value,
    current: AppSettings,
) -> Result<AppSettings, String> {
    let provider_plugins_present = value
        .as_object()
        .is_some_and(|settings| settings.contains_key("providerPlugins"));
    let mut settings: AppSettings =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    // Runtime paths are executable inputs and may only change through the validating runtime API.
    settings.agent_runtimes = current.agent_runtimes;
    if !provider_plugins_present {
        settings.provider_plugins = current.provider_plugins;
    }
    Ok(settings)
}

#[tauri::command]
async fn send_test_robot_notification(channel_id: Option<String>) -> Result<String, String> {
    notifications::send_test_notification(channel_id).await
}

#[tauri::command]
fn app_data_directory() -> Result<String, String> {
    let settings = load_app_settings().map_err(|error| error.to_string())?;
    Ok(configured_app_data_dir(&settings).to_string_lossy().to_string())
}

#[tauri::command]
fn app_data_directory_target_status(path: String) -> Result<AppDataDirectoryTargetStatus, String> {
    read_app_data_directory_target_status(path).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_app_data_directory(
    app: AppHandle,
    path: Option<String>,
    clear_target: bool,
) -> Result<AppSettings, String> {
    let settings = update_app_data_directory(path, clear_target).map_err(|error| error.to_string())?;
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
async fn send_activity_reply(
    state: tauri::State<'_, SharedState>,
    event_id: String,
    message: String,
) -> Result<(), String> {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("reply message is empty".to_string());
    }
    let event = state
        .event_by_id(&event_id)
        .ok_or_else(|| format!("activity not found: {event_id}"))?;
    tauri::async_runtime::spawn_blocking(move || activity_actions::send_reply_to_event(&event, &message))
        .await
        .map_err(|error| format!("reply task failed: {error}"))?
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

    let desktop_companion_state = CodexDesktopCompanionState::default();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            crate::app_log::info("app", &format!("single instance requested args={} cwd={}", args.len(), cwd));
            raise_existing_windows(app);
            let _ = app.emit("single-instance", serde_json::json!({ "args": args, "cwd": cwd }));
        }))
        .plugin(tauri_plugin_updater::Builder::new().build());

    install_platform_plugins(builder)
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(SharedState::default())
        .manage(PendingAppUpdate::default())
        .manage(AgentRuntimeService::default())
        .manage(desktop_companion_state)
        .setup(|app| {
            let setup_span = app_log::PerfSpan::start("startup.total");
            app_log::info("startup", "setup started");
            let handle = app.handle().clone();
            let state = app.state::<SharedState>().inner().clone();
            let (provider_host_state, remote_access_runtime) =
                match ProviderHostState::from_app(&handle) {
                    Ok((state, remote_access)) => {
                        let runtime = match state.gateway() {
                            Some(gateway) => RemoteAccessRuntime::new(remote_access, gateway),
                            None => RemoteAccessRuntime::unavailable(codepet_host::HostError::new(
                                "remote_access_core_unavailable",
                                "Remote access cannot run because the shared Provider Gateway is unavailable",
                            )),
                        };
                        (state, runtime)
                    }
                    Err(error) => {
                        app_log::error(
                            "provider_host",
                            &format!(
                                "failed to initialize bundled Provider Host boundary error={error:?}"
                            ),
                        );
                        (
                            ProviderHostState::unavailable(),
                            RemoteAccessRuntime::unavailable(error),
                        )
                    }
                };
            let runtime_gateway_state = RuntimeGatewayState::new(provider_host_state.gateway());
            if !app.manage(runtime_gateway_state.clone()) {
                return Err("Runtime Gateway state was already managed".into());
            }
            if !app.manage(provider_host_state.clone()) {
                return Err("Provider Host state was already managed".into());
            }
            if !app.manage(remote_access_runtime.clone()) {
                return Err("Remote Access runtime was already managed".into());
            }
            if let Err(error) = start_runtime_gateway_event_bridge(handle.clone(), &runtime_gateway_state) {
                app_log::error(
                    "runtime_gateway",
                    &format!("failed to start remote gateway local event bridge error={error:?}"),
                );
            }
            provider_host_state.start_in_background();
            remote_access_runtime.start_in_background();
            let desktop_companion_state = app.state::<CodexDesktopCompanionState>().inner().clone();
            if let Err(error) = start_codex_desktop_companion_event_bridge(handle.clone(), &desktop_companion_state) {
                app_log::error(
                    "codex_desktop_companion",
                    &format!("failed to start Desktop companion event bridge error={error:?}"),
                );
            }
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
            let collector_handle = handle.clone();
            let collector_state = state.clone();
            tauri::async_runtime::spawn(async move {
                crate::app_log::info("collector", "collector starting");
                if let Err(error) = collector::run_collector(collector_state, collector_handle.clone()).await {
                    crate::app_log::error("collector", &format!("collector exited error={error}"));
                    let _ = collector_handle.emit("collector-error", error.to_string());
                    request_app_exit(&collector_handle, 1);
                }
            });
            app_log::info("startup", "setup finished");
            setup_span.finish_ok(&[]);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_agents,
            set_agent_enabled,
            set_agent_hook_events,
            list_agent_runtimes,
            detect_agent_runtime,
            refresh_agent_runtimes,
            set_agent_runtime_executable,
            clear_agent_runtime_executable,
            get_app_settings,
            update_app_settings,
            send_test_robot_notification,
            app_data_directory,
            app_data_directory_target_status,
            set_app_data_directory,
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
            pet_asset_data_url,
            runtime_gateway_request,
            runtime_gateway_replay,
            codex_desktop_companion_request,
            codex_desktop_companion_replay,
            codex_desktop_companion_snapshot,
            remote_access_status,
            retry_remote_access,
            list_remote_clients,
            start_remote_pairing,
            get_remote_pairing_status,
            copy_remote_pairing_json,
            cancel_remote_pairing,
            list_remote_pairing_requests,
            resolve_remote_pairing_request,
            revoke_remote_credential,
            updates::check_app_update,
            updates::install_app_update
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
                request_app_exit(app, 0);
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

fn handle_run_event(app: &AppHandle, event: tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    if let tauri::RunEvent::Reopen {
        has_visible_windows,
        ..
    } = &event
    {
        if should_restore_main_on_reopen(*has_visible_windows) {
            let _ = open_main_window(app.clone());
        }
    }

    match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            let provider_host = app.state::<ProviderHostState>().inner().clone();
            let remote_access = app.state::<RemoteAccessRuntime>().inner().clone();
            if provider_host.shutdown_completed() && remote_access.shutdown_completed() {
                return;
            }
            api.prevent_exit();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                remote_access.shutdown_once().await;
                if provider_host.shutdown_once().await {
                    request_app_exit(&app, code.unwrap_or(0));
                }
            });
        }
        tauri::RunEvent::Exit => {
            let provider_host = app.state::<ProviderHostState>().inner().clone();
            let remote_access = app.state::<RemoteAccessRuntime>().inner().clone();
            if !provider_host.shutdown_completed() || !remote_access.shutdown_completed() {
                tauri::async_runtime::block_on(async move {
                    remote_access.shutdown_once().await;
                    provider_host.shutdown_once().await;
                });
            }
        }
        _ => {}
    }
}

fn request_app_exit(app: &AppHandle, code: i32) {
    app.exit(code);
}

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
    use super::{merge_app_settings_update, should_restore_main_on_reopen};
    use crate::settings::AppSettings;

    #[test]
    fn dock_reopen_restores_main_even_when_pet_window_is_visible() {
        assert!(should_restore_main_on_reopen(false));
        assert!(should_restore_main_on_reopen(true));
    }

    #[test]
    fn settings_update_preserves_missing_provider_plugins_and_clears_explicit_empty() {
        let mut current = AppSettings::default();
        current.provider_plugins.directories = vec!["/configured/providers".to_string()];
        let missing = merge_app_settings_update(serde_json::json!({}), current.clone()).unwrap();
        assert_eq!(
            missing.provider_plugins.directories,
            vec!["/configured/providers"]
        );

        let cleared = merge_app_settings_update(
            serde_json::json!({ "providerPlugins": { "directories": [] } }),
            current,
        )
        .unwrap();
        assert!(cleared.provider_plugins.directories.is_empty());
    }
}
