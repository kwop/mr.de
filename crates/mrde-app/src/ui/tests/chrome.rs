//! Title bar search, label filters, "Open MR…", and the review context panel.
use super::*;
use crate::ui::context::ContextLayout;
use gpui_component::Root;

fn labelled(iid: u64, title: &str, labels: &[&str]) -> MergeRequest {
    let mut mr = queue_fixture(iid, "2026-11-06T12:00:00Z");
    mr.title = title.into();
    mr.labels = labels.iter().map(|&name| Label::from(name)).collect();
    mr
}
fn draw_at(root: &Entity<Root>, cx: &mut VisualTestContext, width: f32, height: f32) {
    cx.simulate_resize(size(px(width), px(height)));
    for _ in 0..4 {
        cx.draw(
            point(px(0.), px(0.)),
            size(px(width), px(height)),
            |_, _| root.clone(),
        );
        cx.run_until_parked();
    }
}
fn visible(view: &Entity<Cockpit>, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        let state = view.read(cx);
        state
            .visible_queue()
            .iter()
            .map(|&index| state.queue[index].id.iid)
            .collect()
    })
}
fn layout(view: &Entity<Cockpit>, cx: &mut VisualTestContext) -> ContextLayout {
    cx.update(|window, cx| view.read(cx).context_layout(window.viewport_size().width))
}
fn open_iid(view: &Entity<Cockpit>, cx: &mut VisualTestContext) -> Option<u64> {
    cx.update(|_, cx| view.read(cx).detail.as_ref().map(|d| d.mr.id.iid))
}

#[gpui::test]
fn search_and_label_filters_narrow_the_queue_and_adjacent_navigation(cx: &mut TestAppContext) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![
            labelled(1, "Create recurring invoices", &["P::1", "billing"]),
            labelled(2, "Cancel an invoice", &["backend"]),
            labelled(3, "Calendar: drag and drop", &["billing"]),
        ],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        quiet_config(""),
    );
    let cx = &mut visual;
    click("search", cx);
    cx.simulate_input("INVOICE");
    draw(&root, cx);
    assert_eq!(visible(&view, cx), [1, 2], "search ignores case");
    click("label-filter-billing", cx);
    draw(&root, cx);
    assert_eq!(visible(&view, cx), [1], "search and labels combine");
    assert!(cx.debug_bounds("queue-count").is_some());
    click("clear-filters", cx);
    draw(&root, cx);
    assert_eq!(visible(&view, cx), [1, 2, 3]);
    assert!(cx.update(|_, cx| view.read(cx).search.read(cx).value().is_empty()));

    click("label-filter-billing", cx);
    draw(&root, cx);
    click("mr-1-1", cx);
    finish(&view, &root, cx);
    click("next-mr", cx);
    finish(&view, &root, cx);
    assert_eq!(
        open_iid(&view, cx),
        Some(3),
        "next skips the filtered-out MR"
    );
    assert_eq!(actions(&executor), ["select 1", "select 3"]);

    // ⌘K focuses the search from anywhere, here the settings page.
    click("nav-settings", cx);
    draw(&root, cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("calendar");
    draw(&root, cx);
    assert!(
        cx.update(|_, cx| !view.read(cx).settings),
        "search returns to the inbox"
    );
    assert_eq!(visible(&view, cx), [3]);
}

