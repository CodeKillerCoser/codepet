use code_pet_lib::pets::{
    delete_pet, discover_codex_pet_packages, install_pet_package, normalize_active_pet,
    prepare_pet_image, select_pet, ConfiguredPet, PetKind,
};
use code_pet_lib::settings::AppSettings;
use image::RgbaImage;
use std::fs;
use tempfile::tempdir;

#[test]
fn default_pet_data_directory_is_under_app_data_not_workspace() {
    let settings = AppSettings::default();
    let data_dir = code_pet_lib::paths::pets(&settings);
    let workspace = std::env::current_dir().unwrap();

    assert!(data_dir.ends_with("code-pet/pets"));
    assert!(!data_dir.starts_with(workspace));
}

#[test]
fn default_pet_data_directory_follows_custom_app_data_directory() {
    let temp = tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.data.data_directory = Some(
        temp.path()
            .join("code-pet-data")
            .to_string_lossy()
            .to_string(),
    );

    assert_eq!(
        code_pet_lib::paths::pets(&settings),
        temp.path().join("code-pet-data").join("pets")
    );
}

#[test]
fn pet_library_data_directory_overrides_app_data_directory() {
    let temp = tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.data.data_directory = Some(
        temp.path()
            .join("code-pet-data")
            .to_string_lossy()
            .to_string(),
    );
    settings.pet_library.data_directory =
        Some(temp.path().join("pets-only").to_string_lossy().to_string());

    assert_eq!(
        code_pet_lib::paths::pets(&settings),
        temp.path().join("pets-only")
    );
}

#[test]
fn selecting_a_pet_updates_the_active_pet_settings() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "custom".to_string(),
        name: "Custom".to_string(),
        kind: PetKind::Image,
        image_path: Some("/tmp/custom.png".into()),
        source_path: None,
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });

    select_pet(&mut settings, "custom").unwrap();

    assert_eq!(settings.pet.selected_pet_id, "custom");
    assert_eq!(settings.pet.kind, PetKind::Image);
    assert_eq!(settings.pet.image_path.as_deref(), Some("/tmp/custom.png"));
}

#[test]
fn selecting_a_codex_atlas_pet_preserves_its_spritesheet_path() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "codex:deidara-clay-blast".to_string(),
        name: "Deidara Clay Blast".to_string(),
        kind: PetKind::CodexAtlas,
        image_path: Some("/tmp/codex-pet/spritesheet.webp".to_string()),
        source_path: Some("/tmp/codex-pet/pet.json".to_string()),
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });

    select_pet(&mut settings, "codex:deidara-clay-blast").unwrap();

    assert_eq!(settings.pet.selected_pet_id, "codex:deidara-clay-blast");
    assert_eq!(settings.pet.kind, PetKind::CodexAtlas);
    assert_eq!(
        settings.pet.image_path.as_deref(),
        Some("/tmp/codex-pet/spritesheet.webp")
    );
}

#[test]
fn deleting_a_custom_pet_removes_it_from_the_library() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "custom".to_string(),
        name: "Custom".to_string(),
        kind: PetKind::Image,
        image_path: None,
        source_path: None,
        created_at: "2026-05-26T00:00:00Z".to_string(),
    });

    delete_pet(&mut settings, "custom").unwrap();

    assert!(!settings
        .pet_library
        .pets
        .iter()
        .any(|pet| pet.id == "custom"));
    assert!(settings
        .pet_library
        .deleted_pet_ids
        .contains(&"custom".to_string()));
}

#[test]
fn deleting_the_active_pet_falls_back_to_default() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(ConfiguredPet {
        id: "custom".to_string(),
        name: "Custom".to_string(),
        kind: PetKind::Image,
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

    assert!(error.contains("built-in pet cannot be deleted"));
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
    assert!(pets[0]
        .image_path
        .as_deref()
        .unwrap()
        .ends_with("spritesheet.webp"));
    assert!(pets[0]
        .source_path
        .as_deref()
        .unwrap()
        .ends_with("pet.json"));
}

#[test]
fn image_import_preserves_resolution_and_alpha() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.png");
    let output = temp.path().join("portrait.png");
    let image = image::RgbaImage::from_fn(256, 192, |x, y| {
        image::Rgba([x as u8, y as u8, 120, if x < 32 { 0 } else { 255 }])
    });
    image.save(&source).unwrap();
    prepare_pet_image(&source, &output).unwrap();
    assert_eq!(image::open(output).unwrap().to_rgba8(), image);
}

