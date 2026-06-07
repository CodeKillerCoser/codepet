use code_pet_lib::agents::AgentId;
use code_pet_lib::settings::{configured_app_data_dir, AppSettings, ThemeChoice, WhipReactionSound};

#[test]
fn settings_default_to_system_theme() {
    let settings = AppSettings::default();

    assert_eq!(settings.appearance.theme, ThemeChoice::System);
    assert!(settings.data.data_directory.is_none());
    assert!(settings.appearance.running_bubble.background_breathing);
    assert!(!settings.appearance.running_bubble.border_marquee);
    assert_eq!(settings.appearance.running_bubble.animation_ms, 1800);
    assert_eq!(settings.appearance.running_bubble.border_width, 1);
    assert_eq!(settings.pet.image_pixel_size, 48);
    assert_eq!(settings.pet.opacity, 1.0);
    assert_eq!(settings.pet.whip_reaction_sound, WhipReactionSound::None);
    assert!(settings.pet.custom_whip_reaction_sound_path.is_none());
    assert!(settings.activity_filters.title_keywords.is_empty());
    assert!(settings.activity_filters.message_keywords.is_empty());
    assert!(settings.activity_filters.by_agent.is_empty());
    assert!(settings.agents.by_agent.is_empty());
    assert!(settings.updates.ignored_version.is_none());
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
    assert!(settings.data.data_directory.is_none());
    assert!(settings.appearance.running_bubble.background_breathing);
    assert!(!settings.appearance.running_bubble.border_marquee);
    assert_eq!(settings.appearance.running_bubble.border_width, 1);
    assert_eq!(settings.pet.scale, 4);
    assert_eq!(settings.pet.image_pixel_size, 48);
    assert_eq!(settings.pet.opacity, 1.0);
    assert_eq!(settings.pet.whip_reaction_sound, WhipReactionSound::None);
    assert!(settings.pet.custom_whip_reaction_sound_path.is_none());
    assert_eq!(settings.pet.sprite.body, "#111111");
    assert!(!settings.notifications.ring_on_failure);
    assert!(!settings.notifications.ring_on_done);
    assert!(settings.activity_filters.title_keywords.is_empty());
    assert!(settings.updates.ignored_version.is_none());
}

#[test]
fn settings_read_ignored_update_version() {
    let settings: AppSettings = serde_json::from_str(
        r#"{
          "updates": {
            "ignoredVersion": "0.2.0"
          }
        }"#,
    )
    .unwrap();

    assert_eq!(settings.updates.ignored_version.as_deref(), Some("0.2.0"));
}

#[test]
fn settings_read_activity_filter_keywords() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "activityFilters": {
            "titleKeywords": ["memory summary", "生成标题"],
            "messageKeywords": ["Recent Codex threads", "MEMORY.md"]
          }
        }"##,
    )
    .unwrap();

    assert_eq!(settings.activity_filters.title_keywords, vec!["memory summary", "生成标题"]);
    assert_eq!(settings.activity_filters.message_keywords, vec!["Recent Codex threads", "MEMORY.md"]);
}

#[test]
fn settings_read_per_agent_activity_filters_and_hook_preferences() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "activityFilters": {
            "byAgent": {
              "codex": {
                "titleKeywords": ["memory summary"],
                "messageKeywords": ["Recent Codex threads"]
              },
              "claude": {
                "titleKeywords": ["transcript"],
                "messageKeywords": []
              }
            }
          },
          "agents": {
            "byAgent": {
              "codex": {
                "hookEvents": ["UserPromptSubmit", "Stop"]
              }
            }
          }
        }"##,
    )
    .unwrap();

    let codex_filters = settings
        .activity_filters
        .by_agent
        .get(&AgentId::Codex)
        .unwrap();
    let claude_filters = settings
        .activity_filters
        .by_agent
        .get(&AgentId::Claude)
        .unwrap();
    assert_eq!(codex_filters.title_keywords, vec!["memory summary"]);
    assert_eq!(codex_filters.message_keywords, vec!["Recent Codex threads"]);
    assert_eq!(claude_filters.title_keywords, vec!["transcript"]);
    assert_eq!(
        settings
            .agents
            .by_agent
            .get(&AgentId::Codex)
            .unwrap()
            .hook_events,
        vec!["UserPromptSubmit", "Stop"]
    );
}

#[test]
fn settings_read_custom_app_data_directory() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "data": {
            "dataDirectory": "/tmp/code-pet-data"
          }
        }"##,
    )
    .unwrap();

    assert_eq!(settings.data.data_directory.as_deref(), Some("/tmp/code-pet-data"));
    assert_eq!(configured_app_data_dir(&settings), std::path::PathBuf::from("/tmp/code-pet-data"));
}

#[test]
fn settings_read_whip_reaction_sound_personalization() {
    let settings: AppSettings = serde_json::from_str(
        r##"{
          "pet": {
            "whipReactionSound": "custom",
            "customWhipReactionSoundPath": "/tmp/ouch.wav"
          }
        }"##,
    )
    .unwrap();

    assert_eq!(settings.pet.whip_reaction_sound, WhipReactionSound::Custom);
    assert_eq!(settings.pet.custom_whip_reaction_sound_path.as_deref(), Some("/tmp/ouch.wav"));
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
              "borderWidth": 4,
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
    assert_eq!(settings.appearance.running_bubble.border_width, 4);
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
