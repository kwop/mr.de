//! Worktrees page: lists mr.de worktrees and cleans the ones without local work.
use super::Cockpit;
use crate::{backend::Action, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Icon, IconName, Sizable, button::*};

impl Cockpit {
    pub(super) fn worktrees_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.is_busy();
        let mut list = div()
            .id("worktrees")
            .debug_selector(|| "worktrees-list".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if self.worktrees.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(rgb(MUTED))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(px(24.))
                            .text_color(rgb(SUBTLE)),
                    )
                    .child(if busy {
                        "Listing worktrees…"
                    } else {
                        "No worktrees. Reviews, statistics, compliance checks, and questions to Claude create them."
                    }),
            );
        }
        for (index, entry) in self.worktrees.iter().enumerate() {
            let title = self
                .queue
                .iter()
                .find(|mr| mr.id.project == entry.project && mr.id.iid == entry.iid)
                .map(|mr| mr.title.clone())
                .unwrap_or_else(|| format!("Project {} merge request", entry.project));
            let (state, color) = match entry.changes {
                Some(0) => ("Clean".to_string(), SUCCESS),
                Some(n) => (format!("{n} local or ignored files"), WARNING),
                None => ("Cannot inspect".to_string(), ERROR),
            };
            let removable = entry.changes == Some(0);
            let reveal = entry.path.clone();
            let remove = entry.clone();
            list =
                list.child(
                    div()
                        .debug_selector(move || format!("worktree-{index}"))
                        .flex_shrink_0()
                        .px_5()
                        .py_3()
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_color(rgb(HEADING))
                                        .one_line()
                                        .child(crate::ellipsis::ellipsis(title)),
                                )
                                .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                                    format!(
                                        "!{} · commit {}",
                                        entry.iid,
                                        entry.sha.chars().take(12).collect::<String>()
                                    ),
                                ))
                                .child(
                                    div()
                                        .font_family(CODE_FONT)
                                        .text_size(px(11.))
                                        .text_color(rgb(SUBTLE))
                                        .one_line()
                                        .child(crate::ellipsis::ellipsis(
                                            entry.path.display().to_string(),
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .w(px(170.))
                                .flex_shrink_0()
                                .text_size(px(12.))
                                .text_color(rgb(color))
                                .child(state),
                        )
                        .child(
                            Button::new(("reveal-worktree", index))
                                .small()
                                .ghost()
                                .label("Reveal")
                                .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                        )
                        .child(
                            div()
                                .debug_selector(move || format!("clean-worktree-{index}"))
                                .child(
                                    Button::new(("clean-worktree", index))
                                        .small()
                                        .label("Clean")
                                        .disabled(busy || !removable)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.start(
                                                Action::RemoveWorktree(remove.clone()),
                                                "Removing worktree…",
                                                cx,
                                            );
                                        })),
                                ),
                        ),
                );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
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
                                            .text_color(rgb(HEADING))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Worktrees"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(MUTED))
                                            .child(format!("{}", self.worktrees.len())),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(SUBTLE))
                                    .child("Clean never deletes local changes or ignored files; resolve them first."),
                            ),
                    )
                    .child(
                        div().debug_selector(|| "refresh-worktrees".into()).child(
                            Button::new("refresh-worktrees")
                                .small()
                                .ghost()
                                .icon(IconName::Redo)
                                .label("Refresh")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start(Action::Worktrees, "Listing worktrees…", cx);
                                })),
                        ),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
