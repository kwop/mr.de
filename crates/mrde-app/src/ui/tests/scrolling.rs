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
        draw(&root, cx);
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
    draw(&root, cx);
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
