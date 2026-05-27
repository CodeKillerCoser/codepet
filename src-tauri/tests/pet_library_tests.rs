use code_pet_lib::pets::{
    delete_pet, discover_codex_pet_packages, pet_data_directory, pixelate_image, select_pet, ConfiguredPet, PetKind,
};
use code_pet_lib::settings::{AppSettings, PixelPetSprite};
use image::RgbaImage;
use std::fs;
use tempfile::tempdir;

#[test]
fn default_pet_data_directory_is_under_app_data_not_workspace() {
    let settings = AppSettings::default();
    let data_dir = pet_data_directory(&settings);
    let workspace = std::env::current_dir().unwrap();

    assert!(data_dir.ends_with("code-pet/pets"));
    assert!(!data_dir.starts_with(workspace));
}

#[test]
fn selecting_a_pet_updates_the_active_pet_settings() {
    let mut settings = AppSettings::default();
    let sprite = PixelPetSprite {
        body: "#22c55e".to_string(),
        accent: "#ef4444".to_string(),
        eyes: "#f8fafc".to_string(),
    };
    settings.pet_library.pets.push(ConfiguredPet {
        id: "custom".to_string(),
        name: "Custom".to_string(),
        kind: PetKind::Palette,
        sprite: Some(sprite.clone()),
        image_path: None,
        source_path: None,
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });

    select_pet(&mut settings, "custom").unwrap();

    assert_eq!(settings.pet.selected_pet_id, "custom");
    assert_eq!(settings.pet.kind, PetKind::Palette);
    assert_eq!(settings.pet.sprite.body, sprite.body);
    assert_eq!(settings.pet.image_path, None);
}

#[test]
fn selecting_a_codex_atlas_pet_preserves_its_spritesheet_path() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "codex:deidara-clay-blast".to_string(),
        name: "Deidara Clay Blast".to_string(),
        kind: PetKind::CodexAtlas,
        sprite: None,
        image_path: Some("/tmp/codex-pet/spritesheet.webp".to_string()),
        source_path: Some("/tmp/codex-pet/pet.json".to_string()),
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });

    select_pet(&mut settings, "codex:deidara-clay-blast").unwrap();

    assert_eq!(settings.pet.selected_pet_id, "codex:deidara-clay-blast");
    assert_eq!(settings.pet.kind, PetKind::CodexAtlas);
    assert_eq!(settings.pet.image_path.as_deref(), Some("/tmp/codex-pet/spritesheet.webp"));
}

#[test]
fn deleting_a_custom_pet_removes_it_from_the_library() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "custom".to_string(),
        name: "Custom".to_string(),
        kind: PetKind::Palette,
        sprite: None,
        image_path: None,
        source_path: None,
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });

    delete_pet(&mut settings, "custom").unwrap();

    assert!(!settings.pet_library.pets.iter().any(|pet| pet.id == "custom"));
    assert!(settings.pet_library.deleted_pet_ids.contains(&"custom".to_string()));
}

#[test]
fn deleting_the_active_pet_falls_back_to_default() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "custom".to_string(),
        name: "Custom".to_string(),
        kind: PetKind::Palette,
        sprite: None,
        image_path: None,
        source_path: None,
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });
    select_pet(&mut settings, "custom").unwrap();

    delete_pet(&mut settings, "custom").unwrap();

    assert_eq!(settings.pet_library.selected_pet_id, "default");
    assert_eq!(settings.pet.selected_pet_id, "default");
}

#[test]
fn default_pet_cannot_be_deleted() {
    let mut settings = AppSettings::default();

    let error = delete_pet(&mut settings, "default").unwrap_err();

    assert!(error.contains("default pet cannot be deleted"));
}

#[test]
fn discovers_codex_pet_packages_from_pet_json_and_spritesheet() {
    let temp = tempdir().unwrap();
    let pet_dir = temp.path().join("deidara-clay-blast");
    fs::create_dir_all(&pet_dir).unwrap();
    fs::write(
        pet_dir.join("pet.json"),
        r#"{"id":"deidara-clay-blast","displayName":"Deidara Clay Blast","description":"A test pet.","spritesheetPath":"spritesheet.webp"}"#,
    )
    .unwrap();
    RgbaImage::from_pixel(1536, 1872, image::Rgba([0, 0, 0, 0]))
        .save(pet_dir.join("spritesheet.webp"))
        .unwrap();

    let pets = discover_codex_pet_packages(temp.path());

    assert_eq!(pets.len(), 1);
    assert_eq!(pets[0].id, "codex:deidara-clay-blast");
    assert_eq!(pets[0].name, "Deidara Clay Blast");
    assert_eq!(pets[0].kind, PetKind::CodexAtlas);
    assert!(pets[0].image_path.as_deref().unwrap().ends_with("spritesheet.webp"));
    assert!(pets[0].source_path.as_deref().unwrap().ends_with("pet.json"));
}

#[test]
fn pixelate_image_writes_a_small_png_for_pixel_rendering() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.png");
    let output = temp.path().join("pixel.png");
    let image = image::RgbaImage::from_fn(96, 64, |x, y| {
        image::Rgba([
            (x % 255) as u8,
            (y % 255) as u8,
            ((x + y) % 255) as u8,
            255,
        ])
    });
    image.save(&source).unwrap();

    pixelate_image(&source, &output, 48).unwrap();

    let pixelated = image::open(&output).unwrap();
    assert!(pixelated.width() <= 48);
    assert!(pixelated.height() <= 48);
}

#[test]
fn pixelate_image_cuts_out_a_flat_corner_background() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.png");
    let output = temp.path().join("pixel.png");
    let mut image = image::RgbaImage::from_pixel(80, 80, image::Rgba([255, 255, 255, 255]));
    for x in 28..52 {
        for y in 20..60 {
            image.put_pixel(x, y, image::Rgba([239, 68, 68, 255]));
        }
    }
    image.save(&source).unwrap();

    pixelate_image(&source, &output, 48).unwrap();

    let pixelated = image::open(&output).unwrap().to_rgba8();
    assert!(pixelated.width() < 48);
    assert!(pixelated.height() <= 48);
    assert_eq!(pixelated.get_pixel(0, 0)[3], 255);
}
