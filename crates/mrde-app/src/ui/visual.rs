//! Visual tab: before/after screenshots and their pixel difference.
use super::{
    Cockpit,
    widgets::{badge, card, count, heading, muted},
};
use crate::{backend::Action, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Sizable, button::*};
use std::path::Path;

fn frame(label: &'static str, path: Option<&Path>, missing: &'static str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(SUBTLE))
                .child(label),
        )
        .child(
            div()
                .h(px(220.))
                .w_full()
                .rounded(px(6.))
                .bg(rgb(PAGE))
                .border_1()
                .border_color(rgb(LINE))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .child(match path {
                    Some(path) => img(path.to_path_buf())
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => div()
                        .text_size(px(12.))
                        .text_color(rgb(SUBTLE))
                        .child(missing)
                        .into_any_element(),
                }),
        )
}

impl Cockpit {
    pub(super) fn visual_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let configured = !self.config.visual_command.trim().is_empty();
        let report = self.current_visual();
        let mut view = div()
            .debug_selector(|| "visual-view".into())
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                heading(
                    crate::i18n::t("Visual comparison"),
                    report.map(|r| {
                        let changed = r.screens.iter().filter(|s| s.changed != Some(0.0)).count();
                        crate::tr!(
                            "{value0} · {changed} changed",
                            value0 = count(r.screens.len(), "screen", "screens"),
                            changed = changed
                        )
                    }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .debug_selector(|| "capture-visual".into())
                        .child(
                            Button::new("capture-visual")
                                .small()
                                .label(if report.is_some() {
                                    crate::i18n::t("Capture again")
                                } else {
                                    crate::i18n::t("Capture screenshots")
                                })
                                .disabled(!configured || self.is_busy() || self.confirm)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(detail) = this.detail.clone() {
                                        this.start(
                                            Action::Visual(detail),
                                            "Capturing screenshots of the base and MR commits…",
                                            cx,
                                        );
                                    }
                                })),
                        ),
                ),
            );
        if !configured {
            view = view.child(
                card()
                    .child(muted(crate::i18n::t("Set a screenshot command in GitLab settings to compare the interface before and after the MR.")))
                    .child(muted(crate::i18n::t("mr.de runs it in a worktree of the base commit and one of the MR commit, with MRDE_SCREENSHOTS_DIR set to an output folder and MRDE_SIDE set to before or after. It must write PNG files there, for example with Playwright."))),
            );
        } else if report.is_none() {
            view = view.child(muted(crate::tr!("Command: {value0}. It runs only when you click; both worktrees are created if missing.", value0 = self.config.visual_command.trim())));
        }
        let Some(report) = report else {
            return view.into_any_element();
        };
        if report.screens.is_empty() {
            view = view.child(muted(crate::i18n::t(
                "The command produced no PNG file in MRDE_SCREENSHOTS_DIR.",
            )));
        }
        for (index, screen) in report.screens.iter().enumerate() {
            let status = match (screen.changed, &screen.before, &screen.after) {
                (_, None, Some(_)) => badge(crate::i18n::t("new screen"), INFO),
                (_, Some(_), None) => badge(crate::i18n::t("removed screen"), WARNING),
                (Some(0.0), _, _) => badge(crate::i18n::t("unchanged"), SUBTLE),
                // Images of different sizes are not compared pixel by pixel.
                (Some(_), _, _) if screen.diff.is_none() => {
                    badge(crate::i18n::t("size changed"), WARNING)
                }
                (Some(ratio), _, _) => badge(
                    crate::tr!(
                        "{value0}% of pixels changed",
                        value0 = format!("{:.1}", ratio * 100.)
                    ),
                    WARNING,
                ),
                _ => badge(crate::i18n::t("not compared"), SUBTLE),
            };
            view = view.child(
                card()
                    .debug_selector(move || format!("screen-{index}"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(rgb(HEADING))
                                    .child(screen.name.clone()),
                            )
                            .child(status),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(frame("Before", screen.before.as_deref(), "No screenshot"))
                            .child(frame("After", screen.after.as_deref(), "No screenshot"))
                            .child(frame(
                                "Difference",
                                screen.diff.as_deref(),
                                if screen.before.is_some() && screen.after.is_some() {
                                    "Sizes differ: not compared"
                                } else {
                                    "Nothing to compare"
                                },
                            )),
                    ),
            );
        }
        view.into_any_element()
    }
}
