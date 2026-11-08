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
    anchor: Point,
    displays: &'a [Display],
    previous: Option<&str>,
) -> Option<&'a Display> {
    displays.iter().find(|d| {
        d.notch.is_some_and(|n| {
            let margin = if previous == Some(d.id.as_str()) {
                50.
            } else {
                28.
            };
            // The camera is not interactive. Include the accessible strip below it.
            Rect {
                height: n.height + 54. * d.unit,
                ..n
            }
            .expanded(margin * d.unit, 8. * d.unit)
            .contains(anchor)
        })
    })
}
pub fn dock_frame(display: &Display, layout: Layout) -> Rect {
    let notch = display.notch.expect("dock requires a top docking target");
    Rect {
        x: (notch.center().x - layout.width * display.unit / 2.).clamp(
            display.frame.x,
            (display.frame.right() - layout.width * display.unit).max(display.frame.x),
        ),
        y: notch.bottom(),
        width: layout.width * display.unit,
        height: layout.height * display.unit,
    }
}

/// Anchor directly below our own status item; other applications never affect placement.
pub fn menu_area(display: &Display, menu: Rect) -> Rect {
    let top = menu.bottom();
    Rect {
        y: top,
        height: (display.work.bottom() - top).max(0.),
        ..display.work
    }
}

pub fn menu_frame(area: Rect, menu: Rect, layout: Layout, unit: f64) -> Rect {
    let width = (layout.width * unit).min(area.width);
    Rect {
        x: (menu.center().x - width / 2.).clamp(area.x, (area.right() - width).max(area.x)),
        y: area.y,
        width,
        height: (layout.height * unit).min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn display() -> Display {
        Display {
            id: "internal".into(),
            frame: Rect {
                x: -1500.,
                y: -1000.,
                width: 1500.,
                height: 1000.,
            },
            work: Rect {
                x: -1500.,
                y: -960.,
                width: 1500.,
                height: 920.,
            },
            notch: Some(Rect {
                x: -840.,
                y: -1000.,
                width: 180.,
                height: 40.,
            }),
            physical_notch: true,
            unit: 1.,
        }
    }
    fn layout() -> Layout {
        Layout {
            width: 320.,
            height: 140.,
            anchor: Point { x: 220., y: 80. },
            pet_width: 86.,
            pet_height: 94.,
            revision: 0,
        }
    }
    #[test]
    fn resize_preserves_character_not_window_corner() {
        let a = Point { x: -600., y: -400. };
        let old = layout();
        let next = Layout {
            height: 340.,
            anchor: Point { x: 160., y: 280. },
            ..old
        };
        assert_eq!(anchor_in(frame_at(a, next, 1.), next, 1.), a);
        assert_ne!(frame_at(a, old, 1.).x, frame_at(a, next, 1.).x);
    }
    #[test]
    fn notch_accepts_accessible_lower_strip_and_has_hysteresis() {
        let ds = [display()];
        assert!(dock_target(Point { x: -750., y: -915. }, &ds, None).is_some());
        assert!(dock_target(Point { x: -705., y: -800. }, &ds, None).is_none());
        assert!(dock_target(Point { x: -630., y: -950. }, &ds, None).is_none());
        assert!(dock_target(Point { x: -630., y: -950. }, &ds, Some("internal")).is_some());
    }
    #[test]
    fn dock_uses_screen_not_work_area_and_content_height() {
        let d = display();
        let frame = dock_frame(&d, layout());
        assert_eq!(frame.y, -960.);
        assert_eq!(frame.height, 140.);
        assert_eq!(frame.center().x, -750.);
        // Header portraits sit at the left: their position must not shift the whole panel.
        let wider = Layout {
            width: 400.,
            anchor: Point { x: 26., y: 22. },
            ..layout()
        };
        assert_eq!(dock_frame(&display(), wider).center().x, -750.);
    }
    #[test]
    fn fallback_dock_does_not_overlay_the_menu_bar() {
        let mut d = display();
        d.physical_notch = false;
        let compact = Layout {
            height: 22.,
            pet_height: 18.,
            ..layout()
        };
        assert_eq!(dock_frame(&d, compact).y, d.notch.unwrap().bottom());
        d.unit = 2.;
        assert_eq!(dock_frame(&d, compact).y, d.notch.unwrap().bottom());
    }
    #[test]
    fn negative_displays_and_scale_are_supported() {
        let mut d = display();
        d.unit = 1.25;
        let f = clamp(
            frame_at(
                Point {
                    x: -1600.,
                    y: -1100.,
                },
                layout(),
                d.unit,
            ),
            d.work,
        );
        assert_eq!(f.x, -1500.);
        assert_eq!(f.y, -960.);
        assert_eq!(f.width, 400.);
        assert_eq!(
            display_for(Point { x: -2000., y: 0. }, &[d]).unwrap().id,
            "internal"
        );
    }
    #[test]
    fn rejects_invalid_measurements() {
        // A screen-relative panel on a large logical display can exceed 1200px.
        assert!(Layout {
            width: 1690.,
            height: 1296.,
            ..layout()
        }
        .valid());
        assert!(!Layout {
            height: f64::NAN,
            ..layout()
        }
        .valid());
        assert!(!Layout {
            anchor: Point { x: 999., y: 0. },
            ..layout()
        }
        .valid());
    }
    #[test]
    fn menu_panel_touches_status_item_and_caps_height_at_screen_bottom() {
        let d = display();
        let menu = Rect {
            x: -850.,
            y: -1000.,
            width: 110.,
            height: 40.,
        };
        let frame = menu_frame(
            menu_area(&d, menu),
            menu,
            Layout {
                height: 2000.,
                ..layout()
            },
            d.unit,
        );
        assert_eq!(frame.y, menu.bottom());
        assert_eq!(frame.center().x, menu.center().x);
        assert_eq!(frame.bottom(), d.work.bottom());
        // Even a different work-area top must not introduce a gap below the item.
        let shifted_work = Display {
            work: Rect {
                y: -900.,
                height: 860.,
                ..d.work
            },
            ..d
        };
        assert_eq!(menu_area(&shifted_work, menu).y, menu.bottom());
    }
    #[test]
    fn menu_panel_at_screen_edge_keeps_actions_visible_on_scaled_negative_display() {
        let mut d = display();
        d.unit = 1.25;
        let menu = Rect {
            x: -60.,
            y: -1000.,
            width: 50.,
            height: 30.,
        };
        let area = menu_area(&d, menu);
        let frame = menu_frame(area, menu, layout(), d.unit);
        assert!(frame.x >= d.work.x && frame.right() <= d.work.right());
        assert_eq!(frame.width, 400.);
        assert_eq!(frame.y, menu.bottom());
    }
}
