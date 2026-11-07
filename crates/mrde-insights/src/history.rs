//! Code history: per-file risk, who knows each zone, and where removed lines came from.
use mrde_core::insights::{BlameCommit, Expert, FileHistory, Level};
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::LazyLock,
};

static FIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(fix(es|ed)?|bug|hotfix|revert|regression|incident|correct(ion|s)?|crash)\b")
        .expect("valid fix pattern")
});
static REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[#!]\d+").expect("valid reference pattern"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub files: Vec<String>,
}

/// Parses `git log --format=%x1e%H%x1f%an%x1f%ad%x1f%s --name-only`.
pub fn parse_log(output: &str) -> Vec<LogEntry> {
    output
        .split('\u{1e}')
        .filter(|record| !record.trim().is_empty())
        .filter_map(|record| {
            let mut lines = record.lines();
            let header = lines.next()?;
            let mut fields = header.split('\u{1f}');
            Some(LogEntry {
                sha: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                date: fields.next()?.to_string(),
                subject: fields.next().unwrap_or_default().to_string(),
                files: lines
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}

pub fn is_fix(subject: &str) -> bool {
    FIX.is_match(subject)
}

/// Risk from recent churn, fix commits, and the number of authors touching the file.
pub fn file_history(paths: &[(String, bool)], log: &[LogEntry]) -> Vec<FileHistory> {
    let mut out: Vec<FileHistory> = paths
        .iter()
        .map(|(path, new_file)| {
            let commits: Vec<&LogEntry> = log
                .iter()
                .filter(|e| e.files.iter().any(|f| f == path))
                .collect();
            let fixes: Vec<&&LogEntry> = commits.iter().filter(|e| is_fix(&e.subject)).collect();
            let authors: BTreeSet<&str> = commits.iter().map(|e| e.author.as_str()).collect();
            let (commits_count, fixes_count) = (commits.len() as u32, fixes.len() as u32);
            let risk = if *new_file {
                Level::Low
            } else if fixes_count >= 3 || commits_count >= 20 {
                Level::High
            } else if fixes_count >= 1 || commits_count >= 8 {
                Level::Medium
            } else {
                Level::Low
            };
            FileHistory {
                path: path.clone(),
                commits: commits_count,
                fixes: fixes_count,
                authors: authors.len() as u32,
                last_fix: fixes.first().map(|e| format!("{} · {}", e.date, e.subject)),
                risk,
                new_file: *new_file,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.risk
            .cmp(&b.risk)
            .then_with(|| b.fixes.cmp(&a.fixes))
            .then_with(|| b.commits.cmp(&a.commits))
    });
    out
}

/// CODEOWNERS rules as (pattern, owners), in file order; GitLab section headers ignored.
pub fn parse_codeowners(text: &str) -> Vec<(String, Vec<String>)> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty() && !l.starts_with('#') && !l.starts_with('[') && !l.starts_with("^[")
        })
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let pattern = parts.next()?.to_string();
            let owners: Vec<String> = parts.map(str::to_string).collect();
            (!owners.is_empty()).then_some((pattern, owners))
        })
        .collect()
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    let anchored = pattern.starts_with('/');
    let pattern = pattern.trim_start_matches('/').trim_end_matches('/');
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return if anchored || pattern.contains('/') {
            path == pattern || path.starts_with(&format!("{pattern}/"))
        } else {
            path.split('/').any(|part| part == pattern)
        };
    }
    let regex = format!(
        "^{}{}(/.*)?$",
        if anchored || pattern.contains('/') {
            ""
        } else {
            "(.*/)?"
        },
        regex::escape(pattern)
            .replace(r"\*\*", ".*")
            .replace(r"\*", "[^/]*")
    );
    Regex::new(&regex).is_ok_and(|r| r.is_match(path))
}

/// Owners of a directory: the last matching rule wins, as in GitLab and GitHub.
pub fn owners(rules: &[(String, Vec<String>)], directory: &str) -> Vec<String> {
    rules
        .iter()
        .rev()
        .find(|(pattern, _)| glob_matches(pattern, directory))
        .map(|(_, owners)| owners.clone())
        .unwrap_or_default()
}

/// Top authors of a zone with their share of its recent commits.
pub fn expert(zone: &str, log: &[LogEntry], rules: &[(String, Vec<String>)]) -> Expert {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for entry in log {
        *counts.entry(entry.author.as_str()).or_default() += 1;
    }
    let total: u32 = counts.values().sum();
    let mut people: Vec<(String, u32)> = counts
        .into_iter()
        .map(|(author, count)| {
            (
                author.to_string(),
                (count * 100).checked_div(total).unwrap_or(0),
            )
        })
        .collect();
    people.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    people.truncate(3);
    Expert {
        zone: zone.into(),
        people,
        owners: owners(rules, zone),
    }
}

