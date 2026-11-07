//! Window chrome in the style of a mail client: a title bar with search, settings, the
//! review context toggle, and the account; side navigation with label filters; and the
//! "Open MR…" dialog for merge requests outside the queue.
use super::{Cockpit, context::ContextLayout, inbox::DetailTab, labels};
use crate::{backend::Action, ellipsis::ellipsis, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Icon, IconName,
    button::*,
    input::{self, Input},
    tooltip::Tooltip,
};
use mrde_core::inbox::{MrReference, label_facets};

actions!(mrde, [FocusSearch, ToggleContext, OpenMergeRequest]);

/// Labels listed in the sidebar before "more".
const SIDEBAR_LABELS: usize = 8;

/// Shortcuts work wherever the focus is, including when nothing is focused.
pub(super) fn register_shortcuts(window: &Window, cx: &mut Context<Cockpit>) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", FocusSearch, None),
        KeyBinding::new("cmd-alt-b", ToggleContext, None),
        KeyBinding::new("cmd-o", OpenMergeRequest, None),
    ]);
    let handle = window.window_handle();
    let view = cx.weak_entity();
    // The window is busy dispatching the keystroke; act right after.
    let on = move |cx: &mut App, run: fn(&mut Cockpit, &mut Window, &mut Context<Cockpit>)| {
        let view = view.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = view.update(cx, |this, cx| run(this, window, cx));
            });
        });
    };
    let focus = on.clone();
    App::on_action(cx, move |_: &FocusSearch, cx| {
        focus(cx, Cockpit::focus_search)
    });
    let toggle = on.clone();
    App::on_action(cx, move |_: &ToggleContext, cx| {
        toggle(cx, Cockpit::toggle_context)
    });
    App::on_action(cx, move |_: &OpenMergeRequest, cx| {
        on(cx, Cockpit::show_open_dialog)
    });
}

/// Two letters: the initials of the first and last name parts, else the first two letters.
pub(super) fn initials(user: &str) -> String {
    let parts: Vec<&str> = user
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect();
    let letters: String = match parts.as_slice() {
        [] => String::new(),
        [one] => one.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

fn shortcut(text: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .px(px(5.))
        .rounded(px(RADIUS_CHIP))
        .border_1()
        .border_color(rgb(LINE))
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(rgb(SUBTLE))
        .child(text)
}

fn nav_item(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    count: Option<usize>,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_shrink_0()
        .h(px(34.))
        .px_3()
        .rounded(px(RADIUS_CONTROL))
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(rgb(SELECTED))
                .text_color(rgb(HEADING))
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!selected, |d| d.hover(|style| style.bg(rgb(HOVER))))
        .child(Icon::new(icon).size_4())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .one_line()
                .child(ellipsis(crate::i18n::t(label))),
        )
        .children(count.map(|count| {
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(count.to_string())
        }))
}

fn section(title: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .mt_5()
        .mb_1()
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(crate::i18n::t(title))
}

