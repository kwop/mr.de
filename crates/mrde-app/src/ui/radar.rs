//! Native GPUI paths; missing axes leave a gap rather than a zero-risk vertex.
use super::Cockpit;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, tooltip::Tooltip};
use mrde_core::risk::{Axis, Baseline, RiskProfile};
fn vertex(center: Point<Pixels>, radius: Pixels, index: usize, score: f64) -> Point<Pixels> {
    let angle = index as f32 * std::f32::consts::TAU / 8. - std::f32::consts::FRAC_PI_2;
    let radius = radius * (score.clamp(0., 10.) as f32 / 10.);
    center + point(radius * angle.cos(), radius * angle.sin())
}
fn polygon(
    points: &[Point<Pixels>],
    color: Hsla,
    width: Option<Pixels>,
    dashed: bool,
    window: &mut Window,
) {
    let Some(first) = points.first() else { return };
    let mut path = match width {
        Some(w) => PathBuilder::stroke(w),
        None => PathBuilder::fill(),
    };
    if dashed {
        path = path.dash_array(&[px(4.), px(3.)]);
    }
    path.move_to(*first);
    for p in &points[1..] {
        path.line_to(*p);
    }
    if points.len() > 2 {
        path.close();
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
fn drawing(
    scores: [Option<f64>; 8],
    baseline: Option<[Option<f64>; 8]>,
    mini: bool,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let center = bounds.center();
            let radius = if mini {
                bounds.size.width.min(bounds.size.height) / 2. - px(3.)
            } else {
                px(96.)
            };
            let line = cx.theme().border;
            let blue = cx.theme().primary;
            let gray = cx.theme().muted_foreground;
            for ring in if mini {
                vec![10.]
            } else {
                vec![2., 4., 6., 8., 10.]
            } {
                polygon(
                    &(0..8)
                        .map(|i| vertex(center, radius, i, ring))
                        .collect::<Vec<_>>(),
                    line,
                    Some(px(1.)),
                    false,
                    window,
                );
            }
            if !mini {
                for i in 0..8 {
                    polygon(
                        &[center, vertex(center, radius, i, 10.)],
                        line,
                        Some(px(1.)),
                        false,
                        window,
                    );
                }
            }
            if scores.iter().all(Option::is_some) {
                let mut fill = blue;
                fill.a = 0.12;
                polygon(
                    &(0..8)
                        .map(|i| vertex(center, radius, i, scores[i].unwrap()))
                        .collect::<Vec<_>>(),
                    fill,
                    None,
                    false,
                    window,
                );
            }
            for (values, color, dashed) in [
                (scores, blue, false),
                (baseline.unwrap_or([None; 8]), gray, true),
            ] {
                for i in 0..8 {
                    let j = (i + 1) % 8;
                    if let (Some(a), Some(b)) = (values[i], values[j]) {
                        polygon(
                            &[vertex(center, radius, i, a), vertex(center, radius, j, b)],
                            color,
                            Some(px(if mini { 1. } else { 2. })),
                            dashed,
                            window,
                        );
                    }
                }
            }
        },
    )
    .size_full()
}
pub(super) fn miniature(scores: [Option<f64>; 8]) -> AnyElement {
    div()
        .w(px(32.))
        .h(px(32.))
        .flex_shrink_0()
        .child(drawing(scores, None, true))
        .into_any_element()
}
pub(super) fn radar(
    profile: &RiskProfile,
    baseline: Option<&Baseline>,
    cx: &mut Context<Cockpit>,
) -> AnyElement {
    let scores = std::array::from_fn(|i| profile.axes[i].score);
    let center = point(px(200.), px(170.));
    let mut view = div()
        .debug_selector(|| "risk-radar".into())
        .relative()
        .bg(cx.theme().background)
        .w(px(400.))
        .h(px(340.))
        .flex_shrink_0()
        .child(drawing(scores, baseline.map(|b| b.medians), false));
    for axis in Axis::ALL {
        let i = axis.index();
        let score = &profile.axes[i];
        let median = baseline.and_then(|b| b.medians[i]);
        let text = match &score.raw {
            Some(raw) => format!(
                "{}: {:.2} {} → {:.1} / 10 (median: {})\n{}{}",
                axis.label(),
                raw.value,
                raw.unit,
                score.score.unwrap_or(0.),
                median
                    .map(|n| format!("{n:.1}"))
                    .unwrap_or_else(|| "n/a".into()),
                if raw.estimated { "Estimate · " } else { "" },
                raw.method
            ),
            None => format!(
                "{}: n/a\n{}",
                axis.label(),
                score.unavailable.as_deref().unwrap_or("Not measured")
            ),
        };
        let location = vertex(center, px(142.), i, 10.);
        view = view.child(
            div()
                .id(SharedString::from(format!("risk-axis-{}", axis.id())))
                .debug_selector(move || format!("risk-axis-{}", axis.id()))
                .absolute()
                .left(location.x - px(55.))
                .top(location.y - px(17.))
                .w(px(110.))
                .text_center()
                .text_size(px(11.))
                .line_height(px(15.))
                .text_color(cx.theme().foreground)
                .cursor_pointer()
                .child(axis.label())
                .child(
                    div().text_color(cx.theme().muted_foreground).child(
                        score
                            .score
                            .map(|s| format!("{s:.1}"))
                            .unwrap_or_else(|| "n/a".into()),
                    ),
                )
                .tooltip({
                    let text = text.clone();
                    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.risk_axis = Some(axis);
                    cx.notify();
                })),
        );
        if let Some(score) = score.score {
            let position = vertex(center, px(96.), i, score);
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
                            .size(px(12.))
                            .rounded_full()
                            .border_2()
                            .border_color(cx.theme().background)
                            .bg(cx.theme().primary),
                    )
                    .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.risk_axis = Some(axis);
                        cx.notify();
                    })),
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
        assert_eq!(vertex(center, px(5.), 2, 10.), point(px(15.), px(10.)));
        assert_eq!(vertex(center, px(5.), 2, 20.), point(px(15.), px(10.)));
    }
}
