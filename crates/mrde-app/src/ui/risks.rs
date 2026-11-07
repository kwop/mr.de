//! Risks tab: contracts other systems rely on, dependency changes, and risky patterns.
use super::{
    Cockpit,
    widgets::{badge, card, code, count, heading, level_color, level_label, muted, path_line},
};
use crate::theme::*;
use gpui::{prelude::*, *};
use mrde_core::insights::{ContractCategory, ContractSeverity, DependencyChangeKind, Level};

fn severity(severity: ContractSeverity) -> Div {
    match severity {
        ContractSeverity::Breaking => badge("breaking", ERROR),
        ContractSeverity::Review => badge("check", WARNING),
        ContractSeverity::Info => badge("info", SUBTLE),
    }
}

fn category(category: ContractCategory) -> &'static str {
    match category {
        ContractCategory::Database => "Database",
        ContractCategory::Http => "HTTP API",
        ContractCategory::Configuration => "Configuration",
        ContractCategory::Permissions => "Permissions",
        ContractCategory::Schema => "API schemas",
    }
}

fn row(index: usize) -> Div {
    div()
        .px_3()
        .py_2()
        .when(index > 0, |r| r.border_t_1().border_color(rgb(LINE)))
        .flex()
        .flex_col()
        .gap_1()
}

impl Cockpit {
    pub(super) fn risk_count(&self) -> Option<usize> {
        let insights = self.current_insights()?;
        Some(
            insights
                .contracts
                .iter()
                .filter(|c| c.severity != ContractSeverity::Info)
                .count()
                + insights
                    .patterns
                    .iter()
                    .filter(|p| p.level != Level::Low)
                    .count()
                + insights.dependencies.iter().filter(|d| d.major).count(),
        )
    }

