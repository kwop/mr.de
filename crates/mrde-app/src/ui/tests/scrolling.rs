use super::*;
#[gpui::test]
fn history_can_scroll_to_its_last_item(cx: &mut TestAppContext) {
    let (view, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    cx.update(|_, cx| {
        view.update(cx, |state, _| {
            state.apply_preview_scenario_named("focus:history")
        })
    });
    for (width, height, focused) in [
        (1100., 720., true),
        (1280., 900., true),
        (1100., 900., false),
    ] {
        let viewport = size(px(width), px(height));
        cx.simulate_resize(viewport);
        cx.update(|_, cx| view.update(cx, |state, _| state.reader_focused = focused));
        for _ in 0..4 {
            cx.draw(point(px(0.), px(0.)), viewport, |_, _| root.clone());
            cx.run_until_parked();
        }
        cx.update(|_, cx| view.read(cx).detail_scroll.scroll_to_bottom());
        cx.draw(point(px(0.), px(0.)), viewport, |_, _| root.clone());
        let panel = cx.debug_bounds("mr-detail").unwrap();
        let end = cx.debug_bounds("history-end").unwrap();
        assert!(
            end.bottom() <= panel.bottom(),
            "History footer {end:?} outside {panel:?} at {viewport:?}"
        );
        assert!(end.origin.y >= panel.origin.y);
    }
}
#[gpui::test]
fn image_fits_the_window_then_zooms_and_scrolls(cx: &mut TestAppContext) {
    let (view, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    open_portrait(&root, cx);
    let fitted = cx.debug_bounds("image-displayed").unwrap();
    assert!((fitted.size.width / fitted.size.height - 640. / 2400.).abs() < 0.002);
    let canvas = cx.debug_bounds("image-viewer-canvas").unwrap();
    let content = cx.debug_bounds("image-zoom-content").unwrap();
    assert!(content.size.height <= canvas.size.height + px(1.));
    click("image-zoom-in", cx);
    draw(&root, cx);
    let enlarged = cx.debug_bounds("image-zoom-content").unwrap();
    assert!(enlarged.size.height > canvas.size.height);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: canvas.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(-70.), px(-100.))),
        touch_phase: gpui::TouchPhase::Moved,
        ..Default::default()
    });
    draw(&root, cx);
    assert!(cx.debug_bounds("image-zoom-content").unwrap().origin.y < enlarged.origin.y);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: canvas.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-100000.))),
        touch_phase: gpui::TouchPhase::Moved,
        ..Default::default()
    });
    draw(&root, cx);
    assert!(cx.debug_bounds("image-displayed").unwrap().bottom() <= canvas.bottom());
    click("image-fit", cx);
    draw(&root, cx);
    assert!(
        cx.debug_bounds("image-zoom-content").unwrap().size.height <= canvas.size.height + px(1.)
    );
    click("close-image", cx);
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_none()));
}

