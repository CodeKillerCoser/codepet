use crate::settings::{
    load_app_settings, save_app_settings, AppSettings, PixelPetSprite,
};
use chrono::Utc;
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};
pub use crate::settings::{ConfiguredPet, PetKind};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetLibraryView {
    pub data_directory: String,
    pub selected_pet_id: String,
    pub pets: Vec<ConfiguredPet>,
}

pub fn pet_data_directory(settings: &AppSettings) -> PathBuf {
    settings
        .pet_library
        .data_directory
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data_dir().join("pets"))
}

pub fn pet_library_view(settings: &AppSettings) -> PetLibraryView {
    PetLibraryView {
        data_directory: pet_data_directory(settings).to_string_lossy().to_string(),
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
    if let Some(sprite) = pet.sprite.clone() {
        settings.pet.sprite = sprite;
    }
    Ok(())
}

pub fn pixelate_image(source: &Path, output: &Path, max_side: u32) -> io::Result<()> {
    let image = image::open(source).map_err(io::Error::other)?.to_rgba8();
    let image = cut_out_flat_background(image);
    let max_side = max_side.max(8);
    let pixelated = DynamicImage::ImageRgba8(image).resize(max_side, max_side, FilterType::Nearest);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    pixelated
        .save_with_format(output, image::ImageFormat::Png)
        .map_err(io::Error::other)
}

fn cut_out_flat_background(mut image: RgbaImage) -> RgbaImage {
    if let Some(background) = flat_corner_background(&image) {
        for pixel in image.pixels_mut() {
            if close_to_background(*pixel, background) {
                pixel[3] = 0;
            }
        }
    }

    let Some((min_x, min_y, max_x, max_y)) = opaque_bounds(&image) else {
        return image;
    };

    image::imageops::crop_imm(
        &image,
        min_x,
        min_y,
        max_x - min_x + 1,
        max_y - min_y + 1,
    )
    .to_image()
}

fn flat_corner_background(image: &RgbaImage) -> Option<Rgba<u8>> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }

    let corners = [
        *image.get_pixel(0, 0),
        *image.get_pixel(width - 1, 0),
        *image.get_pixel(0, height - 1),
        *image.get_pixel(width - 1, height - 1),
    ];
    let first = corners[0];
    if corners
        .iter()
        .any(|corner| color_distance_squared(*corner, first) > 34 * 34)
    {
        return None;
    }

    let mut channels = [0u16; 4];
    for corner in corners {
        channels[0] += corner[0] as u16;
        channels[1] += corner[1] as u16;
        channels[2] += corner[2] as u16;
        channels[3] += corner[3] as u16;
    }

    Some(Rgba([
        (channels[0] / 4) as u8,
        (channels[1] / 4) as u8,
        (channels[2] / 4) as u8,
        (channels[3] / 4) as u8,
    ]))
}

fn close_to_background(pixel: Rgba<u8>, background: Rgba<u8>) -> bool {
    pixel[3] > 0 && color_distance_squared(pixel, background) <= 42 * 42
}

fn color_distance_squared(first: Rgba<u8>, second: Rgba<u8>) -> i32 {
    let red = first[0] as i32 - second[0] as i32;
    let green = first[1] as i32 - second[1] as i32;
    let blue = first[2] as i32 - second[2] as i32;
    red * red + green * green + blue * blue
}

fn opaque_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = image.width();
    let mut min_y = image.height();
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    found.then_some((min_x, min_y, max_x, max_y))
}

pub fn list_pet_library() -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    normalize_pet_selection(&mut settings)?;
    save_app_settings(&settings).map_err(|error| error.to_string())?;
    Ok(pet_library_view(&settings))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexPetManifest {
    id: Option<String>,
    display_name: Option<String>,
    spritesheet_path: String,
}

pub fn discover_codex_pet_packages(root: &Path) -> Vec<ConfiguredPet> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| codex_pet_package_from_dir(&entry.path()))
        .collect()
}

fn codex_pet_package_from_dir(pet_dir: &Path) -> Option<ConfiguredPet> {
    if !pet_dir.is_dir() {
        return None;
    }

    let manifest_path = pet_dir.join("pet.json");
    let manifest_text = fs::read_to_string(&manifest_path).ok()?;
    let manifest: CodexPetManifest = serde_json::from_str(&manifest_text).ok()?;
    let spritesheet = pet_dir.join(&manifest.spritesheet_path);
    if !is_codex_spritesheet(&spritesheet) {
        return None;
    }

    let folder_id = pet_dir.file_name().and_then(|name| name.to_str()).unwrap_or("pet");
    let manifest_id = manifest
        .id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(folder_id);
    let display_name = manifest
        .display_name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| titleize_pet_id(manifest_id));
    let created_at = fs::metadata(&manifest_path)
        .and_then(|metadata| metadata.modified())
        .map(chrono::DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
        .to_rfc3339();

    Some(ConfiguredPet {
        id: format!("codex:{manifest_id}"),
        name: display_name,
        kind: PetKind::CodexAtlas,
        sprite: None,
        image_path: Some(spritesheet.to_string_lossy().to_string()),
        source_path: Some(manifest_path.to_string_lossy().to_string()),
        created_at,
    })
}

fn is_codex_spritesheet(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("webp" | "png")
    ) && image::image_dimensions(path)
        .map(|(width, height)| width == 1536 && height == 1872)
        .unwrap_or(false)
}

fn titleize_pet_id(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn update_pet_data_directory(path: String) -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    let data_dir = PathBuf::from(path);
    fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    settings.pet_library.data_directory = Some(data_dir.to_string_lossy().to_string());
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

pub fn import_pet_image(source_path: String, name: Option<String>) -> Result<PetLibraryView, String> {
    let mut settings = load_app_settings().map_err(|error| error.to_string())?;
    normalize_pet_selection(&mut settings)?;

    let source = PathBuf::from(&source_path);
    if !source.exists() {
        return Err(format!("image not found: {source_path}"));
    }

    let data_dir = pet_data_directory(&settings);
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

    let pixelated = pet_dir.join("pixel.png");
    pixelate_image(&copied_source, &pixelated, 48).map_err(|error| error.to_string())?;

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
        sprite: Some(settings.pet.sprite.clone()),
        image_path: Some(pixelated.to_string_lossy().to_string()),
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
    let mut roots = vec![pet_data_directory(settings)];
    if let Some(codex_root) = dirs::home_dir().map(|home| home.join(".codex").join("pets")) {
        if !roots.iter().any(|root| root == &codex_root) {
            roots.push(codex_root);
        }
    }

    for pet in roots.iter().flat_map(|root| discover_codex_pet_packages(root)) {
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

fn ensure_profiles(mut pets: Vec<ConfiguredPet>) -> Vec<ConfiguredPet> {
    if pets.iter().any(|pet| pet.id == "default") {
        return pets;
    }

    pets.insert(
        0,
        ConfiguredPet {
            id: "default".to_string(),
            name: "Classic Pixel".to_string(),
            kind: PetKind::Palette,
            sprite: Some(PixelPetSprite {
                body: "#f4c04e".to_string(),
                accent: "#1f2937".to_string(),
                eyes: "#2563eb".to_string(),
            }),
            image_path: None,
            source_path: None,
            created_at: "2026-05-26T00:00:00Z".to_string(),
        },
    );
    pets
}

fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("code-pet")
}
