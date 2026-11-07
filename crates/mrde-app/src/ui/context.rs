//! Review context beside the reader: pre-review, checks, analysis, ticket, and approvals
//! of the open merge request. Docked on wide windows, floating over the reader on narrow
//! ones, and reduced to a rail of counters when collapsed.
use super::{
    Cockpit,
    inbox::DetailTab,
    label_chip, labels,
    widgets::{badge, muted},
};
use crate::{ellipsis::ellipsis, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Icon, IconName, Sizable, button::*, tooltip::Tooltip};
use mrde_core::{
    insights::{ContractSeverity, Level},
    models::*,
};

/// Narrower windows float the open panel over the reader instead of docking it.
pub(super) const DOCK_MIN_WIDTH: f32 = 1360.;
/// Narrower windows hide the collapsed rail; the title bar toggle remains.
pub(super) const RAIL_MIN_WIDTH: f32 = 1200.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContextLayout {
    Docked,
    Overlay,
    Rail,
    Hidden,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Review,
    Checks,
    Analysis,
    Ticket,
    Approvals,
}
impl Part {
    const ALL: [Self; 5] = [
        Self::Review,
        Self::Checks,
        Self::Analysis,
        Self::Ticket,
        Self::Approvals,
    ];
    fn key(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Checks => "checks",
            Self::Analysis => "analysis",
            Self::Ticket => "ticket",
            Self::Approvals => "approvals",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Review => "PRE-REVIEW",
            Self::Checks => "CHECKS",
            Self::Analysis => "ANALYSIS",
            Self::Ticket => "TICKET",
            Self::Approvals => "APPROVALS",
        }
    }
    fn icon(self) -> IconName {
        match self {
            Self::Review => IconName::Bot,
            Self::Checks => IconName::CircleCheck,
            Self::Analysis => IconName::TriangleAlert,
            Self::Ticket => IconName::BookOpen,
            Self::Approvals => IconName::User,
        }
    }
    fn tab(self) -> Option<DetailTab> {
        match self {
            Self::Review => Some(DetailTab::Review),
            Self::Checks => Some(DetailTab::Checks),
            Self::Analysis => Some(DetailTab::Risks),
            Self::Ticket | Self::Approvals => None,
        }
    }
}

fn severity_color(severity: &Severity) -> u32 {
    match severity {
        Severity::Critical | Severity::High => ERROR,
        Severity::Medium => WARNING,
        Severity::Low => SUBTLE,
    }
}
fn severity_label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
    }
}
/// `project#12` from an issue or work item link.
fn ticket_reference(url: &str) -> String {
    let Some((project, rest)) = url.split_once("/-/") else {
        return String::new();
    };
    let iid = rest.split(['/', '#', '?']).nth(1).unwrap_or_default();
    let project = project.rsplit('/').next().unwrap_or_default();
    format!("{project}#{iid}")
}
fn row(label: impl Into<SharedString>, value: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .text_size(px(13.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .one_line()
                .text_color(rgb(MUTED))
                .child(ellipsis(label.into())),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(color))
                .child(value.into()),
        )
}

