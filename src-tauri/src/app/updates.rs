use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct PendingAppUpdate(Mutex<Option<Update>>);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateMetadata {
    pub version: String,
    pub current_version: String,
}

#[tauri::command]
pub async fn check_app_update(
    app: AppHandle,
    pending_update: State<'_, PendingAppUpdate>,
) -> Result<Option<AppUpdateMetadata>, String> {
    let update = build_updater(&app)?.check().await.map_err(|error| error.to_string())?;
    let metadata = update.as_ref().map(|update| AppUpdateMetadata {
        version: update.version.to_string(),
        current_version: update.current_version.to_string(),
    });

    *pending_update
        .0
        .lock()
        .map_err(|_| "pending update state is poisoned".to_string())? = update;

    Ok(metadata)
}

#[tauri::command]
pub async fn install_app_update(
    app: AppHandle,
    pending_update: State<'_, PendingAppUpdate>,
) -> Result<(), String> {
    let update = pending_update
        .0
        .lock()
        .map_err(|_| "pending update state is poisoned".to_string())?
        .take()
        .ok_or_else(|| "there is no pending update to install".to_string())?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    app.restart()
}

fn build_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app.updater_builder();
    if let Some(target) = custom_update_target() {
        builder = builder.target(target);
    }
    builder.build().map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn custom_update_target() -> Option<&'static str> {
    Some("macos-universal")
}

#[cfg(not(target_os = "macos"))]
fn custom_update_target() -> Option<&'static str> {
    None
}
