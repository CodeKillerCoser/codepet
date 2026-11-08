//! System menu entry and panel visibility live behind this platform boundary.
use crate::pet::surface::geometry::{Point, Rect};
use serde::Deserialize;
use tauri::AppHandle;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub label: String,
    pub count: usize,
    pub portrait: Option<String>,
}

pub const SUPPORTED: bool = cfg!(target_os = "macos");
#[derive(Clone, Copy, Debug)]
pub enum Input {
    Enter,
    Down(Point),
    Up(Point),
}

#[cfg(target_os = "macos")]
#[path = "pet_menu/macos.rs"]
mod implementation;
#[cfg(target_os = "macos")]
pub use implementation::{anchor, install, set_visible, show_panel, update};

#[cfg(not(target_os = "macos"))]
pub fn install(_: &AppHandle) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn anchor(_: &AppHandle) -> Option<Rect> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn set_visible(_: &AppHandle, _: bool) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn update(_: &AppHandle, _: &Summary) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn show_panel(_: &AppHandle, _: bool) -> Result<(), String> {
    Ok(())
}