    pub(super) fn risks_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(insights) = self.current_insights() else {
            return self.analyze_prompt(
                "Analyze the worktree to detect contract changes (schema, endpoints, configuration, permissions), dependency upgrades, and risky patterns in added lines.",
                cx,
            );
        };
        let breaking = insights
            .contracts
            .iter()
            .filter(|c| c.severity == ContractSeverity::Breaking)
            .count();
        let review = insights
            .contracts
            .iter()
            .filter(|c| c.severity == ContractSeverity::Review)
            .count();
        let mut view = div()
            .debug_selector(|| "risks-view".into())
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(badge(
                        format!("{breaking} breaking"),
                        if breaking > 0 { ERROR } else { SUBTLE },
                    ))
                    .child(badge(
                        format!("{review} to check"),
                        if review > 0 { WARNING } else { SUBTLE },
                    ))
                    .child(badge(
                        count(insights.patterns.len(), "risky pattern", "risky patterns"),
                        if insights.patterns.iter().any(|p| p.level == Level::High) {
                            ERROR
                        } else {
                            SUBTLE
                        },
                    ))
                    .child(badge(
                        count(
                            insights.dependencies.len(),
                            "dependency change",
                            "dependency changes",
                        ),
                        if insights.dependencies.iter().any(|d| d.major) {
                            WARNING
                        } else {
                            SUBTLE
                        },
                    )),
            )
            .child(heading(
                "Contracts",
                Some("What other systems, deployments, or clients rely on".into()),
            ));
        if insights.contracts.is_empty() {
            view = view.child(muted(
                "No schema, endpoint, configuration, or permission change detected.",
            ));
        }
        let categories = [
            ContractCategory::Database,
            ContractCategory::Http,
            ContractCategory::Schema,
            ContractCategory::Configuration,
            ContractCategory::Permissions,
        ];
        for kind in categories {
            let items: Vec<_> = insights
                .contracts
                .iter()
                .filter(|c| c.category == kind)
                .collect();
            if items.is_empty() {
                continue;
            }
            let mut block = card()
                .debug_selector(move || {
                    format!(
                        "contracts-{}",
                        category(kind).to_lowercase().replace(' ', "-")
                    )
                })
                .p_0()
                .gap_0()
                .child(
                    div()
                        .px_3()
                        .pt_2()
                        .text_size(px(12.))
                        .text_color(rgb(SUBTLE))
                        .child(category(kind).to_uppercase()),
                );
            for (index, contract) in items.into_iter().enumerate() {
                block = block.child(
                    row(index)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(72.))
                                        .flex_shrink_0()
                                        .flex()
                                        .child(severity(contract.severity)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(rgb(HEADING))
                                        .child(contract.title.clone()),
                                ),
                        )
                        .child(path_line(&contract.path, Some(contract.line)).pl(px(84.)))
                        .when(!contract.detail.is_empty(), |r| {
                            r.child(muted(contract.detail.clone()).pl(px(84.)))
                        }),
                );
            }
            view = view.child(block);
        }

        view = view.child(heading(
            "Dependencies",
            Some("Direct dependencies from manifests".into()),
        ));
        if insights.dependencies.is_empty() && insights.lockfiles.is_empty() {
            view = view.child(muted("No dependency change."));
        }
        let mut manifests: Vec<&str> = insights
            .dependencies
            .iter()
            .map(|d| d.manifest.as_str())
            .collect();
        manifests.dedup();
        for manifest in manifests {
            let mut block = card()
                .p_0()
                .gap_0()
                .child(path_line(manifest, None).px_3().pt_2());
            for (index, dependency) in insights
                .dependencies
                .iter()
                .filter(|d| d.manifest == manifest)
                .enumerate()
            {
                let kind = match dependency.kind {
                    DependencyChangeKind::Added => badge("added", SUCCESS),
                    DependencyChangeKind::Removed => badge("removed", ERROR),
                    DependencyChangeKind::Upgraded => badge("upgraded", INFO),
                    DependencyChangeKind::Downgraded => badge("downgraded", WARNING),
                };
                let versions = match (&dependency.before, &dependency.after) {
                    (Some(before), Some(after)) => format!("{before} → {after}"),
                    (None, Some(after)) => after.clone(),
                    (Some(before), None) => before.clone(),
                    (None, None) => String::new(),
                };
                block = block.child(
                    row(index).child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().w(px(84.)).flex_shrink_0().flex().child(kind))
                            .child(code(dependency.name.clone(), HEADING).flex_1())
                            .when(dependency.major, |r| r.child(badge("major", WARNING)))
                            .when(dependency.dev, |r| r.child(badge("dev", SUBTLE)))
                            .child(code(versions, MUTED).flex_shrink_0().max_w(px(260.))),
                    ),
                );
            }
            view = view.child(block);
        }
        for lockfile in &insights.lockfiles {
            view = view.child(muted(format!(
                "{}: {} resolved packages added, {} removed",
                lockfile.path, lockfile.added, lockfile.removed
            )));
        }

        view = view.child(heading(
            "Risky patterns",
            Some("Added lines only; tests excluded where noise is expected".into()),
        ));
        if insights.patterns.is_empty() {
            view = view.child(muted("No risky pattern found in the added lines."));
        } else {
            let mut block = card().debug_selector(|| "patterns".into()).p_0().gap_0();
            for (index, pattern) in insights.patterns.iter().take(80).enumerate() {
                block = block.child(
                    row(index)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(div().w(px(64.)).flex_shrink_0().flex().child(badge(
                                    level_label(pattern.level),
                                    level_color(pattern.level),
                                )))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(rgb(HEADING))
                                        .child(pattern.title.clone()),
                                ),
                        )
                        .child(path_line(&pattern.path, Some(pattern.line)).pl(px(76.)))
                        .child(code(pattern.snippet.clone(), INK).pl(px(76.)))
                        .child(muted(pattern.advice.clone()).pl(px(76.))),
                );
            }
            view = view.child(block);
        }
        view.into_any_element()
    }
}
