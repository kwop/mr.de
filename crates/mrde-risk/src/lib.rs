//! Local risk analysis; no network clients and no implicit Git fetches.
pub mod config;
pub mod detect;
mod javascript;
mod reports;
pub mod scorers;
pub mod setup;
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use config::RiskConfig;
use futures_util::{FutureExt, StreamExt, stream};
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
    /// Changed paths no axis counts: generated, vendored, or matching `exclude`.
    pub excluded: Vec<String>,
    pub files: Vec<FileDiff>,
    pub contents: BTreeMap<String, (Option<String>, Option<String>)>,
    pub contents_error: Option<String>,
    pub coverage: Option<String>,
    coverage_root: PathBuf,
    pub config: RiskConfig,
    snapshots: Option<tempfile::TempDir>,
    snapshot_error: Option<String>,
    pub environment: mrde_checks::environment::Environment,
    pub setup_messages: Vec<String>,
    pub check_errors: BTreeMap<String, String>,
    detection_error: Option<String>,
    prepared_checks: std::collections::BTreeSet<String>,
}
impl MrContext {
    /// Associate an external coverage report with the checkout that produced its paths.
    /// The Git snapshot may live elsewhere; this changes no measured code or files.
    pub fn set_coverage(&mut self, report: Option<String>, source_root: &Path) {
        self.coverage = report;
        self.coverage_root = source_root.into();
    }
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout_seconds)
    }
    async fn snapshot_paths(&self, command: &config::Command) -> Result<(PathBuf, PathBuf)> {
        ensure!(
            command.approved,
            "{} is proposed and needs local approval",
            command.id
        );
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
        // Each tool receives its own pair; a formatting lint cannot change Lizard's input.
        let root = root.join(command.key());
        let before = root.join("base");
        let after = root.join("head");
        mrde_git::risk::export(&self.worktree, &self.base_sha, &before).await?;
        mrde_git::risk::export(&self.worktree, &self.head_sha, &after).await?;
        Ok((before, after))
    }
}
#[async_trait]
pub trait RiskScorer: Send + Sync {
    fn id(&self) -> Axis;
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric>;
}
/// Collect commit data off the UI thread. Lint/complexity receive disposable exports;
/// local modifications, the Git index, and the MR worktree are never changed.
pub fn context<'a>(
    worktree: &'a Path,
    base: &'a str,
    head: &'a str,
    config: RiskConfig,
) -> futures_util::future::BoxFuture<'a, Result<MrContext>> {
    context_impl(worktree, base, head, config).boxed()
}
async fn context_impl(
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
    let skipped = |path: &str| {
        mrde_insights::lang::is_generated(path) || exclude.iter().any(|g| g.is_match(path))
    };
    let parsed = mrde_insights::diff::parse(&diff)
        .into_iter()
        .filter(|f| !f.binary && !skipped(f.path()))
        .collect::<Vec<_>>();
    let excluded = files
        .iter()
        .filter(|f| skipped(&f.path))
        .map(|f| f.path.clone())
        .collect();
    let mut ctx = MrContext {
        worktree: worktree.into(),
        base_sha,
        head_sha: head.into(),
        stats: DiffStats {
            sha: head.into(),
            source: StatsSource::Git,
            files,
        },
        excluded,
        files: parsed,
        contents: BTreeMap::new(),
        contents_error: None,
        coverage: None,
        coverage_root: worktree.into(),
        config,
        snapshots: None,
        snapshot_error: None,
        environment: setup::environment(None),
        setup_messages: vec![],
        check_errors: BTreeMap::new(),
        detection_error: None,
        prepared_checks: Default::default(),
    };
    if ctx.config.autodetect_lint && ctx.config.lint.is_empty() {
        let changed: Vec<_> = ctx.files.iter().map(|f| f.path().to_string()).collect();
        match detect::lint_commands(worktree, head, &changed).await {
            Ok(mut commands) => {
                for command in &mut commands {
                    command.approved = ctx.config.approvals.contains(&command.key());
                }
                ctx.config.detected_lint = commands;
            }
            Err(error) => ctx.detection_error = Some(format!("Lint detection failed: {error:#}")),
        }
    }
    if ctx.files.len() > 1000 {
        ctx.contents_error =
            Some("More than 1000 changed files; content analysis is unavailable".into());
    } else {
        let pending = ctx
            .files
            .iter()
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
            .collect::<Vec<_>>();
        let results = stream::iter(pending)
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
        match tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut text = String::new();
            std::fs::File::open(path)?
                .take(16 * 1024 * 1024 + 1)
                .read_to_string(&mut text)?;
            if text.len() > 16 * 1024 * 1024 {
                return Err(std::io::Error::other("Coverage report exceeds 16 MiB"));
            }
            Ok::<_, std::io::Error>(text)
        })
        .await?
        {
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
    match tempfile::tempdir() {
        Ok(root) => ctx.snapshots = Some(root),
        Err(error) => {
            ctx.snapshot_error = Some(format!("Cannot create isolated command snapshots: {error}"))
        }
    }
    Ok(ctx)
}
pub fn analyze(ctx: MrContext) -> futures_util::future::BoxFuture<'static, RiskReport> {
    analyze_impl(ctx).boxed()
}
async fn analyze_impl(ctx: MrContext) -> RiskReport {
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
    let pending = scorers
        .into_iter()
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
        .collect::<Vec<_>>();
    let results = stream::iter(pending)
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    let mut axes = std::array::from_fn(|i| AxisScore::unavailable(Axis::ALL[i], "Not measured"));
    for score in results {
        let index = score.axis.index();
        axes[index] = score;
    }
    let pending_commands = std::iter::once(&ctx.config.complexity)
        .chain(ctx.config.lint_commands())
        .filter(|c| !c.approved)
        .map(|c| PendingCommand {
            key: c.key(),
            id: c.id.clone(),
            command: c.command.clone(),
            directory: c.directory.clone(),
            preparation: c.preparation.map(|p| {
                let root = c.preparation_directory.as_deref().unwrap_or(&c.directory);
                format!(
                    "{p:?} in {}",
                    if root.is_empty() {
                        "repository root"
                    } else {
                        root
                    }
                )
            }),
        })
        .collect();
    RiskReport {
        setup_messages: ctx.setup_messages,
        profile: RiskProfile {
            axes,
            computed_at: chrono::Utc::now(),
            head_sha: ctx.head_sha,
            base_sha: ctx.base_sha,
            config_key: ctx.config.fingerprint(),
        },
        verdict_thresholds: ctx.config.verdict,
        stats: ctx.stats,
        excluded: ctx.excluded,
        pending_commands,
    }
}
#[cfg(test)]
mod tests;
