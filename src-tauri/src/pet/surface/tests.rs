use super::*;
fn surface() -> Surface {
    Surface {
        initialized: true,
        revision: 1,
        mode: Mode::Docked,
        display_id: "screen".into(),
        dock_x_ratio: 0.5,
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
fn layout(s: &Surface, width: f64, height: f64) -> Layout {
    Layout {
        revision: s.revision,
        width,
        height,
        anchor: Point { x: 30., y: 12. },
        pet_width: 30.,
        pet_height: 20.,
        drag_region: None,
    }
}
#[test]
fn reported_notch_is_one_surface_from_screen_edge_with_content_clearance() {
    let mut s = surface();
    s.displays[0] = Display {
        id: "screen".into(),
        frame: Rect {
            x: 0.,
            y: -982.,
            width: 1512.,
            height: 982.,
        },
        work: Rect {
            x: 0.,
            y: -949.,
            width: 1512.,
            height: 949.,
        },
        notch: Some(Rect {
            x: 663.,
            y: -982.,
            width: 185.,
            height: 32.,
        }),
        physical_notch: true,
        unit: 1.,
    };
    s.place_dock(755.5);
    for (width, height) in [(200., 56.), (440., 340.)] {
        s.layout = Some(layout(&s, width, height));
        let frame = s.layout_frame(s.layout.unwrap()).unwrap();
        assert_eq!(frame.y, -982.);
        assert_eq!(frame.center().x, 755.5);
        assert_eq!(s.view().top_inset, 32.);
        assert!(!s.hide_docked_panel());
    }
    s.place_dock(200.);
    s.layout = Some(layout(&s, 200., 24.));
    assert_eq!(s.view().top_inset, 0.);
    assert_eq!(s.layout_frame(s.layout.unwrap()).unwrap().y, -982.);
}
#[test]
fn no_notch_ignores_menu_work_area_and_collapsed_strip_remains_visible() {
    let mut s = surface();
    s.displays[0].physical_notch = false;
    s.layout = Some(layout(&s, 200., 24.));
    assert_eq!(s.view().top_inset, 0.);
    assert!(!s.hide_docked_panel());
    assert_eq!(s.layout_frame(s.layout.unwrap()).unwrap().y, 0.);
    s.revision += 1;
    assert!(s.hide_docked_panel());
}
#[test]
fn docked_drag_region_covers_header_but_not_task_list_or_desktop() {
    let mut s = surface();
    let mut l = layout(&s, 500., 350.);
    l.drag_region = Some(Rect {
        x: 6.,
        y: 0.,
        width: 488.,
        height: 50.,
    });
    assert!(l.valid());
    s.layout = Some(l);
    s.last_frame = Some(Rect {
        x: -800.,
        y: -1080.,
        width: 500.,
        height: 350.,
    });
    for x in [-790., -550., -315.] {
        assert!(s.drag_hit(Point { x, y: -1050. }));
    }
    assert!(!s.drag_hit(Point {
        x: -550.,
        y: -1000.
    }));
    assert!(!s.drag_hit(Point {
        x: -799.,
        y: -1050.
    }));
    s.mode = Mode::Floating;
    assert!(!s.drag_hit(Point {
        x: -550.,
        y: -1050.
    }));
    assert!(s.drag_hit(s.anchor.unwrap()));
    l.drag_region.as_mut().unwrap().height = 400.;
    assert!(!l.valid());
    l.drag_region.as_mut().unwrap().height = f64::NAN;
    assert!(!l.valid());
}
#[test]
fn horizontal_drag_keeps_current_layout_visible_without_a_resize_handshake() {
    for (width, height) in [(200., 26.), (500., 350.)] {
        let mut s = surface();
        s.displays[0].physical_notch = false;
        s.layout = Some(layout(&s, width, height));
        s.last_frame = s.layout_frame(s.layout.unwrap());
        let revision = s.revision;
        s.begin_drag(Point { x: 500., y: 12. });
        assert_eq!(s.revision, revision);
        assert!(!s.hide_docked_panel());
        s.move_drag(Point { x: 560., y: 12. }, true, false);
        assert!(!s.hide_docked_panel());
        s.move_drag(Point { x: 560., y: 12. }, false, false);
        assert_eq!(s.revision, revision);
        assert!(!s.hide_docked_panel());
        assert_eq!(s.layout_frame(s.layout.unwrap()).unwrap().height, height);
    }
}
#[test]
fn horizontal_drag_stays_docked_and_persists_position_on_release() {
    let mut s = surface();
    s.layout = Some(layout(&s, 200., 64.));
    s.begin_drag(Point { x: 500., y: 60. });
    s.move_drag(Point { x: 180., y: 60. }, true, false);
    assert_eq!(s.mode, Mode::Docked);
    assert_eq!(s.dock_center(), 180.);
    assert_eq!(s.view().top_inset, 0.);
    s.move_drag(Point { x: 180., y: 20. }, false, false);
    assert_eq!(s.saved().dock_x_ratio, Some(0.18));
    assert!(s.drag.is_none());
}
#[test]
fn resize_and_expansion_keep_saved_horizontal_center() {
    let mut s = surface();
    s.dock_x_ratio = 0.27;
    for width in [200., 440.] {
        assert_eq!(
            s.layout_frame(layout(&s, width, 100.)).unwrap().center().x,
            270.
        );
    }
    assert_eq!(s.saved().dock_x_ratio, Some(0.27));
}
#[test]
fn expanded_panel_near_notch_gets_clearance_and_edges_stay_on_screen() {
    let mut s = surface();
    s.dock_x_ratio = 0.25;
    s.layout = Some(layout(&s, 200., 24.));
    assert_eq!(s.top_inset(), 0.);
    s.layout = Some(layout(&s, 440., 300.));
    assert_eq!(s.top_inset(), 40.);
    s.dock_x_ratio = 1.;
    assert_eq!(s.layout_frame(s.layout.unwrap()).unwrap().right(), 1000.);
}
#[test]
fn dragging_down_detaches_and_keeps_character_grab_offset() {
    let mut s = surface();
    s.begin_drag(Point { x: 510., y: 95. });
    s.move_drag(Point { x: 310., y: 405. }, true, false);
    assert_eq!(s.mode, Mode::Floating);
    assert_eq!(s.anchor, Some(Point { x: 300., y: 400. }));
    s.move_drag(Point { x: 310., y: 405. }, false, false);
    assert!(s.drag.is_none());
    assert_eq!(s.mode, Mode::Floating);
}
#[test]
fn floating_drag_can_redock_anywhere_across_screen_top_only_on_release() {
    let mut s = surface();
    s.mode = Mode::Floating;
    s.begin_drag(Point { x: 500., y: 400. });
    s.move_drag(Point { x: 820., y: 20. }, true, false);
    assert_eq!(s.mode, Mode::Floating);
    assert!(s.candidate.is_some());
    s.move_drag(Point { x: 820., y: 20. }, false, false);
    assert_eq!(s.mode, Mode::Docked);
    assert_eq!(s.dock_center(), 820.);
}
#[test]
fn redocking_near_notch_snaps_to_its_center() {
    let mut s = surface();
    s.mode = Mode::Floating;
    s.begin_drag(Point { x: 500., y: 400. });
    s.move_drag(Point { x: 550., y: 70. }, false, false);
    assert_eq!(s.dock_center(), 500.);
    assert_eq!(s.mode, Mode::Docked);
}
#[test]
fn escape_restores_mode_anchor_and_dock_preference() {
    let mut s = surface();
    s.dock_x_ratio = 0.3;
    s.begin_drag(Point { x: 510., y: 95. });
    s.move_drag(Point { x: 100., y: 600. }, true, false);
    s.move_drag(Point { x: 100., y: 600. }, true, true);
    assert_eq!(s.mode, Mode::Docked);
    assert_eq!(s.dock_x_ratio, 0.3);
    assert_eq!(s.anchor, Some(Point { x: 500., y: 90. }));
}
#[test]
fn cross_screen_drag_updates_clearance_and_cancel_returns_to_original_screen() {
    let mut s = surface();
    let mut external = s.displays[0].clone();
    external.id = "external".into();
    external.frame.x = -1000.;
    external.work.x = -1000.;
    external.physical_notch = false;
    s.displays.push(external);
    s.begin_drag(Point { x: 500., y: 50. });
    s.move_drag(Point { x: -700., y: 20. }, true, false);
    assert_eq!(s.display_id, "external");
    assert_eq!(s.top_inset(), 0.);
    assert_eq!(s.mode, Mode::Docked);
    s.cancel_drag();
    assert_eq!(s.display_id, "screen");
    assert_eq!(s.dock_x_ratio, 0.5);
}
#[test]
fn old_saved_preferences_migrate_to_center_and_new_position_roundtrips() {
    let saved: Saved =
        serde_json::from_str(r#"{"mode":"docked","displayId":"1","xRatio":0.9,"yRatio":0.0}"#)
            .unwrap();
    assert_eq!(saved.dock_x_ratio.unwrap_or(0.5), 0.5);
    let mut s = surface();
    s.dock_x_ratio = 0.7;
    let dir = tempfile::tempdir().unwrap();
    s.path = Some(dir.path().join("config/pet-surface.json"));
    s.persist().unwrap();
    let saved: Saved = serde_json::from_slice(&std::fs::read(s.path.unwrap()).unwrap()).unwrap();
    assert_eq!(saved.dock_x_ratio, Some(0.7));
}
#[test]
fn negative_coordinates_and_mixed_dpi_keep_clearance_in_css_units() {
    let mut s = surface();
    let d = &mut s.displays[0];
    d.frame.y = -800.;
    d.work.y = -760.;
    d.notch.as_mut().unwrap().y = -800.;
    d.unit = 2.;
    s.layout = Some(layout(&s, 200., 50.));
    assert_eq!(s.top_inset(), 20.);
    assert_eq!(s.view().notch_width, 90.);
    let f = s.layout_frame(s.layout.unwrap()).unwrap();
    assert_eq!(f.width, 400.);
    assert_eq!(f.y, -800.);
}
#[test]
fn resizing_floating_content_keeps_anchor_and_rejects_invalid_layouts() {
    let a = Point { x: 500., y: 400. };
    let s = surface();
    let l = layout(&s, 320., 340.);
    assert_eq!(anchor_in(frame_at(a, l, 1.25), l, 1.25), a);
    assert!(!Layout {
        height: f64::NAN,
        ..l
    }
    .valid());
    assert!(!Layout {
        anchor: Point { x: 400., y: 20. },
        ..l
    }
    .valid());
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