impl Cockpit {
    /// The panel follows the open merge request, in the inbox only.
    pub(super) fn context_available(&self) -> bool {
        !self.settings
            && !self.worktrees_view
            && !self.threads_view
            && (self.detail.is_some() || self.selected_mr.is_some() || self.opening)
    }
    /// Until toggled, the panel is open on wide windows and collapsed on narrow ones.
    pub(super) fn context_layout(&self, width: Pixels) -> ContextLayout {
        if !self.context_available() {
            return ContextLayout::Hidden;
        }
        if let Some(layout) = self.context_pinned {
            return layout;
        }
        let wide = width >= px(DOCK_MIN_WIDTH);
        match (self.context_open.unwrap_or(wide), wide) {
            (true, true) => ContextLayout::Docked,
            (true, false) => ContextLayout::Overlay,
            (false, _) if width >= px(RAIL_MIN_WIDTH) => ContextLayout::Rail,
            (false, _) => ContextLayout::Hidden,
        }
    }
    pub(super) fn toggle_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.context_available() {
            return;
        }
        let open = matches!(
            self.context_layout(window.viewport_size().width),
            ContextLayout::Docked | ContextLayout::Overlay
        );
        self.context_open = Some(!open);
        self.context_pinned = None;
        cx.notify();
    }
    fn set_context(&mut self, open: bool, cx: &mut Context<Self>) {
        self.context_open = Some(open);
        self.context_pinned = None;
        cx.notify();
    }
    fn current_compliance(&self) -> Option<&Compliance> {
        let detail = self.detail.as_ref()?;
        self.compliance
            .as_ref()
            .filter(|(id, sha, _)| *id == detail.mr.id && *sha == detail.mr.sha)
            .map(|(_, _, compliance)| compliance)
    }
    /// Counter shown on collapsed sections, with its color.
    fn counter(&self, part: Part) -> Option<(usize, u32)> {
        let (count, color) = match part {
            Part::Review => {
                let risks = &self.review.as_ref()?.risks;
                let worst = risks.iter().map(|r| &r.severity).min()?;
                (risks.len(), severity_color(worst))
            }
            Part::Checks => {
                let findings = &self.current_compliance()?.findings;
                let missing = findings
                    .iter()
                    .filter(|f| f.status == RequirementStatus::Missing)
                    .count();
                let gaps = findings
                    .iter()
                    .filter(|f| f.status != RequirementStatus::Met)
                    .count();
                (gaps, if missing > 0 { ERROR } else { WARNING })
            }
            Part::Analysis => {
                let insights = self.current_insights()?;
                let breaking = insights
                    .contracts
                    .iter()
                    .any(|c| c.severity == ContractSeverity::Breaking)
                    || insights.patterns.iter().any(|p| p.level == Level::High);
                (self.risk_count()?, if breaking { ERROR } else { WARNING })
            }
            Part::Ticket => return None,
            Part::Approvals => (self.detail.as_ref()?.mr.approvals? as usize, SUCCESS),
        };
        (count > 0).then_some((count, color))
    }
    /// The most severe counter, for the title bar toggle when the panel is out of sight.
    pub(super) fn context_alert(&self) -> Option<u32> {
        let colors: Vec<u32> = [Part::Review, Part::Checks, Part::Analysis]
            .into_iter()
            .filter_map(|part| self.counter(part).map(|(_, color)| color))
            .collect();
        [ERROR, WARNING]
            .into_iter()
            .find(|color| colors.contains(color))
    }

    pub(super) fn context_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rail = div()
            .debug_selector(|| "context-rail".into())
            .w(px(RAIL_WIDTH))
            .flex_shrink_0()
            .h_full()
            .pb_2()
            .flex()
            .flex_col()
            .items_center()
            .gap_2();
        for part in Part::ALL {
            let counter = self.counter(part);
            let tooltip = match counter {
                Some((count, _)) => format!("{} · {count}", part.title()),
                None => part.title().to_string(),
            };
            rail = rail.child(
                div()
                    .id(SharedString::from(format!("rail-{}", part.key())))
                    .debug_selector(move || format!("rail-{}", part.key()))
                    .relative()
                    .size(px(38.))
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgb(if part == Part::Review { CLAUDE } else { MUTED }))
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(Icon::new(part.icon()).size_4())
                    .children(counter.map(|(count, color)| {
                        div()
                            .absolute()
                            .top(px(1.))
                            .right(px(1.))
                            .min_w(px(16.))
                            .h(px(16.))
                            .px(px(4.))
                            .rounded(px(8.))
                            .bg(rgb(color))
                            .text_color(rgb(PAGE))
                            .text_size(px(10.))
                            .line_height(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .flex()
                            .justify_center()
                            .child(count.to_string())
                    }))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.set_context(true, cx))),
            );
        }
        rail.child(div().flex_1())
            .child(div().w(px(24.)).h(px(1.)).bg(rgb(LINE)))
            .child(
                div().debug_selector(|| "context-expand".into()).child(
                    Button::new("context-expand")
                        .ghost()
                        .icon(IconName::ChevronLeft)
                        .tooltip("Show review context (⌘⌥B)")
                        .on_click(cx.listener(|this, _, _, cx| this.set_context(true, cx))),
                ),
            )
            .into_any_element()
    }

    fn context_section(
        &self,
        part: Part,
        body: impl IntoElement,
        meta: Option<(String, u32)>,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = div()
            .id(SharedString::from(format!("context-{}-title", part.key())))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(11.))
            .text_color(rgb(SUBTLE))
            .font_weight(FontWeight::SEMIBOLD)
            .child(
                Icon::new(part.icon())
                    .size(px(14.))
                    .text_color(rgb(if part == Part::Review { CLAUDE } else { SUBTLE })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .one_line()
                    .child(ellipsis(part.title())),
            )
            .children(meta.map(|(text, color)| {
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(rgb(color))
                    .child(text)
            }))
            .when_some(part.tab(), |header, tab| {
                header
                    .cursor_pointer()
                    .hover(|style| style.text_color(rgb(HEADING)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_detail_tab(tab);
                        cx.notify();
                    }))
            });
        div()
            .debug_selector(move || format!("context-{}", part.key()))
            .flex_shrink_0()
            .min_w_0()
            .p_3()
            .rounded(px(RADIUS_CARD))
            .bg(rgb(READING))
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(body)
    }

    fn review_part(&self, cx: &mut Context<Self>) -> Div {
        let Some(review) = &self.review else {
            return self.context_section(
                Part::Review,
                muted("Not run for this commit. Run a review from the toolbar."),
                None,
                cx,
            );
        };
        let mut risks: Vec<&Risk> = review.risks.iter().collect();
        risks.sort_by_key(|risk| risk.severity.clone());
        let mut body = div().flex().flex_col().gap_2();
        for risk in risks.iter().take(3) {
            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(badge(
                        severity_label(&risk.severity),
                        severity_color(&risk.severity),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .child(risk.description.clone()),
                    ),
            );
        }
        if risks.is_empty() {
            body = body.child(muted("No risk found."));
        } else if risks.len() > 3 {
            body = body.child(muted(format!("{} more in Pre-review", risks.len() - 3)));
        }
        let meta = format!(
            "{}{:.0}% confident",
            self.review_level
                .map(|level| format!("{} · ", level.label()))
                .unwrap_or_default(),
            review.confidence * 100.
        );
        self.context_section(Part::Review, body, Some((meta, MUTED)), cx)
    }

    fn checks_part(&self, cx: &mut Context<Self>) -> Div {
        let Some(compliance) = self.current_compliance() else {
            return self.context_section(
                Part::Checks,
                muted(if self.config.spec_path.trim().is_empty() {
                    "Set a specification folder in Settings to check compliance."
                } else {
                    "Not checked yet. Run Check compliance in Checks."
                }),
                None,
                cx,
            );
        };
        let findings = &compliance.findings;
        let count =
            |status: RequirementStatus| findings.iter().filter(|f| f.status == status).count();
        let (met, partial, missing) = (
            count(RequirementStatus::Met),
            count(RequirementStatus::Partial),
            count(RequirementStatus::Missing),
        );
        let untested = findings
            .iter()
            .filter(|f| f.status == RequirementStatus::Met && f.tests.is_empty())
            .count();
        let (verdict, color) = match compliance.verdict {
            ComplianceVerdict::Compliant => ("Compliant", SUCCESS),
            ComplianceVerdict::Partial => ("Partial", WARNING),
            ComplianceVerdict::NonCompliant => ("Not compliant", ERROR),
            ComplianceVerdict::NoSpecFound => ("No specification", MUTED),
        };
        let total = findings.len().max(1) as f32;
        let segment =
            |n: usize, color: u32| div().h_full().w(relative(n as f32 / total)).bg(rgb(color));
        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(row(
                "Requirements met",
                format!("{met} / {}", findings.len()),
                INK,
            ))
            .child(row(
                "Met without test",
                untested.to_string(),
                if untested > 0 { WARNING } else { INK },
            ))
            .child(
                div()
                    .mt_1()
                    .h(px(6.))
                    .rounded(px(RADIUS_CHIP))
                    .overflow_hidden()
                    .bg(rgb(FIELD))
                    .flex()
                    .child(segment(met, SUCCESS))
                    .child(segment(partial, WARNING))
                    .child(segment(missing, ERROR)),
            );
        self.context_section(Part::Checks, body, Some((verdict.into(), color)), cx)
    }

    fn analysis_part(&self, cx: &mut Context<Self>) -> Div {
        let Some(insights) = self.current_insights() else {
            return self.context_section(
                Part::Analysis,
                muted("Not analyzed yet. Click Analyze in the toolbar."),
                None,
                cx,
            );
        };
        let breaking = insights
            .contracts
            .iter()
            .filter(|c| c.severity == ContractSeverity::Breaking)
            .count();
        let contracts = insights
            .contracts
            .iter()
            .filter(|c| c.severity == ContractSeverity::Review)
            .count();
        let patterns = insights
            .patterns
            .iter()
            .filter(|p| p.level != Level::Low)
            .count();
        let majors = insights.dependencies.iter().filter(|d| d.major).count();
        let overlaps = insights.overlaps.as_ref().map(Vec::len);
        let highlight = |n: usize, color: u32| if n > 0 { color } else { INK };
        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(row(
                "Breaking changes",
                breaking.to_string(),
                highlight(breaking, ERROR),
            ))
            .child(row(
                "Contracts to review",
                contracts.to_string(),
                highlight(contracts, WARNING),
            ))
            .child(row(
                "Risky patterns",
                patterns.to_string(),
                highlight(patterns, WARNING),
            ))
            .child(row(
                "Major upgrades",
                majors.to_string(),
                highlight(majors, WARNING),
            ))
            .child(row(
                "Overlapping MRs",
                overlaps.map_or("unknown".to_string(), |n| n.to_string()),
                INK,
            ));
        self.context_section(Part::Analysis, body, None, cx)
    }

    fn ticket_part(&self, detail: &MrDetail, cx: &mut Context<Self>) -> Div {
        let tickets = primary_tickets(&detail.mr, &detail.issues);
        let mut body = div().flex().flex_col().gap_3();
        if tickets.is_empty() {
            body = body.child(muted("No linked ticket."));
        }
        for (index, ticket) in tickets.into_iter().take(2).enumerate() {
            let url = ticket.web_url.clone();
            body = body.child(
                div()
                    .id(("context-ticket", index))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .cursor_pointer()
                    .child(
                        div()
                            .font_family(CODE_FONT)
                            .text_size(px(CODE_SIZE))
                            .text_color(rgb(INFO))
                            .child(ticket_reference(&ticket.web_url)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .child(ticket.title.clone()),
                    )
                    .child(
                        div().flex().flex_wrap().gap_1().children(
                            labels::display_order(ticket.labels.iter())
                                .into_iter()
                                .take(3)
                                .map(label_chip),
                        ),
                    )
                    .tooltip(|window, cx| {
                        Tooltip::new("Open the ticket in the browser").build(window, cx)
                    })
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
            );
        }
        self.context_section(Part::Ticket, body, None, cx)
    }

    fn approvals_part(&self, detail: &MrDetail, cx: &mut Context<Self>) -> Div {
        let (text, color) = match detail.mr.approvals {
            None => ("Unavailable".to_string(), MUTED),
            Some(0) => ("None yet".to_string(), INK),
            Some(1) => ("1 approval".to_string(), SUCCESS),
            Some(n) => (format!("{n} approvals"), SUCCESS),
        };
        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(row("Approved", text, color))
            .child(row(
                "Commit",
                detail.mr.sha.chars().take(8).collect::<String>(),
                INK,
            ));
        self.context_section(Part::Approvals, body, None, cx)
    }

    pub(super) fn context_panel(&self, overlay: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div()
            .id("context-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        match &self.detail {
            None => body = body.child(muted("Loading merge request…")),
            Some(detail) => {
                body = body
                    .child(self.review_part(cx))
                    .child(self.checks_part(cx))
                    .child(self.analysis_part(cx))
                    .child(self.ticket_part(detail, cx))
                    .child(self.approvals_part(detail, cx));
            }
        }
        div()
            .id("context-panel")
            .debug_selector(|| "context-panel".into())
            .w(px(CONTEXT_WIDTH))
            .flex_shrink_0()
            .h_full()
            .p_3()
            .rounded(px(RADIUS_SHEET))
            .bg(rgb(QUEUE))
            .flex()
            .flex_col()
            .gap_3()
            .when(overlay, |panel| {
                panel
                    .occlude()
                    .border_1()
                    .border_color(rgb(PANEL_LINE))
                    .shadow_lg()
            })
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_1()
                    .child(Icon::new(IconName::Bot).size_4().text_color(rgb(CLAUDE)))
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(HEADING))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Review context"),
                    )
                    .child(
                        div().debug_selector(|| "context-close".into()).child(
                            Button::new("context-close")
                                .ghost()
                                .small()
                                .icon(IconName::ChevronRight)
                                .tooltip("Hide review context (⌘⌥B)")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_context(false, cx)),
                                ),
                        ),
                    ),
            )
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[::core::prelude::v1::test]
    fn ticket_references_name_the_project_and_number() {
        for (url, expected) in [
            (
                "https://gitlab.example.com/acme/platform/-/issues/1203",
                "platform#1203",
            ),
            (
                "https://gitlab.example.com/acme/web/-/work_items/7#note_1",
                "web#7",
            ),
            ("https://gitlab.example.com/acme/web", ""),
        ] {
            assert_eq!(super::ticket_reference(url), expected);
        }
    }
}
