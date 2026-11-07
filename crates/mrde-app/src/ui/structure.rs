//! Structure tab: guided tour and flow (Claude), symbols, impact, and the file graph.
use super::{
    Cockpit,
    widgets::{badge, card, code, count, heading, muted, path_line},
};
use crate::{backend::Action, ellipsis::ellipsis, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Sizable, button::*};
use mrde_core::{
    insights::{FileGraph, SymbolChangeKind, SymbolKind},
    models::MrDetail,
};

fn change_badge(change: SymbolChangeKind) -> Div {
    match change {
        SymbolChangeKind::Added => badge("added", SUCCESS),
        SymbolChangeKind::Removed => badge("removed", ERROR),
        SymbolChangeKind::Modified => badge("modified", INFO),
        SymbolChangeKind::SignatureChanged => badge("signature", WARNING),
    }
}

fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Type => "type",
        SymbolKind::Class => "class",
        SymbolKind::Constant => "constant",
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Mermaid flowchart of the file graph, grouped by top-level directory. The most
/// connected files are highlighted as the core of the change.
pub(super) fn graph_mermaid(graph: &FileGraph) -> String {
    let mut degree = vec![0usize; graph.nodes.len()];
    for edge in &graph.edges {
        degree[edge.from] += 1;
        degree[edge.to] += 1;
    }
    let top = degree.iter().copied().max().unwrap_or(0);
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let group = node.split('/').next().unwrap_or("");
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, members)) => members.push(index),
            None => groups.push((group, vec![index])),
        }
    }
    let label = |text: &str| text.replace('"', "'");
    let mut out = String::from("flowchart LR\n");
    for (index, (group, members)) in groups.iter().enumerate() {
        out.push_str(&format!("  subgraph g{index}[\"{}\"]\n", label(group)));
        for member in members {
            out.push_str(&format!(
                "    n{member}[\"{}\"]\n",
                label(file_name(&graph.nodes[*member]))
            ));
        }
        out.push_str("  end\n");
    }
    for edge in &graph.edges {
        if edge.via == "http" {
            out.push_str(&format!("  n{} -. HTTP .-> n{}\n", edge.from, edge.to));
        } else {
            out.push_str(&format!("  n{} --> n{}\n", edge.from, edge.to));
        }
    }
    let core: Vec<String> = degree
        .iter()
        .enumerate()
        .filter(|(_, d)| top > 1 && **d == top)
        .map(|(i, _)| format!("n{i}"))
        .collect();
    if !core.is_empty() {
        out.push_str("  classDef core fill:#a6763c,stroke:#e2a543,color:#ffffff\n");
        out.push_str(&format!("  class {} core\n", core.join(",")));
    }
    out
}

