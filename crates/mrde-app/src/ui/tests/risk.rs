use super::*;
#[gpui::test]
fn changes_renders_partial_risk_and_file_history_only_on_click(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor::default());
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    let cx = &mut visual;
    cx.update(|_, cx| {
        view.update(cx, |state, _| {
            state.apply_preview_scenario_named("focus:changes")
        })
    });
    draw(&root, cx);
    assert!(cx.debug_bounds("risk-radar").is_some());
    assert!(executor.actions.lock().unwrap().is_empty());
    click("risk-axis-lint", cx);
    draw(&root, cx);
    assert!(cx.debug_bounds("risk-axis-evidence").is_some());
    click("calculate-risk", cx);
    finish(&view, &root, cx);
    assert_eq!(executor.actions.lock().unwrap().as_slice(), ["risk"]);
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.select_risk_file("src/schedule.rs".into(), cx)
        })
    });
    finish(&view, &root, cx);
    assert!(cx.debug_bounds("risk-file-history").is_some());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).risk_file.as_ref().unwrap().2.authors.len()),
        2
    );
    assert!(
        executor
            .actions
            .lock()
            .unwrap()
            .iter()
            .all(|a| !a.starts_with("ask"))
    );
}
