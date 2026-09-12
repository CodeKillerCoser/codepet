//! Shared path policy; this module is the desktop integration boundary.
pub use codepet_paths::{PathManager, DataSettings, default_data_directory, data_directory, settings_file, workspace_directory};

pub fn current() -> std::io::Result<PathManager> { PathManager::discover() }

// Keep Tauri's resource resolution for AppImage/Linux and native bundles here.
pub fn for_package(package: &tauri::PackageInfo) -> std::io::Result<PathManager> {
    let mut paths = current()?;
    paths.resources = tauri::utils::platform::resource_dir(package, &tauri::Env::default())
        .map_err(std::io::Error::other)?;
    Ok(paths)
}

use crate::settings::{AppSettings, load_app_settings, save_app_settings};
use std::{io, path::{Path, PathBuf}};

pub fn data(settings: &AppSettings) -> PathBuf {
    data_directory(&default_data_directory(), &settings.data)
}
pub fn pets(settings: &AppSettings) -> PathBuf {
    settings.pet_library.data_directory.as_deref().map(str::trim).filter(|p| !p.is_empty())
        .map(PathBuf::from).unwrap_or_else(|| data(settings).join("pets"))
}
pub fn spool(settings: &AppSettings) -> PathBuf { data(settings).join("spool/events.jsonl") }
pub fn log_file(data: &Path) -> PathBuf { data.join("logs/code-pet.log") }
pub fn home() -> Option<PathBuf> { codepet_paths::home_directory() }

fn validate_directory(value: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "directory must be absolute")); }
    if path.exists() && !path.is_dir() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "path must be a directory")); }
    Ok(path)
}

// New configuration only: never copy, delete or migrate existing user data.
pub fn set_data_directory(path: Option<String>) -> io::Result<AppSettings> {
    let mut settings = load_app_settings()?;
    settings.data.data_directory = path.filter(|p| !p.trim().is_empty()).map(|p| validate_directory(&p).map(|p| p.to_string_lossy().into_owned())).transpose()?;
    std::fs::create_dir_all(data(&settings))?;
    save_app_settings(&settings)?;
    Ok(settings)
}
pub fn set_pets_directory(path: String) -> io::Result<AppSettings> {
    let path = validate_directory(&path)?;
    let mut settings = load_app_settings()?;
    std::fs::create_dir_all(&path)?;
    settings.pet_library.data_directory = Some(path.to_string_lossy().into_owned());
    save_app_settings(&settings)?;
    Ok(settings)
}
