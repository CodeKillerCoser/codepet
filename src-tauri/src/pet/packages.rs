use crate::settings::{ConfiguredPet, PetKind};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetManifest {
    pub id: Option<String>,
    pub display_name: Option<String>,
    pub spritesheet_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub fn read_package(directory: &Path) -> Result<(PetManifest, PathBuf), String> {
    let root = directory.canonicalize().map_err(|e| e.to_string())?;
    let manifest_path = root.join("pet.json");
    if fs::metadata(&manifest_path)
        .map_err(|e| e.to_string())?
        .len()
        > 65536
    {
        return Err("pet.json is too large".into());
    }
    let manifest: PetManifest =
        serde_json::from_str(&fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let relative = Path::new(&manifest.spritesheet_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("spritesheetPath must be a relative path inside the pet package".into());
    }
    let sheet = root
        .join(relative)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !sheet.starts_with(&root) {
        return Err("spritesheet must stay inside the pet package".into());
    }
    if fs::metadata(&sheet).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("spritesheet exceeds 16 MiB".into());
    }
    let extension = sheet
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "webp" | "png")
        || image::image_dimensions(&sheet).map_err(|e| e.to_string())? != (1536, 1872)
    {
        return Err("pet package requires a 1536×1872 PNG/WebP atlas (8 columns × 9 rows)".into());
    }
    Ok((manifest, sheet))
}

pub fn package_from_dir(directory: &Path) -> Option<ConfiguredPet> {
    let (manifest, sheet) = read_package(directory).ok()?;
    let folder = directory.file_name()?.to_str()?;
    let id = manifest
        .id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(folder);
    let name = manifest
        .display_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(id);
    let manifest_path = directory.join("pet.json").canonicalize().ok()?;
    let created_at = fs::metadata(&manifest_path)
        .and_then(|m| m.modified())
        .map(chrono::DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
        .to_rfc3339();
    Some(ConfiguredPet {
        id: format!("codex:{id}"),
        name: name.into(),
        kind: PetKind::CodexAtlas,
        image_path: Some(sheet.to_string_lossy().into()),
        source_path: Some(manifest_path.to_string_lossy().into()),
        created_at,
    })
}

pub fn discover_codex_pet_packages(root: &Path) -> Vec<ConfiguredPet> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut pets: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| package_from_dir(&entry.path()))
        .collect();
    pets.sort_by(|a, b| a.id.cmp(&b.id));
    pets
}

/// Copies a validated package into a unique managed directory. The original is never edited.
pub fn install_pet_package(library: &Path, source: &Path) -> Result<ConfiguredPet, String> {
    let directory = if source.is_file() {
        if source.file_name().and_then(|name| name.to_str()) != Some("pet.json") {
            return Err("请选择动画包中的 pet.json".into());
        }
        source.parent().ok_or("invalid package path")?
    } else {
        source
    };
    let (mut manifest, sheet) = read_package(directory)?;
    let id = format!("package-{}", uuid::Uuid::new_v4().simple());
    let destination = library.join(&id);
    fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    let result = (|| -> Result<_, String> {
        let filename = format!(
            "spritesheet.{}",
            sheet
                .extension()
                .and_then(|s| s.to_str())
                .ok_or("missing extension")?
        );
        fs::copy(&sheet, destination.join(&filename)).map_err(|e| e.to_string())?;
        manifest.display_name = Some(
            manifest
                .display_name
                .filter(|name| !name.trim().is_empty())
                .or(manifest.id.clone())
                .unwrap_or_else(|| "导入宠物".into()),
        );
        manifest.id = Some(id);
        manifest.spritesheet_path = filename;
        fs::write(
            destination.join("pet.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        package_from_dir(&destination).ok_or_else(|| "invalid copied pet package".into())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result
}
