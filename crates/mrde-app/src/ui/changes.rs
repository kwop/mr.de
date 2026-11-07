//! Experimental combined risk, file statistics and ownership/history workspace.
use super::{
    Cockpit,
    widgets::{card, heading, muted, path_line},
};
use crate::{backend::Action, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, Sizable, button::*};
use mrde_core::risk::Axis;
use mrde_core::{models::*, risk::*};
impl Cockpit {
    pub(super) fn current_risk(&self) -> Option<&RiskReport> {
        let detail = self.detail.as_ref()?;
        self.risk
            .as_ref()
            .filter(|(id, r)| *id == detail.mr.id && r.profile.head_sha == detail.mr.sha)
            .map(|(_, r)| r.as_ref())
    }
    pub(super) fn select_risk_file(&mut self, path: String, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(detail) = self.detail.clone() else {
            return;
        };
        self.risk_selected_file = Some(path.clone());
        self.risk_file = None;
        self.start(Action::RiskFile(detail, path), "Loading file history…", cx);
    }
    pub(super) fn apply_risk(&mut self, id: MrId, report: Box<RiskReport>) {
        if self
            .risk
            .as_ref()
            .is_some_and(|(_, old)| old.profile.config_key != report.profile.config_key)
        {
            self.risk_baseline = None;
        }
        for mr in &mut self.queue {
            if mr.id == id && mr.sha == report.profile.head_sha {
                mr.risk = Some(RiskSummary::from(report.as_ref()));
            }
        }
        mrde_core::scoring::sort_queue(&mut self.queue, self.config.mr_sort, chrono::Utc::now());
        self.stats = Some((id.clone(), report.stats.clone()));
        self.presentation_revision += 1;
        self.status = format!(
            "Risk profile ready · {}/8 axes · unavailable axes are excluded from the mean.",
            report.profile.available()
        );
        self.risk = Some((id, report));
    }
    pub(super) fn changes_view(
        &self,
        detail: &MrDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.is_busy();
        let mut view = div()
            .debug_selector(|| "changes-view".into())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_4()
            .min_w_0()
            .child(heading(
                "Changes",
                Some("Risk · files · ownership · history".into()),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div().debug_selector(|| "calculate-risk".into()).child(
                            Button::new("calculate-risk")
                                .small()
                                .label("Calculate risk")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(detail) = this.detail.clone() {
                                        this.start(
                                            Action::Risk(detail),
                                            "Calculating risk locally…",
                                            cx,
                                        );
                                    }
                                })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "update-baseline".into()).child(
                            Button::new("update-baseline")
                                .small()
                                .ghost()
                                .label("Update team median")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(detail) = this.detail.clone() {
                                        this.start(
                                            Action::RiskBaseline(detail),
                                            "Measuring merged MR history…",
                                            cx,
                                        );
                                    }
                                })),
                        ),
                    ),
            );
        if let Some(report) = self.current_risk() {
            let baseline = self
                .risk_baseline
                .as_ref()
                .filter(|(id, _)| id.project == detail.mr.id.project)
                .map(|(_, b)| b);
            let p = &report.profile;
            let verdict = p
                .verdict(&report.verdict_thresholds)
                .map(|v| v.label())
                .unwrap_or("Unavailable");
            let note = |text: String| muted(text).text_color(cx.theme().muted_foreground);
            let mut info = div()
                .text_color(cx.theme().foreground)
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .child(
                            gpui_component::Icon::new(gpui_component::IconName::Info).size(px(16.)),
                        )
                        .child(format!(
                            "{verdict} review{}",
                            if p.available() < 8 { " · partial" } else { "" }
                        )),
                )
                .child(
                    div()
                        .text_size(px(22.))
                        .text_color(cx.theme().foreground)
                        .child(
                            p.mean()
                                .map(|n| format!("{n:.1} / 10"))
                                .unwrap_or_else(|| "n/a".into()),
                        ),
                )
                .child(note(format!(
                    "{}/8 axes · higher means more risk",
                    p.available()
                )))
                .child(note("Blue: this MR · dashed gray: team median".into()));
            if let Some(b) = baseline {
                info = info.child(note(format!(
                    "{} of {} requested merged MRs; sample counts vary by axis.",
                    b.merged_mrs, b.requested
                )));
                for (index, n) in b.samples.iter().enumerate() {
                    info = info.child(note(format!("{}: n = {n}", Axis::ALL[index].label())));
                }
                if !b.warnings.is_empty() {
                    info = info.child(note(format!(
                        "{} unavailable or partial historical measurements. See debug logs.",
                        b.warnings.len()
                    )));
                }
            } else {
                info=info.child(note("No team baseline yet. Update it explicitly; historical commits absent from the local repository are reported as unavailable.".into()));
            }
            view = view.child(
                card()
                    .bg(cx.theme().background)
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(super::radar::radar(p, baseline, cx))
                            .child(info),
                    ),
            );
            for pending in &report.pending_commands {
                let key = pending.key.clone();
                view = view.child(
                    card()
                        .child(format!("Proposed check: {}", pending.id))
                        .child(muted(format!("Command arguments: {:?}", pending.command)))
                        .child(
                            Button::new(SharedString::from(format!("allow-risk-{key}")))
                                .small()
                                .label("Allow and calculate")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(detail) = this.detail.clone() {
                                        this.start(
                                            Action::ApproveRiskCommand(detail, key.clone()),
                                            "Approving this command and calculating risk…",
                                            cx,
                                        );
                                    }
                                })),
                        ),
                );
            }
            if let Some(axis) = self.risk_axis {
                let score = &p.axes[axis.index()];
                let mut section = card()
                    .debug_selector(|| "risk-axis-evidence".into())
                    .child(heading(axis.label(), None));
                if let Some(raw) = &score.raw {
                    section = section.child(muted(raw.method.clone()));
                    for (index, item) in raw.evidence.iter().take(60).enumerate() {
                        let path = item.path.clone();
                        section = section.child(
                            div()
                                .id(("axis-evidence", index))
                                .cursor_pointer()
                                .child(path_line(&item.path, item.line))
                                .child(muted(item.detail.clone()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_risk_file(path.clone(), cx)
                                })),
                        );
                    }
                    if raw.evidence.len() > 60 {
                        section = section.child(muted(format!(
                            "Showing 60 of {} findings",
                            raw.evidence.len()
                        )));
                    }
                } else {
                    section = section.child(muted(score.unavailable.clone().unwrap_or_default()));
                }
                let report = report.clone();
                section = section.child(
                    Button::new("ask-risk")
                        .small()
                        .label("Ask Claude about this axis")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(detail) = this.detail.clone() {
                                this.start(
                                    Action::AskRisk(detail, axis, Box::new(report.clone())),
                                    "Preparing risk discussion…",
                                    cx,
                                );
                            }
                        })),
                );
                view = view.child(section);
            }
        } else {
            view=view.child(card().child(muted("Calculate an explainable risk profile for this commit. Missing tools produce n/a; Claude never starts automatically.")));
        }
        view = view.child(self.disktree_view(detail, window, cx));
        let files = self.file_stats(detail, window, cx);
        let mut list = card().child(heading(
            "Changed files",
            Some(format!("{} files · select a file for history", files.len())),
        ));
        for (index, file) in files.iter().enumerate() {
            let path = file.path.clone();
            list = list.child(
                div()
                    .id(("risk-file", index))
                    .debug_selector(move || format!("risk-file-{index}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .cursor_pointer()
                    .child(path_line(&file.path, None).flex_1())
                    .child(muted(if file.binary {
                        "binary".into()
                    } else {
                        format!("+{} −{}", file.added, file.removed)
                    }))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_risk_file(path.clone(), cx)),
                    ),
            );
        }
        if let Some(path) = &self.risk_selected_file {
            let mut history = card()
                .debug_selector(|| "risk-file-history".into())
                .child(path_line(path, None));
            if let Some((id, sha, file)) = &self.risk_file
                && *id == detail.mr.id
                && *sha == detail.mr.sha
                && file.path == *path
            {
                history = history.child(heading(
                    "Line ownership",
                    Some("git blame at this commit".into()),
                ));
                for (name, share) in &file.authors {
                    history = history.child(
                        div()
                            .flex()
                            .gap_3()
                            .child(format!("{name}: {share:.1}%"))
                            .child(
                                div().w(px(180.)).h(px(5.)).mt_2().bg(rgb(FIELD)).child(
                                    div()
                                        .w(relative((*share / 100.) as f32))
                                        .h_full()
                                        .bg(cx.theme().primary),
                                ),
                            ),
                    );
                }
                for warning in &file.warnings {
                    history = history.child(muted(warning.clone()));
                }
                if let Some(report) = self.current_risk() {
                    for score in &report.profile.axes {
                        if let Some(raw) = &score.raw {
                            let count = raw.evidence.iter().filter(|e| e.path == *path).count();
                            if count > 0 {
                                history = history.child(muted(format!(
                                    "{}: {count} findings · MR axis {:.1}/10",
                                    score.axis.label(),
                                    score.score.unwrap_or(0.)
                                )));
                            }
                        }
                    }
                }
                history = history.child(heading(
                    "File history",
                    Some("Most recent 30 commits".into()),
                ));
                for commit in &file.commits {
                    history = history.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_2()
                            .child(muted(format!(
                                "{} · {} · {}",
                                commit.date,
                                commit.author,
                                commit.sha.chars().take(8).collect::<String>()
                            )))
                            .child(crate::paragraph::paragraph(commit.subject.clone())),
                    );
                }
            } else {
                history = history.child(muted("Loading file history…"));
            }
            view = view.child(history);
        }
        view.child(list).into_any_element()
    }
}