impl Cockpit {
    /// Indices of the queue entries matching the search and label filters, in queue order.
    pub(super) fn visible_queue(&self) -> Vec<usize> {
        self.queue
            .iter()
            .enumerate()
            .filter(|(_, mr)| self.filter.matches(mr))
            .map(|(index, _)| index)
            .collect()
    }
    fn show_inbox(&mut self) {
        self.settings = false;
        self.worktrees_view = false;
        self.threads_view = false;
        self.confirm = false;
    }
    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_inbox();
        self.search.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    fn toggle_label(&mut self, name: &str, cx: &mut Context<Self>) {
        self.filter.toggle_label(name);
        self.show_inbox();
        self.set_queue_page(0);
        cx.notify();
    }
    pub(super) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.labels.clear();
        self.filter.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.set_queue_page(0);
        cx.notify();
    }
    pub(super) fn show_open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.initializing {
            return;
        }
        if !self.config.provider.available() {
            self.settings = true;
            cx.notify();
            return;
        }
        self.open_dialog = true;
        self.open_error = None;
        self.open_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }
    fn close_open_dialog(&mut self, cx: &mut Context<Self>) {
        self.open_dialog = false;
        self.open_error = None;
        cx.notify();
    }
    pub(super) fn submit_open(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.open_input.read(cx).value().trim().to_string();
        match MrReference::parse(&text, &self.config.gitlab_url) {
            Err(error) => self.open_error = Some(error.to_string()),
            Ok(reference) => {
                self.open_dialog = false;
                self.open_error = None;
                self.show_inbox();
                self.cancel_navigation();
                self.selected_mr = None;
                self.detail = None;
                self.review = None;
                self.review_level = None;
                self.show_detail_tab(DetailTab::Description);
                self.start(
                    Action::Open(text),
                    &crate::tr!(
                        "Opening {value0}!{value1}…",
                        value0 = reference.project,
                        value1 = reference.iid
                    ),
                    cx,
                );
                self.opening = self.navigation_job.is_some();
            }
        }
        cx.notify();
    }

    pub(super) fn titlebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let layout = self.context_layout(window.viewport_size().width);
        let open = matches!(layout, ContextLayout::Docked | ContextLayout::Overlay);
        let available = self.context_available();
        let alert = (layout == ContextLayout::Hidden && available)
            .then(|| self.context_alert())
            .flatten();
        let search = Input::new(&self.search)
            .prefix(Icon::new(IconName::Search).size_4().text_color(rgb(SUBTLE)))
            .when(self.filter.query.is_empty(), |input| {
                input.suffix(shortcut("⌘K"))
            })
            .cleanable(true)
            .bordered(false)
            .bg(rgb(QUEUE))
            .text_size(px(UI_SIZE))
            .rounded(px(RADIUS_CONTROL))
            .h(px(36.));
        let user = self.executor.username(&self.config);
        let provider = self.config.provider.label();
        let avatar = div()
            .id("avatar")
            .debug_selector(|| "avatar".into())
            .flex_shrink_0()
            .size(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center();
        let avatar = match user {
            Some(user) => avatar
                .bg(rgb(0x3f6ea6))
                .text_color(rgb(0xffffff))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials(&user))
                .tooltip(move |window, cx| {
                    Tooltip::new(crate::tr!(
                        "Signed in to {provider} as {user}",
                        provider = provider,
                        user = user
                    ))
                    .build(window, cx)
                }),
            None => avatar
                .text_color(rgb(SUBTLE))
                .child(Icon::new(IconName::CircleUser).size(px(22.)))
                .tooltip(|window, cx| {
                    Tooltip::new(crate::i18n::t(
                        "GitLab identity appears after the first refresh",
                    ))
                    .build(window, cx)
                }),
        };
        div()
            .h(px(TITLEBAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .pr_3()
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .w(px(SIDEBAR_WIDTH))
                    .h_full()
                    .flex_shrink_0()
                    .pl(px(88.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .text_color(rgb(SUBTLE))
                    .child(crate::i18n::t("mr.de")),
            )
            .child(
                div()
                    .debug_selector(|| "search".into())
                    .flex_1()
                    .min_w(px(220.))
                    .max_w(px(560.))
                    .child(search),
            )
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "titlebar-settings".into()).child(
                    Button::new("titlebar-settings")
                        .ghost()
                        .icon(IconName::Settings)
                        .tooltip(crate::i18n::t("Project settings"))
                        .when(self.settings, |b| b.bg(rgb(SELECTED)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm = false;
                            this.settings = true;
                            this.worktrees_view = false;
                            this.threads_view = false;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .debug_selector(|| "toggle-context".into())
                    .relative()
                    .child(
                        Button::new("toggle-context")
                            .ghost()
                            .icon(IconName::PanelRight)
                            .tooltip(if available {
                                "Review context (⌘⌥B)"
                            } else {
                                "Review context: open a merge request first"
                            })
                            .when(open, |b| b.bg(rgb(SELECTED)))
                            .disabled(!available)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_context(window, cx)),
                            ),
                    )
                    .children(alert.map(|color| {
                        div()
                            .debug_selector(|| "context-alert".into())
                            .absolute()
                            .top(px(4.))
                            .right(px(4.))
                            .size(px(8.))
                            .rounded_full()
                            .bg(rgb(color))
                    })),
            )
            .child(avatar)
            .into_any_element()
    }

    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let inbox = !self.settings && !self.worktrees_view && !self.threads_view;
        let facets = label_facets(&self.queue);
        let selected = &self.filter.labels;
        let limit = if self.labels_expanded {
            usize::MAX
        } else {
            SIDEBAR_LABELS
        };
        let mut labels_list = div().flex().flex_col().gap(px(2.));
        for (index, (label, count)) in facets.iter().enumerate() {
            let active = selected.contains(&label.name);
            if index >= limit && !active {
                continue;
            }
            let name = label.name.clone();
            let selector = format!("label-filter-{name}");
            labels_list = labels_list.child(
                div()
                    .id(("label-filter", index))
                    .debug_selector(move || selector.clone())
                    .h(px(30.))
                    .px_3()
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |d| d.bg(rgb(SELECTED)).text_color(rgb(HEADING)))
                    .when(!active, |d| d.hover(|style| style.bg(rgb(HOVER))))
                    .child(
                        div()
                            .flex_shrink_0()
                            .size(px(10.))
                            .rounded(px(RADIUS_CHIP))
                            .bg(rgb(labels::swatch(label))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .one_line()
                            .child(ellipsis(label.name.clone())),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(count.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_label(&name, cx))),
            );
        }
        if facets.len() > SIDEBAR_LABELS {
            labels_list = labels_list.child(
                div()
                    .id("more-labels")
                    .debug_selector(|| "more-labels".into())
                    .h(px(28.))
                    .px_3()
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(
                        Icon::new(if self.labels_expanded {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(12.)),
                    )
                    .child(if self.labels_expanded {
                        "Fewer labels".to_string()
                    } else {
                        crate::tr!("{value0} more", value0 = facets.len() - SIDEBAR_LABELS)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.labels_expanded = !this.labels_expanded;
                        cx.notify();
                    })),
            );
        }
        let worktrees = (!self.worktrees.is_empty()).then_some(self.worktrees.len());
        div()
            .id("sidebar")
            .debug_selector(|| "navigation".into())
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .px_3()
            .pt_1()
            .pb_4()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(13.))
            .child(self.project_switcher(cx))
            .child(
                div().flex().mb_4().child(
                    div()
                        .id("open-mr-cta")
                        .debug_selector(|| "open-mr-cta".into())
                        .h(px(44.))
                        .pl_4()
                        .pr_5()
                        .rounded(px(RADIUS_SHEET))
                        .bg(rgb(READING))
                        .hover(|style| style.bg(rgb(HOVER)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_color(rgb(HEADING))
                        .font_weight(FontWeight::MEDIUM)
                        .child(Icon::new(IconName::Plus).size_4())
                        .child(crate::i18n::t("Open MR…"))
                        .tooltip(|window, cx| {
                            Tooltip::new(crate::i18n::t("Open a merge request from its link (⌘O)"))
                                .build(window, cx)
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_open_dialog(window, cx)),
                        ),
                ),
            )
            .child(
                nav_item(
                    "nav-queue",
                    IconName::Inbox,
                    "Merge requests",
                    Some(self.queue.len()),
                    inbox,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_inbox();
                    cx.notify();
                })),
            )
            .child(
                nav_item(
                    "nav-worktrees",
                    IconName::Folder,
                    "Worktrees",
                    worktrees,
                    self.worktrees_view && !self.settings,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.initializing {
                        this.settings = false;
                        this.worktrees_view = true;
                        this.threads_view = false;
                        this.confirm = false;
                        this.start(Action::Worktrees, "Listing worktrees…", cx);
                    }
                })),
            )
            .child(
                nav_item(
                    "nav-threads",
                    IconName::Bot,
                    "Threads",
                    None,
                    self.threads_view && !self.settings,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings = false;
                    this.worktrees_view = false;
                    this.threads_view = true;
                    this.confirm = false;
                    cx.notify();
                })),
            )
            .when(!facets.is_empty(), |sidebar| {
                sidebar
                    .child(section("Labels").when(!selected.is_empty(), |header| {
                        header.child(
                            div()
                                .id("clear-labels")
                                .debug_selector(|| "clear-labels".into())
                                .cursor_pointer()
                                .text_color(rgb(INFO))
                                .hover(|style| style.text_color(rgb(HEADING)))
                                .child(crate::i18n::t("Clear"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.filter.labels.clear();
                                    this.set_queue_page(0);
                                    cx.notify();
                                })),
                        )
                    }))
                    .child(labels_list)
            })
            .child(section("Settings"))
            .child(
                nav_item(
                    "nav-languages",
                    IconName::Settings,
                    "Languages",
                    None,
                    self.settings && self.language_page,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm = false;
                    this.settings = true;
                    this.language_page = true;
                    this.worktrees_view = false;
                    this.threads_view = false;
                    cx.notify();
                })),
            )
            .child(
                nav_item(
                    "nav-settings",
                    IconName::Settings,
                    "Project settings",
                    None,
                    self.settings && !self.language_page,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm = false;
                    this.settings = true;
                    this.language_page = false;
                    this.worktrees_view = false;
                    this.threads_view = false;
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    pub(super) fn open_dialog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let host = self
            .config
            .gitlab_url
            .split_once("://")
            .map_or(self.config.gitlab_url.as_str(), |(_, host)| host)
            .trim_end_matches('/')
            .to_string();
        let empty = self.open_input.read(cx).value().trim().is_empty();
        div()
            .id("open-mr-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(hsla(0., 0., 0., 0.45))
            .flex()
            .justify_center()
            .items_start()
            .pt(px(120.))
            .on_click(cx.listener(|this, _, _, cx| this.close_open_dialog(cx)))
            .child(
                div()
                    .id("open-mr-dialog")
                    .debug_selector(|| "open-mr-dialog".into())
                    .key_context("OpenMrDialog")
                    .on_action(cx.listener(|this, _: &input::Escape, _, cx| {
                        this.close_open_dialog(cx)
                    }))
                    // Handled here, so a single-line input never receives the newline.
                    .on_action(cx.listener(|this, _: &input::Enter, window, cx| {
                        this.submit_open(window, cx)
                    }))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .w(px(520.))
                    .p_5()
                    .rounded(px(RADIUS_SHEET))
                    .bg(rgb(QUEUE))
                    .border_1()
                    .border_color(rgb(PANEL_LINE))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.))
                            .text_color(rgb(HEADING))
                            .font_weight(FontWeight::MEDIUM)
                            .child(crate::i18n::t("Open a merge request")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(if host.is_empty() {
                                "Connect GitLab in Settings first, then paste a merge request link."
                                    .to_string()
                            } else {
                                crate::tr!("Paste a link from {host}, or a reference such as group/project!123.", host = host)
                            }),
                    )
                    .child(
                        div().debug_selector(|| "open-mr-input".into()).child(
                            Input::new(&self.open_input)
                                .bg(rgb(FIELD))
                                .text_size(px(UI_SIZE))
                                .rounded(px(RADIUS_CONTROL))
                                .h(px(36.)),
                        ),
                    )
                    .children(self.open_error.clone().map(|error| {
                        div()
                            .debug_selector(|| "open-mr-error".into())
                            .text_size(px(12.))
                            .text_color(rgb(ERROR))
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div().debug_selector(|| "open-mr-cancel".into()).child(
                                    Button::new("open-mr-cancel")
                                        .ghost()
                                        .label(crate::i18n::t("Cancel"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.close_open_dialog(cx)
                                        })),
                                ),
                            )
                            .child(
                                div().debug_selector(|| "open-mr-submit".into()).child(
                                    Button::new("open-mr-submit")
                                        .primary()
                                        .label(crate::i18n::t("Open"))
                                        .disabled(empty || self.initializing)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_open(window, cx)
                                        })),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[::core::prelude::v1::test]
    fn initials_use_the_first_and_last_name_parts_or_the_first_two_letters() {
        for (user, expected) in [
            ("jordan.lee", "JL"),
            ("alice", "AL"),
            ("a", "A"),
            ("mary-ann_smith", "MS"),
            ("", ""),
            ("été.ünal", "ÉÜ"),
        ] {
            assert_eq!(super::initials(user), expected, "{user}");
        }
    }
}
