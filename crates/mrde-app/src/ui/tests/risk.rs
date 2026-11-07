use super::*;
use gpui_component::Root;
use mrde_core::risk::Axis;

fn changes(
    cx: &mut TestAppContext,
) -> (
    Arc<FakeExecutor>,
    Entity<Cockpit>,
    Entity<Root>,
    VisualTestContext,
) {
    let executor = Arc::new(FakeExecutor::default());
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        queue_config(),
    );
    visual.update(|_, cx| {
        view.update(cx, |state, _| {
            state.apply_preview_scenario_named("focus:changes")
        })
    });
    (executor, view, root, visual)
}
/// Scrolls the reader so the control sits near its top, then clicks it.
fn reveal(
    selector: &'static str,
    view: &Entity<Cockpit>,
    root: &Entity<Root>,
    cx: &mut VisualTestContext,
) {
    let panel = cx.debug_bounds("mr-detail").unwrap();
    let control = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing control: {selector}"));
    cx.update(|_, cx| {
        let scroll = &view.read(cx).detail_scroll;
        let offset = scroll.offset();
        scroll.set_offset(point(
            offset.x,
            offset.y - (control.origin.y - panel.origin.y - px(16.)),
        ));
    });
    draw(root, cx);
    click(selector, cx);
}
fn actions(executor: &FakeExecutor, name: &str) -> usize {
    executor
        .actions
        .lock()
        .unwrap()
        .iter()
        .filter(|a| *a == name)
        .count()
}

#[gpui::test]
fn changes_renders_partial_risk_and_file_history_only_on_click(cx: &mut TestAppContext) {
    let (executor, view, root, mut visual) = changes(cx);
    let cx = &mut visual;
    draw(&root, cx);
    assert!(cx.debug_bounds("risk-radar").is_some());
    assert!(cx.debug_bounds("risk-axes").is_some());
    assert!(executor.actions.lock().unwrap().is_empty());
    click("risk-axis-lint", cx);
    draw(&root, cx);
    assert!(cx.debug_bounds("risk-axis-evidence").is_some());
    click("calculate-risk", cx);
    finish(&view, &root, cx);
    assert_eq!(executor.actions.lock().unwrap().as_slice(), ["risk"]);
    reveal("risk-file-0", &view, &root, cx);
    finish(&view, &root, cx);
    assert!(cx.debug_bounds("risk-file-history").is_some());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).risk_file.as_ref().unwrap().2.authors.len()),
        3
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

#[gpui::test]
fn axis_rows_toggle_their_evidence(cx: &mut TestAppContext) {
    let (_, view, root, mut visual) = changes(cx);
    let cx = &mut visual;
    draw(&root, cx);
    reveal("risk-row-test_gap", &view, &root, cx);
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).risk_axis),
        Some(Axis::TestGap)
    );
    reveal("risk-row-test_gap", &view, &root, cx);
    draw(&root, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).risk_axis), None);
}

#[gpui::test]
fn files_open_inline_load_once_and_close_on_a_second_click(cx: &mut TestAppContext) {
    let (executor, view, root, mut visual) = changes(cx);
    let cx = &mut visual;
    draw(&root, cx);
    reveal("risk-file-1", &view, &root, cx);
    finish(&view, &root, cx);
    let selected = cx.update(|_, cx| view.read(cx).risk_selected_file.clone());
    assert!(selected.is_some());
    assert!(cx.debug_bounds("risk-file-history").is_some());
    assert_eq!(actions(&executor, "risk-file"), 1);
    // The panel opens right under its row.
    let row = cx.debug_bounds("risk-file-1").unwrap();
    let panel = cx.debug_bounds("risk-file-history").unwrap();
    assert!(panel.top() >= row.bottom() && panel.top() - row.bottom() < px(20.));

    reveal("risk-file-1", &view, &root, cx);
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).risk_selected_file.clone()),
        None
    );
    reveal("risk-file-1", &view, &root, cx);
    draw(&root, cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).risk_selected_file.clone()),
        selected
    );
    assert_eq!(actions(&executor, "risk-file"), 1, "loaded once per commit");
}

#[gpui::test]
fn evidence_files_open_under_their_findings(cx: &mut TestAppContext) {
    let (executor, view, root, mut visual) = changes(cx);
    let cx = &mut visual;
    draw(&root, cx);
    reveal("risk-row-debt", &view, &root, cx);
    draw(&root, cx);
    reveal("risk-evidence-file-0", &view, &root, cx);
    finish(&view, &root, cx);
    assert!(cx.update(|_, cx| view.read(cx).risk_file_in_evidence));
    assert_eq!(actions(&executor, "risk-file"), 1);
    let evidence = cx.debug_bounds("risk-axis-evidence").unwrap();
    let panel = cx.debug_bounds("risk-file-history").unwrap();
    assert!(evidence.top() < panel.top() && panel.bottom() <= evidence.bottom());
}

#[gpui::test]
fn changes_prompt_before_the_first_calculation(cx: &mut TestAppContext) {
    let (executor, view, root, mut visual) = changes(cx);
    let cx = &mut visual;
    cx.update(|_, cx| view.update(cx, |state, _| state.risk = None));
    draw(&root, cx);
    assert!(cx.debug_bounds("risk-prompt").is_some());
    assert!(
        cx.debug_bounds("risk-file-0").is_some(),
        "files stay listed"
    );
    click("calculate-risk", cx);
    finish(&view, &root, cx);
    assert_eq!(actions(&executor, "risk"), 1);
    assert!(cx.debug_bounds("risk-summary").is_some());
}

#[gpui::test]
fn risk_tools_are_prepared_only_when_the_setup_control_is_clicked(cx: &mut TestAppContext) {
    let (executor, view, root, mut visual) = changes(cx);
    let cx = &mut visual;
    draw(&root, cx);
    assert_eq!(actions(&executor, "risk-setup"), 0);
    reveal("setup-risk-tools", &view, &root, cx);
    finish(&view, &root, cx);
    assert_eq!(actions(&executor, "risk-setup"), 1);
    assert!(cx.update(|_, cx| view.read(cx).current_risk().is_some()));
    assert!(
        executor
            .actions
            .lock()
            .unwrap()
            .iter()
            .all(|a| !a.starts_with("ask"))
    );
}
