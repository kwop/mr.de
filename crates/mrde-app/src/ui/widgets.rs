//! Small building blocks shared by the analysis tabs.
use super::Cockpit;
use crate::{backend::Action, ellipsis::ellipsis, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Sizable, button::*};
use mrde_core::insights::{Insights, Level, Tour, VisualReport};

pub(super) const CARD: u32 = 0x24272d;

/// Section title with an optional right-aligned hint. Controls appended by callers sit at
/// the far right. The hint is a single ellipsized line: wrapped text squeezed in a row is
/// painted from a stale min-content measurement by GPUI 0.2.2, one glyph per line.
pub(super) fn heading(title: impl Into<SharedString>, hint: Option<String>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .mt_2()
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(15.))
                .text_color(rgb(HEADING))
                .font_weight(FontWeight::MEDIUM)
                .child(title.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .justify_end()
                .text_size(px(12.))
                .text_color(rgb(SUBTLE))
                .children(hint.map(ellipsis)),
        )
}

pub(super) fn badge(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .rounded(px(RADIUS_CHIP))
        .border_1()
        .border_color(rgb(color))
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(rgb(color))
        .child(text.into())
}

pub(super) fn card() -> Div {
    div()
        .flex_shrink_0()
        .min_w_0()
        .p_3()
        .rounded(px(RADIUS_CARD))
        .border_1()
        .border_color(rgb(LINE))
        .bg(rgb(CARD))
        .flex()
        .flex_col()
        .gap_2()
}

/// `directory/name.ext:line` on one line, the directory dimmed, ending with "…".
pub(super) fn path_line(path: &str, line: Option<u32>) -> Div {
    let (directory, name) = match path.rsplit_once('/') {
        Some((directory, name)) => (format!("{directory}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    };
    let suffix = line.map(|l| format!(":{l}")).unwrap_or_default();
    let text = format!("{directory}{name}{suffix}");
    let split = directory.len();
    div()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .font_family(CODE_FONT)
        .text_size(px(CODE_SIZE))
        .child(ellipsis(text.clone()).colors([
            (0..split, rgb(SUBTLE).into()),
            (split..text.len(), rgb(INK).into()),
        ]))
}

/// One line of code, monospace, ending with "…".
pub(super) fn code(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .font_family(CODE_FONT)
        .text_size(px(CODE_SIZE))
        .text_color(rgb(color))
        .child(ellipsis(text))
}

pub(super) fn level_color(level: Level) -> u32 {
    match level {
        Level::High => ERROR,
        Level::Medium => WARNING,
        Level::Low => SUBTLE,
    }
}

pub(super) fn level_label(level: Level) -> &'static str {
    match level {
        Level::High => "High",
        Level::Medium => "Medium",
        Level::Low => "Low",
    }
}

/// "1 file", "3 files".
pub(super) fn count(value: usize, one: &str, many: &str) -> String {
    if value == 1 {
        format!("1 {one}")
    } else {
        format!("{value} {many}")
    }
}

pub(super) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(rgb(MUTED))
        .child(text.into())
}

impl Cockpit {
    /// Analysis of the displayed commit, if computed.
    pub(super) fn current_insights(&self) -> Option<&Insights> {
        let detail = self.detail.as_ref()?;
        self.insights
            .as_ref()
            .filter(|(id, insights)| *id == detail.mr.id && insights.sha == detail.mr.sha)
            .map(|(_, insights)| insights.as_ref())
    }
    pub(super) fn current_tour(&self) -> Option<&Tour> {
        let detail = self.detail.as_ref()?;
        self.tour
            .as_ref()
            .filter(|(id, sha, _)| *id == detail.mr.id && *sha == detail.mr.sha)
            .map(|(_, _, tour)| tour)
    }
    pub(super) fn current_visual(&self) -> Option<&VisualReport> {
        let detail = self.detail.as_ref()?;
        self.visual
            .as_ref()
            .filter(|(id, sha, _)| *id == detail.mr.id && *sha == detail.mr.sha)
            .map(|(_, _, report)| report)
    }
    pub(super) fn analyze_button(&self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let analyzed = self.current_insights().is_some();
        div()
            .debug_selector(move || id.into())
            .child(
                Button::new(id)
                    .small()
                    .label(if analyzed {
                        "Analyze again"
                    } else {
                        "Analyze worktree"
                    })
                    .disabled(self.is_busy() || self.confirm)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(detail) = this.detail.clone() {
                            this.start(
                                Action::Analyze(detail),
                                "Analyzing the worktree: symbols, history, contracts…",
                                cx,
                            );
                        }
                    })),
            )
            .into_any_element()
    }
    /// Shown in analysis tabs before the worktree has been analyzed.
    pub(super) fn analyze_prompt(&self, text: &'static str, cx: &mut Context<Self>) -> AnyElement {
        card()
            .debug_selector(|| "analyze-prompt".into())
            .child(muted(text))
            .child(muted(
                "The analysis runs Git locally in the MR worktree (created if missing); only overlapping MRs query GitLab.",
            ))
            .child(div().flex().child(self.analyze_button("analyze-from-tab", cx)))
            .into_any_element()
    }
}
