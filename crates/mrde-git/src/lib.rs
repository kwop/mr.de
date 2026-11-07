use anyhow::{Context, bail};
use mrde_core::models::{FileStat, MrDetail};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;

pub fn instance_key(instance: &str) -> String {
    format!("{:x}", Sha256::digest(instance.as_bytes()))
}

async fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    tracing::debug!(
        command = args.first().copied().unwrap_or_default(),
        "Running git"
    );
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Git operation timed out")??;
    if !output.status.success() {
        tracing::warn!(command = args.first().copied().unwrap_or_default(), status = %output.status, "Git command failed");
        bail!(
            "Git operation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

pub struct Worktrees {
    root: PathBuf,
}
impl Worktrees {
    pub fn new(cache: &Path, instance: &str) -> Self {
        Self {
            root: cache.join("git").join(instance_key(instance)),
        }
    }
    fn paths(&self, detail: &MrDetail) -> anyhow::Result<(PathBuf, PathBuf)> {
        let (project, iid, sha) = (detail.mr.id.project, detail.mr.id.iid, &detail.mr.sha);
        Ok((self.repository(project), self.worktree(project, iid, sha)?))
    }
    fn repository(&self, project: u64) -> PathBuf {
        self.root.join(format!("{project}.git"))
    }
    fn worktree(&self, project: u64, iid: u64, sha: &str) -> anyhow::Result<PathBuf> {
        if !is_sha(sha) {
            bail!("Invalid commit SHA");
        }
        Ok(self.root.join(format!("{project}-{iid}-{sha}")))
    }
    /// Existing worktree for this exact commit, if any. Never creates one.
    pub fn existing(&self, detail: &MrDetail) -> Option<PathBuf> {
        let (_, worktree) = self.paths(detail).ok()?;
        worktree.join(".git").exists().then_some(worktree)
    }
    /// Worktrees created by mr.de for this instance, with their local-change count.
    pub async fn list(&self) -> anyhow::Result<Vec<WorktreeEntry>> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(vec![]);
        };
        let mut result = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let mut parts = name.splitn(3, '-');
            let (Some(project), Some(iid), Some(sha)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (Ok(project), Ok(iid)) = (project.parse(), iid.parse()) else {
                continue;
            };
            if !is_sha(sha) || !path.join(".git").exists() {
                continue;
            }
            let changes = git(&path, &STATUS_WITH_IGNORED)
                .await
                .ok()
                .map(|out| out.lines().count());
            result.push(WorktreeEntry {
                project,
                iid,
                sha: sha.to_string(),
                path,
                changes,
            });
        }
        result.sort_by(|a, b| (a.project, a.iid, &a.sha).cmp(&(b.project, b.iid, &b.sha)));
        Ok(result)
    }
    pub async fn prepare(&self, detail: &MrDetail) -> anyhow::Result<PathBuf> {
        self.prepare_commit(detail, &detail.mr.sha).await
    }
    /// A detached worktree at `sha`: the MR head, or another commit of its history such as
    /// the base, used for before/after comparisons.
    pub async fn prepare_commit(&self, detail: &MrDetail, sha: &str) -> anyhow::Result<PathBuf> {
        let repository = self.repository(detail.mr.id.project);
        let worktree = self.worktree(detail.mr.id.project, detail.mr.id.iid, sha)?;
        std::fs::create_dir_all(&self.root)?;
        let repository_text = repository.to_str().context("Invalid repository path")?;
        let worktree_text = worktree.to_str().context("Invalid worktree path")?;
        if !repository.exists() {
            // SSH configuration, including ProxyCommand for Cloudflare, remains owned by the user.
            if !detail.ssh_url.starts_with("ssh://") && !detail.ssh_url.contains('@') {
                bail!("An SSH clone URL is required");
            }
            if detail.ssh_url.starts_with('-') {
                bail!("Invalid SSH clone URL");
            }
            git(&self.root, &["init", "--bare", repository_text]).await?;
            git(&repository, &["remote", "add", "origin", &detail.ssh_url]).await?;
        }
        if worktree.exists() {
            let head = git(&worktree, &["rev-parse", "HEAD"]).await?;
            if head != sha {
                bail!("Existing worktree has a different HEAD");
            }
            return Ok(worktree);
        }
        // GitLab's target-project MR ref also handles forks.
        let reference = format!("refs/merge-requests/{}/head", detail.mr.id.iid);
        git(&repository, &["fetch", "origin", &reference]).await?;
        let fetched = git(&repository, &["rev-parse", "FETCH_HEAD"]).await?;
        if fetched != detail.mr.sha {
            bail!("The MR changed. Refresh before preparing a worktree.");
        }
        if sha != detail.mr.sha {
            let commit = format!("{sha}^{{commit}}");
            if git(&repository, &["cat-file", "-e", &commit])
                .await
                .is_err()
            {
                git(&repository, &["fetch", "origin", sha])
                    .await
                    .context("The commit is not in the MR history and could not be fetched")?;
            }
        }
        git(
            &repository,
            &["worktree", "add", "--detach", worktree_text, sha],
        )
        .await?;
        Ok(worktree)
    }
    pub async fn remove(&self, detail: &MrDetail) -> anyhow::Result<()> {
        self.remove_worktree(detail.mr.id.project, detail.mr.id.iid, &detail.mr.sha)
            .await
    }
    /// Refuses worktrees with local changes or ignored files; never forces removal.
    pub async fn remove_worktree(&self, project: u64, iid: u64, sha: &str) -> anyhow::Result<()> {
        let worktree = self.worktree(project, iid, sha)?;
        if !worktree.exists() {
            return Ok(());
        }
        // `git worktree remove` silently deletes ignored files, such as notes or `.env`.
        if !git(&worktree, &STATUS_WITH_IGNORED).await?.is_empty() {
            bail!(
                "Worktree contains local changes or ignored files; preserve or remove them before cleanup"
            );
        }
        git(
            &self.repository(project),
            &[
                "worktree",
                "remove",
                worktree.to_str().context("Invalid worktree path")?,
            ],
        )
        .await?;
        Ok(())
    }
}

