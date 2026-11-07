//! Explainable measurements over commit contents, never the user's dirty working files.
use crate::{MrContext, RiskScorer};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use mrde_core::risk::{Axis, Evidence, RawMetric};
use std::collections::{BTreeMap, BTreeSet};

fn metric(
    value: f64,
    unit: &str,
    method: &str,
    estimated: bool,
    evidence: Vec<Evidence>,
) -> RawMetric {
    RawMetric {
        value,
        unit: unit.into(),
        method: method.into(),
        estimated,
        evidence,
    }
}
fn evidence(
    path: &str,
    line: Option<u32>,
    detail: impl Into<String>,
    contribution: f64,
) -> Evidence {
    Evidence {
        path: path.into(),
        line,
        detail: detail.into(),
        contribution,
    }
}
pub struct DiffScorer;
pub struct FileScorer;
pub struct TestGapScorer;
pub struct SensitivePathScorer;
pub struct CouplingScorer;
pub struct DebtScorer;
pub struct ComplexityScorer;
pub struct LintScorer;
#[async_trait]
impl RiskScorer for DiffScorer {
    fn id(&self) -> Axis {
        Axis::DiffSize
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        let evidence = ctx
            .stats
            .files
            .iter()
            .map(|f| {
                evidence(
                    &f.path,
                    None,
                    if f.binary {
                        "Binary change: line count unavailable".into()
                    } else {
                        format!("+{} −{}", f.added, f.removed)
                    },
                    f.changed() as f64,
                )
            })
            .collect();
        Ok(metric(
            ctx.stats.files.iter().map(|f| f.changed()).sum::<u64>() as f64,
            "lines",
            "git diff --numstat merge-base…head; binary lines excluded",
            ctx.stats.files.iter().any(|f| f.binary),
            evidence,
        ))
    }
}
#[async_trait]
impl RiskScorer for FileScorer {
    fn id(&self) -> Axis {
        Axis::Files
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        Ok(metric(
            ctx.stats.files.len() as f64,
            "files",
            "git diff --numstat; renames count once",
            false,
            ctx.stats
                .files
                .iter()
                .map(|f| evidence(&f.path, None, "Changed file", 1.))
                .collect(),
        ))
    }
}
fn source(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some(
            "rs" | "go"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "dart"
                | "java"
                | "kt"
                | "swift"
                | "c"
                | "h"
                | "cpp"
                | "cs"
                | "rb"
                | "php"
                | "vue"
                | "svelte"
        )
    )
}
#[async_trait]
impl RiskScorer for TestGapScorer {
    fn id(&self) -> Axis {
        Axis::TestGap
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        let files: Vec<_> = ctx
            .files
            .iter()
            .filter(|f| source(f.path()) && !mrde_insights::lang::is_test(f.path()))
            .collect();
        ensure!(!files.is_empty(), "No changed source files to assess");
        if let Some(report) = &ctx.coverage {
            let coverage = crate::reports::coverage(report, &ctx.worktree)?;
            let mut considered = 0;
            let mut uncovered = 0;
            let mut details = Vec::new();
            for file in &files {
                for (line, _) in file.added_lines() {
                    if let Some(hits) = coverage.get(&(file.path().to_string(), line)) {
                        considered += 1;
                        if *hits == 0 {
                            uncovered += 1;
                            details.push(evidence(
                                file.path(),
                                Some(line),
                                "Uncovered changed executable line",
                                1.,
                            ));
                        }
                    }
                }
            }
            ensure!(
                considered > 0,
                "Coverage report has no matching changed executable lines"
            );
            return Ok(metric(
                uncovered as f64 / considered as f64,
                "fraction",
                "Uncovered / changed executable lines in configured coverage report; report freshness is not verified",
                true,
                details,
            ));
        }
        let tests = ctx
            .files
            .iter()
            .filter(|f| source(f.path()) && mrde_insights::lang::is_test(f.path()))
            .count();
        let gap = (1. - tests as f64 / files.len() as f64).max(0.);
        Ok(metric(
            gap,
            "fraction",
            "Heuristic: 1 − changed test files / changed source files (not coverage)",
            true,
            files
                .iter()
                .map(|f| {
                    evidence(
                        f.path(),
                        None,
                        format!("{tests} changed test files / {} source files", files.len()),
                        gap,
                    )
                })
                .collect(),
        ))
    }
}
#[async_trait]
impl RiskScorer for SensitivePathScorer {
    fn id(&self) -> Axis {
        Axis::SensitivePaths
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        let rules: Vec<_> = ctx
            .config
            .sensitive
            .iter()
            .map(|r| {
                Ok((
                    globset::Glob::new(&r.glob)?.compile_matcher(),
                    r.weight,
                    &r.glob,
                ))
            })
            .collect::<Result<_>>()?;
        let mut weighted = 0.;
        let mut details = Vec::new();
        let total = ctx.stats.files.iter().map(|f| f.changed()).sum::<u64>();
        ensure!(
            total > 0,
            "No textual diff lines; sensitive line share unavailable"
        );
        for f in &ctx.stats.files {
            if let Some((_, weight, pattern)) = rules
                .iter()
                .filter(|(g, _, _)| g.is_match(&f.path))
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                let value = f.changed() as f64 * weight;
                weighted += value;
                details.push(evidence(
                    &f.path,
                    None,
                    format!("{pattern}: {} lines × {weight}", f.changed()),
                    value / total as f64,
                ));
            }
        }
        Ok(metric(
            weighted / total as f64,
            "weighted fraction",
            "Maximum matching glob weight per file × changed lines / all textual changed lines",
            ctx.stats.files.iter().any(|f| f.binary),
            details,
        ))
    }
}
#[async_trait]
impl RiskScorer for DebtScorer {
    fn id(&self) -> Axis {
        Axis::Debt
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        let rules: Vec<_> = ctx
            .config
            .debt
            .iter()
            .map(|r| Ok((regex::Regex::new(&r.regex)?, r)))
            .collect::<Result<_>>()?;
        let mut total = 0.;
        let mut details = Vec::new();
        for f in &ctx.files {
            for (line, text) in f.added_lines() {
                for (pattern, rule) in &rules {
                    let value = pattern.find_iter(text).count() as f64 * rule.weight;
                    if value > 0. {
                        total += value;
                        details.push(evidence(
                            f.path(),
                            Some(line),
                            format!("{}: {}", rule.id, text.trim()),
                            value,
                        ));
                    }
                }
            }
        }
        Ok(metric(
            total,
            "weighted matches",
            "Configured regex matches on added lines; syntax and comments are not interpreted",
            true,
            details,
        ))
    }
}
/// Full manifest dependency names: include target-specific Cargo sections and workspace tables.
fn dependencies(path: &str, text: &str) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    if text.is_empty() {
        return Ok(names);
    }
    match path.rsplit('/').next().unwrap_or(path) {
        "Cargo.toml" => {
            fn collect(value: &toml::Value, names: &mut BTreeSet<String>) {
                if let Some(table) = value.as_table() {
                    for (key, child) in table {
                        if matches!(
                            key.as_str(),
                            "dependencies" | "dev-dependencies" | "build-dependencies"
                        ) {
                            if let Some(deps) = child.as_table() {
                                names.extend(deps.keys().cloned());
                            }
                        } else if child.is_table() {
                            collect(child, names);
                        }
                    }
                }
            }
            collect(&toml::from_str(text)?, &mut names);
        }
        "package.json" => {
            let value: serde_json::Value = serde_json::from_str(text)?;
            for key in [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ] {
                if let Some(deps) = value[key].as_object() {
                    names.extend(deps.keys().cloned());
                }
            }
        }
        "pubspec.yaml" => {
            let value: serde_yaml::Value = serde_yaml::from_str(text)?;
            for key in ["dependencies", "dev_dependencies"] {
                if let Some(deps) = value[key].as_mapping() {
                    names.extend(deps.keys().filter_map(|k| k.as_str().map(str::to_string)));
                }
            }
        }
        "go.mod" => {
            let mut block = false;
            for line in text.lines() {
                let line = line.split("//").next().unwrap_or_default().trim();
                if line == "require (" {
                    block = true;
                    continue;
                }
                if line == ")" {
                    block = false;
                    continue;
                }
                if let Some(entry) = line
                    .strip_prefix("require ")
                    .or_else(|| block.then_some(line))
                {
                    if let Some(name) = entry.split_whitespace().next() {
                        names.insert(name.into());
                    }
                }
            }
        }
        _ => {}
    };
    Ok(names)
}
fn imports(text: &str) -> Result<BTreeMap<String, u32>> {
    // Import targets, not identifiers: changing imported names or formatting is not a new edge.
    let quoted =
        regex::Regex::new(r#"(?:from\s*|import\s*(?:\(\s*)?|require\(\s*)['"]([^'"]+)['"]"#)?;
    let named = regex::Regex::new(
        r"^\s*(?:pub\s+)?(?:use\s+([\w:]+)|from\s+([\w.]+)\s+import|import\s+([\w.]+))",
    )?;
    let mut result = BTreeMap::new();
    let mut go_block = false;
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line == "import (" {
            go_block = true;
            continue;
        }
        if go_block && line == ")" {
            go_block = false;
            continue;
        }
        let target = quoted
            .captures(line)
            .and_then(|c| c.get(1).map(|v| v.as_str().to_string()))
            .or_else(|| {
                named.captures(line).and_then(|c| {
                    c.iter()
                        .skip(1)
                        .flatten()
                        .next()
                        .map(|m| m.as_str().trim_end_matches(':').to_string())
                })
            })
            .or_else(|| {
                go_block
                    .then(|| line.split('"').nth(1).map(str::to_string))
                    .flatten()
            });
        if let Some(target) = target {
            result.entry(target).or_insert(i as u32 + 1);
        }
    }
    Ok(result)
}
#[async_trait]
impl RiskScorer for CouplingScorer {
    fn id(&self) -> Axis {
        Axis::Coupling
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        ensure!(
            ctx.contents_error.is_none(),
            "{}",
            ctx.contents_error.as_deref().unwrap_or_default()
        );
        let mut details = Vec::new();
        for file in &ctx.files {
            let (before, after) = ctx
                .contents
                .get(file.path())
                .context("Missing file contents")?;
            let (before, after) = (
                before.as_deref().unwrap_or_default(),
                after.as_deref().unwrap_or_default(),
            );
            let old = dependencies(file.path(), before)?;
            let new = dependencies(file.path(), after)?;
            details.extend(
                new.difference(&old).map(|name| {
                    evidence(file.path(), None, format!("Added dependency: {name}"), 1.)
                }),
            );
            if source(file.path()) {
                let old = imports(before)?;
                for (name, line) in imports(after)? {
                    if !old.contains_key(&name) {
                        details.push(evidence(
                            file.path(),
                            Some(line),
                            format!("New import target: {name}"),
                            1.,
                        ));
                    }
                }
            }
        }
        Ok(metric(
            details.len() as f64,
            "edges",
            "Added manifest dependencies + new import targets per changed source file; textual import heuristic",
            true,
            details,
        ))
    }
}
async fn command(
    ctx: &MrContext,
    config: &crate::config::Command,
    root: &std::path::Path,
    extra: Vec<String>,
) -> Result<mrde_checks::runner::Output> {
    ensure!(
        config.approved,
        "{} is proposed and needs local approval",
        config.id
    );
    let mut argv = config.command.clone();
    argv.extend(extra);
    mrde_checks::runner::run(
        &mrde_checks::Check {
            id: config.id.clone(),
            command: argv,
            approval: mrde_checks::Approval::Approved,
        },
        root,
        ctx.timeout(),
    )
    .await
}
#[async_trait]
impl RiskScorer for ComplexityScorer {
    fn id(&self) -> Axis {
        Axis::Complexity
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        let (before, after) = ctx.snapshot_paths()?;
        let files: Vec<_> = ctx.files.iter().filter(|f| source(f.path())).collect();
        ensure!(
            !files.is_empty(),
            "No supported source files for complexity"
        );
        let mut totals = Vec::new();
        for (base, root) in [(true, &before), (false, &after)] {
            let selected: Vec<_> = files
                .iter()
                .filter_map(|f| {
                    if base {
                        f.old_path.as_deref()
                    } else {
                        f.new_path.as_deref()
                    }
                })
                .filter(|p| root.join(p).is_file())
                .map(|p| format!("./{p}"))
                .collect();
            if selected.is_empty() {
                totals.push(BTreeMap::new());
                continue;
            }
            let output = command(ctx, &ctx.config.complexity, root, selected).await?;
            ensure!(
                output.code == Some(0),
                "Complexity check failed with exit {:?}",
                output.code
            );
            let mut values = crate::reports::complexity(&output.stdout, root)?;
            if base {
                for file in &files {
                    if let (Some(old), Some(new)) = (&file.old_path, &file.new_path)
                        && old != new
                        && let Some(value) = values.remove(old)
                    {
                        values.insert(new.clone(), value);
                    }
                }
            }
            totals.push(values);
        }
        let mut details = Vec::new();
        let mut delta = 0.;
        let names: BTreeSet<_> = totals.iter().flat_map(|t| t.keys()).collect();
        for path in names {
            let change = totals[1].get(path).copied().unwrap_or(0.)
                - totals[0].get(path).copied().unwrap_or(0.);
            delta += change;
            if change != 0. {
                details.push(evidence(
                    path,
                    None,
                    "Cyclomatic complexity delta (Lizard)",
                    change,
                ));
            }
        }
        Ok(metric(
            delta,
            "CCN delta",
            "Sum of Lizard function CCN after − before; negative deltas normalize to zero",
            false,
            details,
        ))
    }
}
#[async_trait]
impl RiskScorer for LintScorer {
    fn id(&self) -> Axis {
        Axis::Lint
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        ensure!(!ctx.config.lint.is_empty(), "No lint commands configured");
        let (before, after) = ctx.snapshot_paths()?;
        let mut details = Vec::new();
        for config in &ctx.config.lint {
            let mut diagnostics = Vec::new();
            for root in [&before, &after] {
                let result = command(ctx, config, root, vec![]).await?;
                let warnings = crate::reports::diagnostics(config, &result.stdout, root)?;
                ensure!(
                    result.code == Some(0) || result.code == Some(1) && !warnings.is_empty(),
                    "Lint {} failed without usable diagnostics (exit {:?})",
                    config.id,
                    result.code
                );
                diagnostics.push(warnings);
            }
            let mut old = BTreeMap::<_, usize>::new();
            for d in &diagnostics[0] {
                *old.entry(d.key()).or_default() += 1;
            }
            for d in &diagnostics[1] {
                let count = old.entry(d.key()).or_default();
                if *count > 0 {
                    *count -= 1;
                } else {
                    details.push(evidence(
                        &d.path,
                        Some(d.line),
                        format!("{} · {}: {}", config.id, d.rule, d.message),
                        1.,
                    ));
                }
            }
        }
        Ok(metric(
            details.len() as f64,
            "warnings",
            "New diagnostic multiset by path/rule/message; line shifts do not add warnings",
            false,
            details,
        ))
    }
}
