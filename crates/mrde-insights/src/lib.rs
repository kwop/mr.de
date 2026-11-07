//! Understanding complex merge requests from the MR worktree: symbols, impact, file graph,
//! history, contracts, dependencies, and risky patterns. Analysis functions are pure;
//! [`analyze`] gathers their input with read-only Git commands.
pub mod contracts;
pub mod deps;
pub mod diff;
pub mod history;
pub mod imports;
pub mod lang;
pub mod patterns;
pub mod symbols;
pub mod visual;

use diff::FileDiff;
use futures_util::{StreamExt, stream};
use mrde_core::{
    insights::*,
    models::{DiffStats, FileStat, MrDetail, StatsSource},
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

/// Files whose contents are read for symbols, patterns, imports, and dependencies.
const MAX_CONTENT_FILES: usize = 300;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_IMPACT_SYMBOLS: usize = 12;
const MAX_CALLERS: usize = 25;
const MAX_BLAMED_HUNKS: usize = 25;
/// Names too generic for a textual caller search.
const GENERIC: &[&str] = &[
    "init", "main", "new", "get", "set", "run", "handle", "handler", "render", "error", "string",
    "create", "update", "delete", "close", "open", "start", "stop", "value", "props", "state",
    "default", "index", "test", "setup", "build", "config", "options", "data", "result",
];

type Contents = HashMap<String, (Option<String>, Option<String>)>;

fn stats(files: &[FileDiff], sha: &str) -> DiffStats {
    DiffStats {
        sha: sha.into(),
        source: StatsSource::Git,
        files: files
            .iter()
            .map(|f| FileStat {
                path: f.path().into(),
                added: f.added() as u64,
                removed: f.removed() as u64,
                binary: f.binary,
            })
            .collect(),
    }
}

async fn read_contents(
    worktree: &Path,
    base: &str,
    head: &str,
    files: &[FileDiff],
) -> (Contents, Vec<String>) {
    let wanted: Vec<&FileDiff> = files
        .iter()
        .filter(|f| {
            let path = f.path();
            !f.binary
                && !lang::is_generated(path)
                && (lang::of(path) != lang::Lang::Other || deps::is_manifest(path))
        })
        .take(MAX_CONTENT_FILES)
        .collect();
    // Owned inputs keep the futures Send for any caller lifetime.
    let (worktree, base, head) = (worktree.to_path_buf(), base.to_string(), head.to_string());
    let jobs: Vec<(String, Option<String>, Option<String>)> = wanted
        .into_iter()
        .map(|f| (f.path().to_string(), f.old_path.clone(), f.new_path.clone()))
        .collect();
    let results: Vec<_> = stream::iter(jobs)
        .map(|(path, old_path, new_path)| {
            let (worktree, base, head) = (worktree.clone(), base.clone(), head.clone());
            async move {
                let old = match old_path {
                    Some(old_path) => mrde_git::show_file(&worktree, &base, &old_path).await,
                    None => Ok(None),
                };
                let new = match new_path {
                    Some(new_path) => mrde_git::show_file(&worktree, &head, &new_path).await,
                    None => Ok(None),
                };
                (path, old, new)
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut contents = Contents::new();
    let mut warnings = Vec::new();
    for (path, old, new) in results {
        let bounded = |text: anyhow::Result<Option<String>>, warnings: &mut Vec<String>| match text
        {
            Ok(Some(text)) if text.len() <= MAX_FILE_BYTES => Some(text),
            Ok(_) => None,
            Err(error) => {
                warnings.push(format!("{path}: {error:#}"));
                None
            }
        };
        let old = bounded(old, &mut warnings);
        let new = bounded(new, &mut warnings);
        contents.insert(path, (old, new));
    }
    (contents, warnings)
}

fn change_rank(change: SymbolChangeKind) -> u8 {
    match change {
        SymbolChangeKind::SignatureChanged => 0,
        SymbolChangeKind::Removed => 1,
        SymbolChangeKind::Modified => 2,
        SymbolChangeKind::Added => 3,
    }
}

/// Textual callers of changed definitions at head; the MR's own files are marked.
async fn impacts(
    worktree: &Path,
    head: &str,
    symbols: &[SymbolChange],
    changed: &HashSet<String>,
) -> (Vec<Impact>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let candidates = symbols
        .iter()
        .filter(|s| s.change != SymbolChangeKind::Added && !lang::is_test(&s.path))
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Type | SymbolKind::Class
            )
        })
        .filter(|s| {
            let word = s.name.rsplit('.').next().unwrap_or(&s.name);
            word.len() >= 4 && !GENERIC.contains(&word.to_lowercase().as_str())
        })
        .take(MAX_IMPACT_SYMBOLS);
    for symbol in candidates {
        let word = symbol.name.rsplit('.').next().unwrap_or(&symbol.name);
        let specs = lang::pathspecs(lang::of(&symbol.path));
        let output = match mrde_git::grep_word(worktree, head, word, specs).await {
            Ok(output) => output,
            Err(error) => {
                warnings.push(format!("Callers of {}: {error:#}", symbol.name));
                continue;
            }
        };
        let callable = matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method);
        let mut callers: Vec<CallSite> = output
            .lines()
            .filter_map(|line| {
                let rest = line.split_once(':')?.1;
                let (path, rest) = rest.split_once(':')?;
                let (number, text) = rest.split_once(':')?;
                Some((
                    path.to_string(),
                    number.parse::<u32>().ok()?,
                    text.trim().to_string(),
                ))
            })
            .filter(|(path, line, _)| {
                !(path == &symbol.path
                    && *line == symbol.line
                    && symbol.change != SymbolChangeKind::Removed)
            })
            .filter(|(_, _, text)| {
                !callable
                    || text.contains(&format!("{word}("))
                    || text.contains(&format!("{word}<"))
                    || text.contains(&format!("<{word}"))
                    || text.contains(&format!("{word},"))
                    || text.contains(&format!("{word})"))
            })
            .map(|(path, line, text)| CallSite {
                in_mr: changed.contains(&path),
                test: lang::is_test(&path),
                path,
                line,
                text: text.chars().take(160).collect(),
            })
            .collect();
        // Callers the MR did not touch are the ones a reviewer must check.
        callers.sort_by(|a, b| {
            a.in_mr
                .cmp(&b.in_mr)
                .then_with(|| a.test.cmp(&b.test))
                .then_with(|| a.path.cmp(&b.path))
        });
        let total = callers.len();
        callers.truncate(MAX_CALLERS);
        if total > 0 {
            out.push(Impact {
                symbol: symbol.name.clone(),
                path: symbol.path.clone(),
                change: symbol.change,
                more: (total - callers.len()) as u32,
                callers,
            });
        }
    }
    (out, warnings)
}

