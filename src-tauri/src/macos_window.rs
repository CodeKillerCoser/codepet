#![cfg(target_os = "macos")]

use tauri::{AppHandle, Manager};
use tauri_nspanel::{
    objc2_app_kit::{NSWindowCollectionBehavior, NSWindowStyleMask},
    tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt,
};

const PET_WINDOW_LABEL: &str = "pet";

tauri_panel! {
    panel!(PetOverlayPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            becomes_key_only_if_needed: true,
            hides_on_deactivate: false,
            is_floating_panel: true,
            works_when_modal: true
        }
    })
}

pub fn pet_overlay_collection_behavior() -> NSWindowCollectionBehavior {
    CollectionBehavior::new()
        .can_join_all_spaces()
        .full_screen_auxiliary()
        .stationary()
        .ignores_cycle()
        .value()
}

pub fn pet_overlay_style_mask() -> NSWindowStyleMask {
    StyleMask::empty()
        .borderless()
        .nonactivating_panel()
        .value()
}

pub fn configure_pet_overlay_window(app: &AppHandle) -> Result<(), String> {
    let panel = match app.get_webview_panel(PET_WINDOW_LABEL) {
        Ok(panel) => panel,
        Err(_) => {
            let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) else {
                return Ok(());
            };
            window
                .to_panel::<PetOverlayPanel>()
                .map_err(|error| error.to_string())?
        }
    };

    panel.set_level(PanelLevel::Floating.value());
    panel.set_floating_panel(true);
    panel.set_hides_on_deactivate(false);
    panel.set_works_when_modal(true);
    panel.set_has_shadow(false);
    panel.set_transparent(true);
    panel.set_style_mask(pet_overlay_style_mask());
    panel.set_collection_behavior(pet_overlay_collection_behavior());
    panel.show();
    Ok(())
}