#[test]
fn image_import_bounds_large_art_without_changing_aspect_ratio() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.png");
    let output = temp.path().join("portrait.png");
    RgbaImage::from_pixel(2048, 1024, image::Rgba([240, 120, 50, 255]))
        .save(&source)
        .unwrap();
    prepare_pet_image(&source, &output).unwrap();
    assert_eq!(image::image_dimensions(output).unwrap(), (1024, 512));
}

#[test]
fn builtin_profiles_are_restored_and_choices_cannot_be_deleted() {
    let mut settings = AppSettings::default();
    settings.pet_library.pets.clear();
    normalize_active_pet(&mut settings);
    assert_eq!(settings.pet.kind, PetKind::Builtin);
    assert_eq!(settings.pet_library.pets[0].name, "橘猫");
    select_pet(&mut settings, "builtin:panda").unwrap();
    assert!(delete_pet(&mut settings, "builtin:panda").is_err());
    normalize_active_pet(&mut settings);
    assert_eq!(settings.pet.selected_pet_id, "builtin:panda");
    assert_eq!(settings.pet_library.pets.len(), 2);
}

#[test]
fn package_discovery_rejects_traversal_wrong_geometry_and_corrupt_manifests() {
    let temp = tempdir().unwrap();
    let package = temp.path().join("escape");
    fs::create_dir(&package).unwrap();
    RgbaImage::new(1536, 1872)
        .save(temp.path().join("sheet.png"))
        .unwrap();
    fs::write(
        package.join("pet.json"),
        r#"{"spritesheetPath":"../sheet.png"}"#,
    )
    .unwrap();
    assert!(discover_codex_pet_packages(temp.path()).is_empty());
    RgbaImage::new(200, 200)
        .save(package.join("sheet.png"))
        .unwrap();
    fs::write(
        package.join("pet.json"),
        r#"{"spritesheetPath":"sheet.png"}"#,
    )
    .unwrap();
    assert!(discover_codex_pet_packages(temp.path()).is_empty());
    fs::write(package.join("pet.json"), "broken").unwrap();
    assert!(discover_codex_pet_packages(temp.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn package_discovery_rejects_symlink_escape() {
    let temp = tempdir().unwrap();
    let package = temp.path().join("escape");
    fs::create_dir(&package).unwrap();
    RgbaImage::new(1536, 1872)
        .save(temp.path().join("sheet.png"))
        .unwrap();
    std::os::unix::fs::symlink(temp.path().join("sheet.png"), package.join("sheet.png")).unwrap();
    fs::write(
        package.join("pet.json"),
        r#"{"spritesheetPath":"sheet.png"}"#,
    )
    .unwrap();
    assert!(discover_codex_pet_packages(temp.path()).is_empty());
}

#[test]
fn bundled_art_matches_animation_contract() {
    for name in ["orange-cat", "panda"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../frontend/assets/pets/{name}.webp"));
        assert_eq!(image::image_dimensions(&path).unwrap(), (1536, 1872));
        let image = image::open(path).unwrap().to_rgba8();
        assert!(image.pixels().any(|pixel| pixel[3] == 0));
        assert!(image.pixels().any(|pixel| pixel[3] > 12));
    }
}

#[test]
fn imported_packages_survive_source_removal_and_rediscovery() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("original");
    let library = temp.path().join("library");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("pet.json"),
        r#"{"id":"cat","displayName":"My Cat","spritesheetPath":"sheet.png"}"#,
    )
    .unwrap();
    RgbaImage::new(1536, 1872)
        .save(source.join("sheet.png"))
        .unwrap();
    let first = install_pet_package(&library, &source.join("pet.json")).unwrap();
    let second = install_pet_package(&library, &source).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.name, "My Cat");
    let mut settings = AppSettings::default();
    settings.pet_library.pets.push(first.clone());
    select_pet(&mut settings, &first.id).unwrap();
    normalize_active_pet(&mut settings);
    assert_eq!(settings.pet.kind, PetKind::CodexAtlas);
    assert_eq!(settings.pet.image_path, first.image_path);
    fs::remove_dir_all(&source).unwrap();
    let found = discover_codex_pet_packages(&library);
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|pet| pet.id == first.id));
    assert!(std::path::Path::new(first.image_path.as_ref().unwrap()).is_file());
}

#[test]
fn invalid_package_import_leaves_no_partial_library_entry() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("invalid");
    let library = temp.path().join("library");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("pet.json"),
        r#"{"spritesheetPath":"missing.webp"}"#,
    )
    .unwrap();
    assert!(install_pet_package(&library, &source).is_err());
    assert!(!library.exists());
}
