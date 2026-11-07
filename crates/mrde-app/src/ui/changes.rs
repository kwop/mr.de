//! Changes tab: an explainable risk profile, the changed files with their findings, and each
//! file's owners and history, in the presentation of the other analysis tabs.
use super::{
    Cockpit,
    radar::{self, explanation, raw_text, severity},
    status_dot,
    widgets::{CARD, badge, card, code, count, heading, muted, path_line},
};
use crate::{backend::Action, ellipsis::ellipsis, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Icon, IconName, Sizable, button::*};
use mrde_core::risk::Axis;
use mrde_core::{models::*, risk::*};
use std::collections::BTreeMap;

/// Axes whose findings point at files; size and file count describe the whole MR.
const FILE_AXES: [Axis; 6] = [
    Axis::Complexity,
    Axis::TestGap,
    Axis::SensitivePaths,
    Axis::Coupling,
    Axis::Lint,
    Axis::Debt,
];
/// Files listed before pointing to the Files tab.
const LISTED: usize = 60;
/// Evidence lines shown for one axis.
const EVIDENCE: usize = 60;

fn verdict_color(verdict: Verdict) -> u32 {
    match verdict {
        Verdict::Deep => ERROR,
        Verdict::Standard => WARNING,
        Verdict::Quick => SUCCESS,
    }
}

/// Test-gap evidence also lists tested files, which are not findings.
fn finding(axis: Axis, evidence: &Evidence) -> bool {
    axis != Axis::TestGap || evidence.contribution > 0.
}

/// Short chip text for a file's findings on one axis.
fn chip(axis: Axis, findings: usize) -> String {
    match axis {
        Axis::TestGap => "untested".into(),
        Axis::SensitivePaths => "sensitive".into(),
        Axis::Complexity => "complexity".into(),
        Axis::Coupling => crate::tr!("coupling {findings}", findings = findings),
        Axis::Lint => crate::tr!("lint {findings}", findings = findings),
        Axis::Debt => crate::tr!("debt {findings}", findings = findings),
        Axis::DiffSize | Axis::Files | Axis::TicketAge => crate::i18n::t(axis.label()).into(),
    }
}

/// Why the profile asks for this review depth.
fn reason(profile: &RiskProfile, thresholds: &VerdictThresholds) -> String {
    let high: Vec<String> = profile
        .axes
        .iter()
        .filter_map(|a| Some((a.axis, a.score?)))
        .filter(|(_, score)| *score >= thresholds.deep_axis)
        .map(|(axis, score)| format!("{} {score:.1}", crate::i18n::t(axis.label())))
        .collect();
    let mean = profile.mean().unwrap_or_default();
    if !high.is_empty() {
        crate::tr!(
            "{value0} at or above the deep-review threshold ({value1})",
            value0 = high.join(", "),
            value1 = thresholds.deep_axis
        )
    } else if mean >= thresholds.deep_mean {
        crate::tr!("Mean at or above {value0}", value0 = thresholds.deep_mean)
    } else if mean >= thresholds.standard_mean {
        crate::tr!(
            "Mean at or above {value0}; no axis reaches {value1}",
            value0 = thresholds.standard_mean,
            value1 = thresholds.deep_axis
        )
    } else {
        crate::tr!(
            "Mean below {value0}; no axis reaches {value1}",
            value0 = thresholds.standard_mean,
            value1 = thresholds.deep_axis
        )
    }
}

