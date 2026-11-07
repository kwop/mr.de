//! Native GPUI risk radar. Unknown axes are never drawn as zero: their spokes are dashed and
//! labelled n/a, and the outline crosses them with a dashed segment.
use super::{Cockpit, inbox::thousands};
use crate::theme::*;
use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;
use mrde_core::risk::{Axis, AxisScore, Baseline, RawMetric, RiskProfile, VerdictThresholds};

const WIDTH: f32 = 460.;
const HEIGHT: f32 = 300.;
const RADIUS: f32 = 100.;
/// Distance from the center to the axis labels, which extend away from the chart.
const LABELS: f32 = 114.;
const LABEL_WIDTH: f32 = 108.;

/// Green below the standard-review mean, amber up to the deep-review axis threshold, red
/// beyond: the colors of the verdict each axis pushes toward.
pub(super) fn severity(score: f64, thresholds: &VerdictThresholds) -> u32 {
    if score >= thresholds.deep_axis {
        ERROR
    } else if score >= thresholds.standard_mean {
        WARNING
    } else {
        SUCCESS
    }
}

/// "1,110 lines", "42%", "+12 CCN": the measurement behind a score.
pub(super) fn raw_text(raw: &RawMetric) -> String {
    let value = raw.value;
    let number = if value.fract() == 0. && value.abs() < 1e15 {
        let digits = thousands(value.abs() as u64);
        if value < 0. {
            format!("−{digits}")
        } else {
            digits
        }
    } else {
        format!("{value:.1}")
    };
    match raw.unit.as_str() {
        "fraction" => format!("{:.0}%", value * 100.),
        "weighted fraction" => crate::tr!(
            "{value0}% weighted",
            value0 = format!("{:.0}", value * 100.)
        ),
        "CCN delta" => crate::tr!(
            "{value0}{number} CCN",
            value0 = if value > 0. { "+" } else { "" },
            number = number
        ),
        unit => format!("{number} {}", crate::i18n::text(unit)),
    }
}

/// The tooltip of an axis: measurement, score, median, and method or missing reason.
pub(super) fn explanation(score: &AxisScore, median: Option<f64>) -> String {
    let median = median
        .map(|m| crate::tr!(" · team median {m}", m = format!("{m:.1}")))
        .unwrap_or_default();
    match (&score.raw, score.score) {
        (Some(raw), Some(value)) => format!(
            "{}: {} → {value:.1} / 10{median}\n{}{}",
            crate::i18n::t(score.axis.label()),
            raw_text(raw),
            if raw.estimated { "Estimate. " } else { "" },
            raw.method
        ),
        _ => format!(
            "{}: n/a{median}\n{}",
            crate::i18n::t(score.axis.label()),
            score.unavailable.as_deref().unwrap_or("Not measured")
        ),
    }
}