#[gpui::test]
fn review_context_docks_on_wide_windows_collapses_to_a_rail_and_floats_when_narrow(
    cx: &mut TestAppContext,
) {
    let executor = Arc::new(FakeExecutor {
        queue: vec![selected_mr("A short description.")],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor,
        quiet_config(""),
    );
    let cx = &mut visual;
    draw_at(&root, cx, 1440., 900.);
    assert_eq!(layout(&view, cx), ContextLayout::Hidden, "no MR, no panel");
    assert!(cx.debug_bounds("context-panel").is_none());
    click("mr-1-2", cx);
    finish(&view, &root, cx);
    draw_at(&root, cx, 1440., 900.);
    assert_eq!(layout(&view, cx), ContextLayout::Docked);
    let panel = cx.debug_bounds("context-panel").unwrap();
    let docked_reader = cx.debug_bounds("reading-pane").unwrap();
    assert!(
        docked_reader.right() <= panel.origin.x,
        "docked beside the reader"
    );
    assert!(panel.right() <= px(1440.));
    for section in [
        "context-review",
        "context-checks",
        "context-analysis",
        "context-ticket",
        "context-approvals",
    ] {
        let bounds = cx
            .debug_bounds(section)
            .unwrap_or_else(|| panic!("missing {section}"));
        assert!(bounds.origin.x >= panel.origin.x && bounds.right() <= panel.right());
    }

    click("context-close", cx);
    draw_at(&root, cx, 1440., 900.);
    assert_eq!(layout(&view, cx), ContextLayout::Rail);
    let rail = cx.debug_bounds("context-rail").unwrap();
    let reader = cx.debug_bounds("reading-pane").unwrap();
    assert!(reader.right() <= rail.origin.x);
    assert!(reader.size.width > docked_reader.size.width + px(200.));

    cx.simulate_keystrokes("cmd-alt-b");
    draw_at(&root, cx, 1440., 900.);
    assert_eq!(
        layout(&view, cx),
        ContextLayout::Docked,
        "the shortcut reopens it"
    );

    // Narrow windows float the panel over the reader instead of squeezing it.
    draw_at(&root, cx, 1100., 720.);
    assert_eq!(layout(&view, cx), ContextLayout::Overlay);
    let reader = cx.debug_bounds("reading-pane").unwrap();
    let panel = cx.debug_bounds("context-panel").unwrap();
    assert!(
        reader.size.width >= px(850.),
        "reader keeps its width: {reader:?}"
    );
    assert!(panel.origin.x < reader.right() && panel.right() <= px(1100.));
    click("toggle-context", cx);
    draw_at(&root, cx, 1100., 720.);
    assert_eq!(
        layout(&view, cx),
        ContextLayout::Hidden,
        "no rail below its minimum width"
    );
}

#[gpui::test]
fn open_mr_dialog_validates_links_and_keeps_an_mr_outside_the_queue(cx: &mut TestAppContext) {
    let mut outside = selected_mr("Not in the review queue.");
    outside.id.iid = 9;
    outside.title = "Outside the queue".into();
    let listed = selected_mr("Listed.");
    let executor = Arc::new(FakeExecutor {
        queue: vec![listed.clone(), outside],
        ..Default::default()
    });
    let (view, root, mut visual) = setup_with(
        cx,
        Arc::new(ControlledSettings::new(false)),
        executor.clone(),
        quiet_config(""),
    );
    let cx = &mut visual;
    cx.update(|_, cx| {
        view.update(cx, |state, cx| {
            state.queue = vec![listed.clone()];
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-o");
    draw(&root, cx);
    assert!(cx.update(|_, cx| view.read(cx).open_dialog));
    cx.simulate_input("https://other.example.com/acme/platform/-/merge_requests/9");
    cx.simulate_keystrokes("enter");
    draw(&root, cx);
    let error = cx.update(|_, cx| view.read(cx).open_error.clone()).unwrap();
    assert!(error.contains("another GitLab instance"), "{error}");
    assert!(cx.debug_bounds("open-mr-error").is_some());
    assert!(
        actions(&executor).is_empty(),
        "nothing is sent for another host"
    );

    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("acme/platform!9");
    draw(&root, cx);
    click("open-mr-submit", cx);
    finish(&view, &root, cx);
    assert_eq!(actions(&executor), ["open acme/platform!9"]);
    assert_eq!(open_iid(&view, cx), Some(9));
    assert!(cx.update(|_, cx| !view.read(cx).open_dialog));

    // A refresh that does not list it keeps it open; a new commit closes it.
    let deliver = |queue: Vec<MergeRequest>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            view.update(cx, |state, cx| {
                state
                    .sender
                    .send((state.generation, Ok(Outcome::Queue(queue))))
                    .unwrap();
                state.poll(cx);
            })
        });
    };
    deliver(vec![listed.clone()], cx);
    assert_eq!(open_iid(&view, cx), Some(9));
    let mut moved = cx.update(|_, cx| view.read(cx).detail.as_ref().unwrap().mr.clone());
    moved.sha = "c".repeat(40);
    deliver(vec![listed, moved], cx);
    assert_eq!(open_iid(&view, cx), None);

    cx.simulate_keystrokes("cmd-o");
    draw(&root, cx);
    cx.simulate_keystrokes("escape");
    draw(&root, cx);
    assert!(
        cx.update(|_, cx| !view.read(cx).open_dialog),
        "Escape closes it"
    );
}
