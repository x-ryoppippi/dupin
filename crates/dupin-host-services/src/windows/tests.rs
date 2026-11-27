use super::*;

fn display(x: f64, y: f64, width: f64, height: f64) -> Display {
    Display {
        frame: Frame::new(x, y, width, height),
        visible_frame: Frame::new(x, y + 25.0, width, height - 25.0),
        scale: 2.0,
    }
}

fn window(frame: Frame) -> Window {
    Window {
        frame,
        app: App {
            name: "Notes".into(),
            bundle_id: Some("com.apple.Notes".into()),
        },
    }
}

fn service(focused: Option<Window>) -> Service<Fixture> {
    Service::new(Fixture::new(
        vec![
            display(0.0, 0.0, 1440.0, 900.0),
            display(1440.0, 0.0, 1920.0, 1080.0),
        ],
        focused,
    ))
}

#[test]
fn a_frame_on_any_display_moves_the_focused_window() {
    let windows = service(Some(window(Frame::new(10.0, 40.0, 300.0, 200.0))));
    let target = Frame::new(1440.0, 25.0, 960.0, 1055.0);
    assert_eq!(windows.set_frame(target), Ok(target));
    assert_eq!(windows.focused().unwrap().unwrap().frame, target);
    let straddling = Frame::new(1300.0, 100.0, 400.0, 300.0);
    assert_eq!(windows.set_frame(straddling), Ok(straddling));
}

#[test]
fn a_frame_off_every_display_or_out_of_bounds_is_refused() {
    let windows = service(Some(window(Frame::new(10.0, 40.0, 300.0, 200.0))));
    let off = Frame::new(-500.0, 0.0, 400.0, 300.0);
    assert!(matches!(windows.set_frame(off), Err(ServiceError::OutOfScope(_))));
    let touching = Frame::new(3360.0, 0.0, 100.0, 100.0);
    assert!(
        matches!(windows.set_frame(touching), Err(ServiceError::OutOfScope(_))),
        "sharing only an edge with a display isn't on it"
    );
    for frame in [
        Frame::new(f64::NAN, 0.0, 100.0, 100.0),
        Frame::new(0.0, 0.0, 0.0, 100.0),
        Frame::new(0.0, 0.0, 100.0, -1.0),
        Frame::new(0.0, 0.0, MAX_LENGTH + 1.0, 100.0),
        Frame::new(-MAX_COORDINATE - 1.0, 0.0, 100.0, 100.0),
        Frame::new(0.0, 0.0, f64::INFINITY, 100.0),
    ] {
        assert!(
            matches!(windows.set_frame(frame), Err(ServiceError::Invalid(_))),
            "{frame:?}"
        );
    }
    assert!(windows.backend.moves.lock().unwrap().is_empty());
}

#[test]
fn reading_and_moving_need_accessibility_but_displays_do_not() {
    let mut backend = Fixture::new(
        vec![display(0.0, 0.0, 1440.0, 900.0)],
        Some(window(Frame::new(10.0, 40.0, 300.0, 200.0))),
    );
    backend.access = OsAccess::NotDetermined;
    let windows = Service::new(backend);
    assert_eq!(windows.displays().unwrap().len(), 1);
    assert_eq!(
        windows.focused(),
        Err(ServiceError::NeedsPermission(OsPermission::Accessibility))
    );
    assert_eq!(
        windows.set_frame(Frame::new(0.0, 25.0, 720.0, 875.0)),
        Err(ServiceError::NeedsPermission(OsPermission::Accessibility))
    );
    assert!(windows.backend.moves.lock().unwrap().is_empty());
}

#[test]
fn no_focused_window_reads_as_none_and_moving_it_fails() {
    let windows = service(None);
    assert_eq!(windows.focused(), Ok(None));
    assert!(matches!(
        windows.set_frame(Frame::new(0.0, 25.0, 720.0, 875.0)),
        Err(ServiceError::Failed(_))
    ));
}

#[test]
fn what_the_backend_reports_is_bounded() {
    let mut displays: Vec<_> = (0..MAX_DISPLAYS + 4)
        .map(|index| display(index as f64 * 100.0, 0.0, 100.0, 100.0))
        .collect();
    displays[0].scale = f64::NAN;
    let long = window(Frame::new(10.0, 40.0, 300.0, 200.0));
    let long = Window {
        app: App {
            name: "a".repeat(MAX_NAME_BYTES + 10),
            bundle_id: Some("b".repeat(MAX_NAME_BYTES + 1)),
        },
        ..long
    };
    let windows = Service::new(Fixture::new(displays, Some(long)));
    let listed = windows.displays().unwrap();
    assert_eq!(listed.len(), MAX_DISPLAYS);
    assert_eq!(listed[0].frame.x, 100.0, "a display with a bad scale is left out");
    let focused = windows.focused().unwrap().unwrap();
    assert_eq!(focused.app.name.len(), MAX_NAME_BYTES);
    assert_eq!(focused.app.bundle_id, None);
}

#[test]
fn results_take_the_shape_guests_read() {
    let windows = service(Some(window(Frame::new(10.0, 40.0, 300.0, 200.0))));
    assert_eq!(
        serde_json::to_value(&windows.displays().unwrap()[0]).unwrap(),
        serde_json::json!({
            "frame": { "x": 0.0, "y": 0.0, "width": 1440.0, "height": 900.0 },
            "visibleFrame": { "x": 0.0, "y": 25.0, "width": 1440.0, "height": 875.0 },
            "scale": 2.0,
        })
    );
    assert_eq!(
        serde_json::to_value(windows.focused().unwrap()).unwrap(),
        serde_json::json!({
            "frame": { "x": 10.0, "y": 40.0, "width": 300.0, "height": 200.0 },
            "app": { "name": "Notes", "bundleId": "com.apple.Notes" },
        })
    );
}
