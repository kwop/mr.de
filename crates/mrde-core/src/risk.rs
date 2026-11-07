//! Explainable risk measurements. Unknown is never equivalent to zero risk.
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    DiffSize,
    Files,
    Complexity,
    TestGap,
    SensitivePaths,
    Coupling,
    Lint,
    Debt,
    TicketAge,
}
impl Axis {
    pub const COUNT: usize = 9;
    pub const ALL: [Self; Self::COUNT] = [
        Self::DiffSize,
        Self::Files,
        Self::Complexity,
        Self::TestGap,
        Self::SensitivePaths,
        Self::Coupling,
        Self::Lint,
        Self::Debt,
        Self::TicketAge,
    ];
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|a| *a == self)
            .expect("known axis")
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::DiffSize => "diff_size",
            Self::Files => "files",
            Self::Complexity => "complexity",
            Self::TestGap => "test_gap",
            Self::SensitivePaths => "sensitive_paths",
            Self::Coupling => "coupling",
            Self::Lint => "lint",
            Self::Debt => "debt",
            Self::TicketAge => "ticket_age",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::DiffSize => "Diff size",
            Self::Files => "Files touched",
            Self::Complexity => "Added complexity",
            Self::TestGap => "Test gap",
            Self::SensitivePaths => "Sensitive paths",
            Self::Coupling => "Coupling",
            Self::Lint => "New warnings",
            Self::Debt => "Added debt",
            Self::TicketAge => "Ticket age",
        }
    }
}
// Old queue/profile payloads contain eight axes. Extend them with an unknown age;
// never turn an old cache entry into zero risk or fail to open the saved queue.
fn read_values<'de, D, T>(deserializer: D) -> std::result::Result<[T; Axis::COUNT], D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let mut values = Vec::<T>::deserialize(deserializer)?;
    if values.len() == Axis::COUNT - 1 {
        values.push(T::default());
    }
    values
        .try_into()
        .map_err(|_| serde::de::Error::custom("Invalid risk axis count"))
}
fn read_axes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<[AxisScore; Axis::COUNT], D::Error> {
    let mut values = Vec::<AxisScore>::deserialize(deserializer)?;
    if values.len() == Axis::COUNT - 1 {
        values.push(AxisScore::unavailable(
            Axis::TicketAge,
            "Ticket creation date not recorded in this cached profile",
        ));
    }
    if values
        .iter()
        .zip(Axis::ALL)
        .any(|(score, axis)| score.axis != axis)
    {
        return Err(serde::de::Error::custom("Invalid risk axis order"));
    }
    values
        .try_into()
        .map_err(|_| serde::de::Error::custom("Invalid risk axis count"))
}

/// Age is a prompt to revalidate requirements, not evidence that a ticket is obsolete.
pub fn ticket_age(created_at: Option<DateTime<Utc>>, at: DateTime<Utc>) -> Result<RawMetric> {
    let created = created_at.context("No creation date available for a primary linked ticket")?;
    ensure!(
        created <= at,
        "Ticket creation date is later than the review reference date"
    );
    Ok(RawMetric {
        value: (at - created).num_days() as f64,
        unit: "days".into(),
        method: format!(
            "Oldest primary ticket created {}; age as of {}. Revalidate the requirements: age alone does not establish that a ticket is obsolete.",
            created.format("%Y-%m-%d"),
            at.format("%Y-%m-%d")
        ),
        estimated: true,
        evidence: vec![],
    })
}

