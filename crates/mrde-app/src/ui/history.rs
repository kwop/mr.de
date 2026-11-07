//! History tab: why removed code existed, risky files, who knows the code, overlapping MRs.
use super::{
    Cockpit,
    widgets::{badge, card, code, count, heading, level_color, level_label, muted, path_line},
};
use crate::theme::*;
use gpui::{prelude::*, *};
use gpui_component::{Sizable, button::*};

impl Cockpit {
    /// GitLab URL for `#123` (issue) or `!45` (merge request) in the MR's project.
    fn reference_url(&self, reference: &str) -> Option<String> {
        let project = self
            .detail
            .as_ref()?
            .mr
            .web_url
            .split("/-/")
            .next()?
            .to_string();
        let number = &reference[1..];
        Some(match reference.chars().next()? {
            '#' => format!("{project}/-/issues/{number}"),
            '!' => format!("{project}/-/merge_requests/{number}"),
            _ => return None,
        })
    }

    pub(super) fn history_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(insights) = self.current_insights() else {
            return self.analyze_prompt(
                "Analyze the worktree to see why removed code existed, which files have a history of fixes, who knows each zone, and which MRs touch the same files.",
                cx,
            );
        };
        let mut view = div()
            .debug_selector(|| "history-view".into())
            .flex_shrink_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(heading(
                "Why was it there?",
                Some("Origin of the largest removed blocks".into()),
            ));
        if insights.removed.is_empty() {
            view = view.child(muted("The MR does not remove existing lines."));
        }
        for (index, block) in insights.removed.iter().enumerate() {
            let mut lines = div()
                .p_2()
                .rounded(px(6.))
                .bg(rgb(ERROR_BG))
                .flex()
                .flex_col();
            for line in &block.lines {
                lines = lines.child(code(format!("− {line}"), ERROR));
            }
            let mut item = card()
                .debug_selector(move || format!("removed-{index}"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(path_line(&block.path, Some(block.line)).flex_1())
                        .when(block.pure_deletion, |row| {
                            row.child(badge("removed without replacement", WARNING))
                        }),
                )
                .child(lines);
            match &block.origin {
                Some(origin) => {
                    let mut references = div().flex().items_center().gap_2();
                    for reference in &origin.references {
                        if let Some(url) = self.reference_url(reference) {
                            references = references.child(
                                Button::new(SharedString::from(format!("ref-{index}-{reference}")))
                                    .xsmall()
                                    .ghost()
                                    .label(reference.clone())
                                    .on_click(move |_, _, cx| cx.open_url(&url)),
                            );
                        }
                    }
                    item = item
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .child(format!(
                                    "Added {} by {} · {}",
                                    origin.date,
                                    origin.author,
                                    origin.sha.chars().take(8).collect::<String>()
                                ))
                                .child(references),
                        )
                        .child(
                            div()
                                .text_color(rgb(INK))
                                .child(format!("“{}”", origin.summary)),
                        );
                }
                None => {
                    item = item.child(muted(
                        "Origin unknown: the lines predate the available history.",
                    ))
                }
            }
            view = view.child(item);
        }