fn open_portrait(root: &Entity<Root>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        crate::image_viewer::open(
            "https://example.invalid/portrait.svg".into(),
            "Image".into(),
            window,
            cx,
        )
    });
    let resource = gpui::Resource::Uri("https://example.invalid/portrait.svg".into());
    let mut loaded = false;
    for _ in 0..200 {
        draw(root, cx);
        if let Some(image) =
            cx.update(|window, cx| window.get_asset::<gpui::ImgResourceLoader>(&resource, cx))
        {
            image.expect("Portrait image should decode");
            loaded = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(loaded);
    draw(root, cx);
}

#[gpui::test]
fn image_click_zoom_keeps_the_focal_point_and_cycles_back_to_fit(cx: &mut TestAppContext) {
    let (_, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    open_portrait(&root, cx);
    let fitted = cx.debug_bounds("image-displayed").unwrap();
    let focal = point(
        fitted.center().x,
        fitted.origin.y + fitted.size.height * 0.4,
    );
    cx.simulate_mouse_move(focal, None, Modifiers::none());
    draw(&root, cx);
    assert!(
        cx.debug_bounds("image-zoom-cursor").is_some(),
        "Hover shows a magnifier"
    );
    for zoom in [2., 4., 8., 1.] {
        cx.simulate_click(focal, Modifiers::none());
        draw(&root, cx);
        let image = cx.debug_bounds("image-displayed").unwrap();
        assert!((image.size.height / fitted.size.height - zoom).abs() < 0.002);
        if zoom == 1. {
            assert!((image.origin.x - fitted.origin.x).abs() < px(1.));
            assert!((image.origin.y - fitted.origin.y).abs() < px(1.));
        } else {
            assert!(
                ((focal.y - image.origin.y) / image.size.height - 0.4).abs() < 0.002,
                "The image point clicked must stay under the cursor"
            );
        }
        if zoom == 8. {
            assert!(
                cx.debug_bounds("image-reset-cursor").is_some(),
                "The last level signals the return to fit"
            );
        }
    }
    // Slight hand jitter still counts as a click.
    cx.simulate_mouse_down(focal, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        focal + point(px(1.), px(1.)),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(
        focal + point(px(1.), px(1.)),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    draw(&root, cx);
    assert!(
        (cx.debug_bounds("image-displayed").unwrap().size.height / fitted.size.height - 2.).abs()
            < 0.002
    );
}

#[gpui::test]
fn dragging_a_zoomed_image_pans_without_zooming_and_stops_at_the_edges(cx: &mut TestAppContext) {
    let (view, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    open_portrait(&root, cx);
    let canvas = cx.debug_bounds("image-viewer-canvas").unwrap();
    let center = canvas.center();
    for _ in 0..3 {
        cx.simulate_click(center, Modifiers::none());
        draw(&root, cx);
    }
    let before = cx.debug_bounds("image-displayed").unwrap();
    let delta = point(px(-70.), px(-90.));
    drag_image(&root, cx, center, center + delta);
    let after = cx.debug_bounds("image-displayed").unwrap();
    assert_eq!(
        before.size, after.size,
        "Releasing a drag must not trigger another zoom"
    );
    assert!((after.origin.x - before.origin.x - delta.x).abs() < px(1.));
    assert!((after.origin.y - before.origin.y - delta.y).abs() < px(1.));
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: center,
        delta: gpui::ScrollDelta::Pixels(point(px(-100000.), px(-100000.))),
        touch_phase: gpui::TouchPhase::Moved,
        ..Default::default()
    });
    draw(&root, cx);
    let edge = cx.debug_bounds("image-displayed").unwrap();
    // GPUI applies a diagonal wheel event to its dominant axis. Drag farther than
    // the horizontal overflow to reach that edge as well.
    drag_image(
        &root,
        cx,
        center,
        point(canvas.origin.x + px(5.), center.y + delta.y),
    );
    let clamped = cx.debug_bounds("image-displayed").unwrap();
    assert_eq!(clamped.origin.y, edge.origin.y);
    assert!((clamped.right() - canvas.right() + px(12.)).abs() < px(1.));
    drag_image(&root, cx, center, center + delta);
    assert_eq!(
        cx.debug_bounds("image-displayed").unwrap(),
        clamped,
        "Dragging cannot expose blank space beyond the image padding"
    );
    drag_image(&root, cx, center, center - delta);
    let returned = cx.debug_bounds("image-displayed").unwrap();
    assert!((returned.origin.x - clamped.origin.x + delta.x).abs() < px(1.));
    assert!((returned.origin.y - clamped.origin.y + delta.y).abs() < px(1.));
    // Releasing outside the window must end the gesture without a click or stuck grab.
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(center + delta, gpui::MouseButton::Left, Modifiers::none());
    draw(&root, cx);
    cx.simulate_mouse_up(
        point(px(-10.), px(-10.)),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    draw(&root, cx);
    let released = cx.debug_bounds("image-displayed").unwrap();
    cx.simulate_mouse_move(center, None, Modifiers::none());
    draw(&root, cx);
    assert_eq!(released, cx.debug_bounds("image-displayed").unwrap());
    assert_eq!(released.size, before.size);
    cx.simulate_keystrokes("escape");
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).image_viewer.read(cx).source().is_none()));
    open_portrait(&root, cx);
    let reopened = cx.debug_bounds("image-displayed").unwrap();
    assert!(reopened.size.height <= canvas.size.height);
}

fn drag_image(
    root: &Entity<Root>,
    cx: &mut VisualTestContext,
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
) {
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::none());
    draw(root, cx);
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::none());
    draw(root, cx);
}
