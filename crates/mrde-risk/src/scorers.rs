//! Explainable measurements over commit contents, never the user's dirty working files.
use crate::{MrContext, RiskScorer};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use mrde_core::{
    models::FileStat,
    risk::{Axis, Evidence, RawMetric},
};
use mrde_insights::{diff::FileDiff, lang::is_test};
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
/// Changed files every axis counts: generated, vendored, and `exclude` files are skipped.
fn counted(ctx: &MrContext) -> Vec<&FileStat> {
    ctx.stats
        .files
        .iter()
        .filter(|f| !ctx.excluded.contains(&f.path))
        .collect()
}
fn skipped(ctx: &MrContext) -> String {
    match ctx.excluded.len() {
        0 => String::new(),
        1 => "; 1 generated or excluded file not counted".into(),
        n => format!("; {n} generated or excluded files not counted"),
    }
}
/// First line (1-based) of a Rust file's inline test module in the new version: lines from
/// there on are test code.
fn test_module(ctx: &MrContext, file: &FileDiff) -> Option<u32> {
    if !file.path().ends_with(".rs") {
        return None;
    }
    let after = ctx.contents.get(file.path())?.1.as_deref()?;
    after
        .lines()
        .position(|line| line.trim_start().starts_with("#[cfg(test)]"))
        .map(|index| index as u32 + 1)
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
        let files = counted(ctx);
        let evidence = files
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
            files.iter().map(|f| f.changed()).sum::<u64>() as f64,
            "lines",
            &format!(
                "git diff --numstat merge-base…head; binary lines excluded{}",
                skipped(ctx)
            ),
            files.iter().any(|f| f.binary),
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
        let files = counted(ctx);
        Ok(metric(
            files.len() as f64,
            "files",
            &format!("git diff --numstat; renames count once{}", skipped(ctx)),
            false,
            files
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
/// What a test file is about: `user` for `tests/user.rs`, `user_test.go`, `User.test.tsx`,
/// `UserTest.kt`, or `src/user.rs` itself.
fn subject(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    let stem = stem.strip_prefix("test_").unwrap_or(stem);
    ["_tests", "_test", "_spec", "Tests", "Test"]
        .iter()
        .find_map(|suffix| stem.strip_suffix(suffix).filter(|rest| !rest.is_empty()))
        .unwrap_or(stem)
        .to_lowercase()
}
/// A test file covers a source file about the same subject, one whose own module directory
/// holds it (`src/user/tests.rs` for `src/user.rs` or `src/user/mod.rs`), or, for a generic
/// `tests.rs`, the files beside it.
pub(crate) fn covers(test: &str, source: &str) -> bool {
    let (directory, name) = source.rsplit_once('/').unwrap_or(("", source));
    let module = match name {
        "mod.rs" | "lib.rs" | "main.rs" | "__init__.py" => directory,
        _ => source.rsplit_once('.').map_or(source, |(module, _)| module),
    };
    let home = test.rsplit_once('/').map_or("", |(directory, _)| directory);
    let topic = subject(test);
    let generic = matches!(topic.as_str(), "test" | "tests" | "mod");
    home == module || (generic && home == directory) || (!generic && topic == subject(source))
}
#[async_trait]
impl RiskScorer for TestGapScorer {
    fn id(&self) -> Axis {
        Axis::TestGap
    }
    async fn measure(&self, ctx: &MrContext) -> Result<RawMetric> {
        let sources: Vec<_> = ctx
            .files
            .iter()
            .filter(|f| f.new_path.is_some() && source(f.path()) && !is_test(f.path()))
            .collect();
        ensure!(!sources.is_empty(), "No changed source files to assess");
        // Added lines outside an inline test module.
        let code = |file: &FileDiff| -> Vec<u32> {
            let tests = test_module(ctx, file);
            file.added_lines()
                .map(|(line, _)| line)
                .filter(|line| tests.is_none_or(|start| *line < start))
                .collect()
        };
        if let Some(report) = &ctx.coverage {
            let coverage = crate::reports::coverage(report, &ctx.worktree)?;
            let mut considered = 0;
            let mut details = Vec::new();
            for file in &sources {
                for line in code(file) {
                    if let Some(hits) = coverage.get(&(file.path().to_string(), line)) {
                        considered += 1;
                        if *hits == 0 {
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
                details.len() as f64,
                "lines",
                "Changed executable lines the configured coverage report marks as uncovered; report freshness is not verified",
                true,
                details,
            ));
        }
        let markers = regex::Regex::new(
            r"#\[(?:\w+::)?test\b|#\[cfg\(test\)\]|(?:^|\s)(?:describe|it|test)\s*\(|\bdef test_|\bfunc Test|@Test\b",
        )?;
        let tests: Vec<&str> = ctx
            .files
            .iter()
            .filter(|f| is_test(f.path()) && f.added() > 0)
            .map(FileDiff::path)
            .collect();
        let mut untested = 0;
        let mut details = Vec::new();
        for file in &sources {
            let lines = code(file);
            let Some(first) = lines.first().copied() else {
                continue;
            };
            let module = test_module(ctx, file);
            let inline = file.added_lines().any(|(line, text)| {
                module.is_some_and(|start| line >= start) || markers.is_match(text)
            });
            let related = tests.iter().find(|test| covers(test, file.path()));
            details.push(if inline {
                evidence(file.path(), None, "Tests changed in the same file", 0.)
            } else if let Some(test) = related {
                evidence(file.path(), None, format!("Tests changed in {test}"), 0.)
            } else {
                untested += lines.len();
                evidence(
                    file.path(),
                    Some(first),
                    format!("{} added lines without a related test change", lines.len()),
                    lines.len() as f64,
                )
            });
        }
        details.sort_by(|a, b| b.contribution.total_cmp(&a.contribution));
        Ok(metric(
            untested as f64,
            "lines",
            "Added source lines whose file has no related test change in this MR (a test file with the same name, its own test module, or inline tests). Heuristic, not coverage",
            true,
            details,
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
        let files = counted(ctx);
        let total = files.iter().map(|f| f.changed()).sum::<u64>();
        ensure!(
            total > 0,
            "No textual diff lines; sensitive line share unavailable"
        );
        for f in &files {
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
            &format!(
                "Maximum matching glob weight per file × changed lines / all textual changed lines{}",
                skipped(ctx)
            ),
            files.iter().any(|f| f.binary),
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
        // Test code may unwrap and leave markers: debt is measured on product code.
        for f in ctx.files.iter().filter(|f| !is_test(f.path())) {
            let tests = test_module(ctx, f);
            for (line, text) in f.added_lines() {
                if tests.is_some_and(|start| line >= start) {
                    continue;
                }
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
            "Configured regex matches on added product-code lines (test files and inline test modules skipped); syntax and comments are not interpreted",
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
                    && let Some(name) = entry.split_whitespace().next()
                {
                    names.insert(name.into());
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
/// The module an import depends on: `serde` for `serde::Deserialize`, `crate::store` for
/// `crate::store::Store`. `None` for the Rust standard library and relative `self`/`super`.
pub(crate) fn dependency(target: &str) -> Option<String> {
    let mut parts = target.split("::");
    let root = parts.next()?;
    match root {
        "std" | "core" | "alloc" | "self" | "super" => None,
        "crate" => Some(match parts.next() {
            Some(module) => format!("crate::{module}"),
            None => "crate".into(),
        }),
        _ if target.contains("::") => Some(root.into()),
        _ => Some(target.into()),
    }
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
        for file in ctx.files.iter().filter(|f| !is_test(f.path())) {
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
            // A new file's imports describe the new module; existing code that starts
            // depending on another module is what couples the codebase further.
            if source(file.path()) && file.old_path.is_some() && !before.is_empty() {
                let known: BTreeSet<String> = imports(before)?
                    .keys()
                    .filter_map(|t| dependency(t))
                    .collect();
                let tests = test_module(ctx, file);
                let mut added = BTreeMap::new();
                for (target, line) in imports(after)? {
                    if tests.is_some_and(|start| line >= start) {
                        continue;
                    }
                    if let Some(module) = dependency(&target).filter(|m| !known.contains(m)) {
                        let first = added.entry(module).or_insert(line);
                        *first = (*first).min(line);
                    }
                }
                let mut added: Vec<_> = added.into_iter().collect();
                added.sort_by_key(|(_, line)| *line);
                details.extend(added.into_iter().map(|(module, line)| {
                    evidence(
                        file.path(),
                        Some(line),
                        format!("New import target: {module}"),
                        1.,
                    )
                }));
            }
        }
        Ok(metric(
            details.len() as f64,
            "edges",
            "Added manifest dependencies + modules that existing product files import for the first time (standard library and self/super imports ignored); textual heuristic",
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
        let (before, after) = ctx.snapshot_paths(&ctx.config.complexity).await?;
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
            let output = command(ctx, &ctx.config.complexity, root, selected.clone()).await?;
            ensure!(
                output.code == Some(0),
                "Complexity check failed with exit {:?}",
                output.code
            );
            let mut values = crate::reports::complexity(&output.stdout, root)?;
            for path in &selected {
                ensure!(
                    values.contains_key(path.trim_start_matches("./")),
                    "Lizard did not report a changed source file: {path}"
                );
            }
            values.retain(|path, _| selected.iter().any(|p| p.trim_start_matches("./") == path));
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
        let mut details = Vec::new();
        for config in &ctx.config.lint {
            let (before, after) = ctx.snapshot_paths(config).await?;
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
