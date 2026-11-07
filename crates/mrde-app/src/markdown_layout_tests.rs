//! Regression tests use actual GPUI layout, not Markdown source snapshots.
use super::*;
use gpui::{Context, Entity, Modifiers, Render, TestAppContext, VisualTestContext, point, size};
use gpui_component::Root;

struct DocumentFixture {
    source: String,
    diagrams: bool,
}
impl Render for DocumentFixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let document = if self.diagrams {
            view("fixture-markdown", &self.source, None, window, cx)
        } else {
            text_view("fixture-markdown", &self.source, None, window, cx)
                .code_block_actions(|_, _, _| {
                    div()
                        .debug_selector(|| "fixture-code-origin".into())
                        .size(px(1.))
                })
                .into_any_element()
        };
        div()
            .id("fixture-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .font_family(UI_FONT)
            .text_size(px(UI_SIZE))
            .line_height(px(22.))
            .child(
                div()
                    .debug_selector(|| "fixture-document".into())
                    .child(document),
            )
            .child(
                div()
                    .debug_selector(|| "fixture-tail".into())
                    .flex_shrink_0()
                    .h(px(24.))
                    .child("Next section"),
            )
    }
}
fn setup(
    cx: &mut TestAppContext,
    source: String,
    diagrams: bool,
) -> (Entity<Root>, VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::theme::init(cx);
    });
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|_| DocumentFixture { source, diagrams });
        Root::new(view, window, cx)
    });
    let root = window.root(cx).unwrap();
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(360.), px(220.)));
    visual.draw(point(px(0.), px(0.)), size(px(360.), px(220.)), |_, _| {
        root.clone()
    });
    (root, visual)
}

#[gpui::test]
fn fenced_code_has_space_from_adjacent_prose(cx: &mut TestAppContext) {
    let (_, mut visual) = setup(
        cx,
        "Before the code.\n\n```rust\nlet a = 1;\nlet b = 2;\nlet c = 3;\n```\n\n[After the code](https://example.invalid/after-code)"
            .into(),
        false,
    );
    let doc = visual.debug_bounds("fixture-document").unwrap();
    let code = visual.debug_bounds("fixture-code-origin").unwrap();
    // The marker is an absolutely positioned code action, 0.5rem below the block top.
    let marker_inset = visual.update(|window, _| window.rem_size()) * 0.5;
    let before_gap = code.origin.y - marker_inset - doc.origin.y - px(22.);
    assert!(
        before_gap >= px(12.),
        "Code touches the preceding paragraph: gap {before_gap:?}"
    );
    assert!(
        doc.size.height >= px(3. * 22. + 24. + 2. * 22. + 24.),
        "Missing code/paragraph spacing: {doc:?}"
    );
    let code_bottom = code.origin.y - marker_inset + px(3. * 22. + 24. + 2.);
    visual.simulate_click(
        point(doc.origin.x + px(10.), code_bottom + px(6.)),
        Modifiers::none(),
    );
    assert!(
        visual.opened_url().is_none(),
        "The code/prose gap must stay empty"
    );
    visual.simulate_click(
        point(doc.origin.x + px(10.), doc.bottom() - px(11.)),
        Modifiers::none(),
    );
    assert_eq!(
        visual.opened_url().as_deref(),
        Some("https://example.invalid/after-code"),
        "Following prose must remain visible and clickable below the code"
    );
}

#[gpui::test]
fn long_code_document_keeps_following_sections_below_its_full_content(cx: &mut TestAppContext) {
    let (_, mut visual) = setup(
        cx,
        format!(
            "Before.\n\n```text\n{}```\n\nAfter.",
            "a line of code\n".repeat(30)
        ),
        false,
    );
    let doc = visual.debug_bounds("fixture-document").unwrap();
    let tail = visual.debug_bounds("fixture-tail").unwrap();
    assert!(
        doc.size.height >= px(30. * 22. + 2. * 22.),
        "A long code block must not shrink to the viewport: {doc:?}"
    );
    assert!(
        tail.origin.y >= doc.bottom(),
        "Following content must stay below the full document"
    );
}

#[gpui::test]
fn wrapped_code_grows_with_a_narrow_reading_pane(cx: &mut TestAppContext) {
    let (root, mut visual) = setup(
        cx,
        format!("Before.\n\n```text\n{}\n```\n\nAfter.", "word ".repeat(40)),
        false,
    );
    let wide = visual.debug_bounds("fixture-document").unwrap();
    visual.simulate_resize(size(px(180.), px(220.)));
    visual.draw(point(px(0.), px(0.)), size(px(180.), px(220.)), |_, _| {
        root.clone()
    });
    let narrow = visual.debug_bounds("fixture-document").unwrap();
    let tail = visual.debug_bounds("fixture-tail").unwrap();
    assert!(
        narrow.size.height > wide.size.height,
        "Wrapped code must grow vertically"
    );
    assert!(
        tail.origin.y >= narrow.bottom(),
        "Wrapping must push following sections down"
    );
}

#[gpui::test]
fn code_diagram_and_prose_keep_their_own_vertical_space(cx: &mut TestAppContext) {
    let (_, mut visual) = setup(
        cx,
        "Before.\n\n```rust\nlet a = 1;\nlet b = 2;\nlet c = 3;\n```\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\nAfter the diagram.".into(),
        true,
    );
    let doc = visual.debug_bounds("fixture-document").unwrap();
    let diagram = visual.debug_bounds("mermaid-diagram").unwrap();
    let tail = visual.debug_bounds("fixture-tail").unwrap();
    assert!(
        diagram.origin.y >= doc.origin.y + px(4. * 22. + 24. + 24.),
        "The diagram must start after the prose and complete code block"
    );
    assert!(
        doc.bottom() >= diagram.bottom() + px(22.),
        "The final paragraph must have room below the diagram"
    );
    assert!(
        tail.origin.y >= doc.bottom(),
        "Mixed documents must keep their full content height"
    );
}