/// Refresh only the calendar-dependent measurement; commit-based checks stay cached.
pub fn refresh_ticket_age(
    profile: &mut RiskProfile,
    created: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
    curve: &[[f64; 2]],
) {
    let axis = Axis::TicketAge;
    profile.axes[axis.index()] =
        match ticket_age(created, at).and_then(|raw| Ok((normalize(raw.value, curve)?, raw))) {
            Ok((score, raw)) => AxisScore {
                axis,
                score: Some(score),
                raw: Some(raw),
                unavailable: None,
            },
            Err(error) => AxisScore::unavailable(axis, error.to_string()),
        };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub path: String,
    pub line: Option<u32>,
    pub detail: String,
    pub contribution: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMetric {
    pub value: f64,
    pub unit: String,
    pub method: String,
    pub estimated: bool,
    pub evidence: Vec<Evidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisScore {
    pub axis: Axis,
    pub raw: Option<RawMetric>,
    pub score: Option<f64>,
    pub unavailable: Option<String>,
}
impl AxisScore {
    pub fn unavailable(axis: Axis, reason: impl Into<String>) -> Self {
        Self {
            axis,
            raw: None,
            score: None,
            unavailable: Some(reason.into()),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskProfile {
    #[serde(deserialize_with = "read_axes")]
    pub axes: [AxisScore; Axis::COUNT],
    pub computed_at: DateTime<Utc>,
    pub head_sha: String,
    pub base_sha: String,
    pub config_key: String,
}
impl RiskProfile {
    pub fn available(&self) -> usize {
        self.axes.iter().filter(|a| a.score.is_some()).count()
    }
    pub fn mean(&self) -> Option<f64> {
        let values: Vec<f64> = self
            .axes
            .iter()
            .filter_map(|a| a.score)
            .filter(|n| n.is_finite())
            .collect();
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    }
    pub fn verdict(&self, thresholds: &VerdictThresholds) -> Option<Verdict> {
        let mean = self.mean()?;
        Some(
            if self
                .axes
                .iter()
                .filter_map(|a| a.score)
                .any(|s| s >= thresholds.deep_axis)
                || mean >= thresholds.deep_mean
            {
                Verdict::Deep
            } else if mean >= thresholds.standard_mean {
                Verdict::Standard
            } else {
                Verdict::Quick
            },
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Quick,
    Standard,
    Deep,
}
impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Quick => "Quick",
            Self::Standard => "Standard",
            Self::Deep => "Deep",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerdictThresholds {
    pub deep_axis: f64,
    pub deep_mean: f64,
    pub standard_mean: f64,
}
impl Default for VerdictThresholds {
    fn default() -> Self {
        Self {
            deep_axis: 8.,
            deep_mean: 6.,
            standard_mean: 3.5,
        }
    }
}
impl VerdictThresholds {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [self.deep_axis, self.deep_mean, self.standard_mean]
                .iter()
                .all(|v| v.is_finite() && (0.0..=10.0).contains(v))
                && self.standard_mean <= self.deep_mean,
            "Invalid verdict thresholds"
        );
        Ok(())
    }
}
pub fn validate_curve(points: &[[f64; 2]]) -> Result<()> {
    ensure!(
        points.len() >= 2,
        "Normalization requires at least two points"
    );
    ensure!(
        points
            .iter()
            .all(|p| p[0].is_finite() && p[1].is_finite() && (0.0..=10.0).contains(&p[1])),
        "Normalization points must be finite with scores in 0..10"
    );
    ensure!(
        points
            .windows(2)
            .all(|p| p[0][0] < p[1][0] && p[0][1] <= p[1][1]),
        "Normalization points must have increasing raw values and nondecreasing scores"
    );
    Ok(())
}
pub fn normalize(value: f64, points: &[[f64; 2]]) -> Result<f64> {
    validate_curve(points)?;
    ensure!(value.is_finite(), "The measured value is not finite");
    if value <= points[0][0] {
        return Ok(points[0][1]);
    }
    for pair in points.windows(2) {
        let ([x0, y0], [x1, y1]) = (pair[0], pair[1]);
        if value <= x1 {
            return Ok(y0 + (value - x0) / (x1 - x0) * (y1 - y0));
        }
    }
    Ok(points.last().expect("validated curve")[1])
}
pub fn median(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<_> = values.into_iter().filter(|n| n.is_finite()).collect();
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        None
    } else if n % 2 == 1 {
        Some(values[n / 2])
    } else {
        Some(values[n / 2 - 1] / 2. + values[n / 2] / 2.)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    #[serde(deserialize_with = "read_values")]
    pub medians: [Option<f64>; Axis::COUNT],
    #[serde(deserialize_with = "read_values")]
    pub samples: [usize; Axis::COUNT],
    pub requested: usize,
    pub merged_mrs: usize,
    pub computed_at: DateTime<Utc>,
    pub warnings: Vec<String>,
}
impl Baseline {
    pub fn from_profiles(profiles: &[RiskProfile], requested: usize, merged_mrs: usize) -> Self {
        Self {
            medians: std::array::from_fn(|i| {
                median(profiles.iter().filter_map(|p| p.axes[i].score))
            }),
            samples: std::array::from_fn(|i| {
                profiles
                    .iter()
                    .filter_map(|p| p.axes[i].score)
                    .filter(|v| v.is_finite())
                    .count()
            }),
            requested,
            merged_mrs,
            computed_at: Utc::now(),
            warnings: vec![],
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileContext {
    pub path: String,
    pub authors: Vec<(String, f64)>,
    pub commits: Vec<FileCommit>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCommit {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    /// Between the merge base and the MR head: part of the change under review.
    #[serde(default)]
    pub in_mr: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskReport {
    #[serde(default)]
    pub setup_messages: Vec<String>,
    pub profile: RiskProfile,
    pub verdict_thresholds: VerdictThresholds,
    pub stats: crate::models::DiffStats,
    /// Changed paths no axis counts: lockfiles, generated or vendored code, `exclude` globs.
    #[serde(default)]
    pub excluded: Vec<String>,
    pub pending_commands: Vec<PendingCommand>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingCommand {
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub preparation: Option<String>,
    pub key: String,
    pub id: String,
    pub command: Vec<String>,
}

/// Compact cached queue presentation; raw evidence is loaded only for the selected MR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    #[serde(deserialize_with = "read_values")]
    pub scores: [Option<f64>; Axis::COUNT],
    pub mean: Option<f64>,
    pub head_sha: String,
    pub computed_at: DateTime<Utc>,
}
impl From<&RiskReport> for RiskSummary {
    fn from(report: &RiskReport) -> Self {
        Self {
            scores: std::array::from_fn(|i| report.profile.axes[i].score),
            mean: report.profile.mean(),
            head_sha: report.profile.head_sha.clone(),
            computed_at: report.profile.computed_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(scores: &[Option<f64>]) -> RiskProfile {
        RiskProfile {
            axes: std::array::from_fn(|i| AxisScore {
                axis: Axis::ALL[i],
                score: scores.get(i).copied().flatten(),
                raw: None,
                unavailable: None,
            }),
            computed_at: Utc::now(),
            head_sha: "head".into(),
            base_sha: "base".into(),
            config_key: "v1".into(),
        }
    }
    #[test]
    fn ticket_age_uses_days_and_revalidates_a_cached_commit_as_time_passes() {
        let created: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let curve = [[0., 0.], [30., 0.], [90., 4.], [180., 8.], [365., 10.]];
        let mut report = profile(&[Some(1.); Axis::COUNT]);
        let measured = report.computed_at;
        for (days, score, verdict) in [
            (30, 0., Verdict::Quick),
            (90, 4., Verdict::Quick),
            (180, 8., Verdict::Deep),
            (365, 10., Verdict::Deep),
        ] {
            refresh_ticket_age(
                &mut report,
                Some(created),
                created + chrono::Duration::days(days),
                &curve,
            );
            let age = &report.axes[Axis::TicketAge.index()];
            assert_eq!(age.score, Some(score));
            assert_eq!(age.raw.as_ref().unwrap().value, days as f64);
            assert_eq!(report.verdict(&VerdictThresholds::default()), Some(verdict));
            assert_eq!(
                report.computed_at, measured,
                "technical measurements stay cached"
            );
        }
        refresh_ticket_age(&mut report, None, created, &curve);
        assert!(report.axes[Axis::TicketAge.index()].score.is_none());
        assert!(ticket_age(Some(created), created - chrono::Duration::days(1)).is_err());
    }

    #[test]
    fn old_eight_axis_caches_restore_with_an_unknown_ticket_age() {
        let original = profile(&[Some(2.); Axis::COUNT]);
        let mut json = serde_json::to_value(&original).unwrap();
        json["axes"].as_array_mut().unwrap().pop();
        let restored: RiskProfile = serde_json::from_value(json).unwrap();
        assert_eq!(restored.available(), 8);
        assert!(restored.axes[Axis::TicketAge.index()].score.is_none());
        let summary: RiskSummary = serde_json::from_value(serde_json::json!({
            "scores": [1,2,3,4,5,6,7,8], "mean": 4.5, "head_sha": "head",
            "computed_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(summary.scores[Axis::TicketAge.index()], None);
        let mut baseline =
            serde_json::to_value(Baseline::from_profiles(&[original], 30, 1)).unwrap();
        baseline["samples"].as_array_mut().unwrap().pop();
        baseline["medians"].as_array_mut().unwrap().pop();
        let restored: Baseline = serde_json::from_value(baseline).unwrap();
        assert_eq!(restored.samples[Axis::TicketAge.index()], 0);
        assert_eq!(restored.medians[Axis::TicketAge.index()], None);
    }

    #[test]
    fn baseline_counts_only_available_measurements_per_axis() {
        let baseline = Baseline::from_profiles(
            &[
                profile(&[Some(2.), None]),
                profile(&[Some(6.), Some(8.)]),
                profile(&[None, Some(10.)]),
            ],
            30,
            3,
        );
        assert_eq!(baseline.medians[0], Some(4.));
        assert_eq!(baseline.medians[1], Some(9.));
        assert_eq!(baseline.samples[..3], [2, 2, 0]);
        assert_eq!(baseline.medians[2], None);
    }
    #[test]
    fn normalization_interpolates_clamps_and_rejects_invalid_curves() {
        let points = [[0., 0.], [50., 2.], [300., 5.], [1000., 8.], [3000., 10.]];
        for (raw, score) in [
            (-1., 0.),
            (25., 1.),
            (50., 2.),
            (175., 3.5),
            (3000., 10.),
            (5000., 10.),
        ] {
            assert_eq!(normalize(raw, &points).unwrap(), score);
        }
        for points in [
            vec![],
            vec![[0., 0.]],
            vec![[0., 0.], [0., 10.]],
            vec![[0., 5.], [1., 2.]],
            vec![[0., 0.], [1., 11.]],
        ] {
            assert!(validate_curve(&points).is_err());
        }
        assert!(normalize(f64::NAN, &points).is_err());
    }
    #[test]
    fn baseline_uses_each_axes_available_samples_including_even_medians() {
        assert_eq!(median([9., 1., 3.]), Some(3.));
        assert_eq!(median([8., 2., 6., 4.]), Some(5.));
        assert_eq!(median([f64::NAN]), None);
        let baseline = Baseline::from_profiles(
            &[profile(&[Some(2.), None]), profile(&[Some(8.), Some(7.)])],
            30,
            30,
        );
        assert_eq!(baseline.medians[0], Some(5.));
        assert_eq!(baseline.medians[1], Some(7.));
        assert_eq!(baseline.samples[..2], [2, 1]);
        assert_eq!(baseline.medians[2], None);
    }
    #[test]
    fn verdict_respects_boundaries_and_never_turns_unknown_axes_into_zero() {
        let t = VerdictThresholds::default();
        for (scores, expected) in [
            (vec![Some(3.49); 8], Some(Verdict::Quick)),
            (vec![Some(3.5); 8], Some(Verdict::Standard)),
            (vec![Some(6.); 8], Some(Verdict::Deep)),
            (vec![Some(8.), None], Some(Verdict::Deep)),
            (vec![None; 8], None),
        ] {
            assert_eq!(profile(&scores).verdict(&t), expected);
        }
        assert_eq!(profile(&[Some(8.), None]).mean(), Some(8.));
    }
}
