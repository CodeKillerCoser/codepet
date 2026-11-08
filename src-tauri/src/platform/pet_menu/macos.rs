use super::*;
use crate::pet::surface::geometry::Point;
use base64::Engine;
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::{rc::Retained, runtime::AnyObject};
use std::{cell::RefCell, ptr::NonNull};
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_nspanel::objc2_app_kit::{NSEvent, NSEventMask};
use tauri_nspanel::ManagerExt;

const ID: &str = "code-pet-companion";
thread_local! {
    static MONITORS: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) };
    static VISIBLE: RefCell<bool> = const { RefCell::new(false) };
    static SUMMARY: RefCell<Option<Summary>> = const { RefCell::new(None) };
}

pub fn install(app: &AppHandle) -> Result<(), String> {
    let fallback = image::load_from_memory(include_bytes!(
        "../../../../frontend/assets/pets/panda.webp"
    ))
    .map_err(|e| e.to_string())?
    .crop_imm(0, 0, 192, 128)
    .resize_exact(36, 24, image::imageops::FilterType::Lanczos3)
    .to_rgba8();
    let tray = TrayIconBuilder::with_id(ID)
        .icon(Image::new_owned(fallback.into_raw(), 36, 24))
        .title("就绪")
        .tooltip("Code Pet · 点击查看任务，拖动头像切换为悬浮")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            let mouse = NSEvent::mouseLocation();
            let point = Point {
                x: mouse.x,
                y: -mouse.y,
            };
            let input = match event {
                TrayIconEvent::Enter { .. } => Some(Input::Enter),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Down,
                    ..
                } => Some(Input::Down(point)),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => Some(Input::Up(point)),
                _ => None,
            };
            if let Some(input) = input {
                crate::pet::surface::menu_input(tray.app_handle(), input);
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    tray.set_visible(false).map_err(|e| e.to_string())?;
    let handle = app.clone();
    let local = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let point = NSEvent::mouseLocation();
        crate::pet::surface::menu_outside(
            &handle,
            Point {
                x: point.x,
                y: -point.y,
            },
        );
        event.as_ptr()
    });
    let handle = app.clone();
    let global = RcBlock::new(move |_event: NonNull<NSEvent>| {
        let point = NSEvent::mouseLocation();
        crate::pet::surface::menu_outside(
            &handle,
            Point {
                x: point.x,
                y: -point.y,
            },
        );
    });
    MONITORS.with(|monitors| {
        let mut monitors = monitors.borrow_mut();
        let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
        if let Some(m) =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) }
        {
            monitors.push(m);
        }
        if let Some(m) = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global) {
            monitors.push(m);
        }
    });
    Ok(())
}

pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    VISIBLE.with(|old| {
        if *old.borrow() != visible {
            app.tray_by_id(ID)
                .ok_or("pet menu unavailable")?
                .set_visible(visible)
                .map_err(|e| e.to_string())?;
            *old.borrow_mut() = visible;
        }
        Ok(())
    })
}

pub fn update(app: &AppHandle, summary: &Summary) -> Result<(), String> {
    SUMMARY.with(|old| {
        if old.borrow().as_ref() == Some(summary) {
            return Ok(());
        }
        let tray = app.tray_by_id(ID).ok_or("pet menu unavailable")?;
        if old.borrow().as_ref().and_then(|s| s.portrait.as_ref()) != summary.portrait.as_ref() {
            if let Some(encoded) = &summary.portrait {
                if encoded.len() > 32_768 {
                    return Err("menu portrait too large".into());
                }
                let png = encoded
                    .strip_prefix("data:image/png;base64,")
                    .ok_or("menu portrait must be PNG")?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(png)
                    .map_err(|e| e.to_string())?;
                // Check dimensions before allocating a decoded image.
                let reader = image::ImageReader::with_format(
                    std::io::Cursor::new(&bytes),
                    image::ImageFormat::Png,
                );
                let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
                if w == 0 || h == 0 || w > 64 || h > 64 {
                    return Err("invalid menu portrait dimensions".into());
                }
                let rgba = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?
                    .to_rgba8();
                tray.set_icon(Some(Image::new_owned(rgba.into_raw(), w, h)))
                    .map_err(|e| e.to_string())?;
            }
        }
        let label: String = summary
            .label
            .chars()
            .filter(|c| !c.is_control())
            .take(8)
            .collect();
        let title = if summary.count > 0 {
            format!("{} · {}", label, summary.count.min(999))
        } else {
            label
        };
        tray.set_title(Some(title)).map_err(|e| e.to_string())?;
        *old.borrow_mut() = Some(summary.clone());
        Ok(())
    })
}

/// Read AppKit points directly, avoiding tray-event physical coordinates on mixed-DPI displays.
pub fn anchor(app: &AppHandle) -> Option<Rect> {
    app.tray_by_id(ID)?
        .with_inner_tray_icon(|tray| {
            let item = tray.ns_status_item()?;
            let button = item.button(MainThreadMarker::new()?)?;
            let window = button.window()?;
            let f = window.frame();
            let screen = window.screen()?.frame();
            // A newly inserted item briefly has an unplaced frame at (0, 0).
            // Do not use that frame as a popup anchor or drag target.
            if f.size.width <= 0.
                || f.size.height <= 0.
                || (f.origin.y + f.size.height - screen.origin.y - screen.size.height).abs() > 2.
            {
                return None;
            }
            Some(Rect {
                x: f.origin.x,
                y: -f.origin.y - f.size.height,
                width: f.size.width,
                height: f.size.height,
            })
        })
        .ok()
        .flatten()
}

pub fn show_panel(app: &AppHandle, visible: bool) -> Result<(), String> {
    let panel = app.get_webview_panel("pet").map_err(|e| format!("{e:?}"))?;
    if visible {
        if !panel.as_panel().isVisible() {
            panel.show();
        }
    } else if panel.as_panel().isVisible() {
        panel.hide();
    }
    Ok(())
}
