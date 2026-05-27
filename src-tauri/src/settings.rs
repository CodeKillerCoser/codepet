use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub pet: PetSettings,
    #[serde(default)]
    pub pet_library: PetLibrarySettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub theme: ThemeChoice,
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
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,
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

impl Default for PetKind {
    fn default() -> Self {
        Self::Palette
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
    #[serde(default = "default_repeat_seconds")]
    pub repeat_seconds: u16,
    #[serde(default)]
    pub quiet_hours_enabled: bool,
    #[serde(default = "default_quiet_start")]
    pub quiet_hours_start: String,
    #[serde(default = "default_quiet_end")]
    pub quiet_hours_end: String,
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
        }
    }
}

impl Default for ThemeChoice {
    fn default() -> Self {
        Self::System
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
            always_on_top: default_always_on_top(),
        }
    }
}

impl Default for PetLibrarySettings {
    fn default() -> Self {
        Self {
            data_directory: None,
            selected_pet_id: default_pet_id(),
            pets: default_pet_profiles(),
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
            appearance: AppearanceSettings::default(),
            pet: PetSettings::default(),
            pet_library: PetLibrarySettings::default(),
            notifications: NotificationSettings::default(),
        }
    }
}

pub fn default_sprite() -> PixelPetSprite {
    PixelPetSprite {
        body: "#f4c04e".to_string(),
        accent: "#1f2937".to_string(),
        eyes: "#2563eb".to_string(),
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

fn default_always_on_top() -> bool {
    true
}

fn default_true() -> bool {
    true
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

fn settings_path() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("code-pet")
        .join("settings.json")
}
