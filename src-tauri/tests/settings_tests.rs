use code_pet_lib::settings::{AppSettings, ThemeChoice};

#[test]
fn settings_default_to_system_theme() {
    let settings = AppSettings::default();

    assert_eq!(settings.appearance.theme, ThemeChoice::System);
}

#[test]
fn settings_keep_existing_values_when_theme_field_is_missing() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "pet": {
            "sprite": { "body": "#111111", "accent": "#222222", "eyes": "#333333" },
            "scale": 4,
            "alwaysOnTop": true
          },
          "notifications": {
            "sound": "bell",
            "customSoundPath": null,
            "ringOnPermission": true,
            "ringOnFailure": false,
            "repeatSeconds": 45,
            "quietHoursEnabled": false,
            "quietHoursStart": "22:00",
            "quietHoursEnd": "08:00"
          }
        }"##,
    )
    .unwrap();

    assert_eq!(settings.appearance.theme, ThemeChoice::System);
    assert_eq!(settings.pet.scale, 4);
    assert_eq!(settings.pet.sprite.body, "#111111");
    assert!(!settings.notifications.ring_on_failure);
}