const STATUS_WITH_IGNORED: [&str; 4] = [
    "status",
    "--porcelain",
    "--untracked-files=all",
    "--ignored",
];

fn is_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub project: u64,
    pub iid: u64,
    pub sha: String,
    pub path: PathBuf,
    /// Modified, untracked, or ignored entries; `None` when Git could not inspect it.
    pub changes: Option<usize>,
}

/// Exact per-file line counts between two commits. Fetches the base commit when the MR
/// ref history does not contain it.
pub async fn numstat(worktree: &Path, base: &str, head: &str) -> anyhow::Result<Vec<FileStat>> {
    ensure_commits(worktree, base, head).await?;
    let output = git(
        worktree,
        &["diff", "--numstat", "-z", "--find-renames", base, head],
    )
    .await?;
    Ok(parse_numstat(&output))
}

/// Validates both commits and fetches the base when the MR ref history lacks it.
pub async fn ensure_commits(worktree: &Path, base: &str, head: &str) -> anyhow::Result<()> {
    if !is_sha(base) || !is_sha(head) {
        bail!("Invalid commit SHA");
    }
    let commit = format!("{base}^{{commit}}");
    if git(worktree, &["cat-file", "-e", &commit]).await.is_err() {
        git(worktree, &["fetch", "origin", base])
            .await
            .context("The base commit is not available locally and could not be fetched")?;
    }
    Ok(())
}

/// Runs a read-only query; `allowed` lists exit codes that mean "no result" (git grep: 1).
async fn query(dir: &Path, args: &[&str], allowed: &[i32]) -> anyhow::Result<String> {
    Ok(query_code(dir, args, allowed).await?.1)
}

