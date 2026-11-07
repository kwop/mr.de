//! Inbox A: a compact queue above a full-width reader with pinned actions.
use super::{Cockpit, label_chip, pipeline_dot};
use crate::{backend::Action, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, IconName, Sizable, button::*, scroll::Scrollbar};
use mrde_core::models::MrId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailTab {
    Description,
    Structure,
    Files,
    Review,
    Statistics,
    History,
    Risks,
    Checks,
    Visual,
}
impl DetailTab {
    const ALL: [Self; 9] = [
        Self::Description,
        Self::Structure,
        Self::Files,
        Self::Review,
        Self::Statistics,
        Self::History,
        Self::Risks,
        Self::Checks,
        Self::Visual,
    ];
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Structure => "structure",
            Self::Files => "files",
            Self::Review => "review",
            Self::Statistics => "statistics",
            Self::History => "history",
            Self::Risks => "risks",
            Self::Checks => "checks",
            Self::Visual => "visual",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Description => "Description",
            Self::Structure => "Structure",
            Self::Files => "Files",
            Self::Review => "Pre-review",
            Self::Statistics => "Statistics",
            Self::History => "History",
            Self::Risks => "Risks",
            Self::Checks => "Checks",
            Self::Visual => "Visual",
        }
    }
}
/// "1,730 lines · 1 approval"; unknown values stay "?" rather than zero.
pub(super) fn mr_metrics(lines: Option<u64>, approvals: Option<u64>) -> String {
    let count = |value: Option<u64>, one: &str, many: &str| match value {
        Some(1) => format!("1 {one}"),
        Some(n) => format!("{} {many}", thousands(n)),
        None => format!("? {many}"),
    };
    format!(
        "{}  ·  {}",
        count(lines, "line", "lines"),
        count(approvals, "approval", "approvals")
    )
}

pub(super) fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

