pub use super::images::prepare_pet_image;
pub use super::packages::discover_codex_pet_packages;
pub use super::packages::install_pet_package;
use crate::settings::{default_pet_profiles, load_app_settings, save_app_settings, AppSettings};
pub use crate::settings::{ConfiguredPet, PetKind};
use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetLibraryView {
    pub data_directory: String,
    pub selected_pet_id: String,
    pub pets: Vec<ConfiguredPet>,
}
pub fn pet_library_view(settings: &AppSettings) -> PetLibraryView {
    PetLibraryView {
        data_directory: crate::paths::pets(settings).to_string_lossy().to_string(),
        selected_pet_id: settings.pet_library.selected_pet_id.clone(),
        pets: ensure_profiles(settings.pet_library.pets.clone()),
    }
}

pub fn select_pet(settings: &mut AppSettings, pet_id: &str) -> Result<(), String> {
    let pets = ensure_profiles(settings.pet_library.pets.clone());
    let pet = pets
        .iter()
        .find(|candidate| candidate.id == pet_id)
        .cloned()
        .ok_or_else(|| format!("pet not found: {pet_id}"))?;

    settings.pet_library.pets = pets;
    settings.pet_library.selected_pet_id = pet.id.clone();
    settings.pet.selected_pet_id = pet.id.clone();
    settings.pet.kind = pet.kind.clone();
    settings.pet.image_path = pet.image_path.clone();
    Ok(())
}

pub fn delete_pet(settings: &mut AppSettings, pet_id: &str) -> Result<(), String> {
    if default_pet_profiles().iter().any(|pet| pet.id == pet_id) {
        return Err("built-in pet cannot be deleted".to_string());
    }

    settings.pet_library.pets = ensure_profiles(settings.pet_library.pets.clone());
    let original_len = settings.pet_library.pets.len();
    settings.pet_library.pets.retain(|pet| pet.id != pet_id);
    if settings.pet_library.pets.len() == original_len {
        return Err(format!("pet not found: {pet_id}"));
    }

    if !settings
        .pet_library
        .deleted_pet_ids
        .iter()
        .any(|id| id == pet_id)
    {
        settings
            .pet_library
            .deleted_pet_ids
            .push(pet_id.to_string());
    }

    if settings.pet_library.selected_pet_id == pet_id || settings.pet.selected_pet_id == pet_id {
        select_pet(settings, "default")?;
    }
    Ok(())
}

pub fn list_pet_library() -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    normalize_pet_selection(&mut settings)?;
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    Ok(pet_library_view(&settings))
}

pub fn switch_pet(pet_id: String) -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    select_pet(&mut settings, &pet_id)?;
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    Ok(pet_library_view(&settings))
}

pub fn remove_pet_from_library(pet_id: String) -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    normalize_pet_selection(&mut settings)?;
    let pet = settings
        .pet_library
        .pets
        .iter()
        .find(|candidate| candidate.id == pet_id)
        .cloned()
        .ok_or_else(|| format!("pet not found: {pet_id}"))?;
    let removable_dir = managed_pet_directory(&settings, &pet);

    delete_pet(&mut settings, &pet_id)?;
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    if let Some(path) = removable_dir {
        let _ = fs::remove_dir_all(path);
    }
    Ok(pet_library_view(&settings))
}

pub fn import_pet_image(
    source_path: String,
    name: Option<String>,
) -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    normalize_pet_selection(&mut settings)?;

    let source = PathBuf::from(&source_path);
    if !source.exists() {
        return Err(format!("image not found: {source_path}"));
    }

    let data_dir = crate::paths::pets(&settings);
    fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    let id = format!("image-{}", Uuid::new_v4().simple());
    let pet_dir = data_dir.join(&id);
    fs::create_dir_all(&pet_dir).map_err(|error| error.to_string())?;

    let source_extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .filter(|extension| matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp"))
        .unwrap_or_else(|| "png".to_string());
    let copied_source = pet_dir.join(format!("source.{source_extension}"));
    fs::copy(&source, &copied_source).map_err(|error| error.to_string())?;

    let prepared = pet_dir.join("portrait.png");
    if let Err(error) = prepare_pet_image(&copied_source, &prepared) {
        let _ = fs::remove_dir_all(&pet_dir);
        return Err(error.to_string());
    }

    let display_name = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            source
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Imported Pet")
                .to_string()
        });
    let pet = ConfiguredPet {
        id: id.clone(),
        name: display_name,
        kind: PetKind::Image,
        image_path: Some(prepared.to_string_lossy().to_string()),
        source_path: Some(copied_source.to_string_lossy().to_string()),
        created_at: Utc::now().to_rfc3339(),
    };

    settings.pet_library.pets.push(pet);
    select_pet(&mut settings, &id)?;
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    Ok(pet_library_view(&settings))
}

