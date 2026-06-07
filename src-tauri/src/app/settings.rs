use crate::agents::AgentId;
use crate::theme_defaults;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub data: DataSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub pet: PetSettings,
    #[serde(default)]
    pub pet_library: PetLibrarySettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub activity_filters: ActivityFilterSettings,
    #[serde(default)]
    pub agents: AgentSettings,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSettings {
    #[serde(default)]
    pub data_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: ThemeChoice,
    #[serde(default)]
    pub running_bubble: RunningBubbleSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningBubbleSettings {
    #[serde(default = "default_true")]
    pub background_breathing: bool,
    #[serde(default)]
    pub border_marquee: bool,
    #[serde(default = "default_running_bubble_background")]
    pub background_color: String,
    #[serde(default = "default_running_bubble_border")]
    pub border_color: String,
    #[serde(default = "default_running_bubble_border_width")]
    pub border_width: u8,
    #[serde(default = "default_running_bubble_animation_ms")]
    pub animation_ms: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetSettings {
    #[serde(default = "default_pet_id")]
    pub selected_pet_id: String,
    #[serde(default)]
    pub kind: PetKind,
    #[serde(default = "default_sprite")]
    pub sprite: PixelPetSprite,
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default = "default_scale")]
    pub scale: u8,
    #[serde(default = "default_image_pixel_size")]
    pub image_pixel_size: u32,
    #[serde(default = "default_pet_opacity")]
    pub opacity: f32,
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,
    #[serde(default)]
    pub whip_reaction_sound: WhipReactionSound,
    #[serde(default)]
    pub custom_whip_reaction_sound_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PixelPetSprite {
    pub body: String,
    pub accent: String,
    pub eyes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetLibrarySettings {
    #[serde(default)]
    pub data_directory: Option<String>,
    #[serde(default = "default_pet_id")]
    pub selected_pet_id: String,
    #[serde(default = "default_pet_profiles")]
    pub pets: Vec<ConfiguredPet>,
    #[serde(default)]
    pub deleted_pet_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredPet {
    pub id: String,
    pub name: String,
    pub kind: PetKind,
    #[serde(default)]
    pub sprite: Option<PixelPetSprite>,
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PetKind {
    Palette,
    Image,
    CodexAtlas,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhipReactionSound {
    None,
    Pa,
    Scream,
    Custom,
}

impl Default for PetKind {
    fn default() -> Self {
        Self::Palette
    }
}

impl Default for WhipReactionSound {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    #[serde(default)]
    pub sound: SoundChoice,
    #[serde(default)]
    pub custom_sound_path: Option<String>,
    #[serde(default = "default_true")]
    pub ring_on_permission: bool,
    #[serde(default = "default_true")]
    pub ring_on_failure: bool,
    #[serde(default = "default_true")]
    pub ring_on_done: bool,
    #[serde(default = "default_repeat_seconds")]
    pub repeat_seconds: u16,
    #[serde(default)]
    pub quiet_hours_enabled: bool,
    #[serde(default = "default_quiet_start")]
    pub quiet_hours_start: String,
    #[serde(default = "default_quiet_end")]
    pub quiet_hours_end: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityFilterSettings {
    #[serde(default)]
    pub title_keywords: Vec<String>,
    #[serde(default)]
    pub message_keywords: Vec<String>,
    #[serde(default)]
    pub by_agent: HashMap<AgentId, ActivityKeywordFilterSettings>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityKeywordFilterSettings {
    #[serde(default)]
    pub title_keywords: Vec<String>,
    #[serde(default)]
    pub message_keywords: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default)]
    pub by_agent: HashMap<AgentId, AgentPreferenceSettings>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPreferenceSettings {
    #[serde(default)]
    pub hook_events: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SoundChoice {
    Blip,
    Chime,
    Bell,
    Custom,
    Silent,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            running_bubble: RunningBubbleSettings::default(),
        }
    }
}

impl Default for ThemeChoice {
    fn default() -> Self {
        Self::System
    }
}

impl Default for RunningBubbleSettings {
    fn default() -> Self {
        Self {
            background_breathing: true,
            border_marquee: false,
            background_color: default_running_bubble_background(),
            border_color: default_running_bubble_border(),
            border_width: default_running_bubble_border_width(),
            animation_ms: default_running_bubble_animation_ms(),
        }
    }
}

impl Default for PetSettings {
    fn default() -> Self {
        Self {
            selected_pet_id: default_pet_id(),
            kind: PetKind::Palette,
            sprite: default_sprite(),
            image_path: None,
            scale: default_scale(),
            image_pixel_size: default_image_pixel_size(),
            opacity: default_pet_opacity(),
            always_on_top: default_always_on_top(),
            whip_reaction_sound: WhipReactionSound::default(),
            custom_whip_reaction_sound_path: None,
        }
    }
}

impl Default for PetLibrarySettings {
    fn default() -> Self {
        Self {
            data_directory: None,
            selected_pet_id: default_pet_id(),
            pets: default_pet_profiles(),
            deleted_pet_ids: Vec::new(),
        }
    }
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            sound: SoundChoice::default(),
            custom_sound_path: None,
            ring_on_permission: true,
            ring_on_failure: true,
            ring_on_done: true,
            repeat_seconds: default_repeat_seconds(),
            quiet_hours_enabled: false,
            quiet_hours_start: default_quiet_start(),
            quiet_hours_end: default_quiet_end(),
        }
    }
}

impl Default for SoundChoice {
    fn default() -> Self {
        Self::Blip
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            data: DataSettings::default(),
            appearance: AppearanceSettings::default(),
            pet: PetSettings::default(),
            pet_library: PetLibrarySettings::default(),
            notifications: NotificationSettings::default(),
            activity_filters: ActivityFilterSettings::default(),
            agents: AgentSettings::default(),
        }
    }
}

pub fn default_sprite() -> PixelPetSprite {
    PixelPetSprite {
        body: theme_defaults::DEFAULT_PET_BODY.to_string(),
        accent: theme_defaults::DEFAULT_PET_ACCENT.to_string(),
        eyes: theme_defaults::DEFAULT_PET_EYES.to_string(),
    }
}

fn default_pet_id() -> String {
    "default".to_string()
}

fn default_pet_profiles() -> Vec<ConfiguredPet> {
    vec![ConfiguredPet {
        id: default_pet_id(),
        name: "Classic Pixel".to_string(),
        kind: PetKind::Palette,
        sprite: Some(default_sprite()),
        image_path: None,
        source_path: None,
        created_at: "2026-05-26T00:00:00Z".to_string(),
    }]
}

fn default_scale() -> u8 {
    3
}

fn default_image_pixel_size() -> u32 {
    48
}

fn default_pet_opacity() -> f32 {
    1.0
}

fn default_always_on_top() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_running_bubble_background() -> String {
    theme_defaults::DEFAULT_RUNNING_BUBBLE_BACKGROUND.to_string()
}

fn default_running_bubble_border() -> String {
    theme_defaults::DEFAULT_RUNNING_BUBBLE_BORDER.to_string()
}

fn default_running_bubble_border_width() -> u8 {
    1
}

fn default_running_bubble_animation_ms() -> u16 {
    1800
}

fn default_repeat_seconds() -> u16 {
    30
}

fn default_quiet_start() -> String {
    "22:00".to_string()
}

fn default_quiet_end() -> String {
    "08:00".to_string()
}

pub fn load_app_settings() -> io::Result<AppSettings> {
    let path = settings_path();
    if !path.exists() {
        let settings = AppSettings::default();
        save_app_settings(&settings)?;
        return Ok(settings);
    }

    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save_app_settings(settings: &AppSettings) -> io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(settings)?)
}

pub fn default_app_data_dir() -> PathBuf {
    default_data_root().join("code-pet")
}

pub fn configured_app_data_dir(settings: &AppSettings) -> PathBuf {
    settings
        .data
        .data_directory
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_app_data_dir)
}

