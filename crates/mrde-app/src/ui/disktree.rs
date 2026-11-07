//! Center-panel "disktree": a treemap of the most changed code areas.
//!
//! Scrolling re-renders the whole view on every frame, so the statistics, layout, and
//! labels are computed once per MR commit and statistics source, then only drawn.
use super::Cockpit;
use crate::theme::*;
use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Sizable, button::*};
use mrde_core::{
    insights::{FileHistory, Level},
    models::*,
    stats::treemap,
};
use std::{collections::HashMap, sync::Arc};

/// Width-to-height ratio used for layout; the container keeps a fixed height.
const ASPECT: f64 = 3.;
const HEIGHT: f32 = 220.;
/// Beyond this, each zone's smaller files share a tile: readable and cheap to draw.
const MAX_TILES: usize = 120;
const ZONE_COLORS: [u32; 8] = [
    0x3f6ea6, 0x4c8c5c, 0xa6763c, 0x8657a6, 0x3c9494, 0xa64e5e, 0x7c8a3c, 0x5c68a6,
];

pub(super) fn zone_color(index: usize) -> u32 {
    ZONE_COLORS[index % ZONE_COLORS.len()]
}

fn risk_color(file: Option<&FileHistory>) -> u32 {
    match file {
        Some(f) if f.new_file => 0x3f6ea6,
        Some(f) => match f.risk {
            Level::High => 0xb5534f,
            Level::Medium => 0xa6803a,
            Level::Low => 0x4c7c5c,
        },
        None => 0x555a63,
    }
}

struct TileView {
    path: Option<String>,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    color: u32,
    label: Option<SharedString>,
    tooltip: SharedString,
}

struct Model {
    summary: SharedString,
    tiles: Vec<TileView>,
    legend: Vec<(u32, SharedString)>,
}

pub(super) struct Cached {
    pub files: Arc<Vec<FileStat>>,
    pub lookup: HashMap<String, usize>,
    pub widest: u64,
    model: Option<Model>,
}

fn build(stats: &DiffStats, history: Option<&[FileHistory]>) -> Option<Model> {
    if stats.files.is_empty() {
        return None;
    }
    let added: u64 = stats.files.iter().map(|f| f.added).sum();
    let removed: u64 = stats.files.iter().map(|f| f.removed).sum();
    let map = treemap(&stats.files, ASPECT, 1., MAX_TILES);
    let tiles = map
        .tiles
        .iter()
        .map(|tile| {
            let r = tile.rect;
            let (label, tooltip) = match tile.file {
                Some(index) => {
                    let file = &stats.files[index];
                    (
                        file.path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&file.path)
                            .to_string(),
                        format!("{}\n+{} −{}", file.path, file.added, file.removed),
                    )
                }
                None => (
                    crate::tr!("+{value0} files", value0 = tile.files),
                    crate::tr!(
                        "{value0} smaller files in {value1}\n{value2} lines changed",
                        value0 = tile.files,
                        value1 = map.zones[tile.zone].name,
                        value2 = tile.changed
                    ),
                ),
            };
            let show_label = r.w / ASPECT > 0.07 && r.h > 0.12;
            let color = match (history, tile.file) {
                (Some(history), Some(index)) => {
                    risk_color(history.iter().find(|h| h.path == stats.files[index].path))
                }
                (Some(_), None) => risk_color(None),
                (None, _) => zone_color(tile.zone),
            };
            TileView {
                path: tile.file.map(|i| stats.files[i].path.clone()),
                left: (r.x / ASPECT) as f32,
                top: r.y as f32,
                width: (r.w / ASPECT) as f32,
                height: r.h as f32,
                color,
                label: show_label.then(|| label.into()),
                tooltip: tooltip.into(),
            }
        })
        .collect();
    let legend = match history {
        Some(_) => vec![
            (
                risk_color(Some(&risk_sample(Level::High, false))),
                "High risk: frequent changes and fixes".into(),
            ),
            (
                risk_color(Some(&risk_sample(Level::Medium, false))),
                "Medium".into(),
            ),
            (
                risk_color(Some(&risk_sample(Level::Low, false))),
                "Low".into(),
            ),
            (
                risk_color(Some(&risk_sample(Level::Low, true))),
                "New file".into(),
            ),
            (risk_color(None), "No history".into()),
        ],
        None => map
            .zones
            .iter()
            .enumerate()
            .take(10)
            .map(|(index, zone)| {
                (
                    zone_color(index),
                    format!("{} · {}", short_zone(&zone.name), zone.changed).into(),
                )
            })
            .collect(),
    };
    Some(Model {
        summary: crate::tr!(
            "{value0} files · +{added} −{removed} · {value1}",
            value0 = stats.files.len(),
            added = added,
            removed = removed,
            value1 = match stats.source {
                StatsSource::Git => "exact (git)",
                StatsSource::GitLab => "estimate (GitLab diff)",
            }
        )
        .into(),
        tiles,
        legend,
    })
}

fn risk_sample(risk: Level, new_file: bool) -> FileHistory {
    FileHistory {
        path: String::new(),
        commits: 0,
        fixes: 0,
        authors: 0,
        last_fix: None,
        risk,
        new_file,
    }
}

/// `…/application/usecases` for deep zones: the legend stays short.
fn short_zone(name: &str) -> String {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 2 {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    } else {
        name.to_string()
    }
}

impl Cockpit {
    fn exact_stats(&self, detail: &MrDetail) -> Option<&DiffStats> {
        self.stats
            .as_ref()
            .filter(|(id, stats)| *id == detail.mr.id && stats.sha == detail.mr.sha)
            .map(|(_, stats)| stats)
    }

