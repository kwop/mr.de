//! Frontend-specific output; risk measurements remain in mrde-risk.
use crate::ScanReport;
use mrde_core::risk::Axis;
use std::fmt::Write;

fn safe(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn text(report: &ScanReport) -> String {
    let mode = match report.input.mode {
        crate::InputMode::WorkingTree => "Working tree",
        crate::InputMode::Commits => "Commits",
    };
    let mut out = format!("MR.DE risk radar — {mode}\n");
    let summary = &report.summary;
    let verdict = summary.verdict.map_or("Unavailable", |v| v.label());
    let mean = summary
        .mean
        .map_or_else(|| "n/a".into(), |v| format!("{v:.1}/10"));
    let _ = writeln!(
        out,
        "{verdict} review · mean {mean} · {}/9 measured axes{}\n",
        summary.measured_axes,
        if summary.partial { " · partial" } else { "" }
    );
    for score in &report.report.profile.axes {
        let label = score.axis.label();
        if let (Some(value), Some(raw)) = (score.score, &score.raw) {
            let filled = (value * 2.).round().clamp(0., 20.) as usize;
            let bar = format!("{}{}", "#".repeat(filled), "-".repeat(20 - filled));
            let _ = writeln!(
                out,
                "{label:20} {value:4.1}/10 [{bar}] {:.2} {}{}",
                raw.value,
                safe(&raw.unit),
                if raw.estimated { " (estimate)" } else { "" }
            );
            for item in raw.evidence.iter().take(3) {
                let line = item.line.map_or_else(String::new, |n| format!(":{n}"));
                let _ = writeln!(out, "  {}{line}: {}", safe(&item.path), safe(&item.detail));
            }
        } else {
            let _ = writeln!(
                out,
                "{label:20} n/a — {}",
                safe(score.unavailable.as_deref().unwrap_or("No measurement"))
            );
        }
    }
    for command in &report.report.pending_commands {
        let _ = writeln!(
            out,
            "\nProposed command {}: {}\n  fingerprint: {}",
            safe(&command.id),
            serde_json::to_string(&command.command).unwrap_or_default(),
            command.key
        );
    }
    for warning in &report.warnings {
        let _ = writeln!(out, "\nNote: {}", safe(warning));
    }
    out
}

fn point(index: usize, radius: f64) -> (f64, f64) {
    let angle =
        index as f64 * std::f64::consts::TAU / Axis::COUNT as f64 - std::f64::consts::FRAC_PI_2;
    (340. + radius * angle.cos(), 240. + radius * angle.sin())
}

/// Standalone SVG: partial profiles have gaps, never a fabricated zero point.
pub fn svg(report: &ScanReport) -> String {
    let mut out = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 680 520" role="img" aria-labelledby="title description"><title id="title">MR.DE risk radar</title><desc id="description">Nine risk axes from zero to ten. Missing axes have no point. Team median is unavailable in standalone scans.</desc><rect width="680" height="520" rx="12" fill="#292c32"/><g font-family="system-ui,sans-serif" font-size="13" fill="#eef0f4">"##,
    );
    for level in 1..=5 {
        let points = (0..Axis::COUNT)
            .map(|i| {
                let (x, y) = point(i, level as f64 * 30.);
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            out,
            r##"<polygon points="{points}" fill="none" stroke="#535965"/>"##
        );
    }
    for axis in Axis::ALL {
        let (x, y) = point(axis.index(), 150.);
        let _ = write!(
            out,
            r##"<line x1="340" y1="240" x2="{x:.1}" y2="{y:.1}" stroke="#535965"/>"##
        );
        let (x, y) = point(axis.index(), 195.);
        let value = report.report.profile.axes[axis.index()]
            .score
            .map_or_else(|| "n/a".into(), |s| format!("{s:.1}/10"));
        let _ = write!(
            out,
            r#"<text x="{x:.1}" y="{y:.1}" text-anchor="middle">{}<tspan x="{x:.1}" dy="18">{value}</tspan></text>"#,
            axis.label()
        );
    }
    let scores = &report.report.profile.axes;
    if scores.iter().all(|s| s.score.is_some()) {
        let points = scores
            .iter()
            .map(|s| {
                let (x, y) = point(s.axis.index(), s.score.unwrap_or(0.) * 15.);
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            out,
            r##"<polygon points="{points}" fill="#8ab4f8" fill-opacity="0.12"/>"##
        );
    }
    for axis in Axis::ALL {
        let i = axis.index();
        if let Some(value) = scores[i].score {
            let (x, y) = point(i, value * 15.);
            if let Some(next) = scores[(i + 1) % 8].score {
                let (nx, ny) = point((i + 1) % 8, next * 15.);
                let _ = write!(
                    out,
                    r##"<line x1="{x:.1}" y1="{y:.1}" x2="{nx:.1}" y2="{ny:.1}" stroke="#8ab4f8" stroke-width="2"/>"##
                );
            }
            let _ = write!(
                out,
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="4" fill="#8ab4f8" stroke="#292c32" stroke-width="2"/>"##
            );
        }
    }
    let verdict = report.summary.verdict.map_or("Unavailable", |v| v.label());
    let mean = report
        .summary
        .mean
        .map_or_else(|| "n/a".into(), |n| format!("{n:.1}/10"));
    let _ = write!(
        out,
        r#"<text x="340" y="480" text-anchor="middle">{verdict} review · mean {mean} · {}/9 measured axes</text><text x="340" y="503" text-anchor="middle">Team median unavailable</text></g></svg>"#,
        report.summary.measured_axes
    );
    out
}
