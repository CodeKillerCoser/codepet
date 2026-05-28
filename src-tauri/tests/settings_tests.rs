use code_pet_lib::settings::{AppSettings, ThemeChoice};

#[test]
fn settings_default_to_system_theme() {
    let settings = AppSettings::default();

    assert_eq!(settings.appearance.theme, ThemeChoice::System);
    assert!(settings.appearance.running_bubble.background_breathing);
    assert!(!settings.appearance.running_bubble.border_marquee);
    assert_eq!(settings.appearance.running_bubble.animation_ms, 1800);
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
            "ringOnDone": false,
            "repeatSeconds": 45,
            "quietHoursEnabled": false,
            "quietHoursStart": "22:00",
            "quietHoursEnd": "08:00"
          }
        }"##,
    )
    .unwrap();

    assert_eq!(settings.appearance.theme, ThemeChoice::System);
    assert!(settings.appearance.running_bubble.background_breathing);
    assert!(!settings.appearance.running_bubble.border_marquee);
    assert_eq!(settings.pet.scale, 4);
    assert_eq!(settings.pet.sprite.body, "#111111");
    assert!(!settings.notifications.ring_on_failure);
    assert!(!settings.notifications.ring_on_done);
}

#[test]
fn settings_read_running_bubble_personalization() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "appearance": {
            "theme": "dark",
            "runningBubble": {
              "backgroundBreathing": false,
              "borderMarquee": true,
              "backgroundColor": "#102a43",
              "borderColor": "#f59e0b",
              "animationMs": 950
            }
          }
        }"##,
    )
    .unwrap();

    assert!(!settings.appearance.running_bubble.background_breathing);
    assert!(settings.appearance.running_bubble.border_marquee);
    assert_eq!(settings.appearance.running_bubble.background_color, "#102a43");
    assert_eq!(settings.appearance.running_bubble.border_color, "#f59e0b");
    assert_eq!(settings.appearance.running_bubble.animation_ms, 950);
}

#[test]
fn settings_enable_done_ringing_when_field_is_missing() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "notifications": {
            "sound": "bell",
            "ringOnPermission": true,
            "ringOnFailure": true
          }
        }"##,
    )
    .unwrap();

    assert!(settings.notifications.ring_on_done);
}
