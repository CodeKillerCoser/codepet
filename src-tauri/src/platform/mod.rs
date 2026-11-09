pub mod app_updates;
pub mod host_identity;
pub mod main_window;
pub mod pet_surface;
pub mod pet_menu;
pub mod permissions;

#[cfg(target_os = "macos")]
pub mod macos_window;

#[cfg(target_os = "macos")]
pub mod power;