impl Cockpit {
    pub(super) fn show_detail_tab(&mut self, tab: DetailTab) {
        self.detail_tab = tab;
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
    }
    pub(super) fn open_mr(&mut self, id: MrId, cx: &mut Context<Self>) {
        if self.initializing || self.confirm {
            return;
        }
        self.selected_mr = Some(id.clone());
        self.detail = None;
        self.review = None;
        self.review_level = None;
        self.show_detail_tab(DetailTab::Description);
        self.start(Action::Select(id), "Loading MR details…", cx);
    }
    /// Position of the open MR among the queue entries matching the filters.
    fn selected_position(&self) -> Option<usize> {
        let id = self
            .selected_mr
            .as_ref()
            .or_else(|| self.detail.as_ref().map(|d| &d.mr.id))?;
        self.visible_queue()
            .iter()
            .position(|&index| &self.queue[index].id == id)
    }
    fn move_mr(&mut self, offset: isize, cx: &mut Context<Self>) {
        let Some(position) = self
            .selected_position()
            .and_then(|index| index.checked_add_signed(offset))
        else {
            return;
        };
        let Some(&index) = self.visible_queue().get(position) else {
            return;
        };
        let id = self.queue[index].id.clone();
        if self.initializing || self.confirm {
            return;
        }
        if position / super::QUEUE_PAGE_SIZE != self.queue_page {
            self.set_queue_page(position / super::QUEUE_PAGE_SIZE);
        }
        self.open_mr(id, cx);
    }
    pub(super) fn reader_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.confirm;
        let position = self.selected_position();
        div()
            .debug_selector(|| "reader-toolbar".into())
            .flex_shrink_0()
            .min_h(px(48.))
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .flex_wrap()
            .bg(rgb(TOOLBAR))
            .when(self.reader_focused, |bar| bar.rounded_t(px(RADIUS_SHEET)))
            .border_b_1()
            .border_color(rgb(PANEL_LINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div().debug_selector(|| "close-mr".into()).child(
                            Button::new("close-mr")
                                .small()
                                .h(px(32.))
                                .ghost()
                                .label("Close")
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_navigation();
                                    this.selected_mr = None;
                                    this.detail = None;
                                    this.review = None;
                                    this.confirm = false;
                                    this.reader_focused = false;
                                    this.show_detail_tab(DetailTab::Description);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "previous-mr".into()).child(
                            Button::new("previous-mr")
                                .small()
                                .h(px(32.))
                                .ghost()
                                .icon(IconName::ChevronLeft)
                                .tooltip("Previous merge request")
                                .disabled(locked || position.is_none_or(|i| i == 0))
                                .on_click(cx.listener(|this, _, _, cx| this.move_mr(-1, cx))),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "next-mr".into()).child(
                            Button::new("next-mr")
                                .small()
                                .h(px(32.))
                                .ghost()
                                .icon(IconName::ChevronRight)
                                .tooltip("Next merge request")
                                .disabled(
                                    locked
                                        || position
                                            .is_none_or(|i| i + 1 >= self.visible_queue().len()),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.move_mr(1, cx))),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "focus-reader".into()).child(
                            Button::new("focus-reader")
                                .small()
                                .h(px(32.))
                                .ghost()
                                .label(if self.reader_focused {
                                    "Show queue"
                                } else {
                                    "Focus"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reader_focused = !this.reader_focused;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .when(self.detail.is_some(), |bar| {
                bar.child(self.actions_panel(cx))
            })
            .into_any_element()
    }
    pub(super) fn detail_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut pane = div()
            .id("detail")
            .debug_selector(|| "mr-detail".into())
            .track_scroll(&self.detail_scroll)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .overflow_y_scroll()
            .px_5();
        let Some(detail) = &self.detail else {
            return pane
                .child(if self.navigation_job.is_some() {
                    "Loading merge request…"
                } else {
                    "Could not load this merge request. Select it again to retry."
                })
                .into_any_element();
        };
        let content = match self.detail_tab {
            DetailTab::Description => self.description_view(detail, window, cx),
            DetailTab::Files => self.files_view(detail, window, cx),
            DetailTab::Review => self.review_view(window, cx),
            DetailTab::Statistics => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.statistics_controls(cx))
                .child(self.disktree_view(detail, window, cx))
                .into_any_element(),
            DetailTab::Structure => self.structure_view(detail, window, cx),
            DetailTab::History => self.history_view(cx),
            DetailTab::Risks => self.risks_view(cx),
            DetailTab::Visual => self.visual_view(cx),
            DetailTab::Checks => div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.compliance_controls(cx))
                .child(self.compliance_view(window, cx))
                .into_any_element(),
        };
        // A segmented control that hugs its tabs.
        let mut tabs = div()
            .debug_selector(|| "detail-tabs".into())
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(RADIUS_CONTROL))
            .bg(rgb(QUEUE));
        for tab in DetailTab::ALL {
            let selected = self.detail_tab == tab;
            let risks = self.risk_count();
            let symbols = self.current_insights().map(|i| i.symbols.len());
            // Counts stay short so the nine tabs fit on one line at the minimum window
            // width; the tooltip says what is counted.
            let count = match tab {
                DetailTab::Files => Some((detail.files.len(), "changed files")),
                DetailTab::Structure => symbols.map(|n| (n, "changed symbols")),
                DetailTab::Review => self
                    .review
                    .as_ref()
                    .map(|r| (r.risks.len(), "risks found by Claude")),
                DetailTab::Risks => {
                    risks.map(|n| (n, "contract, dependency, and pattern findings"))
                }
                _ => None,
            };
            let label = match count {
                Some((n, _)) => format!("{} · {n}", tab.label()),
                None => tab.label().to_string(),
            };
            tabs = tabs.child(
                div()
                    .debug_selector(move || format!("tab-{}", tab.key()))
                    .child(
                        Button::new(SharedString::from(format!("detail-tab-{}", tab.key())))
                            .small()
                            .compact()
                            .ghost()
                            .rounded(px(RADIUS_CHIP + 1.))
                            .when(selected, |b| b.bg(rgb(SELECTED)))
                            .label(label)
                            .when_some(count, |b, (n, what)| b.tooltip(format!("{n} {what}")))
                            .disabled(self.confirm)
                            .text_color(rgb(if selected { HEADING } else { MUTED }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.show_detail_tab(tab);
                                cx.notify();
                            })),
                    ),
            );
        }
        // A definite preferred width makes Taffy measure wrapped titles at the final
        // column width. `flex_1().max_w(900)` undercounted their height, leaving the last
        // History items beyond the scroll limit. The column shrinks on narrow windows.
        let column = div()
            .debug_selector(|| "reading-column".into())
            .w(px(900.))
            .max_w_full()
            .min_w_0()
            .h_auto()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "{} / !{} · {}",
                        detail.mr.project, detail.mr.id.iid, detail.mr.author
                    )),
            )
            .child(
                div()
                    .debug_selector(|| "mr-title".into())
                    .text_size(px(23.))
                    .line_height(px(30.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(HEADING))
                    .child(crate::paragraph::paragraph(detail.mr.title.clone())),
            )
            .child(
                div()
                    .debug_selector(|| "mr-meta".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(pipeline_dot(detail.mr.pipeline.as_deref()))
                    .child(format!(
                        "{} · {}h · {}",
                        detail.mr.pipeline.as_deref().unwrap_or("No pipeline"),
                        (chrono::Utc::now() - detail.mr.created_at)
                            .num_hours()
                            .max(0),
                        mr_metrics(detail.mr.changed_lines, detail.mr.approvals)
                    )),
            )
            .child(
                div().flex().flex_wrap().gap_1().children(
                    super::labels::display_order(detail.mr.all_labels())
                        .into_iter()
                        .map(label_chip),
                ),
            )
            .child(div().flex().mt_1().child(tabs))
            .child(
                // A stretched flex column, not `w_full()` or a block: GPUI 0.2.2 then
                // resolves descendants' percentage widths against the window, and the
                // tab content overflowed the reading column.
                div()
                    .debug_selector(|| "detail-tab-content".into())
                    .min_w_0()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .child(content),
            );
        pane = pane.child(
            div()
                .flex()
                .items_start()
                .flex_shrink_0()
                .py_4()
                .justify_center()
                .child(column),
        );
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(pane)
            .child(Scrollbar::vertical(&self.detail_scroll))
            .into_any_element()
    }
}
