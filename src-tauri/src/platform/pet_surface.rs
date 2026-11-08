//! Platform boundary: all surface coordinates are top-down native units.
//! macOS uses AppKit points (not Tauri physical coordinates); other platforms use pixels.
use crate::pet::surface::geometry::{Display, Point, Rect};
use tauri::AppHandle;

pub struct Environment {
    pub displays: Vec<Display>,
    pub frame: Rect,
    pub pointer: Point,
    pub pressed: bool,
    pub cancel: bool,
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::*;
    use block2::RcBlock;
    use objc2::{msg_send, MainThreadMarker};
    use objc2::{rc::Retained, runtime::AnyObject};
    use std::{
        cell::RefCell,
        ptr::NonNull,
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
    };
    use tauri_nspanel::objc2_app_kit::{NSEventMask, NSEventType};
    static LEFT_DOWN: AtomicBool = AtomicBool::new(false);
    thread_local! { static MONITORS: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) }; }
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use tauri_nspanel::{
        objc2_app_kit::{NSEvent, NSScreen},
        ManagerExt,
    };
    unsafe extern "C" {
        fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    }
    fn rect(r: NSRect) -> Rect {
        Rect {
            x: r.origin.x,
            y: -(r.origin.y + r.size.height),
            width: r.size.width,
            height: r.size.height,
        }
    }
    pub fn read(app: &AppHandle) -> Result<Environment, String> {
        let mtm = MainThreadMarker::new().ok_or("surface must run on main thread")?;
        let screens = NSScreen::screens(mtm);
        let displays = screens
            .iter()
            .map(|s| {
                let frame = rect(s.frame());
                let description = s.deviceDescription();
                let number = description.objectForKey(&NSString::from_str("NSScreenNumber"));
                let id = number
                    .map(|n| {
                        let value: u32 = unsafe { msg_send![&*n, unsignedIntValue] };
                        value.to_string()
                    })
                    .unwrap_or_else(|| s.localizedName().to_string());
                let top = s.safeAreaInsets().top;
                let notch = if top > 0. {
                    let left = s.auxiliaryTopLeftArea();
                    let right = s.auxiliaryTopRightArea();
                    let (x, width) = if left.size.width > 0. && right.size.width > 0. {
                        let x = left.origin.x + left.size.width;
                        (x, (right.origin.x - x).max(1.))
                    } else {
                        (frame.center().x - 90., 180.)
                    };
                    Some(Rect {
                        x,
                        y: frame.y,
                        width,
                        height: top,
                    })
                } else {
                    Some(Rect {
                        x: frame.center().x - 90.,
                        y: frame.y,
                        width: 180.,
                        height: (rect(s.visibleFrame()).y - frame.y).max(24.),
                    })
                };
                Display {
                    id,
                    frame,
                    work: rect(s.visibleFrame()),
                    notch,
                    physical_notch: top > 0.,
                    unit: 1.,
                }
            })
            .collect();
        let panel = app.get_webview_panel("pet").map_err(|e| format!("{e:?}"))?;
        let frame = rect(panel.as_panel().frame());
        let mouse = NSEvent::mouseLocation();
        Ok(Environment {
            displays,
            frame,
            pointer: Point {
                x: mouse.x,
                y: -mouse.y,
            },
            pressed: LEFT_DOWN.load(Ordering::Relaxed) || NSEvent::pressedMouseButtons() & 1 != 0,
            cancel: unsafe { CGEventSourceKeyState(1, 53) },
        })
    }
    pub fn install_drag_monitor(app: &AppHandle) {
        let press = Rc::new(RefCell::new(None::<Point>));
        let dragging = Rc::new(RefCell::new(false));
        let handle = app.clone();
        let local = RcBlock::new(move |raw: NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { raw.as_ref() };
            let kind = event.r#type();
            if kind == NSEventType::LeftMouseUp {
                LEFT_DOWN.store(false, Ordering::Relaxed);
            }
            let Ok(panel) = handle.get_webview_panel("pet") else {
                return raw.as_ptr();
            };
            if event.windowNumber() != panel.as_panel().windowNumber() {
                return raw.as_ptr();
            }
            let p = panel
                .as_panel()
                .convertPointToScreen(event.locationInWindow());
            let point = Point { x: p.x, y: -p.y };
            if kind == NSEventType::LeftMouseDown {
                LEFT_DOWN.store(true, Ordering::Relaxed);
                *dragging.borrow_mut() = false;
                *press.borrow_mut() =
                    crate::pet::surface::native_hit(&handle, point).then_some(point);
            } else if kind == NSEventType::LeftMouseDragged {
                if let Some(origin) = *press.borrow() {
                    if *dragging.borrow() || (point.x - origin.x).hypot(point.y - origin.y) >= 5. {
                        *dragging.borrow_mut() = true;
                        crate::pet::surface::native_drag(&handle, origin, point, false);
                        return std::ptr::null_mut();
                    }
                }
            } else if kind == NSEventType::LeftMouseUp {
                let origin = press.borrow_mut().take();
                if *dragging.borrow() {
                    *dragging.borrow_mut() = false;
                    if let Some(origin) = origin {
                        crate::pet::surface::native_drag(&handle, origin, point, true);
                    }
                    return std::ptr::null_mut();
                }
            } else if kind == NSEventType::KeyDown && event.keyCode() == 53 && *dragging.borrow() {
                *dragging.borrow_mut() = false;
                press.borrow_mut().take();
                crate::pet::surface::native_cancel(&handle);
                return std::ptr::null_mut();
            }
            raw.as_ptr()
        });
        let global = RcBlock::new(|_: NonNull<NSEvent>| {
            LEFT_DOWN.store(false, Ordering::Relaxed);
        });
        MONITORS.with(|monitors| {
            let mut monitors = monitors.borrow_mut();
            if !monitors.is_empty() {
                return;
            }
            // Local monitor preserves ordinary clicks; consumes only an established pet drag.
            if let Some(m) = unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                    NSEventMask::LeftMouseDown
                        | NSEventMask::LeftMouseDragged
                        | NSEventMask::LeftMouseUp
                        | NSEventMask::KeyDown,
                    &local,
                )
            } {
                monitors.push(m);
            }
            // Mouse-up in another application ends the same drag without requiring focus.
            if let Some(m) = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseUp,
                &global,
            ) {
                monitors.push(m);
            }
        });
    }
    pub fn apply(app: &AppHandle, frame: Rect, elevated: bool) -> Result<(), String> {
        let panel = app.get_webview_panel("pet").map_err(|e| format!("{e:?}"))?;
        let ns = panel.as_panel();
        ns.setLevel(if elevated { 27 } else { 3 });
        let next = NSRect::new(
            NSPoint::new(frame.x, -frame.y - frame.height),
            NSSize::new(frame.width, frame.height),
        );
        if ns.frame() != next {
            ns.setFrame_display(next, true);
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod implementation {
    use super::*;
    use tauri::{Manager, PhysicalPosition, PhysicalSize};
    pub fn read(app: &AppHandle) -> Result<Environment, String> {
        let w = app
            .get_webview_window("pet")
            .ok_or("pet window unavailable")?;
        let displays = w
            .available_monitors()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                let a = m.work_area();
                Display {
                    id: format!(
                        "{}:{}:{}",
                        m.name().map(String::as_str).unwrap_or("display"),
                        p.x,
                        p.y
                    ),
                    frame: Rect {
                        x: p.x as f64,
                        y: p.y as f64,
                        width: s.width as f64,
                        height: s.height as f64,
                    },
                    work: Rect {
                        x: a.position.x as f64,
                        y: a.position.y as f64,
                        width: a.size.width as f64,
                        height: a.size.height as f64,
                    },
                    notch: None,
                    physical_notch: false,
                    unit: m.scale_factor(),
                }
            })
            .collect();
        let p = w.outer_position().map_err(|e| e.to_string())?;
        let s = w.inner_size().map_err(|e| e.to_string())?;
        let pointer = w.cursor_position().map_err(|e| e.to_string())?;
        Ok(Environment {
            displays,
            frame: Rect {
                x: p.x as f64,
                y: p.y as f64,
                width: s.width as f64,
                height: s.height as f64,
            },
            pointer: Point {
                x: pointer.x,
                y: pointer.y,
            },
            pressed: false,
            cancel: false,
        })
    }
    pub fn apply(app: &AppHandle, frame: Rect, _elevated: bool) -> Result<(), String> {
        let w = app
            .get_webview_window("pet")
            .ok_or("pet window unavailable")?;
        w.set_size(PhysicalSize::new(
            frame.width.round() as u32,
            frame.height.round() as u32,
        ))
        .map_err(|e| e.to_string())?;
        w.set_position(PhysicalPosition::new(
            frame.x.round() as i32,
            frame.y.round() as i32,
        ))
        .map_err(|e| e.to_string())
    }
}
#[cfg(target_os = "macos")]
pub use implementation::install_drag_monitor;
pub use implementation::{apply, read};
#[cfg(not(target_os = "macos"))]
pub fn install_drag_monitor(_: &AppHandle) {}
pub const TRACKS_DRAG: bool = cfg!(target_os = "macos");