pub fn current_app_data_dir() -> PathBuf {
    load_app_settings()
        .map(|settings| configured_app_data_dir(&settings))
        .unwrap_or_else(|_| default_app_data_dir())
}

pub fn update_app_data_directory(path: Option<String>) -> io::Result<AppSettings> {
    let mut settings = load_app_settings()?;
    let old_data_dir = configured_app_data_dir(&settings);
    settings.data.data_directory = path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let new_data_dir = configured_app_data_dir(&settings);
    if !same_directory(&old_data_dir, &new_data_dir) {
        if destination_inside_source(&old_data_dir, &new_data_dir) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "new data directory cannot be inside the current data directory",
            ));
        }
        copy_app_data_directory_contents(&old_data_dir, &new_data_dir)?;
        rewrite_default_pet_library_paths(&mut settings, &old_data_dir, &new_data_dir);
    } else {
        fs::create_dir_all(&new_data_dir)?;
    }
    save_app_settings(&settings)?;
    Ok(settings)
}

fn copy_app_data_directory_contents(source: &Path, destination: &Path) -> io::Result<()> {
    if same_directory(source, destination) {
        return Ok(());
    }
    if !source.exists() {
        fs::create_dir_all(destination)?;
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("settings.json")
        {
            continue;
        }
        copy_path_if_missing(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_path_if_missing(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::metadata(source)?;
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path_if_missing(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() && !destination.exists() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn rewrite_default_pet_library_paths(
    settings: &mut AppSettings,
    old_data_dir: &Path,
    new_data_dir: &Path,
) {
    if settings
        .pet_library
        .data_directory
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .is_some()
    {
        return;
    }

    let old_pet_dir = old_data_dir.join("pets");
    let new_pet_dir = new_data_dir.join("pets");
    settings.pet.image_path =
        rewrite_path_setting(settings.pet.image_path.take(), &old_pet_dir, &new_pet_dir);
    for pet in &mut settings.pet_library.pets {
        pet.image_path = rewrite_path_setting(pet.image_path.take(), &old_pet_dir, &new_pet_dir);
        pet.source_path = rewrite_path_setting(pet.source_path.take(), &old_pet_dir, &new_pet_dir);
    }
}

fn rewrite_path_setting(value: Option<String>, old_root: &Path, new_root: &Path) -> Option<String> {
    value.map(|path| {
        let current = PathBuf::from(&path);
        current
            .strip_prefix(old_root)
            .map(|relative| new_root.join(relative).to_string_lossy().to_string())
            .unwrap_or(path)
    })
}

fn same_directory(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn destination_inside_source(source: &Path, destination: &Path) -> bool {
    if destination.starts_with(source) {
        return true;
    }
    match (source.canonicalize(), destination.canonicalize()) {
        (Ok(source), Ok(destination)) => destination.starts_with(source),
        _ => false,
    }
}

fn settings_path() -> PathBuf {
    default_app_data_dir().join("settings.json")
}

fn default_data_root() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_data_dir_skips_settings_and_preserves_target_files() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("old-data");
        let destination = temp.path().join("new-data");
        fs::create_dir_all(source.join("logs"))?;
        fs::create_dir_all(&destination)?;
        fs::write(source.join("settings.json"), "old settings")?;
        fs::write(source.join("token-usage.json"), "old usage")?;
        fs::write(source.join("logs").join("code-pet.log"), "old log")?;
        fs::write(destination.join("token-usage.json"), "existing usage")?;

        copy_app_data_directory_contents(&source, &destination)?;

        assert_eq!(
            fs::read_to_string(destination.join("token-usage.json"))?,
            "existing usage"
        );
        assert_eq!(
            fs::read_to_string(destination.join("logs").join("code-pet.log"))?,
            "old log"
        );
        assert!(!destination.join("settings.json").exists());
        Ok(())
    }

    #[test]
    fn destination_inside_source_detects_nested_target() {
        let source = PathBuf::from("data");

        assert!(destination_inside_source(&source, &source.join("nested")));
        assert!(!destination_inside_source(&source, Path::new("other-data")));
    }

    #[test]
    fn rewrite_default_pet_library_paths_moves_managed_pet_files() {
        let old_data = PathBuf::from("old").join("code-pet");
        let new_data = PathBuf::from("new").join("code-pet");
        let old_image = old_data
            .join("pets")
            .join("image-1")
            .join("pixelated-48.png");
        let old_source = old_data.join("pets").join("image-1").join("source.png");
        let new_image = new_data
            .join("pets")
            .join("image-1")
            .join("pixelated-48.png");
        let new_source = new_data.join("pets").join("image-1").join("source.png");
        let mut settings = AppSettings::default();
        settings.pet.image_path = Some(old_image.to_string_lossy().to_string());
        settings.pet_library.pets.push(ConfiguredPet {
            id: "image-1".to_string(),
            name: "Imported".to_string(),
            kind: PetKind::Image,
            sprite: None,
            image_path: Some(old_image.to_string_lossy().to_string()),
            source_path: Some(old_source.to_string_lossy().to_string()),
            created_at: "2026-06-08T00:00:00Z".to_string(),
        });

        rewrite_default_pet_library_paths(&mut settings, &old_data, &new_data);

        let expected_image = new_image.to_string_lossy().to_string();
        let expected_source = new_source.to_string_lossy().to_string();
        assert_eq!(
            settings.pet.image_path.as_deref(),
            Some(expected_image.as_str())
        );
        let pet = settings.pet_library.pets.last().unwrap();
        assert_eq!(pet.image_path.as_deref(), Some(expected_image.as_str()));
        assert_eq!(pet.source_path.as_deref(), Some(expected_source.as_str()));
    }

    #[test]
    fn rewrite_default_pet_library_paths_keeps_explicit_pet_directory() {
        let old_data = PathBuf::from("old").join("code-pet");
        let new_data = PathBuf::from("new").join("code-pet");
        let old_image = old_data
            .join("pets")
            .join("image-1")
            .join("pixelated-48.png");
        let mut settings = AppSettings::default();
        settings.pet_library.data_directory = Some("/external/pets".to_string());
        settings.pet.image_path = Some(old_image.to_string_lossy().to_string());

        rewrite_default_pet_library_paths(&mut settings, &old_data, &new_data);

        let expected_image = old_image.to_string_lossy().to_string();
        assert_eq!(
            settings.pet.image_path.as_deref(),
            Some(expected_image.as_str())
        );
    }
}
