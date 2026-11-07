//! Isolated, writable worktrees for Super Resolver. Review worktrees stay untouched.
use super::{git, instance_key};
use anyhow::{Context, bail};
use mrde_core::resolver::ResolverTarget;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

pub struct ResolverWorktree {
    root: PathBuf,
}
impl ResolverWorktree {
    pub fn new(cache: &Path, instance: &str, thread: &str) -> anyhow::Result<Self> {
        if thread.len() != 36 || !thread.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            bail!("Invalid resolver thread ID");
        }
        Ok(Self {
            root: cache
                .join("resolver")
                .join(instance_key(instance))
                .join(thread),
        })
    }
    pub fn directory(&self) -> PathBuf {
        self.root.join("worktree")
    }
    fn repository(&self) -> PathBuf {
        self.root.join("repository.git")
    }
    pub async fn prepare(&self, target: &ResolverTarget) -> anyhow::Result<PathBuf> {
        if !super::is_sha(&target.head_sha) || !super::is_sha(&target.target_sha) {
            bail!("Invalid resolver commit SHA");
        }
        let source_ok = ssh_url(&target.source_ssh);
        let target_ok = ssh_url(&target.target_ssh);
        if !source_ok || !target_ok {
            bail!("Super Resolver requires SSH repository URLs");
        }
        if target.source_project == target.id.project
            && target.source_branch == target.target_branch
        {
            bail!("Refusing to write to the target branch");
        }
        std::fs::create_dir_all(&self.root)?;
        git(
            &self.root,
            &["check-ref-format", "--branch", &target.source_branch],
        )
        .await?;
        git(
            &self.root,
            &["check-ref-format", "--branch", &target.target_branch],
        )
        .await?;
        let repo = self.repository();
        let directory = self.directory();
        if !repo.exists() {
            git(
                &self.root,
                &[
                    "init",
                    "--bare",
                    repo.to_str().context("Invalid repository path")?,
                ],
            )
            .await?;
            git(&repo, &["remote", "add", "source", &target.source_ssh]).await?;
            git(&repo, &["remote", "add", "target", &target.target_ssh]).await?;
        }
        for (name, expected) in [
            ("source", &target.source_ssh),
            ("target", &target.target_ssh),
        ] {
            if git(&repo, &["remote", "get-url", name]).await? != *expected {
                bail!("Repository URL changed; start a new resolver thread");
            }
        }
        git(
            &repo,
            &[
                "fetch",
                "--no-tags",
                "source",
                &format!("refs/heads/{}", target.source_branch),
            ],
        )
        .await?;
        if git(&repo, &["rev-parse", "FETCH_HEAD"]).await? != target.head_sha {
            bail!("Source branch changed while preparing the worktree");
        }
        git(
            &repo,
            &[
                "fetch",
                "--no-tags",
                "target",
                &format!("refs/heads/{}", target.target_branch),
            ],
        )
        .await?;
        if git(&repo, &["rev-parse", "FETCH_HEAD"]).await? != target.target_sha {
            bail!("Target branch changed while preparing the worktree; resume to retry");
        }
        if !directory.exists() {
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    directory.to_str().context("Invalid worktree path")?,
                    &target.head_sha,
                ],
            )
            .await?;
        }
        Ok(directory)
    }
    pub async fn head(&self) -> anyhow::Result<String> {
        git(&self.directory(), &["rev-parse", "HEAD"]).await
    }
    async fn rebasing(&self) -> anyhow::Result<bool> {
        for name in ["rebase-merge", "rebase-apply"] {
            let path = git(&self.directory(), &["rev-parse", "--git-path", name]).await?;
            if self.directory().join(path).exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Returns false only when the worktree contains conflicts requiring repair.
    pub async fn rebase(&self, target_sha: &str) -> anyhow::Result<bool> {
        let directory = self.directory();
        let rebasing = self.rebasing().await?;
        if rebasing {
            // diff --check detects leftover conflict markers before staging resolutions.
            if git(&directory, &["diff", "--check"]).await.is_err() {
                return Ok(false);
            }
            git(&directory, &["add", "-A"]).await?;
        }
        if !rebasing {
            // Manual takeover may leave edits without a commit. Preserve them before rebase.
            self.commit().await?;
        }
        let mut command = Command::new("git");
        command
            .current_dir(&directory)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("GIT_SEQUENCE_EDITOR", "true")
            .kill_on_drop(true)
            .arg("rebase");
        if rebasing {
            command.arg("--continue");
        } else {
            command.arg(target_sha);
        }
        let result = tokio::time::timeout(Duration::from_secs(180), command.output())
            .await
            .context("Rebase timed out")??;
        if result.status.success() {
            return Ok(true);
        }
        if !git(&directory, &["diff", "--name-only", "--diff-filter=U"])
            .await?
            .is_empty()
        {
            return Ok(false);
        }
        bail!(
            "Rebase stopped: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    pub async fn commit(&self) -> anyhow::Result<String> {
        let directory = self.directory();
        if self.rebasing().await? {
            bail!("Finish the rebase before publishing corrections");
        }
        git(&directory, &["diff", "--check"]).await?;
        git(&directory, &["diff", "--cached", "--check"]).await?;
        if !git(&directory, &["status", "--porcelain"])
            .await?
            .is_empty()
        {
            git(&directory, &["add", "-A"]).await?;
            git(
                &directory,
                &[
                    "commit",
                    "-m",
                    "Fix merge request pipeline with Super Resolver",
                ],
            )
            .await?;
        }
        self.head().await
    }
    pub async fn push(
        &self,
        target: &ResolverTarget,
        expected_sha: &str,
    ) -> anyhow::Result<String> {
        if !super::is_sha(expected_sha) {
            bail!("Invalid expected source commit");
        }
        let head = self.head().await?;
        if head == expected_sha {
            return Ok(head);
        }
        if target.source_project == target.id.project
            && target.source_branch == target.target_branch
        {
            bail!("Refusing to push to the target branch");
        }
        let branch = format!("refs/heads/{}", target.source_branch);
        let lease = format!("--force-with-lease={branch}:{expected_sha}");
        git(
            &self.directory(),
            &[
                "push",
                "--porcelain",
                &lease,
                "source",
                &format!("HEAD:{branch}"),
            ],
        )
        .await
        .context(
            "Push refused; the remote branch may have changed or require additional permissions",
        )?;
        Ok(head)
    }
}
fn ssh_url(url: &str) -> bool {
    #[cfg(test)]
    if url.starts_with("file://") {
        return true;
    }
    !url.starts_with('-')
        && (url.starts_with("ssh://")
            || (url.contains('@') && url.contains(':') && !url.contains("://")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrde_core::models::MrId;
    struct Fixture {
        _temp: tempfile::TempDir,
        upstream: PathBuf,
        tree: ResolverWorktree,
        target: ResolverTarget,
    }
    async fn fixture(conflict: bool) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let upstream = temp.path().join("upstream");
        std::fs::create_dir_all(&upstream).unwrap();
        git(&upstream, &["init", "-b", "main"]).await.unwrap();
        git(&upstream, &["config", "user.name", "Test"])
            .await
            .unwrap();
        git(&upstream, &["config", "user.email", "test@example.com"])
            .await
            .unwrap();
        std::fs::write(upstream.join("file.txt"), "base\n").unwrap();
        git(&upstream, &["add", "."]).await.unwrap();
        git(&upstream, &["commit", "-m", "Base"]).await.unwrap();
        git(&upstream, &["checkout", "-b", "fix"]).await.unwrap();
        std::fs::write(upstream.join("file.txt"), "source\n").unwrap();
        git(&upstream, &["commit", "-am", "Source"]).await.unwrap();
        let head_sha = git(&upstream, &["rev-parse", "HEAD"]).await.unwrap();
        git(&upstream, &["checkout", "main"]).await.unwrap();
        std::fs::write(
            upstream.join(if conflict { "file.txt" } else { "target.txt" }),
            "target\n",
        )
        .unwrap();
        git(&upstream, &["add", "."]).await.unwrap();
        git(&upstream, &["commit", "-m", "Target"]).await.unwrap();
        let target_sha = git(&upstream, &["rev-parse", "HEAD"]).await.unwrap();
        let url = format!("file://{}", upstream.display());
        let target = ResolverTarget {
            id: MrId { project: 1, iid: 2 },
            source_project: 1,
            source_branch: "fix".into(),
            target_branch: "main".into(),
            source_ssh: url.clone(),
            target_ssh: url,
            head_sha,
            target_sha,
        };
        let tree = ResolverWorktree::new(
            temp.path(),
            "https://gitlab.example.com",
            "00000000-0000-4000-8000-000000000001",
        )
        .unwrap();
        tree.prepare(&target).await.unwrap();
        git(&tree.directory(), &["config", "user.name", "Test"])
            .await
            .unwrap();
        git(
            &tree.directory(),
            &["config", "user.email", "test@example.com"],
        )
        .await
        .unwrap();
        Fixture {
            _temp: temp,
            upstream,
            tree,
            target,
        }
    }
    #[tokio::test]
    async fn rebase_and_correction_publish_only_the_source_branch() {
        let f = fixture(false).await;
        assert!(f.tree.rebase(&f.target.target_sha).await.unwrap());
        std::fs::write(f.tree.directory().join("fix.txt"), "correction\n").unwrap();
        let sha = f.tree.commit().await.unwrap();
        assert_eq!(
            f.tree.push(&f.target, &f.target.head_sha).await.unwrap(),
            sha
        );
        assert_eq!(git(&f.upstream, &["rev-parse", "fix"]).await.unwrap(), sha);
        assert_eq!(
            git(&f.upstream, &["rev-parse", "main"]).await.unwrap(),
            f.target.target_sha
        );
    }
    #[tokio::test]
    async fn conflicts_are_preserved_until_resolved_and_can_be_continued() {
        let f = fixture(true).await;
        assert!(!f.tree.rebase(&f.target.target_sha).await.unwrap());
        assert!(!f.tree.rebase(&f.target.target_sha).await.unwrap());
        assert!(f.tree.commit().await.is_err());
        std::fs::write(f.tree.directory().join("file.txt"), "source and target\n").unwrap();
        assert!(f.tree.rebase(&f.target.target_sha).await.unwrap());
        f.tree.push(&f.target, &f.target.head_sha).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(f.tree.directory().join("file.txt")).unwrap(),
            "source and target\n"
        );
    }
    #[tokio::test]
    async fn competing_push_is_rejected_and_dirty_worktree_survives_prepare() {
        let f = fixture(false).await;
        std::fs::write(f.tree.directory().join("local.txt"), "keep me\n").unwrap();
        f.tree.prepare(&f.target).await.unwrap();
        assert!(f.tree.directory().join("local.txt").exists());
        f.tree.commit().await.unwrap();
        git(&f.upstream, &["branch", "-f", "fix", "main"])
            .await
            .unwrap();
        assert!(f.tree.push(&f.target, &f.target.head_sha).await.is_err());
        assert_eq!(
            git(&f.upstream, &["rev-parse", "fix"]).await.unwrap(),
            f.target.target_sha
        );
    }
}
