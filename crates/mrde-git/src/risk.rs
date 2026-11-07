//! Strictly local Git queries for risk scoring. Preparation/fetching belongs to the caller.
use super::*;
use mrde_core::risk::{FileCommit, FileContext};
use std::collections::BTreeMap;

pub async fn diff(
    worktree: &Path,
    base: &str,
    head: &str,
) -> anyhow::Result<(String, String, Vec<FileStat>)> {
    if !is_sha(base) || !is_sha(head) {
        bail!("Invalid risk commit SHA");
    }
    let base = query(worktree, &["merge-base", base, head], &[])
        .await?
        .trim()
        .to_string();
    let stat = query(
        worktree,
        &[
            "diff",
            "--numstat",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            &base,
            head,
            "--",
        ],
        &[],
    )
    .await?;
    let diff = query(
        worktree,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "-U0",
            &base,
            head,
            "--",
        ],
        &[],
    )
    .await?;
    Ok((base, diff, parse_numstat(&stat)))
}
/// Export a commit to an owned, empty directory using a temporary Git index. Git's own
/// checkout preserves executable bits, without changing the user's index or worktree.
pub async fn export(worktree: &Path, sha: &str, destination: &Path) -> anyhow::Result<()> {
    if !is_sha(sha) {
        bail!("Invalid snapshot SHA");
    }
    std::fs::create_dir_all(destination)?;
    let index = destination
        .parent()
        .context("Missing snapshot parent")?
        .join(format!("index-{sha}"));
    let prefix = format!("{}/", destination.display());
    for args in [
        vec!["read-tree", sha],
        vec!["checkout-index", "--all", "--force", "--prefix", &prefix],
    ] {
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            Command::new("git")
                .current_dir(worktree)
                .env("GIT_INDEX_FILE", &index)
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        if !output.status.success() {
            bail!(
                "Cannot export commit snapshot: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    let _ = std::fs::remove_file(index);
    Ok(())
}
pub async fn file_context(
    worktree: &Path,
    base: &str,
    head: &str,
    path: &str,
) -> anyhow::Result<FileContext> {
    if path.is_empty()
        || Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("Invalid repository path");
    }
    if !is_sha(base) || !is_sha(head) {
        bail!("Invalid history SHA");
    }
    let mut result = FileContext {
        path: path.into(),
        ..Default::default()
    };
    let mut counts = BTreeMap::<String, u64>::new();
    match query(
        worktree,
        &["blame", "--line-porcelain", head, "--", path],
        &[],
    )
    .await
    {
        Ok(text) => {
            for line in text.lines() {
                if let Some(author) = line.strip_prefix("author ") {
                    *counts.entry(author.into()).or_default() += 1;
                }
            }
        }
        Err(_) => match query(
            worktree,
            &["blame", "--line-porcelain", base, "--", path],
            &[],
        )
        .await
        {
            Ok(text) => {
                for line in text.lines() {
                    if let Some(author) = line.strip_prefix("author ") {
                        *counts.entry(author.into()).or_default() += 1;
                    }
                }
            }
            Err(e) => result
                .warnings
                .push(format!("Line ownership unavailable: {e}")),
        },
    }
    let total = counts.values().sum::<u64>();
    result.authors = counts
        .into_iter()
        .map(|(name, n)| (name, n as f64 * 100. / total.max(1) as f64))
        .collect();
    result
        .authors
        .sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let own: std::collections::HashSet<String> = query(
        worktree,
        &["rev-list", &format!("{base}..{head}"), "--", path],
        &[],
    )
    .await
    .map(|text| text.lines().map(str::to_string).collect())
    .unwrap_or_default();
    match query(
        worktree,
        &[
            "log",
            "--follow",
            "-30",
            "--format=%H%x1f%an%x1f%ad%x1f%s",
            "--date=short",
            head,
            "--",
            path,
        ],
        &[],
    )
    .await
    {
        Ok(text) => {
            for line in text.lines() {
                let parts: Vec<_> = line.splitn(4, '\u{1f}').collect();
                if parts.len() == 4 {
                    result.commits.push(FileCommit {
                        in_mr: own.contains(parts[0]),
                        sha: parts[0].into(),
                        author: parts[1].into(),
                        date: parts[2].into(),
                        subject: parts[3].into(),
                    });
                }
            }
        }
        Err(e) => result
            .warnings
            .push(format!("File history unavailable: {e}")),
    }
    Ok(result)
}
