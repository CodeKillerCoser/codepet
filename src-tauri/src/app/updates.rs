pub use crate::platform::app_updates::AppUpdateMetadata;
use crate::platform::app_updates::{self, PendingUpdate};
use std::sync::Mutex;
use tauri::{AppHandle, State};

#[derive(Default)]
pub struct PendingAppUpdate(Mutex<Option<PendingUpdate>>);

#[tauri::command]
pub async fn check_app_update(
    app: AppHandle,
    pending_update: State<'_, PendingAppUpdate>,
) -> Result<Option<AppUpdateMetadata>, String> {
    // A failed check must not leave an older installer available to a later action.
    *pending_update
        .0
        .lock()
        .map_err(|_| "pending update state is poisoned")? = None;
    let update = app_updates::check(&app).await?;
    let metadata = update.as_ref().map(PendingUpdate::metadata);
    *pending_update
        .0
        .lock()
        .map_err(|_| "pending update state is poisoned")? = update;
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
        .map_err(|_| "pending update state is poisoned")?
        .clone()
        .ok_or("there is no pending update to install")?;
    // Keep the pending update on failure so the same dialog can retry.
    app_updates::install(&app, &update).await?;
    *pending_update
        .0
        .lock()
        .map_err(|_| "pending update state is poisoned")? = None;
    Ok(())
}