fn normalize_pet_selection(settings: &mut AppSettings) -> Result<(), String> {
    settings.pet_library.pets = ensure_profiles(settings.pet_library.pets.clone());
    add_discovered_codex_pets(settings);
    let selected = if settings
        .pet_library
        .pets
        .iter()
        .any(|pet| pet.id == settings.pet_library.selected_pet_id)
    {
        settings.pet_library.selected_pet_id.clone()
    } else {
        settings
            .pet_library
            .pets
            .first()
            .map(|pet| pet.id.clone())
            .unwrap_or_else(|| "default".to_string())
    };
    select_pet(settings, &selected)
}

fn add_discovered_codex_pets(settings: &mut AppSettings) {
    let mut roots = vec![crate::paths::pets(settings)];
    if let Some(codex_root) = crate::paths::home().map(|home| home.join(".codex").join("pets")) {
        if !roots.iter().any(|root| root == &codex_root) {
            roots.push(codex_root);
        }
    }

    for pet in roots
        .iter()
        .flat_map(|root| discover_codex_pet_packages(root))
        .filter(|pet| {
            !settings
                .pet_library
                .deleted_pet_ids
                .iter()
                .any(|id| id == &pet.id)
        })
    {
        if let Some(existing) = settings
            .pet_library
            .pets
            .iter_mut()
            .find(|candidate| candidate.id == pet.id)
        {
            *existing = pet;
        } else {
            settings.pet_library.pets.push(pet);
        }
    }
}

fn managed_pet_directory(settings: &AppSettings, pet: &ConfiguredPet) -> Option<PathBuf> {
    let data_dir = crate::paths::pets(settings).canonicalize().ok()?;
    let source = PathBuf::from(pet.source_path.as_ref()?);
    let parent = source.parent()?.canonicalize().ok()?;
    let folder = parent.file_name()?.to_str()?;
    (parent.parent() == Some(data_dir.as_path())
        && (folder.starts_with("image-") || folder.starts_with("package-")))
    .then_some(parent)
}

fn ensure_profiles(mut pets: Vec<ConfiguredPet>) -> Vec<ConfiguredPet> {
    let defaults = default_pet_profiles();
    pets.retain(|pet| !defaults.iter().any(|default| default.id == pet.id));
    defaults.into_iter().chain(pets).collect()
}

/// Restore built-in profiles and keep the active selection consistent with the library.
pub fn normalize_active_pet(settings: &mut AppSettings) {
    settings.pet_library.pets = ensure_profiles(std::mem::take(&mut settings.pet_library.pets));
    let selected = settings.pet_library.selected_pet_id.clone();
    if select_pet(settings, &selected).is_err() {
        let _ = select_pet(settings, "default");
    }
}

pub fn import_pet_package(source_path: String) -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|e| e.to_string())?;
    normalize_pet_selection(&mut settings)?;
    let pet = install_pet_package(&crate::paths::pets(&settings), Path::new(&source_path))?;
    let removable_dir = managed_pet_directory(&settings, &pet);
    let pet_id = pet.id.clone();
    settings.pet_library.pets.push(pet);
    select_pet(&mut settings, &pet_id)?;
    if let Err(error) = save_app_settings(&settings) {
        if let Some(directory) = removable_dir {
            let _ = fs::remove_dir_all(directory);
        }
        return Err(error.to_string());
    }
    Ok(pet_library_view(&settings))
}
