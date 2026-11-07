//! Headless radar application service, reusable by CLI and future frontends.
//! No GPUI, credential store, network client, or Claude subprocess dependency.
mod git;
pub mod render;

use anyhow::{Context, Result, ensure};
use mrde_core::risk::{Axis, PendingCommand, RiskReport, Verdict};
use mrde_risk::config::RiskConfig;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::Read, path::PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

/// Explicit input: callers choose their settings folder; tests never read user settings.
#[derive(Debug, Clone)]
pub struct ScanRequest {
    pub repository: PathBuf,
    pub base: String,
    /// None captures current files; Some compares committed revisions.
    pub head: Option<String>,
    pub config_dir: PathBuf,
    pub project: String,
    pub run_checks: bool,
    /// Explicitly prepare recognized tools and dependencies; may download packages.
    pub prepare_tools: bool,
    /// Optional managed-tool cache; callers/tests can isolate all installation writes.
    pub tool_cache: Option<PathBuf>,
    /// Exact command fingerprints, approved for this invocation only.
    pub approved_commands: HashSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    WorkingTree,
    Commits,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanInput {
    pub repository: PathBuf,
    pub mode: InputMode,
    pub requested_base: String,
    pub requested_head: Option<String>,
    pub source_head: String,
    pub analyzed_tree: String,
    pub checks_enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub verdict: Option<Verdict>,
    pub mean: Option<f64>,
    pub measured_axes: usize,
    pub partial: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub schema_version: u32,
    pub tool_version: String,
    pub input: ScanInput,
    pub summary: Summary,
    pub report: RiskReport,
    /// Standalone scans have no forge identity/history; never fabricate a team median.
    pub baseline: Option<mrde_core::risk::Baseline>,
    pub warnings: Vec<String>,
}

pub async fn scan(request: ScanRequest) -> Result<ScanReport> {
    ensure!(
        request.run_checks || request.approved_commands.is_empty(),
        "--allow-command requires --run-checks"
    );
    let snapshot =
        git::Snapshot::prepare(&request.repository, &request.base, request.head.as_deref()).await?;
    let config_root = tempfile::tempdir()?;
    if let Some(config) = snapshot.file("risk.toml", 1024 * 1024).await? {
        std::fs::write(config_root.path().join("risk.toml"), config)?;
    }
    ensure!(
        !request.prepare_tools || request.run_checks,
        "--prepare-tools requires --run-checks"
    );
    let config = RiskConfig::load(
        &request.config_dir,
        &request.project,
        config_root.path(),
        &request.approved_commands,
    )?;
    let mut warnings =
        vec!["Team median unavailable: standalone scans do not load forge history.".into()];
    let coverage_result: Result<Option<String>> = async {
        Ok(if let Some(path) = config.coverage_file(&snapshot.source) {
            if request.head.is_none() {
                // Coverage is often ignored by Git. Restrict this read to the source tree,
                // bound its size, and let the engine report malformed data as n/a.
                let source = snapshot.source.clone();
                tokio::task::spawn_blocking(move || -> Result<Option<String>> {
                    if !path.try_exists()? {
                        return Ok(None);
                    }
                    ensure!(
                        path.canonicalize()?.starts_with(&source),
                        "Coverage report escapes the repository"
                    );
                    let mut report = String::new();
                    std::fs::File::open(path)?
                        .take(16 * 1024 * 1024 + 1)
                        .read_to_string(&mut report)?;
                    ensure!(
                        report.len() <= 16 * 1024 * 1024,
                        "Coverage report exceeds 16 MiB"
                    );
                    Ok(Some(report))
                })
                .await??
            } else {
                snapshot
                    .file(&config.coverage_path, 16 * 1024 * 1024)
                    .await?
                    .map(String::from_utf8)
                    .transpose()
                    .context("Coverage report is not UTF-8")?
            }
        } else {
            None
        })
    }
    .await;
    let coverage = match coverage_result {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("Coverage unavailable: {error:#}"));
            Some(format!("Invalid coverage report: {error:#}"))
        }
    };
    if coverage.is_some() {
        warnings.push(
            "Coverage report freshness is not verified; the report must match the analyzed code."
                .into(),
        );
    }
    if !request.run_checks {
        warnings.push("External complexity and lint checks are disabled; use --run-checks to run locally trusted commands.".into());
    }
    if request.head.is_none() {
        warnings.push("The analyzed head is a temporary snapshot, not a commit in your repository. Submodule contents are not traversed.".into());
    }
    let mut context =
        mrde_risk::context(&snapshot.root, &snapshot.base, &snapshot.head, config).await?;
    context.set_coverage(coverage, &snapshot.source);
    if let Some(cache) = &request.tool_cache {
        context.environment = mrde_risk::setup::environment(Some(cache));
    }
    let commands: Vec<_> = std::iter::once(&context.config.complexity)
        .chain(context.config.lint_commands())
        .collect();
    for approved in &request.approved_commands {
        ensure!(
            commands.iter().any(|c| c.key() == *approved),
            "An approved command fingerprint is not in the current configuration; scan again before approving it"
        );
    }
    let pending: Vec<_> = commands
        .iter()
        .filter(|c| !c.approved)
        .map(|c| PendingCommand {
            key: c.key(),
            id: c.id.clone(),
            command: c.command.clone(),
            directory: c.directory.clone(),
            preparation: c.preparation.map(|p| format!("{p:?}")),
        })
        .collect();
    let lint_configured = !context.config.lint_commands().is_empty();
    if !request.run_checks {
        for command in context.config.commands_mut() {
            command.approved = false;
        }
    }
    if request.prepare_tools {
        let cache = request
            .tool_cache
            .clone()
            .or_else(mrde_risk::setup::default_cache)
            .context("Tool cache unavailable; set --tool-cache")?;
        mrde_risk::setup::prepare(&mut context, &cache).await;
    }
    let mut report = mrde_risk::analyze(context).await;
    report.pending_commands = pending;
    if !request.run_checks {
        for axis in [Axis::Complexity, Axis::Lint] {
            if axis == Axis::Complexity || lint_configured {
                report.profile.axes[axis.index()] = mrde_core::risk::AxisScore::unavailable(
                    axis,
                    "External checks disabled; use --run-checks to enable locally trusted commands",
                );
            }
        }
    }
    let summary = Summary {
        verdict: report.profile.verdict(&report.verdict_thresholds),
        mean: report.profile.mean(),
        measured_axes: report.profile.available(),
        partial: report.profile.available() != Axis::ALL.len(),
    };
    Ok(ScanReport {
        schema_version: SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").into(),
        input: ScanInput {
            repository: snapshot.source.clone(),
            mode: if request.head.is_some() {
                InputMode::Commits
            } else {
                InputMode::WorkingTree
            },
            requested_base: request.base,
            requested_head: request.head,
            source_head: snapshot.source_head.clone(),
            analyzed_tree: snapshot.tree.clone(),
            checks_enabled: request.run_checks,
        },
        summary,
        report,
        baseline: None,
        warnings,
    })
}