fn angle(index: usize) -> f32 {
    index as f32 * std::f32::consts::TAU / Axis::COUNT as f32 - std::f32::consts::FRAC_PI_2
}
fn vertex(center: Point<Pixels>, radius: Pixels, index: usize, score: f64) -> Point<Pixels> {
    let angle = angle(index);
    let radius = radius * (score.clamp(0., 10.) as f32 / 10.);
    center + point(radius * angle.cos(), radius * angle.sin())
}
fn alpha(color: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(color).into();
    color.a = alpha;
    color
}
fn stroke(
    points: &[Point<Pixels>],
    color: Hsla,
    width: Pixels,
    dashed: bool,
    closed: bool,
    window: &mut Window,
) {
    let Some(first) = points.first() else { return };
    let mut path = PathBuilder::stroke(width);
    if dashed {
        path = path.dash_array(&[px(4.), px(3.)]);
    }
    path.move_to(*first);
    for point in &points[1..] {
        path.line_to(*point);
    }
    if closed {
        path.close();
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
fn fill(points: &[Point<Pixels>], color: Hsla, window: &mut Window) {
    let Some(first) = points.first() else { return };
    let mut path = PathBuilder::fill();
    path.move_to(*first);
    for point in &points[1..] {
        path.line_to(*point);
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
fn ring(center: Point<Pixels>, radius: Pixels, score: f64) -> Vec<Point<Pixels>> {
    (0..Axis::COUNT)
        .map(|i| vertex(center, radius, i, score))
        .collect()
}
/// Solid edges join neighbouring known values; fainter dashed edges cross unknown axes.
fn outline(
    values: [Option<f64>; mrde_core::risk::Axis::COUNT],
    center: Point<Pixels>,
    radius: Pixels,
    color: Hsla,
    width: Pixels,
    dashed: bool,
    window: &mut Window,
) {
    let known: Vec<(usize, f64)> = (0..Axis::COUNT)
        .filter_map(|i| Some((i, values[i]?)))
        .collect();
    let edges = match known.len() {
        0 | 1 => 0,
        2 => 1,
        n => n,
    };
    for k in 0..edges {
        let (i, a) = known[k];
        let (j, b) = known[(k + 1) % known.len()];
        let neighbours = (i + 1) % Axis::COUNT == j;
        let mut color = color;
        if !neighbours {
            color.a *= 0.5;
        }
        stroke(
            &[vertex(center, radius, i, a), vertex(center, radius, j, b)],
            color,
            width,
            dashed || !neighbours,
            false,
            window,
        );
    }
}
fn drawing(
    scores: [Option<f64>; mrde_core::risk::Axis::COUNT],
    medians: Option<[Option<f64>; mrde_core::risk::Axis::COUNT]>,
    deep: Option<f64>,
    mini: bool,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let radius = if mini {
                bounds.size.width.min(bounds.size.height) / 2. - px(2.)
            } else {
                px(RADIUS)
            };
            let grid = alpha(PANEL_LINE, 0.9);
            let rings: &[f64] = if mini { &[10.] } else { &[2., 4., 6., 8., 10.] };
            for &score in rings.iter().filter(|r| Some(**r) != deep) {
                stroke(
                    &ring(center, radius, score),
                    grid,
                    px(1.),
                    false,
                    true,
                    window,
                );
            }
            if let Some(deep) = deep {
                let line = alpha(ERROR, 0.6);
                stroke(
                    &ring(center, radius, deep),
                    line,
                    px(1.),
                    true,
                    true,
                    window,
                );
            }
            if !mini {
                for (i, score) in scores.iter().enumerate() {
                    let (color, dashed) = match score {
                        Some(_) => (alpha(PANEL_LINE, 0.7), false),
                        None => (alpha(SUBTLE, 0.5), true),
                    };
                    let spoke = [center, vertex(center, radius, i, 10.)];
                    stroke(&spoke, color, px(1.), dashed, false, window);
                }
            }
            let known: Vec<_> = (0..Axis::COUNT)
                .filter_map(|i| Some(vertex(center, radius, i, scores[i]?)))
                .collect();
            if known.len() >= 3 {
                fill(&known, alpha(INFO, if mini { 0.35 } else { 0.22 }), window);
            }
            if let Some(medians) = medians {
                let color = alpha(MUTED, 0.95);
                outline(medians, center, radius, color, px(1.5), true, window);
            }
            let width = px(if mini { 1.5 } else { 2. });
            outline(
                scores,
                center,
                radius,
                rgb(INFO).into(),
                width,
                false,
                window,
            );
        },
    )
    .size_full()
}
pub(super) fn miniature(scores: [Option<f64>; mrde_core::risk::Axis::COUNT]) -> AnyElement {
    let known: Vec<_> = scores.iter().flatten().copied().collect();
    let tooltip = if known.is_empty() {
        "Risk not calculated".to_string()
    } else {
        crate::tr!(
            "Risk {value0}/10 · {value1}/9 axes · cached",
            value0 = format!("{:.1}", known.iter().sum::<f64>() / known.len() as f64),
            value1 = known.len()
        )
    };
    div()
        .id("risk-miniature")
        .w(px(32.))
        .h(px(32.))
        .flex_shrink_0()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(drawing(scores, None, None, true))
        .into_any_element()
}
pub(super) fn radar(
    profile: &RiskProfile,
    baseline: Option<&Baseline>,
    thresholds: &VerdictThresholds,
    selected: Option<Axis>,
    cx: &mut Context<Cockpit>,
) -> AnyElement {
    let scores = std::array::from_fn(|i| profile.axes[i].score);
    let center = point(px(WIDTH / 2.), px(HEIGHT / 2.));
    let mut view = div()
        .debug_selector(|| "risk-radar".into())
        .relative()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .flex_shrink_0()
        .child(drawing(
            scores,
            baseline.map(|b| b.medians),
            Some(thresholds.deep_axis),
            false,
        ));
    for axis in Axis::ALL {
        let i = axis.index();
        let score = &profile.axes[i];
        let median = baseline.and_then(|b| b.medians[i]);
        let text = explanation(score, median);
        let color = score.score.map_or(SUBTLE, |s| severity(s, thresholds));
        let chosen = selected == Some(axis);
        let anchor = vertex(center, px(LABELS), i, 10.);
        let (cos, sin) = (angle(i).cos(), angle(i).sin());
        let values = div()
            .flex()
            .gap_1()
            .child(
                div()
                    .text_color(rgb(color))
                    .font_weight(FontWeight::MEDIUM)
                    .child(score.score.map_or("n/a".into(), |s| format!("{s:.1}"))),
            )
            .when_some(median, |row, median| {
                row.child(div().text_color(rgb(SUBTLE)).child(crate::tr!(
                    "· med {median}",
                    median = format!("{median:.1}")
                )))
            });
        let label = div()
            .id(SharedString::from(format!("risk-axis-{}", axis.id())))
            .debug_selector(move || format!("risk-axis-{}", axis.id()))
            .absolute()
            .w(px(LABEL_WIDTH))
            .flex()
            .flex_col()
            .text_size(px(12.))
            .line_height(px(16.))
            .cursor_pointer()
            .child(
                div()
                    .text_color(rgb(if chosen { INFO } else { INK }))
                    .when(chosen, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(crate::i18n::t(axis.label())),
            )
            .child(values)
            .tooltip({
                let text = text.clone();
                move |window, cx| Tooltip::new(text.clone()).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_risk_axis(axis, cx)));
        // Side labels start at their anchor and extend outward; top and bottom ones center.
        let label = if cos > 0.3 {
            label.left(anchor.x).top(anchor.y - px(16.)).items_start()
        } else if cos < -0.3 {
            label
                .left(anchor.x - px(LABEL_WIDTH))
                .top(anchor.y - px(16.))
                .items_end()
                .text_right()
        } else {
            let label = label.left(anchor.x - px(LABEL_WIDTH / 2.)).items_center();
            if sin < 0. {
                label.top(anchor.y - px(32.))
            } else {
                label.top(anchor.y)
            }
        };
        view = view.child(label);
        if let Some(value) = score.score {
            let position = vertex(center, px(RADIUS), i, value);
            view = view.child(
                div()
                    .id(SharedString::from(format!("risk-point-{}", axis.id())))
                    .absolute()
                    .left(position.x - px(10.))
                    .top(position.y - px(10.))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(if chosen { 14. } else { 11. }))
                            .rounded_full()
                            .border_2()
                            .border_color(rgb(super::widgets::CARD))
                            .bg(rgb(color)),
                    )
                    .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_risk_axis(axis, cx))),
            );
        }
    }
    view.into_any_element()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn vertices_have_consistent_orientation_and_clamp_scores() {
        let center = point(px(10.), px(10.));
        assert_eq!(vertex(center, px(5.), 0, 0.), center);
        let top = vertex(center, px(5.), 0, 10.);
        assert!((top.x - px(10.)).abs() < px(0.001));
        assert_eq!(top.y, px(5.));
        assert_eq!(
            vertex(center, px(5.), 2, 20.),
            vertex(center, px(5.), 2, 10.)
        );
        assert_eq!(ring(center, px(5.), 10.).len(), Axis::COUNT);
    }
    #[::core::prelude::v1::test]
    fn measurements_read_as_units_and_severity_follows_the_verdict_thresholds() {
        let raw = |value: f64, unit: &str| RawMetric {
            value,
            unit: unit.into(),
            method: String::new(),
            estimated: false,
            evidence: vec![],
        };
        assert_eq!(raw_text(&raw(1110., "lines")), "1,110 lines");
        assert_eq!(raw_text(&raw(0.42, "weighted fraction")), "42% weighted");
        assert_eq!(raw_text(&raw(12., "CCN delta")), "+12 CCN");
        assert_eq!(raw_text(&raw(-3., "CCN delta")), "−3 CCN");
        assert_eq!(
            raw_text(&raw(4.5, "weighted matches")),
            "4.5 weighted matches"
        );
        let thresholds = VerdictThresholds::default();
        assert_eq!(severity(3.4, &thresholds), SUCCESS);
        assert_eq!(severity(3.5, &thresholds), WARNING);
        assert_eq!(severity(8., &thresholds), ERROR);
    }
}