/// Section title inside a card: smaller than [`heading`], with a right-aligned hint.
fn subheading(title: &'static str, hint: Option<String>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(13.))
                .text_color(rgb(HEADING))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
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

/// Axes with at most one finding per file, shown on the file's row.
fn per_file(axis: Axis) -> bool {
    matches!(
        axis,
        Axis::DiffSize | Axis::Files | Axis::Complexity | Axis::TestGap | Axis::SensitivePaths
    )
}

/// A per-file finding, beside the file's path.
fn file_summary(axis: Axis, item: &Evidence) -> Option<Div> {
    let note = |text: String| {
        div()
            .max_w(px(320.))
            .min_w_0()
            .one_line()
            .text_size(px(12.))
            .text_color(rgb(SUBTLE))
            .child(ellipsis(text))
    };
    let pair = |chip: Div, text: String| {
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .child(chip)
            .child(note(text))
    };
    let detail = item.detail.as_str();
    Some(match axis {
        Axis::TestGap if item.contribution > 0. => pair(
            badge(crate::i18n::t("untested"), WARNING),
            count(item.contribution as usize, "added line", "added lines"),
        ),
        Axis::TestGap => pair(
            badge(crate::i18n::t("tested"), SUCCESS),
            detail
                .strip_prefix("Tests changed ")
                .unwrap_or(detail)
                .to_string(),
        ),
        Axis::SensitivePaths => match detail.split_once(": ") {
            Some((pattern, rest)) => pair(badge(pattern.to_string(), ERROR), rest.to_string()),
            None => note(detail.into()),
        },
        Axis::Complexity => div().flex_shrink_0().child(badge(
            crate::tr!("{value0} CCN", value0 = format!("{:+}", item.contribution)),
            if item.contribution > 0. {
                WARNING
            } else {
                SUCCESS
            },
        )),
        Axis::DiffSize => {
            let (added, removed) = detail.strip_prefix('+')?.split_once(" −")?;
            div()
                .flex()
                .flex_shrink_0()
                .gap_2()
                .font_family(CODE_FONT)
                .text_size(px(12.))
                .child(div().text_color(rgb(SUCCESS)).child(format!("+{added}")))
                .child(div().text_color(rgb(ERROR)).child(format!("−{removed}")))
        }
        _ => return None,
    })
}

/// One evidence line: its location, then the finding drawn like the other tabs draw code.
fn evidence_line(axis: Axis, item: &Evidence) -> Div {
    let text = |value: String, color: u32| {
        div()
            .flex_1()
            .min_w_0()
            .one_line()
            .text_size(px(12.))
            .text_color(rgb(color))
            .child(ellipsis(value))
    };
    let added = |value: &str| {
        div()
            .flex_1()
            .min_w_0()
            .px_2()
            .rounded(px(4.))
            .bg(rgb(SUCCESS_BG))
            .child(code(format!("+ {}", value.trim()), SUCCESS))
    };
    let detail = item.detail.as_str();
    let content = match axis {
        Axis::Debt => match detail.split_once(": ") {
            Some((rule, line)) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(badge(rule.to_string(), WARNING))
                .child(added(line)),
            None => text(detail.into(), MUTED),
        },
        Axis::Coupling => {
            let (kind, color, name) = match detail.split_once(": ") {
                Some(("Added dependency", name)) => ("dependency", WARNING, name),
                Some((_, name)) => ("import", INFO, name),
                None => ("import", INFO, detail),
            };
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(badge(kind, color))
                .child(code(name.to_string(), INK))
        }
        Axis::Lint => match detail.split_once(": ") {
            Some((rule, message)) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(badge(rule.to_string(), WARNING))
                .child(text(message.into(), INK)),
            None => text(detail.into(), INK),
        },
        _ => text(detail.into(), MUTED),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .w(px(44.))
                .flex_shrink_0()
                .font_family(CODE_FONT)
                .text_size(px(11.))
                .text_color(rgb(SUBTLE))
                .child(item.line.map(|line| format!("L{line}")).unwrap_or_default()),
        )
        .child(content.flex_1().min_w_0())
}

impl Cockpit {
    pub(super) fn current_risk(&self) -> Option<&RiskReport> {
        let detail = self.detail.as_ref()?;
        self.risk
            .as_ref()
            .filter(|(id, r)| *id == detail.mr.id && r.profile.head_sha == detail.mr.sha)
            .map(|(_, r)| r.as_ref())
    }
    pub(super) fn toggle_risk_axis(&mut self, axis: Axis, cx: &mut Context<Self>) {
        self.risk_axis = (self.risk_axis != Some(axis)).then_some(axis);
        cx.notify();
    }
    /// Opens a file's owners and history under its row in the file list.
    pub(super) fn select_risk_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.open_risk_file(path, false, cx);
    }
    /// Disktree selection is navigation, not the inline row's open/close toggle.
    pub(super) fn jump_to_risk_file(
        &mut self,
        path: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.risk_selected_file.as_deref() != Some(&path) || self.risk_file_in_evidence {
            self.open_risk_file(path.clone(), false, cx);
        }
        if self.risk_selected_file.as_deref() == Some(&path) && !self.risk_file_in_evidence {
            self.detail_tab = super::inbox::DetailTab::Changes;
            self.risk_scroll_pending.set(true);
            cx.notify();
        }
    }
    /// Opens or closes a file's owners and history, under its row in the file list or, with
    /// `in_evidence`, under its findings in the axis evidence. Loads it once per commit.
    pub(super) fn open_risk_file(
        &mut self,
        path: String,
        in_evidence: bool,
        cx: &mut Context<Self>,
    ) {
        if self.risk_selected_file.as_deref() == Some(path.as_str())
            && self.risk_file_in_evidence == in_evidence
        {
            self.risk_selected_file = None;
            cx.notify();
            return;
        }
        let Some(detail) = self.detail.clone() else {
            return;
        };
        let loaded = self.risk_file.as_ref().is_some_and(|(id, sha, file)| {
            *id == detail.mr.id && *sha == detail.mr.sha && file.path == path
        });
        if !loaded && self.is_busy() {
            return;
        }
        self.risk_file_in_evidence = in_evidence;
        self.risk_selected_file = Some(path.clone());
        if loaded {
            cx.notify();
        } else {
            self.risk_file = None;
            self.start(
                Action::RiskFile(detail, path),
                "Loading file owners and history…",
                cx,
            );
        }
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
        self.status = crate::tr!(
            "Risk profile ready · {value0}/9 axes · unavailable axes are excluded from the mean.",
            value0 = report.profile.available()
        );
        for message in &report.setup_messages {
            self.log(message);
        }
        self.risk = Some((id, report));
    }

    pub(super) fn changes_view(
        &self,
        detail: &MrDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let report = self.current_risk();
        let mut view = div()
            .debug_selector(|| "changes-view".into())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_4()
            .min_w_0()
            .child(heading(
                crate::i18n::t("Changes"),
                Some(crate::i18n::t("Risk profile · changed files · owners and history").into()),
            ));
        match report {
            Some(report) => {
                view = view.child(self.risk_controls(cx));
                for message in &report.setup_messages {
                    view = view.child(card().child(muted(message.clone())));
                }
                for pending in &report.pending_commands {
                    view = view.child(self.pending_command(pending, cx));
                }
                view = view.child(self.risk_summary(detail, report, cx));
                if let Some(axis) = self.risk_axis {
                    view = view.child(self.axis_evidence(detail, report, axis, cx));
                }
            }
            None => {
                view = view.child(
                    card()
                        .debug_selector(|| "risk-prompt".into())
                        .child(muted(
                            crate::i18n::t("Calculate an explainable risk profile of this commit: size, files, complexity, tests, sensitive paths, coupling, warnings, debt, and ticket age."),
                        ))
                        .child(muted(
                            crate::i18n::t("It runs locally in the MR worktree (created if missing). Missing tools leave an axis n/a, never zero. Claude never starts automatically."),
                        ))
                        .child(div().flex().gap_2().flex_wrap().child(self.calculate_button(cx)).child(self.setup_tools_button(cx))),
                );
            }
        }
        view.child(self.disktree_view(detail, window, cx))
            .child(self.changed_files(detail, report, window, cx))
            .child(
                div()
                    .debug_selector(|| "changes-end".into())
                    .h(px(1.))
                    .flex_shrink_0(),
            )
            .into_any_element()
    }

    fn setup_tools_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div().debug_selector(|| "setup-risk-tools".into()).child(
            Button::new("setup-risk-tools").small().label(crate::i18n::t("Detect and install tools"))
                .disabled(self.is_busy())
                .tooltip(crate::i18n::t("Detect linters for the changed code, install supported tools, and prepare approved checks. Downloads may be required."))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(detail) = this.detail.clone() {
                        this.start(Action::SetupRiskTools(detail), "Detecting and installing risk tools…", cx);
                    }
                }))
        ).into_any_element()
    }

    fn calculate_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .debug_selector(|| "calculate-risk".into())
            .child(
                Button::new("calculate-risk")
                    .small()
                    .label(if self.current_risk().is_some() {
                        crate::i18n::t("Calculate again")
                    } else {
                        crate::i18n::t("Calculate risk")
                    })
                    .disabled(self.is_busy())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(detail) = this.detail.clone() {
                            this.start(Action::Risk(detail), "Calculating risk locally…", cx);
                        }
                    })),
            )
            .into_any_element()
    }

    fn risk_controls(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .child(
                div().debug_selector(|| "update-baseline".into()).child(
                    Button::new("update-baseline")
                        .small()
                        .ghost()
                        .label(crate::i18n::t("Update team median"))
                        .disabled(self.is_busy())
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
            )
            .child(self.setup_tools_button(cx))
            .child(self.calculate_button(cx))
    }

    fn pending_command(&self, pending: &PendingCommand, cx: &mut Context<Self>) -> AnyElement {
        let key = pending.key.clone();
        card()
            .debug_selector(|| "risk-pending-command".into())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(badge(crate::i18n::t("proposed"), WARNING))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(INK))
                            .child(crate::tr!("Proposed check: {value0}", value0 = pending.id)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("allow-risk-{key}")))
                            .xsmall()
                            .label(crate::i18n::t("Allow, install and calculate"))
                            .disabled(self.is_busy())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(detail) = this.detail.clone() {
                                    this.start(
                                        Action::ApproveRiskCommand(detail, key.clone()),
                                        "Preparing the approved check and calculating risk…",
                                        cx,
                                    );
                                }
                            })),
                    ),
            )
            .child(code(pending.command.join(" "), INK))
            .when(!pending.directory.is_empty(), |card| card.child(muted(crate::tr!("Package: {value0}", value0 = pending.directory))))
            .when_some(pending.preparation.clone(), |card, preparation| card.child(muted(crate::tr!("Dependency setup: {preparation}. Locked dependencies are prepared in isolated snapshots; downloads may be required.", preparation = preparation))))
            .child(muted(
                crate::i18n::t("It runs without a shell in disposable snapshots of the base and head commits, only after you allow this exact command."),
            ))
            .into_any_element()
    }

    fn risk_summary(
        &self,
        detail: &MrDetail,
        report: &RiskReport,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let baseline = self
            .risk_baseline
            .as_ref()
            .filter(|(id, _)| id.project == detail.mr.id.project)
            .map(|(_, b)| b);
        let profile = &report.profile;
        let thresholds = &report.verdict_thresholds;
        let verdict = profile.verdict(thresholds);
        let color = verdict.map_or(SUBTLE, verdict_color);
        let partial = profile.available() < Axis::COUNT;
        let missing: Vec<&str> = profile
            .axes
            .iter()
            .filter(|a| a.score.is_none())
            .map(|a| crate::i18n::t(a.axis.label()))
            .collect();
        let legend = |swatch: Div, text: String| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(
                    div()
                        .w(px(18.))
                        .flex_shrink_0()
                        .flex()
                        .justify_center()
                        .child(swatch),
                )
                .child(text)
        };
        let dashes = div()
            .flex()
            .gap(px(2.))
            .children((0..3).map(|_| div().w(px(4.)).h(px(2.)).bg(rgb(MUTED))));
        let mut info = div()
            .flex_1()
            .min_w(px(260.))
            .pt_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "risk-verdict".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(10.)).rounded_full().bg(rgb(color)))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(color))
                            .child(match verdict {
                                Some(v) => crate::tr!("{value0} review", value0 = crate::i18n::t(v.label())),
                                None => "Review depth unknown".into(),
                            }),
                    )
                    .when(partial, |row| row.child(badge(crate::i18n::t("partial"), SUBTLE))),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(30.))
                            .line_height(px(34.))
                            .text_color(rgb(HEADING))
                            .child(profile.mean().map_or("n/a".into(), |m| format!("{m:.1}"))),
                    )
                    .child(
                        div()
                            .pb_1()
                            .text_size(px(13.))
                            .text_color(rgb(SUBTLE))
                            .child(crate::tr!("/ 10 · mean of {value0} axes", value0 = profile.available())),
                    ),
            )
            .child(muted(reason(profile, thresholds)))
            .when(partial, |info| {
                info.child(muted(crate::tr!("Not measured: {value0}. Unknown axes are left out of the mean, never counted as zero.", value0 = missing.join(", "))))
            })
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(legend(
                        div().w(px(16.)).h(px(2.)).bg(rgb(INFO)),
                        crate::i18n::t("This MR").into(),
                    ))
                    .child(legend(
                        dashes,
                        match baseline {
                            Some(b) => crate::tr!("Team median of {value0} merged MRs", value0 = b.merged_mrs),
                            None => crate::i18n::t("Team median: not measured yet").into(),
                        },
                    ))
                    .child(legend(
                        div()
                            .size(px(10.))
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(ERROR)),
                        crate::tr!("Deep-review threshold ({value0})", value0 = thresholds.deep_axis),
                    )),
            );
        match baseline {
            Some(b) => {
                info = info.child(muted(crate::tr!("Median saved {value0} · updates reuse unchanged commit profiles", value0 = b.computed_at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M"))));
                // Axes never measured (no lint command, say) are already n/a.
                let fewest = b.samples.iter().copied().filter(|n| *n > 0).min();
                if let Some(fewest) = fewest.filter(|n| *n < b.merged_mrs) {
                    info = info.child(muted(crate::tr!("Each axis is measured on {fewest} to {value0} of them.", fewest = fewest, value0 = b.samples.iter().copied().max().unwrap_or_default())));
                }
                if !b.warnings.is_empty() {
                    info = info.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(WARNING))
                            .child(crate::tr!("{value0} unavailable historical measurements: see Debug logs.", value0 = b.warnings.len())),
                    );
                }
            }
            None => {
                info = info.child(muted(
                    crate::i18n::t("Update team median measures uncached merged MRs locally and saves the results. Later updates reuse unchanged commit profiles."),
                ))
            }
        }
        info = info.child(muted(crate::tr!(
            "Measured {value0} at {value1}",
            value0 = profile
                .computed_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M"),
            value1 = profile.head_sha.chars().take(8).collect::<String>()
        )));
        let mut rows = div()
            .debug_selector(|| "risk-axes".into())
            .border_t_1()
            .border_color(rgb(LINE))
            .pt_2()
            .flex()
            .flex_col();
        for axis in Axis::ALL {
            rows = rows.child(self.axis_row(profile, baseline, thresholds, axis, cx));
        }
        card()
            .debug_selector(|| "risk-summary".into())
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_4()
                    .child(radar::radar(
                        profile,
                        baseline,
                        thresholds,
                        self.risk_axis,
                        cx,
                    ))
                    .child(info),
            )
            .child(rows)
            .into_any_element()
    }

    /// An axis as a table row: severity chip, label, score bar with the median tick, value.
    fn axis_row(
        &self,
        profile: &RiskProfile,
        baseline: Option<&Baseline>,
        thresholds: &VerdictThresholds,
        axis: Axis,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let score = &profile.axes[axis.index()];
        let median = baseline.and_then(|b| b.medians[axis.index()]);
        let color = score.score.map_or(SUBTLE, |s| severity(s, thresholds));
        let selected = self.risk_axis == Some(axis);
        let tooltip = explanation(score, median);
        let measure: AnyElement = match score.score {
            Some(value) => div()
                .flex_1()
                .min_w(px(80.))
                .h(px(12.))
                .relative()
                .flex()
                .items_center()
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(rgb(FIELD))
                        .child(
                            div()
                                .w(relative((value / 10.).clamp(0., 1.) as f32))
                                .h_full()
                                .rounded(px(3.))
                                .bg(rgb(color)),
                        ),
                )
                .when_some(median, |bar, median| {
                    bar.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative((median / 10.).clamp(0., 1.) as f32))
                            .w(px(2.))
                            .bg(rgb(MUTED)),
                    )
                })
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w(px(80.))
                .one_line()
                .text_size(px(12.))
                .text_color(rgb(SUBTLE))
                .child(ellipsis(
                    score
                        .unavailable
                        .as_deref()
                        .map(crate::i18n::text)
                        .unwrap_or_else(|| crate::i18n::t("Not measured").into()),
                ))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!("risk-row-{}", axis.id())))
            .debug_selector(move || format!("risk-row-{}", axis.id()))
            .flex()
            .items_center()
            .gap_3()
            .h(px(30.))
            .px_2()
            .rounded(px(RADIUS_CHIP))
            .cursor_pointer()
            .hover(|row| row.bg(rgb(HOVER)))
            .when(selected, |row| row.bg(rgb(SELECTED)))
            .child(status_dot(color, score.score.is_none()).size(px(8.)))
            .child(
                div()
                    .w(px(128.))
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .text_color(rgb(if selected { HEADING } else { INK }))
                    .child(crate::i18n::t(crate::i18n::t(axis.label()))),
            )
            .child(
                div()
                    .w(px(32.))
                    .flex_shrink_0()
                    .font_family(CODE_FONT)
                    .text_size(px(12.))
                    .text_color(rgb(color))
                    .child(score.score.map_or("n/a".into(), |s| format!("{s:.1}"))),
            )
            .child(measure)
            .child(
                div()
                    .w(px(150.))
                    .flex_shrink_0()
                    .one_line()
                    .justify_end()
                    .text_size(px(12.))
                    .text_color(rgb(SUBTLE))
                    .child(ellipsis(
                        score.raw.as_ref().map(raw_text).unwrap_or_default(),
                    )),
            )
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_risk_axis(axis, cx)))
            .into_any_element()
    }

    fn axis_evidence(
        &self,
        detail: &MrDetail,
        report: &RiskReport,
        axis: Axis,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let score = &report.profile.axes[axis.index()];
        let color = score
            .score
            .map_or(SUBTLE, |s| severity(s, &report.verdict_thresholds));
        let busy = self.is_busy();
        let ask = {
            let report = report.clone();
            Button::new("ask-risk")
                .xsmall()
                .ghost()
                .icon(Icon::new(IconName::Bot).text_color(rgb(CLAUDE)))
                .label(crate::i18n::t("Ask Claude about this axis"))
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(detail) = this.detail.clone() {
                        this.start(
                            Action::AskRisk(detail, axis, Box::new(report.clone())),
                            "Preparing risk discussion…",
                            cx,
                        );
                    }
                }))
        };
        let mut section = card().debug_selector(|| "risk-axis-evidence".into()).child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().size(px(10.)).rounded_full().bg(rgb(color)))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(15.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(HEADING))
                        .child(crate::i18n::t(crate::i18n::t(axis.label()))),
                )
                .child(badge(
                    score.score.map_or("n/a".into(), |s| format!("{s:.1} / 10")),
                    color,
                ))
                .when_some(score.raw.as_ref(), |row, raw| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(rgb(SUBTLE))
                            .child(raw_text(raw)),
                    )
                    .when(raw.estimated, |row| {
                        row.child(badge(crate::i18n::t("estimate"), SUBTLE))
                    })
                })
                .child(div().flex_1())
                .child(div().flex_shrink_0().child(ask))
                .child(
                    div().flex_shrink_0().child(
                        Button::new("close-risk-axis")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(crate::i18n::t("Close"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.toggle_risk_axis(axis, cx)),
                            ),
                    ),
                ),
        );
        let Some(raw) = &score.raw else {
            return section
                .child(muted(
                    score
                        .unavailable
                        .as_deref()
                        .map(crate::i18n::text)
                        .unwrap_or_else(|| crate::i18n::t("Not measured").into()),
                ))
                .into_any_element();
        };
        section = section.child(muted(raw.method.clone()));
        if axis == Axis::TicketAge {
            for issue in primary_tickets(&detail.mr, &detail.issues) {
                let url = issue.web_url.clone();
                section = section.child(
                    Button::new(SharedString::from(format!("risk-ticket-{url}")))
                        .small()
                        .ghost()
                        .label(issue.title.clone())
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            return section.into_any_element();
        }
        // Files with the largest contribution first; their lines in file order.
        let mut groups: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
        for item in &raw.evidence {
            groups.entry(item.path.as_str()).or_default().push(item);
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|a, b| {
            let total = |items: &[&Evidence]| items.iter().map(|e| e.contribution).sum::<f64>();
            total(&b.1).total_cmp(&total(&a.1)).then(a.0.cmp(b.0))
        });
        if groups.is_empty() {
            return section
                .child(muted(crate::i18n::t(
                    "No findings: nothing in this change raises this axis.",
                )))
                .into_any_element();
        }
        let mut list = div()
            .rounded(px(RADIUS_CARD))
            .border_1()
            .border_color(rgb(LINE))
            .flex()
            .flex_col();
        let mut shown = 0;
        for (index, (path, mut items)) in groups.into_iter().enumerate() {
            if shown >= EVIDENCE {
                break;
            }
            items.sort_by_key(|e| e.line);
            let findings = items.iter().filter(|e| finding(axis, e)).count();
            let open =
                self.risk_file_in_evidence && self.risk_selected_file.as_deref() == Some(path);
            let file = path.to_string();
            let mut group = div()
                .px_3()
                .py_2()
                .when(index > 0, |g| g.border_t_1().border_color(rgb(LINE)))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .id(("risk-evidence-file", index))
                        .debug_selector(move || format!("risk-evidence-file-{index}"))
                        .flex()
                        .items_center()
                        .gap_3()
                        .cursor_pointer()
                        .child(
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(14.))
                            .text_color(rgb(SUBTLE)),
                        )
                        .child(path_line(path, None).flex_1())
                        .when(per_file(axis), |row| {
                            row.children(items.iter().filter_map(|e| file_summary(axis, e)))
                        })
                        .when(!per_file(axis), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(12.))
                                    .text_color(rgb(SUBTLE))
                                    .child(count(findings, "finding", "findings")),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_risk_file(file.clone(), true, cx)
                        })),
                );
            if per_file(axis) {
                shown += 1;
            } else {
                for item in items.into_iter().take(EVIDENCE - shown) {
                    shown += 1;
                    group = group.child(evidence_line(axis, item).pl(px(22.)));
                }
            }
            if open {
                group = group.child(self.file_panel(detail, path, Some(report), cx));
            }
            list = list.child(group);
        }
        section = section.child(list);
        if raw.evidence.len() > shown {
            section = section.child(muted(crate::tr!(
                "Showing {shown} of {value0} findings",
                shown = shown,
                value0 = raw.evidence.len()
            )));
        }
        section.into_any_element()
    }

    fn changed_files(
        &self,
        detail: &MrDetail,
        report: Option<&RiskReport>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cache = self.disktree_cached(detail, window, cx);
        let stat = |path: &str| {
            cache
                .as_ref()
                .and_then(|c| c.lookup.get(path).map(|i| c.files[*i].clone()))
        };
        let widest = cache.as_ref().map_or(1, |c| c.widest);
        // Findings per file and axis, colored by that axis's severity.
        let mut findings: BTreeMap<&str, Vec<(Axis, usize, u32)>> = BTreeMap::new();
        if let Some(report) = report {
            for axis in FILE_AXES {
                let score = &report.profile.axes[axis.index()];
                let (Some(raw), Some(value)) = (&score.raw, score.score) else {
                    continue;
                };
                let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
                for item in raw.evidence.iter().filter(|e| finding(axis, e)) {
                    *counts.entry(item.path.as_str()).or_default() += 1;
                }
                let color = severity(value, &report.verdict_thresholds);
                for (path, n) in counts {
                    findings.entry(path).or_default().push((axis, n, color));
                }
            }
        }
        let mut files: Vec<(&ChangedFile, Option<FileStat>)> = detail
            .files
            .iter()
            .map(|file| (file, stat(&file.new_path)))
            .collect();
        files.sort_by_key(|(_, stat)| std::cmp::Reverse(stat.as_ref().map_or(0, |s| s.changed())));
        let selected = self
            .risk_selected_file
            .as_deref()
            .filter(|_| !self.risk_file_in_evidence);
        let mut list = div()
            .debug_selector(|| "risk-files".into())
            .flex()
            .flex_col()
            .min_w_0();
        // A file selected in the disktree but not listed opens above the list.
        if let Some(path) = selected
            && !files
                .iter()
                .take(LISTED)
                .any(|(file, _)| file.new_path == path)
        {
            list = list.child(self.file_panel(detail, path, report, cx));
        }
        for (index, (file, stat)) in files.iter().take(LISTED).enumerate() {
            let open = selected == Some(file.new_path.as_str());
            let mut marks: Vec<AnyElement> = findings
                .get(file.new_path.as_str())
                .into_iter()
                .flatten()
                .map(|(axis, n, color)| badge(chip(*axis, *n), *color).into_any_element())
                .collect();
            if report.is_some_and(|r| r.excluded.contains(&file.new_path)) {
                marks.push(badge(crate::i18n::t("not scored"), SUBTLE).into_any_element());
            }
            let path = file.new_path.clone();
            list = list.child(
                div()
                    .id(("risk-file", index))
                    .debug_selector(move || format!("risk-file-{index}"))
                    .px_2()
                    .cursor_pointer()
                    .hover(|row| row.bg(rgb(HOVER)))
                    .when(open, |row| row.bg(rgb(SELECTED)))
                    .child(Self::file_row(file, stat.as_ref(), widest, index, marks))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_risk_file(path.clone(), cx)),
                    ),
            );
            if open {
                list = list.child(self.file_panel(detail, &file.new_path, report, cx));
            }
        }
        if files.len() > LISTED {
            list = list.child(div().pt_2().child(muted(crate::tr!(
                "{value0} smaller files are in the Files tab.",
                value0 = files.len() - LISTED
            ))));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child(heading(
                crate::i18n::t("Changed files"),
                Some(crate::tr!(
                    "{value0} · largest first · click a file for its owners and history",
                    value0 = count(files.len(), "file", "files")
                )),
            ))
            .child(list)
            .into_any_element()
    }

    /// A file's findings, line owners, and recent commits, styled like the History tab.
    fn file_panel(
        &self,
        detail: &MrDetail,
        path: &str,
        report: Option<&RiskReport>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pending = self.risk_scroll_pending.clone();
        let scroll = self.detail_scroll.clone();
        let mut panel = div()
            .on_children_prepainted(move |bounds, window, cx| {
                if pending.get()
                    && let Some(first) = bounds.first()
                {
                    pending.set(false);
                    // Align the first content row just below the reader's top after
                    // layout. ScrollAnchor uses native frame callbacks, which the
                    // GPUI test platform does not deliver; defer works in both modes.
                    let offset = point(
                        px(0.),
                        scroll.offset().y + scroll.bounds().top() - first.top() + px(16.),
                    );
                    let scroll = scroll.clone();
                    window.defer(cx, move |window, _| {
                        scroll.set_offset(offset);
                        window.refresh();
                    });
                }
            })
            .id(SharedString::from(format!("risk-history-{path}")))
            .debug_selector(|| "risk-file-history".into())
            .my_2()
            .ml(px(22.))
            .p_3()
            .rounded(px(RADIUS_CARD))
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .gap_3()
            .min_w_0();
        let mut chips = div().flex().flex_wrap().items_center().gap_2();
        let mut any = false;
        if let Some(report) = report {
            for axis in FILE_AXES {
                let score = &report.profile.axes[axis.index()];
                let (Some(raw), Some(value)) = (&score.raw, score.score) else {
                    continue;
                };
                let n = raw
                    .evidence
                    .iter()
                    .filter(|e| e.path == path && finding(axis, e))
                    .count();
                if n == 0 {
                    continue;
                }
                any = true;
                chips = chips.child(
                    div()
                        .id(SharedString::from(format!("file-axis-{}", axis.id())))
                        .cursor_pointer()
                        .child(badge(
                            format!("{} · {n}", crate::i18n::t(axis.label())),
                            severity(value, &report.verdict_thresholds),
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.risk_axis = Some(axis);
                            cx.notify();
                        })),
                );
            }
        }
        panel = panel.child(subheading(
            "Findings",
            Some(if report.is_none() {
                "Calculate risk to see this file's findings".into()
            } else if any {
                "Click an axis for its evidence".into()
            } else {
                "No risk finding in this file".into()
            }),
        ));
        if any {
            panel = panel.child(chips);
        }
        let Some((_, _, file)) = self.risk_file.as_ref().filter(|(id, sha, file)| {
            *id == detail.mr.id && *sha == detail.mr.sha && file.path == path
        }) else {
            return panel
                .child(muted(crate::i18n::t("Loading owners and history…")))
                .into_any_element();
        };
        let mut owners = div().flex().flex_wrap().items_center().gap_x_4().gap_y_1();
        for (name, share) in &file.authors {
            owners = owners.child(
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
                            .child(format!("{share:.0}%")),
                    ),
            );
        }
        panel = panel
            .child(subheading(
                "Line owners",
                Some("git blame at the MR head".into()),
            ))
            .child(if file.authors.is_empty() {
                muted(crate::i18n::t("No owner information.")).into_any_element()
            } else {
                owners.into_any_element()
            });
        let mine = file.commits.iter().filter(|c| c.in_mr).count();
        panel = panel.child(subheading(
            "Recent commits",
            Some(if mine > 0 {
                crate::tr!(
                    "{value0} · {mine} in this MR",
                    value0 = count(file.commits.len(), "commit", "commits"),
                    mine = mine
                )
            } else {
                count(file.commits.len(), "commit", "commits")
            }),
        ));
        let mut commits = div()
            .rounded(px(RADIUS_CARD))
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(CARD))
            .flex()
            .flex_col();
        for (index, commit) in file.commits.iter().enumerate() {
            commits = commits.child(
                div()
                    .px_3()
                    .h(px(30.))
                    .when(index > 0, |row| row.border_t_1().border_color(rgb(LINE)))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .font_family(CODE_FONT)
                            .text_size(px(11.))
                            .text_color(rgb(SUBTLE))
                            .child(commit.sha.chars().take(8).collect::<String>()),
                    )
                    .when(commit.in_mr, |row| {
                        row.child(badge(crate::i18n::t("this MR"), INFO))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .one_line()
                            .text_size(px(13.))
                            .text_color(rgb(INK))
                            .child(ellipsis(commit.subject.clone())),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(rgb(SUBTLE))
                            .child(format!("{} · {}", commit.author, commit.date)),
                    ),
            );
        }
        panel = panel.child(if file.commits.is_empty() {
            muted(crate::i18n::t("No commits found for this file.")).into_any_element()
        } else {
            commits.into_any_element()
        });
        for warning in &file.warnings {
            panel = panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(WARNING))
                    .child(warning.clone()),
            );
        }
        panel.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(scores: [Option<f64>; mrde_core::risk::Axis::COUNT]) -> RiskProfile {
        RiskProfile {
            axes: std::array::from_fn(|i| AxisScore {
                axis: Axis::ALL[i],
                score: scores[i],
                raw: None,
                unavailable: None,
            }),
            computed_at: chrono::Utc::now(),
            head_sha: "a".into(),
            base_sha: "b".into(),
            config_key: "test".into(),
        }
    }
    #[::core::prelude::v1::test]
    fn verdict_reasons_name_the_axes_or_the_mean_that_decide() {
        let thresholds = VerdictThresholds::default();
        let mut scores = [Some(1.); mrde_core::risk::Axis::COUNT];
        scores[Axis::SensitivePaths.index()] = Some(8.6);
        assert_eq!(
            reason(&profile(scores), &thresholds),
            "Sensitive paths 8.6 at or above the deep-review threshold (8)"
        );
        assert_eq!(
            reason(
                &profile([Some(4.); mrde_core::risk::Axis::COUNT]),
                &thresholds
            ),
            "Mean at or above 3.5; no axis reaches 8"
        );
        assert_eq!(
            reason(
                &profile([Some(1.); mrde_core::risk::Axis::COUNT]),
                &thresholds
            ),
            "Mean below 3.5; no axis reaches 8"
        );
    }
}