impl Cockpit {
    fn tour_section(&self, detail: &MrDetail, cx: &mut Context<Self>) -> Div {
        let locked = self.is_busy() || self.confirm;
        let tour = self.current_tour();
        let stats = self.file_stats(detail);
        let mut section = div().flex().flex_col().gap_3().child(
            heading(
                "Guided tour",
                tour.map(|t| {
                    format!(
                        "{} · about {} min",
                        count(t.steps.len(), "step", "steps"),
                        t.minutes
                    )
                }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .debug_selector(|| "generate-tour".into())
                    .child(
                        Button::new("generate-tour")
                            .small()
                            .label(if tour.is_some() {
                                "Regenerate"
                            } else {
                                "Generate tour"
                            })
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(detail) = this.detail.clone() {
                                    this.start(
                                        Action::Tour(detail),
                                        "Claude is preparing a guided tour of the change…",
                                        cx,
                                    );
                                }
                            })),
                    ),
            ),
        );
        let Some(tour) = tour else {
            return section.child(muted(
                "Claude groups the change into reading steps, in order, and draws its main flow. It runs only when you click.",
            ));
        };
        for (index, step) in tour.steps.iter().enumerate() {
            let (added, removed) = step.files.iter().fold((0, 0), |(a, r), file| {
                stats
                    .iter()
                    .find(|s| &s.path == file)
                    .map(|s| (a + s.added, r + s.removed))
                    .unwrap_or((a, r))
            });
            let mut files = div().flex().flex_wrap().gap_1();
            for file in &step.files {
                files = files.child(
                    div()
                        .max_w(px(260.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .bg(rgb(FIELD))
                        .font_family(CODE_FONT)
                        .text_size(px(11.))
                        .text_color(rgb(INK))
                        .overflow_hidden()
                        .flex()
                        .child(ellipsis(file_name(file).to_string())),
                );
            }
            section = section.child(
                card()
                    .debug_selector(move || format!("tour-step-{index}"))
                    .when(step.core, |c| c.border_color(rgb(WARNING)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(22.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(if step.core { WARNING } else { SELECTED }))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.))
                                    .text_color(rgb(if step.core { PAGE } else { HEADING }))
                                    .child(format!("{}", index + 1)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(rgb(HEADING))
                                    .child(step.title.clone()),
                            )
                            .when(step.core, |row| row.child(badge("core of the MR", WARNING)))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(12.))
                                    .text_color(rgb(SUBTLE))
                                    .child(format!(
                                        "{} · +{added} −{removed}",
                                        count(step.files.len(), "file", "files")
                                    )),
                            ),
                    )
                    .child(muted(step.summary.clone()))
                    .child(files),
            );
        }
        if !tour.outside.is_empty() {
            section = section.child(muted(format!(
                "Outside the tour ({}): {}",
                tour.outside.len(),
                tour.outside
                    .iter()
                    .map(|f| file_name(f))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if let Some(diagram) = &tour.diagram {
            section = section
                .child(heading(
                    "Main flow",
                    Some("NEW marks new or changed steps".into()),
                ))
                .child(
                    div()
                        .debug_selector(|| "tour-diagram".into())
                        .child(crate::mermaid::element(diagram)),
                );
        }
        section
    }

    pub(super) fn structure_view(
        &self,
        detail: &MrDetail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut view = div()
            .debug_selector(|| "structure-view".into())
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.tour_section(detail, cx));
        let Some(insights) = self.current_insights() else {
            return view
                .child(heading("Symbols, impact, and file graph", None))
                .child(self.analyze_prompt(
                    "Analyze the worktree to list changed definitions, the code calling them, and how the changed files depend on each other.",
                    cx,
                ))
                .into_any_element();
        };
        let mut symbols = card().debug_selector(|| "symbols".into()).p_0().gap_0();
        for (index, symbol) in insights.symbols.iter().take(200).enumerate() {
            let mut row = div()
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
                                .w(px(76.))
                                .flex_shrink_0()
                                .flex()
                                .child(change_badge(symbol.change)),
                        )
                        .child(
                            code(symbol.name.clone(), HEADING)
                                .flex_shrink_0()
                                .max_w(px(320.)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(SUBTLE))
                                .child(kind_label(symbol.kind)),
                        )
                        .child(path_line(&symbol.path, Some(symbol.line)).flex_1())
                        .child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .gap_2()
                                .font_family(CODE_FONT)
                                .text_size(px(11.))
                                .child(
                                    div()
                                        .text_color(rgb(SUCCESS))
                                        .child(format!("+{}", symbol.added)),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(ERROR))
                                        .child(format!("−{}", symbol.removed)),
                                ),
                        ),
                );
            if symbol.change == SymbolChangeKind::SignatureChanged {
                if let Some(before) = &symbol.before {
                    row = row.child(code(format!("before  {before}"), ERROR).pl(px(88.)));
                }
                if let Some(after) = &symbol.after {
                    row = row.child(code(format!("after   {after}"), SUCCESS).pl(px(88.)));
                }
            }
            symbols = symbols.child(row);
        }
        view = view
            .child(heading(
                "Changed symbols",
                Some(count(insights.symbols.len(), "definition", "definitions")),
            ))
            .child(if insights.symbols.is_empty() {
                muted("No function, type, or class definition changed in a supported language (Go, TypeScript/JavaScript, Rust, Python).").into_any_element()
            } else {
                symbols.into_any_element()
            });

        view = view.child(heading(
            "Impact",
            Some("Textual callers at the MR commit; check the ones the MR did not modify".into()),
        ));
        if insights.impacts.is_empty() {
            view = view.child(muted(
                "No caller found for the modified, removed, or re-signed definitions.",
            ));
        }
        for (index, impact) in insights.impacts.iter().enumerate() {
            let outside = impact.callers.iter().filter(|c| !c.in_mr).count();
            let mut block = card()
                .debug_selector(move || format!("impact-{index}"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(code(impact.symbol.clone(), HEADING).flex_shrink_0())
                        .child(change_badge(impact.change))
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(12.))
                                .text_color(rgb(if outside > 0 { WARNING } else { SUBTLE }))
                                .child(format!(
                                    "{} · {outside} outside the MR",
                                    count(
                                        impact.callers.len() + impact.more as usize,
                                        "caller",
                                        "callers"
                                    )
                                )),
                        ),
                );
            for caller in &impact.callers {
                block = block.child(
                    div()
                        .pt_1()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(div().w(px(96.)).flex_shrink_0().flex().child(
                                    if caller.in_mr {
                                        badge("in MR", SUCCESS)
                                    } else {
                                        badge("not modified", WARNING)
                                    },
                                ))
                                .child(path_line(&caller.path, Some(caller.line)).flex_1())
                                .when(caller.test, |row| row.child(badge("test", INFO))),
                        )
                        .child(code(caller.text.clone(), MUTED).pl(px(108.))),
                );
            }
            if impact.more > 0 {
                block = block.child(muted(format!(
                    "{} not shown.",
                    count(impact.more as usize, "more match", "more matches")
                )));
            }
            view = view.child(block);
        }

        view = view.child(heading(
            "File graph",
            Some(format!(
                "{} · {}",
                count(insights.graph.nodes.len(), "linked file", "linked files"),
                count(insights.graph.edges.len(), "link", "links")
            )),
        ));
        view = view.child(if insights.graph.edges.is_empty() {
            muted("No import or HTTP link between the changed files.").into_any_element()
        } else {
            div()
                .debug_selector(|| "file-graph".into())
                .child(crate::mermaid::element(&graph_mermaid(&insights.graph)))
                .into_any_element()
        });
        view.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrde_core::insights::GraphEdge;

    // `gpui::*` also exports a `test` attribute; use the standard one.
    #[::core::prelude::v1::test]
    fn file_graph_renders_as_a_mermaid_flowchart() {
        let graph = FileGraph {
            nodes: vec![
                "front/api.ts".into(),
                "back/series.go".into(),
                "back/usecase.go".into(),
                "back/entity.go".into(),
            ],
            edges: vec![
                GraphEdge {
                    from: 0,
                    to: 1,
                    via: "http".into(),
                },
                GraphEdge {
                    from: 1,
                    to: 2,
                    via: "import".into(),
                },
                GraphEdge {
                    from: 2,
                    to: 3,
                    via: "import".into(),
                },
            ],
        };
        let source = graph_mermaid(&graph);
        assert!(source.contains("n0 -. HTTP .-> n1"));
        assert!(source.contains("subgraph g1[\"back\"]"));
        assert!(source.contains("class n1,n2 core"));
        crate::mermaid::render(&source).expect("the renderer accepts the generated graph");
    }
}