    pub(super) fn file_stats(
        &self,
        detail: &MrDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Arc<Vec<FileStat>> {
        self.disktree_cached(detail, window, cx)
            .map(|cache| cache.files.clone())
            .unwrap_or_default()
    }

    pub(super) fn disktree_cached(
        &self,
        detail: &MrDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Cached>> {
        let exact = self.exact_stats(detail);
        let risk = self.risk_colors && self.current_insights().is_some();
        let key = format!(
            "statistics:{}:{}:{}:{}:{}:{risk}",
            self.config.namespace(),
            detail.mr.id.project,
            detail.mr.id.iid,
            detail.mr.sha,
            self.presentation_revision
        );
        crate::deferred::load(key.into(), window, cx, || {
            let exact = exact.cloned();
            let changed = if exact.is_none() {
                detail.files.clone()
            } else {
                vec![]
            };
            let history = self
                .current_insights()
                .filter(|_| risk)
                .map(|i| i.history.clone());
            let sha = detail.mr.sha.clone();
            move || {
                let truncated = changed.iter().any(|f| f.too_large || f.collapsed);
                let stats = exact.unwrap_or_else(|| DiffStats {
                    sha,
                    source: StatsSource::GitLab,
                    files: changed
                        .iter()
                        .filter(|f| !f.too_large && !f.collapsed)
                        .map(mrde_core::stats::file_stat)
                        .collect(),
                });
                let model = if truncated {
                    None
                } else {
                    build(&stats, history.as_deref())
                };
                let lookup = stats
                    .files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.path.clone(), i))
                    .collect();
                let widest = stats
                    .files
                    .iter()
                    .map(FileStat::changed)
                    .max()
                    .unwrap_or(1)
                    .max(1);
                Cached {
                    files: Arc::new(stats.files),
                    lookup,
                    widest,
                    model,
                }
            }
        })
        .and_then(Result::ok)
    }

    pub(super) fn disktree_view(
        &self,
        detail: &MrDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cache = self.disktree_cached(detail, window, cx);
        let model = cache.as_ref().and_then(|c| c.model.as_ref());
        let analyzed = self.current_insights().is_some();
        let mut toggle = div()
            .flex()
            .gap_1()
            .p(px(2.))
            .rounded(px(6.))
            .bg(rgb(FIELD));
        for (risk, label) in [(false, "Zones"), (true, "Risk")] {
            let selected = self.risk_colors == risk;
            toggle = toggle.child(
                div()
                    .debug_selector(move || format!("disktree-{}", label.to_lowercase()))
                    .child(
                        Button::new(SharedString::from(format!("disktree-{label}")))
                            .xsmall()
                            .ghost()
                            .label(crate::i18n::t(label))
                            .when(selected, |b| b.bg(rgb(SELECTED)).text_color(rgb(HEADING)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.risk_colors = risk;
                                cx.notify();
                            })),
                    ),
            );
        }
        let mut header = div().flex().items_center().justify_between().gap_3().child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(HEADING))
                        .child(crate::i18n::t("Disktree")),
                )
                .when(analyzed, |row| row.child(toggle)),
        );
        if let Some(model) = &model {
            header = header.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(SUBTLE))
                    .child(model.summary.clone()),
            );
        }
        let zone = div()
            .debug_selector(|| "disktree".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(header);
        let Some(model) = model else {
            return zone
                .child(
                    div()
                        .h(px(64.))
                        .rounded(px(RADIUS_CARD))
                        .border_1()
                        .border_color(rgb(LINE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child(if cache.is_none() {
                            "Preparing file statistics…"
                        } else if detail.files.is_empty() && self.exact_stats(detail).is_none() {
                            "No changed files."
                        } else {
                            "GitLab truncated some diffs. Click Compute statistics to map the changes."
                        }),
                )
                .into_any_element();
        };
        let mut canvas = div()
            .debug_selector(|| "disktree-map".into())
            .relative()
            .w_full()
            .h(px(HEIGHT))
            .rounded(px(RADIUS_CARD))
            .overflow_hidden()
            .bg(rgb(FIELD));
        for (index, tile) in model.tiles.iter().enumerate() {
            let tooltip = tile.tooltip.clone();
            canvas = canvas.child(
                div()
                    .id(("disktree-tile", index))
                    .debug_selector(move || format!("disktree-tile-{index}"))
                    .when_some(tile.path.clone(), |tile, path| {
                        tile.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.jump_to_risk_file(path.clone(), window, cx)
                            }))
                    })
                    .absolute()
                    .left(relative(tile.left))
                    .top(relative(tile.top))
                    .w(relative(tile.width))
                    .h(relative(tile.height))
                    .p(px(1.))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .child(
                        div()
                            .size_full()
                            .rounded(px(2.))
                            .bg(rgb(tile.color))
                            .overflow_hidden()
                            .px_1()
                            .text_size(px(11.))
                            .line_height(px(15.))
                            .text_color(rgb(HEADING))
                            .when_some(tile.label.clone(), |d, label| {
                                d.child(div().one_line().child(crate::ellipsis::ellipsis(label)))
                            }),
                    ),
            );
        }
        let legend =
            div()
                .flex()
                .flex_wrap()
                .gap_x_3()
                .gap_y_1()
                .children(model.legend.iter().map(|(color, label)| {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(div().size(px(8.)).rounded(px(2.)).bg(rgb(*color)))
                        .child(label.clone())
                }));
        zone.child(canvas).child(legend).into_any_element()
    }
}
