//! A single writer for pet geometry. UI supplies natural layout; native owns placement and drag lifetime.
pub mod geometry;
use crate::platform::pet_surface as platform;
use geometry::*;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Floating,
    Docked,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    mode: Mode,
    display_id: String,
    x_ratio: f64,
    y_ratio: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    revision: u64,
    mode: Mode,
    dragging: bool,
    snap_candidate: bool,
    can_dock: bool,
    available_height: f64,
    available_width: f64,
    native_drag: bool,
    physical_notch: bool,
}
#[derive(Clone)]
struct Drag {
    grab: Point,
    origin: Saved,
    anchor: Point,
    display_id: String,
}
#[derive(Default)]
struct Surface {
    initialized: bool,
    revision: u64,
    mode: Mode,
    display_id: String,
    displays: Vec<Display>,
    anchor: Option<Point>,
    layout: Option<Layout>,
    drag: Option<Drag>,
    candidate: Option<String>,
    path: Option<PathBuf>,
    last_frame: Option<Rect>,
    last_view: Option<View>,
}
#[derive(Default)]
pub struct SurfaceState(Mutex<Surface>, tokio::sync::Notify);

impl Surface {
    fn display(&self) -> Option<&Display> {
        self.displays
            .iter()
            .find(|d| d.id == self.display_id)
            .or_else(|| self.displays.first())
    }
    fn saved(&self) -> Saved {
        let Some(d) = self.display() else {
            return Saved::default();
        };
        let a = self.anchor.unwrap_or(d.work.center());
        Saved {
            mode: self.mode,
            display_id: d.id.clone(),
            x_ratio: ((a.x - d.work.x) / d.work.width.max(1.)).clamp(0., 1.),
            y_ratio: ((a.y - d.work.y) / d.work.height.max(1.)).clamp(0., 1.),
        }
    }
    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec(&self.saved()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    }
    fn initialize(&mut self, env: &platform::Environment) {
        let settings = crate::settings::load_app_settings().unwrap_or_default();
        let path = crate::paths::data(&settings).join("config/pet-surface.json");
        let saved: Option<Saved> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        self.path = Some(path);
        self.displays = env.displays.clone();
        let d = saved
            .as_ref()
            .and_then(|s| self.displays.iter().find(|d| d.id == s.display_id))
            .or_else(|| display_for(env.frame.center(), &self.displays));
        if let Some(d) = d {
            self.display_id = d.id.clone();
            self.mode =
                if saved.as_ref().is_some_and(|s| s.mode == Mode::Docked) && d.notch.is_some() {
                    Mode::Docked
                } else {
                    Mode::Floating
                };
            self.anchor = Some(
                saved
                    .filter(|s| s.x_ratio.is_finite() && s.y_ratio.is_finite())
                    .map(|s| Point {
                        x: d.work.x + s.x_ratio.clamp(0., 1.) * d.work.width,
                        y: d.work.y + s.y_ratio.clamp(0., 1.) * d.work.height,
                    })
                    .unwrap_or(Point {
                        x: d.work.right() - 95. * d.unit,
                        y: d.work.bottom() - 95. * d.unit,
                    }),
            );
        }
        self.initialized = true;
        self.revision = 1;
    }
    fn view(&self) -> View {
        let available = self
            .display()
            .map(|d| {
                let top = if self.mode == Mode::Docked {
                    d.notch.map(Rect::bottom).unwrap_or(d.work.y)
                } else {
                    d.work.y
                };
                ((d.work.bottom() - top) / d.unit).max(60.)
            })
            .unwrap_or(700.);
        View {
            revision: self.revision,
            mode: self.mode,
            dragging: self.drag.is_some(),
            snap_candidate: self.candidate.is_some(),
            can_dock: self.displays.iter().any(|d| d.notch.is_some()),
            available_height: available,
            available_width: self
                .display()
                .map(|d| d.work.width / d.unit)
                .unwrap_or(400.),
            native_drag: platform::TRACKS_DRAG,
            physical_notch: self.display().is_some_and(|d| d.physical_notch),
        }
    }
    fn begin_drag(&mut self, pointer: Point) {
        if self.drag.is_none() {
            if let Some(anchor) = self.anchor {
                self.drag = Some(Drag {
                    grab: Point {
                        x: pointer.x - anchor.x,
                        y: pointer.y - anchor.y,
                    },
                    origin: self.saved(),
                    anchor,
                    display_id: self.display_id.clone(),
                });
                self.mode = Mode::Floating;
                self.revision += 1;
            }
        }
    }
    fn move_drag(&mut self, pointer: Point, pressed: bool, cancel: bool) -> bool {
        let Some(drag) = self.drag.clone() else {
            return false;
        };
        if cancel {
            self.cancel_drag();
            return true;
        }
        let anchor = Point {
            x: pointer.x - drag.grab.x,
            y: pointer.y - drag.grab.y,
        };
        self.anchor = Some(anchor);
        if let Some(d) = display_for(anchor, &self.displays) {
            if d.id != self.display_id {
                self.display_id = d.id.clone();
                self.revision += 1;
            }
        }
        self.candidate =
            dock_target(anchor, &self.displays, self.candidate.as_deref()).map(|d| d.id.clone());
        if !pressed {
            if let Some(id) = self.candidate.take() {
                self.display_id = id;
                self.mode = Mode::Docked;
            }
            self.drag = None;
            self.revision += 1;
            return true;
        }
        false
    }
    fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            self.mode = drag.origin.mode;
            self.display_id = drag.display_id;
            self.anchor = Some(drag.anchor);
            if self.display().is_none_or(|d| d.notch.is_none()) {
                self.mode = Mode::Floating;
            }
            self.candidate = None;
            self.revision += 1;
        }
    }
    fn update_environment(&mut self, env: &platform::Environment) {
        if self.displays != env.displays {
            let saved = self.saved();
            self.displays = env.displays.clone();
            if !self.displays.iter().any(|d| d.id == self.display_id) {
                if let Some(d) = display_for(env.pointer, &self.displays) {
                    self.display_id = d.id.clone();
                }
            }
            if self.drag.is_none() {
                if let Some(d) = self.display() {
                    self.anchor = Some(Point {
                        x: d.work.x + saved.x_ratio * d.work.width,
                        y: d.work.y + saved.y_ratio * d.work.height,
                    });
                }
            }
            if self.display().is_none_or(|d| d.notch.is_none()) {
                self.mode = Mode::Floating;
            }
            self.revision += 1;
        }
    }
    fn apply(&mut self, app: &AppHandle) -> Result<(), String> {
        let (Some(layout), Some(d), Some(anchor)) = (self.layout, self.display(), self.anchor)
        else {
            return Ok(());
        };
        let frame = if self.mode == Mode::Docked && d.notch.is_some() {
            dock_frame(d, layout)
        } else {
            let f = frame_at(anchor, layout, d.unit);
            if self.drag.is_some() {
                f
            } else {
                clamp(f, d.work)
            }
        };
        let anchor = anchor_in(frame, layout, d.unit);
        platform::apply(app, frame, self.mode == Mode::Docked || self.drag.is_some())?;
        self.anchor = Some(anchor);
        self.last_frame = Some(frame);
        Ok(())
    }
}