async fn query_code(dir: &Path, args: &[&str], allowed: &[i32]) -> anyhow::Result<(i32, String)> {
    tracing::debug!(
        command = args.first().copied().unwrap_or_default(),
        "Running git query"
    );
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Git query timed out")??;
    let code = output.status.code().unwrap_or(-1);
    if !output.status.success() && !allowed.contains(&code) {
        bail!(
            "Git query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok((code, String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// Unified diff without context lines, with renames, from base to head.
pub async fn diff_hunks(worktree: &Path, base: &str, head: &str) -> anyhow::Result<String> {
    ensure_commits(worktree, base, head).await?;
    query(
        worktree,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            "-U0",
            base,
            head,
        ],
        &[],
    )
    .await
}

/// File contents at a commit, or `None` when the file does not exist there or is binary.
pub async fn show_file(worktree: &Path, rev: &str, path: &str) -> anyhow::Result<Option<String>> {
    if !is_sha(rev) {
        bail!("Invalid commit SHA");
    }
    let spec = format!("{rev}:{path}");
    let (code, text) = query_code(worktree, &["show", "--no-color", &spec], &[128]).await?;
    Ok((code == 0 && !text.contains('\0')).then_some(text))
}

/// `git blame --porcelain` for a line range at a commit.
pub async fn blame(
    worktree: &Path,
    rev: &str,
    path: &str,
    start: u32,
    count: u32,
) -> anyhow::Result<String> {
    if !is_sha(rev) {
        bail!("Invalid commit SHA");
    }
    let range = format!("{start},+{}", count.max(1));
    query(
        worktree,
        &["blame", "--porcelain", "-L", &range, rev, "--", path],
        &[],
    )
    .await
}

/// History of the given paths up to `rev`: records separated by `\x1e`, fields by `\x1f`
/// (sha, author, date, subject), followed by the changed file names.
pub async fn log_files(
    worktree: &Path,
    rev: &str,
    since: &str,
    paths: &[String],
) -> anyhow::Result<String> {
    if !is_sha(rev) {
        bail!("Invalid commit SHA");
    }
    let since = format!("--since={since}");
    let mut output = String::new();
    // Keep argument lists well below the platform limit.
    for chunk in paths.chunks(200) {
        let mut args = vec![
            "log",
            "--no-color",
            "--no-merges",
            &since,
            "--format=%x1e%H%x1f%an%x1f%ad%x1f%s",
            "--date=short",
            "--name-only",
            rev,
            "--",
        ];
        args.extend(chunk.iter().map(String::as_str));
        output.push_str(&query(worktree, &args, &[]).await?);
    }
    Ok(output)
}

/// Whole-word, fixed-string matches at a commit, as `rev:path:line:text` lines.
pub async fn grep_word(
    worktree: &Path,
    rev: &str,
    word: &str,
    pathspecs: &[&str],
) -> anyhow::Result<String> {
    if !is_sha(rev) {
        bail!("Invalid commit SHA");
    }
    let mut args = vec!["grep", "-n", "-I", "-w", "-F", "-e", word, rev, "--"];
    args.extend(pathspecs);
    query(worktree, &args, &[1]).await
}

/// Paths tracked at a commit.
pub async fn list_files(worktree: &Path, rev: &str) -> anyhow::Result<Vec<String>> {
    if !is_sha(rev) {
        bail!("Invalid commit SHA");
    }
    Ok(query(worktree, &["ls-tree", "-r", "--name-only", rev], &[])
        .await?
        .lines()
        .map(str::to_string)
        .collect())
}

/// `-z` format: `added\tremoved\tpath\0`, or `added\tremoved\t\0old\0new\0` for renames.
fn parse_numstat(output: &str) -> Vec<FileStat> {
    let mut parts = output.split('\0');
    let mut files = Vec::new();
    while let Some(record) = parts.next() {
        let record = record.trim_start_matches('\n');
        if record.is_empty() {
            continue;
        }
        let mut columns = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = parts.next();
            parts.next().unwrap_or_default()
        } else {
            path
        };
        files.push(FileStat {
            path: path.to_string(),
            added: added.parse().unwrap_or(0),
            removed: removed.parse().unwrap_or(0),
            binary: added == "-",
        });
    }
    files
}

/// Pre-reviews must refer to the exact commit, not files changed in a chat session.
pub async fn ensure_clean(worktree: &Path) -> anyhow::Result<()> {
    if !git(
        worktree,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .await?
    .is_empty()
    {
        bail!(
            "Worktree contains local changes. Restore them before running a commit-based pre-review."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn detects_untracked_work_before_cleanup_or_review() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init"]).await.unwrap();
        ensure_clean(dir.path()).await.unwrap();
        std::fs::write(dir.path().join("notes.txt"), "Keep this work").unwrap();
        assert!(ensure_clean(dir.path()).await.is_err());
        assert!(dir.path().join("notes.txt").exists());
    }
    #[test]
    fn instance_directories_cannot_escape_cache() {
        assert_eq!(instance_key("https://gitlab.example.com").len(), 64);
        assert!(!instance_key("../../other").contains('/'));
        assert_ne!(instance_key("https://one"), instance_key("https://two"));
    }
    #[tokio::test]
    async fn prepares_exact_mr_commit_reuses_it_and_protects_dirty_work() {
        let temporary = tempfile::tempdir().unwrap();
        let origin = temporary.path().join("source");
        std::fs::create_dir(&origin).unwrap();
        git(&origin, &["init"]).await.unwrap();
        git(&origin, &["config", "user.name", "MR.DE tests"])
            .await
            .unwrap();
        git(&origin, &["config", "user.email", "tests@example.invalid"])
            .await
            .unwrap();
        std::fs::write(origin.join("file.txt"), "Original content\n").unwrap();
        std::fs::write(origin.join(".gitignore"), "*.local\n").unwrap();
        git(&origin, &["add", "file.txt", ".gitignore"])
            .await
            .unwrap();
        git(
            &origin,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                "Fixture",
            ],
        )
        .await
        .unwrap();
        let sha = git(&origin, &["rev-parse", "HEAD"]).await.unwrap();
        git(&origin, &["update-ref", "refs/merge-requests/2/head", &sha])
            .await
            .unwrap();
        let detail: MrDetail = serde_json::from_value(serde_json::json!({
            "mr": {"id":{"project":1,"iid":2},"title":"Test MR","description":"",
                "project":"test","author":"test","web_url":"https://example.invalid/mr/2",
                "sha":sha,"base_sha":sha,"created_at":"2026-01-01T00:00:00Z","labels":[],
                "pipeline":null,"approvals":null,"changed_lines":null},
            "files":[],"issues":[],"ssh_url":"git@example.invalid:test.git"
        }))
        .unwrap();
        let trees = Worktrees::new(temporary.path(), "https://example.invalid");
        std::fs::create_dir_all(&trees.root).unwrap();
        let (repository, _) = trees.paths(&detail).unwrap();
        git(
            &trees.root,
            &["init", "--bare", repository.to_str().unwrap()],
        )
        .await
        .unwrap();
        git(
            &repository,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        )
        .await
        .unwrap();
        let worktree = trees.prepare(&detail).await.unwrap();
        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]).await.unwrap(), sha);
        assert!(
            trees
                .prepare_commit(&detail, &"0".repeat(40))
                .await
                .is_err(),
            "unknown commit"
        );
        assert_eq!(trees.prepare(&detail).await.unwrap(), worktree);
        assert_eq!(trees.existing(&detail), Some(worktree.clone()));
        let listed = trees.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].project, listed[0].iid), (1, 2));
        assert_eq!(listed[0].changes, Some(0));
        std::fs::write(worktree.join("file.txt"), "Keep my edits").unwrap();
        assert_eq!(trees.list().await.unwrap()[0].changes, Some(1));
        assert!(trees.remove(&detail).await.is_err());
        assert_eq!(
            std::fs::read_to_string(worktree.join("file.txt")).unwrap(),
            "Keep my edits"
        );
        git(&worktree, &["restore", "file.txt"]).await.unwrap();
        std::fs::write(worktree.join("notes.local"), "Ignored but valuable").unwrap();
        assert!(trees.remove(&detail).await.is_err());
        assert!(worktree.join("notes.local").exists());
        std::fs::remove_file(worktree.join("notes.local")).unwrap();
        trees.remove_worktree(1, 2, &sha).await.unwrap();
        assert!(!worktree.exists());
        assert!(trees.list().await.unwrap().is_empty());
        assert_eq!(trees.existing(&detail), None);
        let mut stale = detail.clone();
        stale.mr.sha = "0".repeat(40);
        assert!(trees.prepare(&stale).await.is_err());
    }
    async fn commit(dir: &Path, message: &str) -> String {
        git(dir, &["add", "-A"]).await.unwrap();
        git(
            dir,
            &[
                "-c",
                "user.name=MR.DE tests",
                "-c",
                "user.email=tests@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                message,
            ],
        )
        .await
        .unwrap();
        git(dir, &["rev-parse", "HEAD"]).await.unwrap()
    }
    #[tokio::test]
    async fn numstat_reports_lines_renames_and_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init"]).await.unwrap();
        std::fs::create_dir(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/a.rs"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(repo.join("old name.txt"), "same content here\n".repeat(20)).unwrap();
        let base = commit(repo, "base").await;
        std::fs::write(repo.join("src/a.rs"), "one\n2\nthree\nfour\n").unwrap();
        std::fs::rename(repo.join("old name.txt"), repo.join("new name.txt")).unwrap();
        std::fs::write(repo.join("logo.png"), [0u8, 159, 146, 150, 0, 1]).unwrap();
        let head = commit(repo, "head").await;
        let mut stats = numstat(repo, &base, &head).await.unwrap();
        stats.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].path, "logo.png");
        assert!(stats[0].binary);
        assert_eq!(
            (stats[1].path.as_str(), stats[1].changed()),
            ("new name.txt", 0)
        );
        assert_eq!(
            (stats[2].path.as_str(), stats[2].added, stats[2].removed),
            ("src/a.rs", 2, 1)
        );
        assert!(numstat(repo, "not-a-sha", &head).await.is_err());
    }
}