async fn roots(worktree: &Path, head: &str, tracked: &[String]) -> imports::Roots {
    let mut roots = imports::Roots::default();
    for path in tracked {
        let (directory, name) = path.rsplit_once('/').unwrap_or(("", path));
        match name {
            "go.mod" => {
                if let Ok(Some(text)) = mrde_git::show_file(worktree, head, path).await
                    && let Some(module) =
                        text.lines().find_map(|l| l.trim().strip_prefix("module "))
                {
                    roots
                        .go_modules
                        .push((directory.into(), module.trim().into()));
                }
            }
            "package.json" if !path.contains("node_modules/") => {
                roots.packages.push(directory.into())
            }
            "Cargo.toml" => roots.crates.push(directory.into()),
            _ => {}
        }
    }
    roots
}

pub async fn analyze(worktree: &Path, detail: &MrDetail) -> anyhow::Result<Insights> {
    let (base, head) = (detail.mr.base_sha.as_str(), detail.mr.sha.as_str());
    let files = diff::parse(&mrde_git::diff_hunks(worktree, base, head).await?);
    let mut insights = Insights {
        sha: head.into(),
        stats: Some(stats(&files, head)),
        ..Default::default()
    };
    let (contents, warnings) = read_contents(worktree, base, head, &files).await;
    insights.warnings.extend(warnings);
    let content = |path: &str| contents.get(path).cloned().unwrap_or_default();

    for file in &files {
        let (old, new) = content(file.path());
        if lang::of(file.path()) != lang::Lang::Other && !lang::is_generated(file.path()) {
            insights
                .symbols
                .extend(symbols::compare(file, old.as_deref(), new.as_deref()));
            insights
                .patterns
                .extend(patterns::scan(file, new.as_deref()));
        }
        if deps::is_manifest(file.path()) {
            insights.dependencies.extend(deps::compare(
                file.path(),
                old.as_deref(),
                new.as_deref(),
            ));
        }
        if deps::is_lockfile(file.path()) {
            let added: Vec<&str> = file.added_lines().map(|(_, l)| l).collect();
            let removed: Vec<&str> = file.removed_lines().map(|(_, l)| l).collect();
            let (added, removed) = deps::lockfile_counts(file.path(), &added, &removed);
            insights.lockfiles.push(LockfileChange {
                path: file.path().into(),
                added: added as u32,
                removed: removed as u32,
            });
        }
    }
    insights.symbols.sort_by(|a, b| {
        change_rank(a.change)
            .cmp(&change_rank(b.change))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    insights.patterns.sort_by(|a, b| {
        a.level
            .cmp(&b.level)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });

    let tracked = mrde_git::list_files(worktree, head)
        .await
        .unwrap_or_else(|error| {
            insights.warnings.push(format!("Listing files: {error:#}"));
            Vec::new()
        });
    let mut env_docs = Vec::new();
    for path in tracked.iter().filter(|p| contracts::is_env_doc(p)).take(20) {
        if let Ok(Some(text)) = mrde_git::show_file(worktree, head, path).await {
            env_docs.push(text);
        }
    }
    insights.contracts = contracts::detect(&files, &env_docs);

    let sources: Vec<(String, String)> = files
        .iter()
        .filter_map(|f| content(f.path()).1.map(|text| (f.path().to_string(), text)))
        .collect();
    let roots = roots(worktree, head, &tracked).await;
    insights.graph = imports::graph(&sources, &roots, &contracts::added_routes(&files));

    let changed: HashSet<String> = files.iter().map(|f| f.path().to_string()).collect();
    let (impacts, warnings) = impacts(worktree, head, &insights.symbols, &changed).await;
    insights.impacts = impacts;
    insights.warnings.extend(warnings);

    let existing: Vec<String> = files.iter().filter_map(|f| f.old_path.clone()).collect();
    match mrde_git::log_files(worktree, base, "6 months ago", &existing).await {
        Ok(output) => {
            let log = history::parse_log(&output);
            let paths: Vec<(String, bool)> = files
                .iter()
                .map(|f| (f.path().to_string(), f.old_path.is_none()))
                .collect();
            insights.history = history::file_history(&paths, &log);
        }
        Err(error) => insights.warnings.push(format!("History: {error:#}")),
    }

    let codeowners = [
        "CODEOWNERS",
        ".github/CODEOWNERS",
        ".gitlab/CODEOWNERS",
        "docs/CODEOWNERS",
    ]
    .into_iter()
    .find(|p| tracked.iter().any(|t| t == p));
    let rules = match codeowners {
        Some(path) => mrde_git::show_file(worktree, head, path)
            .await
            .ok()
            .flatten()
            .map(|t| history::parse_codeowners(&t))
            .unwrap_or_default(),
        None => Vec::new(),
    };
    if let Some(stats) = &insights.stats {
        for zone in mrde_core::stats::zones(&stats.files)
            .into_iter()
            .filter(|z| z.name != "(root)")
            .take(8)
        {
            match mrde_git::log_files(
                worktree,
                head,
                "12 months ago",
                std::slice::from_ref(&zone.name),
            )
            .await
            {
                Ok(output) => {
                    let log = history::parse_log(&output);
                    if !log.is_empty() {
                        insights
                            .experts
                            .push(history::expert(&zone.name, &log, &rules));
                    }
                }
                Err(error) => insights
                    .warnings
                    .push(format!("Experts for {}: {error:#}", zone.name)),
            }
        }
    }

    let mut hunks: Vec<(&FileDiff, &diff::Hunk)> = files
        .iter()
        .filter(|f| f.old_path.is_some() && !lang::is_generated(f.path()) && !f.binary)
        .flat_map(|f| {
            f.hunks
                .iter()
                .filter(|h| !h.removed.is_empty())
                .map(move |h| (f, h))
        })
        .collect();
    hunks.sort_by_key(|(_, hunk)| std::cmp::Reverse(hunk.removed.len()));
    for (file, hunk) in hunks.into_iter().take(MAX_BLAMED_HUNKS) {
        let old_path = file.old_path.as_deref().unwrap_or_default();
        let origin =
            match mrde_git::blame(worktree, base, old_path, hunk.old_start, hunk.old_count).await {
                Ok(output) => history::parse_blame(&output),
                Err(error) => {
                    insights
                        .warnings
                        .push(format!("Blame {old_path}: {error:#}"));
                    None
                }
            };
        insights.removed.push(RemovedBlock {
            path: old_path.into(),
            line: hunk.old_start,
            lines: hunk.removed.iter().take(12).cloned().collect(),
            pure_deletion: hunk.added.is_empty(),
            origin,
        });
    }
    insights
        .removed
        .sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    Ok(insights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrde_core::models::{MergeRequest, MrId};
    use tokio::process::Command;

    async fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
    async fn commit(dir: &Path, author: &str, message: &str) -> String {
        git(dir, &["add", "-A"]).await;
        let name = format!("user.name={author}");
        git(
            dir,
            &[
                "-c",
                &name,
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-q",
                "-m",
                message,
            ],
        )
        .await;
        git(dir, &["rev-parse", "HEAD"]).await
    }
    fn write(dir: &Path, path: &str, text: &str) {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[tokio::test]
    async fn analyzes_a_real_repository_end_to_end() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        git(repo, &["init", "-q"]).await;
        write(
            repo,
            "backend/go.mod",
            "module example\n\nrequire github.com/jackc/pgx/v5 v5.5.4\n",
        );
        write(
            repo,
            "backend/usecases/create.go",
            "package usecases\n\nfunc ValidateSchedule(t T) error {\n\tif t.ReturnAt.Before(t.PickupAt) {\n\t\treturn ErrInvalid\n\t}\n\treturn nil\n}\n",
        );
        write(
            repo,
            "backend/api/handlers.go",
            "package api\n\nimport \"example/usecases\"\n\nfunc Handle() { usecases.ValidateSchedule(t) }\n",
        );
        write(repo, "env-sample", "DATABASE_URL=postgres://\n");
        write(repo, "CODEOWNERS", "/backend/ @lea\n");
        commit(repo, "Léa", "feat: invoices").await;
        write(
            repo,
            "backend/usecases/create.go",
            "package usecases\n\nfunc ValidateSchedule(t T) error {\n\treturn nil\n}\n",
        );
        commit(repo, "Léa", "fix: due date before issue date (#1187)").await;
        let base = git(repo, &["rev-parse", "HEAD"]).await;
        write(
            repo,
            "backend/usecases/create.go",
            "package usecases\n\nfunc ValidateSchedule(t T, series bool) error {\n\treturn nil\n}\n",
        );
        write(
            repo,
            "backend/usecases/series.go",
            "package usecases\n\nfunc CreateSeries(ds []D) {\n\tfor _, d := range ds {\n\t\trepo.Insert(ctx, d)\n\t}\n\tlimit := os.Getenv(\"SERIES_MAX\")\n}\n",
        );
        write(
            repo,
            "backend/api/handlers.go",
            "package api\n\nimport \"example/usecases\"\n\nfunc Handle() { usecases.ValidateSchedule(t, false); r.POST(\"/api/series\", h) }\n",
        );
        write(
            repo,
            "backend/migrations/000041_series.up.sql",
            "ALTER TABLE invoices ADD COLUMN schedule_id uuid NOT NULL;\n",
        );
        write(
            repo,
            "backend/go.mod",
            "module example\n\nrequire github.com/jackc/pgx/v5 v6.0.0\n",
        );
        let head = commit(repo, "Mara", "feat: series").await;
        let detail = MrDetail {
            mr: MergeRequest {
                risk: None,
                id: MrId {
                    project: 1,
                    iid: 231,
                },
                title: "Series".into(),
                description: String::new(),
                project: "p".into(),
                author: "mara".into(),
                web_url: "https://example.invalid".into(),
                sha: head,
                base_sha: base,
                created_at: chrono::Utc::now(),
                labels: vec![],
                ticket_labels: vec![],
                pipeline: None,
                approvals: None,
                changed_lines: None,
            },
            files: vec![],
            issues: vec![],
            ssh_url: String::new(),
        };
        let insights = analyze(repo, &detail).await.unwrap();
        assert!(insights.warnings.is_empty(), "{:?}", insights.warnings);
        assert_eq!(insights.stats.as_ref().unwrap().files.len(), 5);
        let symbol = insights
            .symbols
            .iter()
            .find(|s| s.name == "ValidateSchedule")
            .unwrap();
        assert_eq!(symbol.change, SymbolChangeKind::SignatureChanged);
        assert!(
            insights
                .symbols
                .iter()
                .any(|s| s.name == "CreateSeries" && s.change == SymbolChangeKind::Added)
        );
        let impact = insights
            .impacts
            .iter()
            .find(|i| i.symbol == "ValidateSchedule")
            .unwrap();
        assert!(
            impact
                .callers
                .iter()
                .any(|c| c.path == "backend/api/handlers.go" && c.in_mr)
        );
        assert!(
            insights
                .graph
                .nodes
                .contains(&"backend/api/handlers.go".to_string())
        );
        assert!(insights.patterns.iter().any(|p| p.rule == "query-in-loop"));
        assert!(
            insights
                .contracts
                .iter()
                .any(|c| c.title == "New variable SERIES_MAX")
        );
        assert!(
            insights
                .contracts
                .iter()
                .any(|c| c.severity == ContractSeverity::Breaking)
        );
        assert!(
            insights
                .dependencies
                .iter()
                .any(|d| d.major && d.name == "github.com/jackc/pgx/v5")
        );
        let history = insights
            .history
            .iter()
            .find(|h| h.path == "backend/usecases/create.go")
            .unwrap();
        assert_eq!((history.commits, history.fixes), (2, 1));
        assert!(insights.experts.iter().any(|e| e.owners == ["@lea"]));
        let removed = insights
            .removed
            .iter()
            .find(|r| r.path == "backend/usecases/create.go")
            .unwrap();
        assert!(
            removed.origin.as_ref().unwrap().summary.contains("#1187")
                || removed
                    .origin
                    .as_ref()
                    .unwrap()
                    .summary
                    .contains("invoices")
        );
    }
}