enum Action {
    Read,
    Layout(Layout),
    Dock,
    Float,
    Begin,
    Cancel,
    Tick,
    NativeDrag {
        origin: Point,
        point: Point,
        released: bool,
    },
}
async fn perform(app: AppHandle, action: Action) -> Result<View, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(perform_on_main(&handle, action));
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

fn perform_on_main(handle: &AppHandle, action: Action) -> Result<View, String> {
    let env = platform::read(&handle)?;
    let state = handle.state::<SurfaceState>();
    let mut s = state.0.lock().map_err(|_| "surface state poisoned")?;
    if !s.initialized {
        s.initialize(&env);
    }
    s.update_environment(&env);
    let mut save = false;
    match action {
        Action::NativeDrag {
            origin,
            point,
            released,
        } => {
            if s.drag.is_none() {
                s.begin_drag(origin);
                state.1.notify_one();
            }
            save = s.move_drag(point, !released, false);
        }
        Action::Read => {}
        Action::Layout(layout) => {
            if !layout.valid() {
                return Err("invalid pet layout".into());
            }
            if layout.revision == s.revision {
                let available = s.view();
                if layout.width > available.available_width + 1.
                    || layout.height > available.available_height + 1.
                {
                    return Err("pet layout exceeds current display".into());
                }
                s.layout = Some(layout);
            }
        }
        Action::Dock => {
            s.drag = None;
            s.candidate = None;
            let d = s
                .display()
                .filter(|d| d.notch.is_some())
                .or_else(|| s.displays.iter().find(|d| d.notch.is_some()))
                .ok_or("当前显示器不支持顶部停靠")?;
            s.display_id = d.id.clone();
            s.mode = Mode::Docked;
            s.revision += 1;
            save = true;
        }
        Action::Float => {
            s.drag = None;
            s.candidate = None;
            s.mode = Mode::Floating;
            s.revision += 1;
            save = true;
            if let (Some(d), Some(mut a)) = (s.display(), s.anchor) {
                a.y += 80. * d.unit;
                s.anchor = Some(a);
            }
        }
        Action::Begin => {
            if !platform::TRACKS_DRAG {
                return Err("use system window dragging on this platform".into());
            }
            if s.drag.is_none() && env.pressed {
                s.begin_drag(env.pointer);
                state.1.notify_one();
                if let Some(w) = handle.get_webview_window("pet") {
                    let _ = w.set_ignore_cursor_events(false);
                }
            }
        }
        Action::Cancel => {
            s.cancel_drag();
            save = true;
        }
        Action::Tick => {
            if s.drag.is_some() {
                save = s.move_drag(env.pointer, env.pressed, env.cancel);
            } else if s.mode == Mode::Floating && s.last_frame.is_some_and(|f| f != env.frame) {
                // Native system dragging on platforms without the custom drag adapter.
                if let Some(layout) = s.layout {
                    if let Some(d) = display_for(env.frame.center(), &s.displays).cloned() {
                        s.anchor = Some(anchor_in(env.frame, layout, d.unit));
                        if s.display_id != d.id {
                            s.display_id = d.id.clone();
                            s.revision += 1;
                        }
                        save = true;
                    }
                }
            }
        }
    }
    s.apply(&handle)?;
    if save {
        s.persist()?;
    }
    let view = s.view();
    if s.last_view.as_ref() != Some(&view) {
        let _ = handle.emit_to("pet", "pet-surface-changed", &view);
        s.last_view = Some(view.clone());
    }
    Ok(view)
}

