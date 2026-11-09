use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn right(self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(self) -> f64 {
        self.y + self.height
    }
    pub fn center(self) -> Point {
        Point {
            x: self.x + self.width / 2.,
            y: self.y + self.height / 2.,
        }
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn expanded(self, x: f64, y: f64) -> Self {
        Self {
            x: self.x - x,
            y: self.y - y,
            width: self.width + 2. * x,
            height: self.height + 2. * y,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub id: String,
    pub frame: Rect,
    pub work: Rect,
    pub notch: Option<Rect>,
    pub physical_notch: bool,
    /// Native coordinate units per CSS pixel. macOS backend uses AppKit points.
    pub unit: f64,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub width: f64,
    pub height: f64,
    pub anchor: Point,
    pub pet_width: f64,
    pub pet_height: f64,
    pub revision: u64,
    #[serde(default)]
    pub drag_region: Option<Rect>,
}
impl Layout {
    pub fn valid(self) -> bool {
        [self.width, self.height, self.anchor.x, self.anchor.y]
            .iter()
            .all(|x| x.is_finite())
            && self.width >= 40.
            // Numeric sanity only; the owner checks against the current display.
            && self.width <= 32768.
            && self.height >= 20.
            && self.height <= 32768.
            && self.pet_width > 0.
            && self.pet_width <= self.width
            && self.pet_height > 0.
            && self.pet_height <= self.height
            && self.anchor.x >= 0.
            && self.anchor.x <= self.width
            && self.anchor.y >= 0.
            && self.anchor.y <= self.height
            && self.drag_region.is_none_or(|r| {
                [r.x, r.y, r.width, r.height].iter().all(|n| n.is_finite())
                    && r.x >= 0. && r.y >= 0. && r.width > 0. && r.height > 0.
                    && r.right() <= self.width && r.bottom() <= self.height
            })
    }
}
pub fn frame_at(anchor: Point, layout: Layout, unit: f64) -> Rect {
    Rect {
        x: anchor.x - layout.anchor.x * unit,
        y: anchor.y - layout.anchor.y * unit,
        width: layout.width * unit,
        height: layout.height * unit,
    }
}
pub fn anchor_in(frame: Rect, layout: Layout, unit: f64) -> Point {
    Point {
        x: frame.x + layout.anchor.x * unit,
        y: frame.y + layout.anchor.y * unit,
    }
}
pub fn clamp(frame: Rect, area: Rect) -> Rect {
    Rect {
        x: frame
            .x
            .clamp(area.x, (area.right() - frame.width).max(area.x)),
        y: frame
            .y
            .clamp(area.y, (area.bottom() - frame.height).max(area.y)),
        ..frame
    }
}
pub fn display_for(point: Point, displays: &[Display]) -> Option<&Display> {
    displays
        .iter()
        .find(|d| d.frame.contains(point))
        .or_else(|| {
            displays.iter().min_by(|a, b| {
                let distance = |d: &Display| {
                    let c = d.frame.center();
                    (c.x - point.x).powi(2) + (c.y - point.y).powi(2)
                };
                distance(a).total_cmp(&distance(b))
            })
        })
}
pub fn dock_target<'a>(
    pointer: Point,
    displays: &'a [Display],
    previous: Option<&str>,
) -> Option<&'a Display> {
    displays.iter().find(|d| {
        d.notch.is_some()
            && Rect {
                x: d.frame.x,
                y: d.frame.y - 8. * d.unit,
                width: d.frame.width,
                height: d
                    .notch
                    .filter(|_| d.physical_notch)
                    .map(|n| n.height)
                    .unwrap_or(0.)
                    + (if previous == Some(d.id.as_str()) {
                        72.
                    } else {
                        54.
                    }) * d.unit,
            }
            .contains(pointer)
    })
}
pub fn dock_frame(display: &Display, layout: Layout, center: f64) -> Rect {
    let width = layout.width * display.unit;
    Rect {
        x: (center - width / 2.).clamp(
            display.frame.x,
            (display.frame.right() - width).max(display.frame.x),
        ),
        y: display.frame.y,
        width,
        height: layout.height * display.unit,
    }
}
/// The black surface starts at the screen edge; only content is inset below hardware.
/// Use the clamped window span, so expansion near a notch also keeps text safe.
pub fn dock_inset(display: &Display, center: f64, width: f64) -> f64 {
    let x = (center - width / 2.).clamp(
        display.frame.x,
        (display.frame.right() - width).max(display.frame.x),
    );
    display
        .notch
        .filter(|_| display.physical_notch)
        .filter(|n| x < n.right() && x + width > n.x)
        .map(|n| n.bottom() - display.frame.y)
        .unwrap_or(0.)
}