        view = view.child(heading(
            "Risk by file",
            Some("Last 6 months on the target branch: changes, fix commits, authors".into()),
        ));
        let mut risks = card().debug_selector(|| "file-risks".into()).p_0().gap_0();
        for (index, file) in insights.history.iter().take(40).enumerate() {
            let (label, color) = if file.new_file {
                ("New", INFO)
            } else {
                (level_label(file.risk), level_color(file.risk))
            };
            risks = risks.child(
                div()
                    .px_3()
                    .py_2()
                    .when(index > 0, |r| r.border_t_1().border_color(rgb(LINE)))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(64.))
                                    .flex_shrink_0()
                                    .flex()
                                    .child(badge(label, color)),
                            )
                            .child(path_line(&file.path, None).flex_1())
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(12.))
                                    .text_color(rgb(SUBTLE))
                                    .child(if file.new_file {
                                        "new file".to_string()
                                    } else {
                                        format!(
                                            "{} · {} · {}",
                                            count(file.commits as usize, "change", "changes"),
                                            count(file.fixes as usize, "fix", "fixes"),
                                            count(file.authors as usize, "author", "authors")
                                        )
                                    }),
                            ),
                    )
                    .when_some(file.last_fix.clone(), |row, fix| {
                        row.child(code(format!("last fix  {fix}"), MUTED).pl(px(76.)))
                    }),
            );
        }
        view = view.child(if insights.history.is_empty() {
            muted("No history available for the changed files.").into_any_element()
        } else {
            risks.into_any_element()
        });

        view = view.child(heading(
            "Who knows this code",
            Some("Commits in the last 12 months per zone".into()),
        ));
        if insights.experts.is_empty() {
            view = view.child(muted("No recent commits in the changed zones."));
        }
        let mut experts = card().debug_selector(|| "experts".into()).p_0().gap_0();
        for (index, expert) in insights.experts.iter().enumerate() {
            let mut people = div().flex().flex_wrap().items_center().gap_3();
            for (name, share) in &expert.people {
                people = people.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_color(rgb(INK)).child(name.clone()))
                        .child(
                            div()
                                .w(px(60.))
                                .h(px(5.))
                                .rounded(px(3.))
                                .bg(rgb(FIELD))
                                .child(
                                    div()
                                        .w(px(0.6 * *share as f32))
                                        .h_full()
                                        .rounded(px(3.))
                                        .bg(rgb(INFO)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(SUBTLE))
                                .child(format!("{share}%")),
                        ),
                );
            }
            for owner in &expert.owners {
                people = people.child(badge(format!("owner {owner}"), SUCCESS));
            }
            experts = experts.child(
                div()
                    .px_3()
                    .py_2()
                    .when(index > 0, |r| r.border_t_1().border_color(rgb(LINE)))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(path_line(&expert.zone, None))
                    .child(people),
            );
        }
        if !insights.experts.is_empty() {
            view = view.child(experts);
        }

        view = view.child(heading(
            "Overlapping merge requests",
            Some("Open, or merged in the last 14 days, touching the same files".into()),
        ));
        match &insights.overlaps {
            None => {
                view = view.child(muted(
                    "GitLab was not reachable during the analysis; overlaps are unknown.",
                ))
            }
            Some(overlaps) if overlaps.is_empty() => {
                view = view.child(muted(
                    "No other open or recently merged MR touches these files.",
                ))
            }
            Some(overlaps) => {
                for (index, overlap) in overlaps.iter().enumerate() {
                    let url = overlap.web_url.clone();
                    let mut item = card()
                        .debug_selector(move || format!("overlap-{index}"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(if overlap.merged {
                                    badge("merged", INFO)
                                } else {
                                    badge("open", WARNING)
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .flex()
                                        .text_color(rgb(HEADING))
                                        .child(crate::ellipsis::ellipsis(format!(
                                            "!{} {}",
                                            overlap.iid, overlap.title
                                        ))),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(12.))
                                        .text_color(rgb(SUBTLE))
                                        .child(count(
                                            overlap.files.len(),
                                            "common file",
                                            "common files",
                                        )),
                                )
                                .child(
                                    div().flex_shrink_0().child(
                                        Button::new(SharedString::from(format!(
                                            "open-overlap-{}",
                                            overlap.iid
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label("Open")
                                        .on_click(move |_, _, cx| cx.open_url(&url)),
                                    ),
                                ),
                        );
                    for file in overlap.files.iter().take(6) {
                        item = item.child(path_line(file, None));
                    }
                    if overlap.merged {
                        item = item.child(muted(
                            "Merged after this MR started? Check that the MR is rebased on it.",
                        ));
                    }
                    view = view.child(item);
                }
            }
        }
        view.child(
            div()
                .debug_selector(|| "history-end".into())
                .h(px(1.))
                .flex_shrink_0(),
        )
        .into_any_element()
    }
}