/// The commit that last touched most of the lines in a `git blame --porcelain` range.
pub fn parse_blame(output: &str) -> Option<BlameCommit> {
    let mut lines_per_commit: BTreeMap<String, u32> = BTreeMap::new();
    let mut meta: HashMap<String, (String, i64, String)> = HashMap::new();
    let mut current = String::new();
    for line in output.lines() {
        if line.starts_with('\t') {
            continue;
        }
        let first = line.split(' ').next().unwrap_or_default();
        if first.len() == 40 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
            current = first.to_string();
            *lines_per_commit.entry(current.clone()).or_default() += 1;
            meta.entry(current.clone()).or_default();
        } else if let Some(author) = line.strip_prefix("author ") {
            meta.entry(current.clone()).or_default().0 = author.into();
        } else if let Some(time) = line.strip_prefix("author-time ") {
            meta.entry(current.clone()).or_default().1 = time.parse().unwrap_or(0);
        } else if let Some(summary) = line.strip_prefix("summary ") {
            meta.entry(current.clone()).or_default().2 = summary.into();
        }
    }
    let (sha, _) = lines_per_commit
        .iter()
        .filter(|(sha, _)| !sha.bytes().all(|b| b == b'0'))
        .max_by_key(|(_, count)| **count)?;
    let (author, time, summary) = meta.get(sha).cloned().unwrap_or_default();
    Some(BlameCommit {
        sha: sha.clone(),
        author,
        date: chrono::DateTime::from_timestamp(time, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
        references: REFERENCE
            .find_iter(&summary)
            .map(|m| m.as_str().to_string())
            .collect(),
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_risk_from_churn_and_fix_commits() {
        let log = parse_log(
            "\u{1e}a\u{1f}Léa\u{1f}2026-03-12\u{1f}fix: reject a due date before the issue date (#1187)\n\nsrc/create.go\nsrc/other.go\n\u{1e}b\u{1f}Karim\u{1f}2026-02-01\u{1f}feat: calendar\n\nsrc/create.go\n",
        );
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].files, ["src/create.go", "src/other.go"]);
        let history = file_history(
            &[("src/create.go".into(), false), ("src/new.go".into(), true)],
            &log,
        );
        assert_eq!(history[0].path, "src/create.go");
        assert_eq!(
            (history[0].commits, history[0].fixes, history[0].authors),
            (2, 1, 2)
        );
        assert_eq!(history[0].risk, Level::Medium);
        assert!(history[0].last_fix.as_deref().unwrap().contains("#1187"));
        assert_eq!(history[1].risk, Level::Low);
        assert!(history[1].new_file);
    }

    #[test]
    fn matches_codeowners_and_ranks_experts() {
        let rules = parse_codeowners(
            "# owners\n* @team\n[Backend]\n/backend/src/migrations/ @karim\ndocs/** @writers\n*.go @gophers\n",
        );
        assert_eq!(owners(&rules, "backend/src/migrations"), ["@karim"]);
        assert_eq!(owners(&rules, "frontend/src"), ["@team"]);
        let log = parse_log(
            "\u{1e}a\u{1f}Léa\u{1f}d\u{1f}s\n\u{1e}b\u{1f}Léa\u{1f}d\u{1f}s\n\u{1e}c\u{1f}Karim\u{1f}d\u{1f}s\n",
        );
        let expert = expert("backend/src/migrations", &log, &rules);
        assert_eq!(
            expert.people,
            [("Léa".to_string(), 66), ("Karim".to_string(), 33)]
        );
        assert_eq!(expert.owners, ["@karim"]);
    }

    #[test]
    fn picks_the_main_commit_of_a_blamed_range() {
        let sha = "9f3c2e1".repeat(5) + "abcde";
        let other = "1".repeat(40);
        let porcelain = format!(
            "{sha} 84 84 2\nauthor Léa M.\nauthor-time 1741776000\nsummary fix: reject a due date before the issue date (incident #1187, !164)\n\tif x {{\n{sha} 85 85\n\treturn\n{other} 86 86 1\nauthor Bob\nauthor-time 1\nsummary init\n\t}}\n"
        );
        let commit = parse_blame(&porcelain).unwrap();
        assert_eq!(commit.sha, sha);
        assert_eq!(commit.author, "Léa M.");
        assert_eq!(commit.date, "2025-03-12");
        assert_eq!(commit.references, ["#1187", "!164"]);
    }
}
