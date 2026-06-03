#![cfg(target_os = "macos")]

use code_pet_lib::macos_window::{pet_overlay_collection_behavior, pet_overlay_style_mask};
use tauri_nspanel::objc2_app_kit::{NSWindowCollectionBehavior, NSWindowStyleMask};

#[test]
fn pet_overlay_panel_can_become_key_for_text_entry() {
    let source = include_str!("../src/macos_window.rs");

    assert!(source.contains("can_become_key_window: true"));
    assert!(!source.contains("can_become_key_window: false"));
}

#[test]
fn pet_overlay_window_joins_all_spaces_and_full_screen_spaces() {
    let behavior = pet_overlay_collection_behavior();

    assert!(behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
    assert!(behavior.contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
    assert!(behavior.contains(NSWindowCollectionBehavior::Stationary));
    assert!(behavior.contains(NSWindowCollectionBehavior::IgnoresCycle));
}

#[test]
fn pet_overlay_window_uses_nonactivating_panel_style() {
    let style = pet_overlay_style_mask();

    assert!(style.contains(NSWindowStyleMask::NonactivatingPanel));
    assert!(!style.contains(NSWindowStyleMask::Resizable));
    assert!(!style.contains(NSWindowStyleMask::Titled));
}