pub fn start(app: AppHandle) {
    platform::install_drag_monitor(&app);
    tauri::async_runtime::spawn(async move {
        loop {
            let (initialized, dragging) = {
                let state = app.state::<SurfaceState>();
                let s = state.0.lock().unwrap();
                (s.initialized, s.drag.is_some())
            };
            {
                let state = app.state::<SurfaceState>();
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(if dragging{16}else{400})) => {},
                    _ = state.1.notified() => {},
                }
            }
            if initialized {
                if let Err(e) = perform(app.clone(), Action::Tick).await {
                    crate::app_log::error("pet.surface", &e);
                }
            }
        }
    });
}
#[tauri::command]
pub async fn pet_surface_state(app: AppHandle) -> Result<View, String> {
    perform(app, Action::Read).await
}
#[tauri::command]
pub async fn pet_surface_layout(app: AppHandle, layout: Layout) -> Result<View, String> {
    perform(app, Action::Layout(layout)).await
}
#[tauri::command]
pub async fn pet_surface_mode(app: AppHandle, mode: Mode) -> Result<View, String> {
    perform(
        app,
        if mode == Mode::Docked {
            Action::Dock
        } else {
            Action::Float
        },
    )
    .await
}
#[tauri::command]
pub async fn pet_surface_drag(app: AppHandle, cancel: bool) -> Result<View, String> {
    perform(
        app,
        if cancel {
            Action::Cancel
        } else {
            Action::Begin
        },
    )
    .await
}
#[tauri::command]
pub async fn pet_surface_pointer(app: AppHandle) -> Result<Point, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = platform::read(&handle).map(|e| {
            let unit = display_for(e.frame.center(), &e.displays)
                .map(|d| d.unit)
                .unwrap_or(1.);
            Point {
                x: (e.pointer.x - e.frame.x) / unit,
                y: (e.pointer.y - e.frame.y) / unit,
            }
        });
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface() -> Surface {
        Surface {
            initialized: true,
            revision: 1,
            mode: Mode::Docked,
            display_id: "screen".into(),
            anchor: Some(Point { x: 500., y: 90. }),
            displays: vec![Display {
                id: "screen".into(),
                frame: Rect {
                    x: 0.,
                    y: 0.,
                    width: 1000.,
                    height: 800.,
                },
                work: Rect {
                    x: 0.,
                    y: 40.,
                    width: 1000.,
                    height: 720.,
                },
                notch: Some(Rect {
                    x: 410.,
                    y: 0.,
                    width: 180.,
                    height: 40.,
                }),
                physical_notch: true,
                unit: 1.,
            }],
            ..Default::default()
        }
    }
    #[test]
    fn detach_preserves_grab_offset_and_release_outside_stays_floating() {
        let mut s = surface();
        s.begin_drag(Point { x: 510., y: 95. });
        assert_eq!(s.mode, Mode::Floating);
        assert!(!s.move_drag(Point { x: 310., y: 405. }, true, false));
        assert_eq!(s.anchor, Some(Point { x: 300., y: 400. }));
        assert!(s.move_drag(Point { x: 310., y: 405. }, false, false));
        assert!(s.drag.is_none());
        assert_eq!(s.mode, Mode::Floating);
        assert!(s.candidate.is_none());
    }
    #[test]
    fn release_in_target_docks_only_on_mouse_up() {
        let mut s = surface();
        s.mode = Mode::Floating;
        s.anchor = Some(Point { x: 500., y: 400. });
        s.begin_drag(Point { x: 500., y: 400. });
        assert!(!s.move_drag(Point { x: 500., y: 70. }, true, false));
        assert_eq!(s.mode, Mode::Floating);
        assert!(s.candidate.is_some());
        assert!(s.move_drag(Point { x: 500., y: 70. }, false, false));
        assert_eq!(s.mode, Mode::Docked);
        assert!(s.candidate.is_none());
    }
    #[test]
    fn escape_restores_origin_and_clears_drag() {
        let mut s = surface();
        s.begin_drag(Point { x: 510., y: 95. });
        s.move_drag(Point { x: 100., y: 600. }, true, false);
        s.move_drag(Point { x: 100., y: 600. }, true, true);
        assert_eq!(s.mode, Mode::Docked);
        assert_eq!(s.anchor, Some(Point { x: 500., y: 90. }));
        assert!(s.drag.is_none());
    }
    #[test]
    fn unplugging_screen_falls_back_and_invalidates_layout_epoch() {
        let mut s = surface();
        let old = s.revision;
        let env = platform::Environment {
            displays: vec![Display {
                id: "external".into(),
                frame: Rect {
                    x: -1200.,
                    y: -300.,
                    width: 1200.,
                    height: 900.,
                },
                work: Rect {
                    x: -1200.,
                    y: -300.,
                    width: 1200.,
                    height: 900.,
                },
                notch: None,
                physical_notch: false,
                unit: 1.25,
            }],
            frame: Rect::default(),
            pointer: Point::default(),
            pressed: false,
            cancel: false,
        };
        s.update_environment(&env);
        assert_eq!(s.display_id, "external");
        assert_eq!(s.mode, Mode::Floating);
        assert!(s.revision > old);
        assert!(s.display().unwrap().work.contains(s.anchor.unwrap()));
        assert!(!s.view().can_dock);
        assert_eq!(s.view().available_width, 960.);
        assert_eq!(s.view().available_height, 720.);
    }
    #[test]
    fn saves_only_surface_preferences_as_normalized_coordinates() {
        let mut s = surface();
        let directory = tempfile::tempdir().unwrap();
        s.path = Some(directory.path().join("config/pet-surface.json"));
        s.persist().unwrap();
        let saved: Saved =
            serde_json::from_slice(&std::fs::read(s.path.as_ref().unwrap()).unwrap()).unwrap();
        assert_eq!(saved.mode, Mode::Docked);
        assert_eq!(saved.x_ratio, 0.5);
        assert_eq!(saved.display_id, "screen");
    }
}

/// Called only by the platform event adapter, on the main thread.
pub fn native_hit(app: &AppHandle, point: Point) -> bool {
    let state = app.state::<SurfaceState>();
    let s = state.0.lock().unwrap();
    let (Some(l), Some(a), Some(d)) = (s.layout, s.anchor, s.display()) else {
        return false;
    };
    Rect {
        x: a.x - l.pet_width * d.unit / 2.,
        y: a.y - l.pet_height * d.unit / 2.,
        width: l.pet_width * d.unit,
        height: l.pet_height * d.unit,
    }
    .contains(point)
}
pub fn native_drag(app: &AppHandle, origin: Point, point: Point, released: bool) {
    if let Err(e) = perform_on_main(
        app,
        Action::NativeDrag {
            origin,
            point,
            released,
        },
    ) {
        crate::app_log::error("pet.surface", &e);
    }
}
pub fn native_cancel(app: &AppHandle) {
    let _ = perform_on_main(app, Action::Cancel);
}
