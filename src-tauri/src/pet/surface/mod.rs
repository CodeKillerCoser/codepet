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
    #[serde(default)]
    dock_x_ratio: Option<f64>,
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
    top_inset: f64,
    notch_width: f64,
    native_drag: bool,
    physical_notch: bool,
}
#[derive(Clone)]
struct Drag {
    grab: Point,
    origin: Saved,
    anchor: Point,
    display_id: String,
    center_grab: f64,
    detached: bool,
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
    dock_x_ratio: f64,
}
#[derive(Default)]
pub struct SurfaceState(Mutex<Surface>, tokio::sync::Notify);

impl Surface {
    fn drag_hit(&self, point: Point) -> bool {
        let (Some(l), Some(a), Some(d)) = (self.layout, self.anchor, self.display()) else {
            return false;
        };
        if self.mode == Mode::Docked {
            if let (Some(region), Some(frame)) = (l.drag_region, self.last_frame) {
                return Rect {
                    x: frame.x + region.x * d.unit,
                    y: frame.y + region.y * d.unit,
                    width: region.width * d.unit,
                    height: region.height * d.unit,
                }
                .contains(point);
            }
        }
        Rect {
            x: a.x - l.pet_width * d.unit / 2.,
            y: a.y - l.pet_height * d.unit / 2.,
            width: l.pet_width * d.unit,
            height: l.pet_height * d.unit,
        }
        .contains(point)
    }
    fn hide_docked_panel(&self) -> bool {
        self.mode == Mode::Docked && self.layout.is_none_or(|l| l.revision != self.revision)
    }
    fn dock_center(&self) -> f64 {
        self.display()
            .map(|d| d.frame.x + self.dock_x_ratio * d.frame.width)
            .unwrap_or(0.)
    }
    fn top_inset(&self) -> f64 {
        if self.mode != Mode::Docked {
            return 0.;
        }
        self.display()
            .map(|d| {
                dock_inset(
                    d,
                    self.dock_center(),
                    self.layout.map(|l| l.width).unwrap_or(200.) * d.unit,
                ) / d.unit
            })
            .unwrap_or(0.)
    }
    fn place_dock(&mut self, pointer_x: f64) {
        if let Some(d) = self.display() {
            let center = d
                .notch
                .filter(|_| d.physical_notch)
                .filter(|n| (pointer_x - n.center().x).abs() <= n.width / 2. + 24. * d.unit)
                .map(|n| n.center().x)
                .unwrap_or(pointer_x);
            self.dock_x_ratio = ((center - d.frame.x) / d.frame.width).clamp(0., 1.);
        }
    }
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
            dock_x_ratio: Some(self.dock_x_ratio),
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
        self.dock_x_ratio = saved
            .as_ref()
            .and_then(|s| s.dock_x_ratio)
            .filter(|x| x.is_finite())
            .unwrap_or(0.5)
            .clamp(0., 1.);
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
                    d.frame.y
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
            top_inset: self.top_inset(),
            notch_width: self
                .display()
                .and_then(|d| {
                    d.notch
                        .filter(|_| d.physical_notch)
                        .map(|n| n.width / d.unit)
                })
                .unwrap_or(0.),
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
                    center_grab: pointer.x
                        - self
                            .last_frame
                            .map(|f| f.center().x)
                            .unwrap_or(self.dock_center()),
                    detached: self.mode == Mode::Floating,
                });
                // Pointer capture does not change layout. Keep the measured frame visible.
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
        let before_mode = self.mode;
        let before_inset = self.top_inset();
        let anchor = Point {
            x: pointer.x - drag.grab.x,
            y: pointer.y - drag.grab.y,
        };
        self.anchor = Some(anchor);
        if let Some(d) = display_for(pointer, &self.displays) {
            if d.id != self.display_id {
                self.display_id = d.id.clone();
                self.revision += 1;
            }
        }
        let target =
            dock_target(pointer, &self.displays, self.candidate.as_deref()).map(|d| d.id.clone());
        if !drag.detached && target.as_deref() == Some(self.display_id.as_str()) {
            self.mode = Mode::Docked;
            self.place_dock(pointer.x - drag.center_grab);
            self.candidate = None;
        } else {
            self.drag.as_mut().unwrap().detached = true;
            self.mode = Mode::Floating;
            self.candidate = target;
        }
        if !pressed {
            if let Some(id) = self.candidate.take() {
                self.display_id = id;
                self.place_dock(pointer.x);
                self.mode = Mode::Docked;
            }
            self.drag = None;
        }
        if before_mode != self.mode || before_inset != self.top_inset() {
            self.revision += 1;
        }
        !pressed
    }

    fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            self.mode = drag.origin.mode;
            self.dock_x_ratio = drag.origin.dock_x_ratio.unwrap_or(0.5);
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
    fn layout_frame(&self, layout: Layout) -> Option<Rect> {
        let d = self.display()?;
        let anchor = self.anchor?;
        Some(if self.mode == Mode::Docked && d.notch.is_some() {
            dock_frame(d, layout, self.dock_center())
        } else {
            let f = frame_at(anchor, layout, d.unit);
            if self.drag.is_some() {
                f
            } else {
                clamp(f, d.work)
            }
        })
    }
    fn apply(&mut self, app: &AppHandle) -> Result<(), String> {
        if self.hide_docked_panel() {
            platform::show(app, false)?;
            return Ok(());
        }
        let (Some(layout), Some(d)) = (self.layout, self.display()) else {
            return Ok(());
        };
        let Some(frame) = self.layout_frame(layout) else {
            return Ok(());
        };
        let anchor = anchor_in(frame, layout, d.unit);
        platform::apply(app, frame, self.mode == Mode::Docked || self.drag.is_some())?;
        self.anchor = Some(anchor);
        self.last_frame = Some(frame);
        platform::show(app, true)?;
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
    let old_inset = s.top_inset();
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
    if old_inset != s.top_inset() {
        s.revision += 1;
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
pub async fn pet_surface_plan_layout(
    app: AppHandle,
    layout: Layout,
) -> Result<serde_json::Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<serde_json::Value, String> {
            let env = platform::read(&handle)?;
            let state = handle.state::<SurfaceState>();
            let s = state.0.lock().unwrap();
            let d = s.display().ok_or("display unavailable")?;
            if s.mode != Mode::Docked || !layout.valid() || layout.revision != s.revision {
                return Err("stale top panel transition".into());
            }
            let available = s.view();
            if layout.width > available.available_width + 1.
                || layout.height > available.available_height + 1.
            {
                return Err("pet layout exceeds current display".into());
            }
            let target = dock_frame(d, layout, s.dock_center());
            let css = |r: Rect| Rect { x: r.x / d.unit, y: r.y / d.unit, width: r.width / d.unit, height: r.height / d.unit };
            Ok(serde_json::json!({"from":css(env.frame), "to":css(target), "topInset":dock_inset(d, s.dock_center(), layout.width * d.unit) / d.unit}))
        })();
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn pet_surface_begin_frame(app: AppHandle) -> Result<u64, String> {
    platform::begin_frame(app).await
}
#[tauri::command]
pub async fn pet_surface_end_frame(app: AppHandle, token: u64, abort: bool) -> Result<(), String> {
    platform::end_frame(app, token, abort).await
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
mod tests;

/// Called only by the platform event adapter, on the main thread.
pub fn native_hit(app: &AppHandle, point: Point) -> bool {
    let state = app.state::<SurfaceState>();
    let s = state.0.lock().unwrap();
    s.drag_hit(point)
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

pub fn should_show_window(app: &AppHandle) -> bool {
    let state = app.state::<SurfaceState>();
    let s = state.0.lock().unwrap();
    !s.hide_docked_panel()
}
