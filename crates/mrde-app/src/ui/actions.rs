//! Pinned reader actions and controls/results inside their corresponding tabs.
use super::{
    Cockpit, section_label, status_dot,
    widgets::{badge, card, muted, path_line},
};
use crate::{backend::Action, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Sizable, button::*};
use mrde_core::models::*;

fn block(title: &'static str, hint: String, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(13.))
                .text_color(rgb(HEADING))
                .child(title),
        )
        // A flex row keeps buttons at their natural width instead of stretching.
        .child(div().flex().child(control))
        .child(
            div()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(rgb(SUBTLE))
                .child(hint),
        )
}

impl Cockpit {
    pub(super) fn actions_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.is_busy() || self.confirm;
        div()
            .debug_selector(|| "mr-actions".into())
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().debug_selector(|| "run-review".into()).child(
                    Button::new("toolbar-review")
                        .small()
                        .h(px(32.))
                        .primary()
                        .icon(gpui_component::IconName::Bot)
                        .label(format!("Run {} review", self.level.label().to_lowercase()))
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(detail) = this.detail.clone() {
                                this.show_detail_tab(super::inbox::DetailTab::Review);
                                this.start(
                                    Action::Review(detail, this.level),
                                    "Running review: preparing worktree and Claude…",
                                    cx,
                                );
                            }
                        })),
                ),
            )
            .child(
                div().debug_selector(|| "analyze".into()).child(
                    Button::new("toolbar-analyze")
                        .small()
                        .h(px(32.))
                        .icon(gpui_component::IconName::ChartPie)
                        .label("Analyze")
                        .tooltip("Symbols, impact, history, contracts, and risky patterns from the worktree")
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(detail) = this.detail.clone() {
                                this.start(
                                    Action::Analyze(detail),
                                    "Analyzing the worktree: symbols, history, contracts…",
                                    cx,
                                );
                            }
                        })),
                ),
            )
            .child(
                div().debug_selector(|| "ask-claude".into()).child(
                    Button::new("ask-claude")
                        .small()
                        .h(px(32.))
                        .ghost()
                        .icon(gpui_component::IconName::SquareTerminal)
                        .label("Ask Claude")
                        .tooltip("Ask Claude Code about this merge request in Threads. Read-only: plan mode, nothing is edited or pushed.")
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(detail) = this.detail.clone() {
                                this.start(
                                    Action::Ask(detail, this.review.clone()),
                                    "Preparing the worktree and context for Claude Code…",
                                    cx,
                                );
                            }
                        })),
                ),
            )
            .child(div().debug_selector(|| "super-resolver".into()).child(
                Button::new("super-resolver").small().h(px(32.)).label("Super Resolver")
                    .tooltip("Rebase, resolve conflicts, and push pipeline fixes to the source branch. Up to three repair attempts.")
                    .disabled(self.initializing || self.confirm)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(detail) = &this.detail {
                            this.threads.update(cx, |threads, cx| threads.start(detail, cx));
                            this.threads_view = true; this.settings = false; this.worktrees_view = false;
                            cx.notify();
                        }
                    })),
            ))
            .child(
                div().debug_selector(|| "approve".into()).child(
                    Button::new("approve")
                        .small()
                        .h(px(32.))
                        .icon(gpui_component::IconName::Check)
                        .label("Approve")
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.interrupt_refresh();
                            this.confirm = true;
                            cx.notify();
                        })),
                ),
            )
            .child(
                Button::new("open-mr")
                    .small()
                    .h(px(32.))
                    .ghost()
                    .icon(gpui_component::IconName::ExternalLink)
                    .tooltip("Open in GitLab")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(detail) = &this.detail {
                            cx.open_url(&detail.mr.web_url);
                        }
                    })),
            )
            .into_any_element()
    }
    pub(super) fn approval_confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = &self.detail else {
            return div().into_any_element();
        };
        let busy = self.is_busy();
        div()
            .p_3()
            .rounded(px(RADIUS_CARD))
            .border_1()
            .border_color(rgb(WARNING))
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.))
            .child(format!(
                "Approve {} !{} at commit {}?",
                detail.mr.project,
                detail.mr.id.iid,
                detail.mr.sha.chars().take(12).collect::<String>()
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div().debug_selector(|| "confirm-approval".into()).child(
                            Button::new("confirm")
                                .small()
                                .primary()
                                .label("Confirm approval")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(detail) = this.detail.clone() {
                                        this.start(
                                            Action::Approve(detail),
                                            "Approving on GitLab…",
                                            cx,
                                        );
                                    }
                                })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "cancel-approval".into()).child(
                            Button::new("cancel-approval")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm = false;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .flex_shrink_0()
            .mx_3()
            .my_2()
            .into_any_element()
    }
    pub(super) fn review_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.is_busy() || self.confirm;
        let mut levels = div()
            .flex()
            .gap_1()
            .p(px(2.))
            .rounded(px(RADIUS_CARD))
            .bg(rgb(FIELD));
        for (index, level) in ReviewLevel::ALL.into_iter().enumerate() {
            let selected = self.level == level;
            levels = levels.child(
                div()
                    .debug_selector(move || format!("level-{}", level.label().to_lowercase()))
                    .child(
                        Button::new(("review-level", index))
                            .small()
                            .ghost()
                            .w(px(96.))
                            .label(level.label())
                            .when(selected, |b| b.bg(rgb(SELECTED)).text_color(rgb(HEADING)))
                            .disabled(locked)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.level = level;
                                cx.notify();
                            })),
                    ),
            );
        }
        let panel = div().child(block(
            "Review depth",
            format!(
                "{} Start Claude with the Run review button above.",
                self.level.description()
            ),
            levels,
        ));
        panel.into_any_element()
    }
    pub(super) fn statistics_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.is_busy() || self.confirm;
        let panel = div().child(block(
            "Statistics",
            "Exact line counts for the disktree, from git in the MR worktree. Reuses the worktree when it exists.".into(),
            div().debug_selector(|| "statistics".into()).child(
                Button::new("statistics")
                    .small()
                    .label("Compute statistics")
                    .disabled(locked)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(detail) = this.detail.clone() {
                            this.start(Action::Statistics(detail), "Computing statistics…", cx);
                        }
                    })),
            ),
        ));

        panel.into_any_element()
    }
    pub(super) fn compliance_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.is_busy() || self.confirm;
        let spec = self.config.spec_dir();
        let panel = div().child(block(
            "Compliance check",
            // The folder as configured (`~/…`), not expanded to the home directory.
            match &spec {
                Some(_) => format!(
                    "Checks the ticket and the change against the specification in {}.",
                    self.config.spec_path.trim()
                ),
                None => "Set the specification folder in GitLab settings first.".into(),
            },
            div().debug_selector(|| "compliance-check".into()).child(
                Button::new("compliance-check")
                    .small()
                    .label("Check compliance")
                    .disabled(locked || spec.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(detail) = this.detail.clone() {
                            this.start(
                                Action::Compliance(detail),
                                "Checking compliance with the specification…",
                                cx,
                            );
                        }
                    })),
            ),
        ));
        panel.into_any_element()
    }

    pub(super) fn compliance_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = &self.detail else {
            return div().into_any_element();
        };
        let Some((_, _, result)) = self
            .compliance
            .as_ref()
            .filter(|(id, sha, _)| *id == detail.mr.id && *sha == detail.mr.sha)
        else {
            return div().into_any_element();
        };
        let (verdict, color) = match result.verdict {
            ComplianceVerdict::Compliant => ("Compliant", SUCCESS),
            ComplianceVerdict::Partial => ("Partially compliant", WARNING),
            ComplianceVerdict::NonCompliant => ("Not compliant", ERROR),
            ComplianceVerdict::NoSpecFound => ("No specification found", SUBTLE),
        };
        let mut panel = div()
            .debug_selector(|| "compliance-result".into())
            .min_w_0()
            .border_t_1()
            .border_color(rgb(LINE))
            .pt_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(HEADING))
                    .child(status_dot(color, false))
                    .child(format!("Compliance · {verdict}")),
            )
            .child(crate::markdown::view(
                SharedString::from(format!(
                    "compliance-{}-{}-{}",
                    detail.mr.id.project, detail.mr.id.iid, detail.mr.sha
                )),
                &result.summary,
                Some((&self.config.gitlab_url, &detail.mr.web_url)),
                window,
                cx,
            ))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(format!("Confidence  {:.0}%", result.confidence * 100.)),
            );
        if !result.spec_files.is_empty() {
            panel = panel.child(section_label("Specification"));
            for file in &result.spec_files {
                panel = panel.child(
                    div()
                        .font_family(CODE_FONT)
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(file.clone()),
                );
            }
        }
        if !result.findings.is_empty() {
            let count = |status: RequirementStatus| {
                result
                    .findings
                    .iter()
                    .filter(|f| f.status == status)
                    .count()
            };
            let untested = result
                .findings
                .iter()
                .filter(|f| f.status == RequirementStatus::Met && f.tests.is_empty())
                .count();
            panel = panel
                .child(section_label("Traceability"))
                .child(muted(format!(
                    "{} met · {} partial · {} missing · {} unclear · {untested} met without a test",
                    count(RequirementStatus::Met),
                    count(RequirementStatus::Partial),
                    count(RequirementStatus::Missing),
                    count(RequirementStatus::Unclear),
                )));
            // One block per requirement with labeled Spec/Code/Tests lines: fixed table
            // columns left file names unreadable in a ~760 pt reading column.
            let labeled = |label: &'static str, lines: Vec<AnyElement>| {
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .w(px(44.))
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .line_height(px(18.))
                            .text_color(rgb(SUBTLE))
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(lines),
                    )
            };
            let mut matrix = card()
                .debug_selector(|| "traceability".into())
                .p_0()
                .gap_0();
            for (index, finding) in result.findings.iter().enumerate() {
                let (status, color) = match finding.status {
                    RequirementStatus::Met => ("met", SUCCESS),
                    RequirementStatus::Partial => ("partial", WARNING),
                    RequirementStatus::Missing => ("missing", ERROR),
                    RequirementStatus::Unclear => ("unclear", SUBTLE),
                };
                let code: Vec<AnyElement> = finding
                    .code
                    .iter()
                    .take(6)
                    .map(|location| {
                        let (path, line) = match location.rsplit_once(':') {
                            Some((path, line)) if line.parse::<u32>().is_ok() => {
                                (path, line.parse().ok())
                            }
                            _ => (location.as_str(), None),
                        };
                        path_line(path, line).into_any_element()
                    })
                    .collect();
                let tests: Vec<AnyElement> = if finding.tests.is_empty() {
                    vec![if finding.status == RequirementStatus::Met {
                        div()
                            .flex()
                            .child(badge("no test", WARNING))
                            .into_any_element()
                    } else {
                        muted("—").into_any_element()
                    }]
                } else {
                    finding
                        .tests
                        .iter()
                        .take(6)
                        .map(|test| path_line(test, None).into_any_element())
                        .collect()
                };
                matrix = matrix.child(
                    div()
                        .debug_selector(move || format!("requirement-{index}"))
                        .px_3()
                        .py_2()
                        .when(index > 0, |row| row.border_t_1().border_color(rgb(LINE)))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(rgb(HEADING))
                                        .child(finding.requirement.clone()),
                                )
                                .child(div().flex_shrink_0().child(badge(status, color))),
                        )
                        .child(muted(finding.evidence.clone()))
                        .children(finding.spec_file.as_ref().map(|file| {
                            labeled("Spec", vec![path_line(file, None).into_any_element()])
                        }))
                        .child(labeled(
                            "Code",
                            if code.is_empty() {
                                vec![muted("—").into_any_element()]
                            } else {
                                code
                            },
                        ))
                        .child(labeled("Tests", tests)),
                );
            }
            panel = panel.child(matrix);
        }
        panel.into_any_element()
    }
}
