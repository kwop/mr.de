//! Local risk analysis; no network clients and no implicit Git fetches.
pub mod config;
mod reports;
pub mod scorers;
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use config::RiskConfig;
use futures_util::{StreamExt, stream};
use mrde_core::{
    models::{DiffStats, StatsSource},
    risk::*,
};
use mrde_insights::diff::FileDiff;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct MrContext {
    pub worktree: PathBuf,
    pub base_sha: String,
    pub head_sha: String,
    pub stats: DiffStats,
    pub files: Vec<FileDiff>,
    pub contents: BTreeMap<String, (Option<String>, Option<String>)>,
    pub contents_error: Option<String>,
    pub coverage: Option<String>,
    pub config: RiskConfig,
    snapshots: Option<tempfile::TempDir>,
    snapshot_error: Option<String>,
}
impl MrContext {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout_seconds)
    }
    fn snapshot_paths(&self) -> Result<(PathBuf, PathBuf)> {
        let root = self
            .snapshots
            .as_ref()
            .with_context(|| {
                self.snapshot_error
                    .as_deref()
                    .unwrap_or("Snapshots unavailable")
                    .to_string()
            })?
            .path();
        Ok((root.join("base"), root.join("head")))
    }
}
#[async_trait]
pub trait RiskScorer: Send + Sync {
    fn id(&self) -> Axis;
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric>;
}
/// Collect commit data off the UI thread. Lint/complexity receive disposable exports;
/// local modifications, the Git index, and the MR worktree are never changed.
pub async fn context(
    worktree: &Path,
    base: &str,
    head: &str,
    config: RiskConfig,
) -> Result<MrContext> {
    config.validate()?;
    let (base_sha, diff, files) = mrde_git::risk::diff(worktree, base, head).await?;
    ensure!(
        diff.len() <= 32 * 1024 * 1024,
        "Diff exceeds the 32 MiB risk-analysis limit"
    );
    let exclude: Vec<_> = config
        .exclude
        .iter()
        .map(|g| Ok(globset::Glob::new(g)?.compile_matcher()))
        .collect::<Result<_>>()?;
    let parsed = mrde_insights::diff::parse(&diff)
        .into_iter()
        .filter(|f| !f.binary && !exclude.iter().any(|g| g.is_match(f.path())))
        .collect::<Vec<_>>();
    let mut ctx = MrContext {
        worktree: worktree.into(),
        base_sha,
        head_sha: head.into(),
        stats: DiffStats {
            sha: head.into(),
            source: StatsSource::Git,
            files,
        },
        files: parsed,
        contents: BTreeMap::new(),
        contents_error: None,
        coverage: None,
        config,
        snapshots: None,
        snapshot_error: None,
    };
    if ctx.files.len() > 1000 {
        ctx.contents_error =
            Some("More than 1000 changed files; content analysis is unavailable".into());
    } else {
        let results = stream::iter(&ctx.files)
            .map(|f| async {
                let before = match &f.old_path {
                    Some(p) => mrde_git::show_file(worktree, &ctx.base_sha, p).await?,
                    None => None,
                };
                let after = match &f.new_path {
                    Some(p) => mrde_git::show_file(worktree, head, p).await?,
                    None => None,
                };
                ensure!(
                    before.as_ref().map_or(0, String::len) <= 1024 * 1024
                        && after.as_ref().map_or(0, String::len) <= 1024 * 1024,
                    "File exceeds 1 MiB content limit: {}",
                    f.path()
                );
                Ok::<_, anyhow::Error>((f.path().to_string(), (before, after)))
            })
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;
        for result in results {
            match result {
                Ok((path, contents)) => {
                    ctx.contents.insert(path, contents);
                }
                Err(e) => ctx.contents_error = Some(e.to_string()),
            }
        }
    }
    if let Some(path) = ctx.config.coverage_file(worktree) {
        match tokio::task::spawn_blocking(move || std::fs::read_to_string(path)).await? {
            Ok(text) => {
                ensure!(
                    text.len() <= 16 * 1024 * 1024,
                    "Coverage report exceeds 16 MiB"
                );
                ctx.coverage = Some(text);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => ctx.coverage = Some(format!("Invalid coverage report: {e}")),
        }
    }
    if ctx.config.complexity.approved || ctx.config.lint.iter().any(|c| c.approved) {
        let snapshots = tempfile::tempdir()?;
        let result = tokio::time::timeout(ctx.timeout(), async {
            mrde_git::risk::export(worktree, &ctx.base_sha, &snapshots.path().join("base")).await?;
            mrde_git::risk::export(worktree, head, &snapshots.path().join("head")).await
        })
        .await;
        match result {
            Ok(Ok(())) => ctx.snapshots = Some(snapshots),
            other => ctx.snapshot_error = Some(format!("Snapshot preparation failed: {other:?}")),
        }
    }
    Ok(ctx)
}
pub async fn analyze(ctx: MrContext) -> RiskReport {
    use scorers::*;
    let scorers: Vec<Box<dyn RiskScorer>> = vec![
        Box::new(DiffScorer),
        Box::new(FileScorer),
        Box::new(ComplexityScorer),
        Box::new(TestGapScorer),
        Box::new(SensitivePathScorer),
        Box::new(CouplingScorer),
        Box::new(LintScorer),
        Box::new(DebtScorer),
    ];
    let results = stream::iter(scorers)
        .map(|scorer| {
            let ctx = &ctx;
            async move {
                let axis = scorer.id();
                let result = tokio::time::timeout(ctx.timeout(), scorer.measure(ctx)).await;
                match result {
                    Ok(Ok(raw)) if raw.value.is_finite() => {
                        match normalize(raw.value, &ctx.config.normalization[axis.id()]) {
                            Ok(score) => AxisScore {
                                axis,
                                raw: Some(raw),
                                score: Some(score),
                                unavailable: None,
                            },
                            Err(e) => AxisScore::unavailable(axis, e.to_string()),
                        }
                    }
                    Ok(Ok(_)) => AxisScore::unavailable(axis, "Measurement is not finite"),
                    Ok(Err(e)) => AxisScore::unavailable(axis, format!("{e:#}")),
                    Err(_) => AxisScore::unavailable(
                        axis,
                        format!("Scorer timed out after {}s", ctx.config.timeout_seconds),
                    ),
                }
            }
        })
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    let mut axes = std::array::from_fn(|i| AxisScore::unavailable(Axis::ALL[i], "Not measured"));
    for score in results {
        let index = score.axis.index();
        axes[index] = score;
    }
    let pending_commands = std::iter::once(&ctx.config.complexity)
        .chain(&ctx.config.lint)
        .filter(|c| !c.approved)
        .map(|c| PendingCommand {
            key: c.key(),
            id: c.id.clone(),
            command: c.command.clone(),
        })
        .collect();
    RiskReport {
        profile: RiskProfile {
            axes,
            computed_at: chrono::Utc::now(),
            head_sha: ctx.head_sha,
            base_sha: ctx.base_sha,
            config_key: ctx.config.fingerprint(),
        },
        verdict_thresholds: ctx.config.verdict,
        stats: ctx.stats,
        pending_commands,
    }
}
#[cfg(test)]
mod tests;
