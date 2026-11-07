use super::*;

#[gpui::test]
fn debug_logs_copy_exact_plain_text_and_include_new_lines(cx: &mut TestAppContext) {
    let (view, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    let first = "12:00:00  Tool output: <path> & `value`\n{\"diagnostic\": 1}";
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.logs_open = true;
            state.logs = [first.into()].into();
            cx.notify();
        })
    });
    draw(&root, cx);
    click("copy-debug-logs", cx);
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), first);
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            assert!(state.logs_copied);
            state.log("Bun dependencies are ready.");
            assert!(!state.logs_copied);
            cx.notify();
        })
    });
    draw(&root, cx);
    click("copy-debug-logs", cx);
    let copied = cx.read_from_clipboard().unwrap().text().unwrap();
    assert!(copied.starts_with(first));
    assert!(copied.ends_with("Bun dependencies are ready."));
    assert!(!copied.contains("&lt;"));
}

#[gpui::test]
fn debug_log_text_can_be_selected_and_copied_with_the_keyboard(cx: &mut TestAppContext) {
    let (view, root, mut visual) = setup(cx, Arc::new(ControlledSettings::new(false)));
    let cx = &mut visual;
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.logs_open = true;
            state.logs = ["12:00:00  Literal <error> & details\n    Retry setup.".into()].into();
            cx.notify();
        })
    });
    draw(&root, cx);
    let bounds = cx.debug_bounds("debug-log-text").unwrap();
    let start = bounds.origin + point(px(1.), px(2.));
    let end = point(bounds.right() - px(1.), bounds.bottom() - px(1.));
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::none());
    draw(&root, cx);
    cx.simulate_mouse_move(end, gpui::MouseButton::Left, Modifiers::none());
    draw(&root, cx);
    cx.simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::none());
    draw(&root, cx);
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap().trim(),
        "12:00:00  Literal <error> & details\n    Retry setup."
    );
}
