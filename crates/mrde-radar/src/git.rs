//! Read source Git state, write only to an owned temporary repository.
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

async fn limited(mut input: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = input.read(&mut chunk).await?;
        if count == 0 {
            return Ok(result);
        }
        ensure!(
            result.len() + count <= limit,
            "Git output exceeded the scan limit"
        );
        result.extend_from_slice(&chunk[..count]);
    }
}

pub async fn run(
    root: &Path,
    args: &[&str],
    input: &[u8],
    worktree: Option<&Path>,
    isolated: bool,
) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    // Explicit --repo wins over environment inherited from a shell or another worktree.
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|key| key.starts_with("GIT_")) {
            command.env_remove(name);
        }
    }
    command
        .current_dir(worktree.unwrap_or(root))
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if isolated {
        command.env("GIT_CONFIG_GLOBAL", "/dev/null");
    }
    if let Some(worktree) = worktree {
        command
            .env("GIT_DIR", root.join(".git"))
            .env("GIT_WORK_TREE", worktree);
    }
    command.args(args);
    let mut child = command
        .spawn()
        .context("Cannot start Git; install it and check PATH")?;
    let mut stdin = child.stdin.take().context("Missing Git stdin")?;
    let stdout = child.stdout.take().context("Missing Git stdout")?;
    let stderr = child.stderr.take().context("Missing Git stderr")?;
    let (status, stdout, stderr, ()) = tokio::time::timeout(Duration::from_secs(120), async {
        tokio::try_join!(
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            limited(stdout, 64 * 1024 * 1024),
            limited(stderr, 128 * 1024),
            async {
                stdin.write_all(input).await?;
                drop(stdin);
                Ok(())
            },
        )
    })
    .await
    .context("Git scan timed out after 120 seconds")??;
    ensure!(
        status.success(),
        "Git {} failed: {}",
        args.first().copied().unwrap_or("query"),
        String::from_utf8_lossy(&stderr).trim()
    );
    Ok(stdout)
}

pub async fn text(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(run(root, args, &[], None, false).await?)?
        .trim_end_matches('\n')
        .into())
}

pub async fn resolve(root: &Path, reference: &str) -> Result<String> {
    ensure!(
        !reference.is_empty() && !reference.contains('\0'),
        "Invalid commit reference"
    );
    let sha = text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )
    .await
    .with_context(|| {
        format!("Commit {reference:?} is unavailable locally; no fetch was attempted")
    })?;
    ensure!(
        matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "Git returned an invalid commit ID"
    );
    Ok(sha)
}

pub struct Snapshot {
    pub root: PathBuf,
    pub source: PathBuf,
    pub source_head: String,
    pub head: String,
    pub base: String,
    pub tree: String,
    _directory: TempDir,
}
impl Snapshot {
    pub async fn prepare(
        repository: &Path,
        base_ref: &str,
        head_ref: Option<&str>,
    ) -> Result<Self> {
        // mrde-git's query helpers inherit these variables. Reject them before calling
        // the shared engine, rather than allowing them to redirect a snapshot export.
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_NAMESPACE",
        ] {
            ensure!(
                std::env::var_os(name).is_none(),
                "Unset {name} before scanning; it overrides the isolated Git repository"
            );
        }
        let source = PathBuf::from(text(repository, &["rev-parse", "--show-toplevel"]).await?)
            .canonicalize()
            .context("Cannot find the repository root")?;
        let source_head = resolve(&source, "HEAD").await?;
        let base = resolve(&source, base_ref).await?;
        let objects = text(
            &source,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
        )
        .await?;
        ensure!(
            !objects.contains(['\n', '\r']),
            "Object directory names containing line breaks are unsupported"
        );
        let format = text(&source, &["rev-parse", "--show-object-format"]).await?;
        ensure!(
            matches!(format.as_str(), "sha1" | "sha256"),
            "Unsupported Git object format"
        );
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("repository");
        std::fs::create_dir(&root)?;
        run(
            &root,
            &[
                "init",
                "-q",
                "--template=",
                &format!("--object-format={format}"),
            ],
            &[],
            None,
            true,
        )
        .await?;
        std::fs::write(
            root.join(".git/objects/info/alternates"),
            format!("{objects}\n"),
        )?;
        // Local overrides also apply to the existing engine's Git subprocesses.
        for (key, value) in [
            ("core.fsmonitor", "false"),
            ("core.hooksPath", "/dev/null"),
            ("core.attributesFile", "/dev/null"),
            ("gc.auto", "0"),
            ("maintenance.auto", "false"),
        ] {
            run(&root, &["config", key, value], &[], None, true).await?;
        }
        // Do not run user-level smudge/clean helpers during command snapshot exports.
        // Names only: command contents (which may contain secrets) are never read here.
        let keys = text(
            &source,
            &[
                "config",
                "--name-only",
                "--get-regexp",
                r"^filter\..*\.(clean|smudge|process|required)$",
            ],
        )
        .await
        .unwrap_or_default();
        for key in keys.lines() {
            let value = if key.ends_with(".required") {
                "false"
            } else {
                ""
            };
            run(&root, &["config", key, value], &[], None, true).await?;
        }
        let shallow = PathBuf::from(
            text(
                &source,
                &[
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    "shallow",
                ],
            )
            .await?,
        );
        if shallow.is_file() {
            std::fs::copy(shallow, root.join(".git/shallow"))?;
        }
        let head = if let Some(reference) = head_ref {
            resolve(&source, reference).await?
        } else {
            let entries = run(&source, &["ls-files", "--stage", "-z"], &[], None, false).await?;
            for entry in entries.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                let header = entry
                    .split(|b| *b == b'\t')
                    .next()
                    .context("Missing index entry")?;
                ensure!(
                    header.ends_with(b" 0"),
                    "Resolve unmerged index entries before scanning the working tree"
                );
            }
            let flags = run(&source, &["ls-files", "-v", "-z"], &[], None, false).await?;
            ensure!(
                !flags.split(|b| *b == 0).any(|e| e
                    .first()
                    .is_some_and(|b| *b == b'S' || b.is_ascii_lowercase())),
                "Working-tree scans do not support skip-worktree or assume-unchanged entries; use --head for a committed scan"
            );
            run(
                &root,
                &["update-index", "-z", "--index-info"],
                &entries,
                None,
                true,
            )
            .await?;
            // Source Git applies .gitignore, .git/info/exclude and global excludes.
            let paths = run(
                &source,
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ],
                &[],
                None,
                false,
            )
            .await?;
            if !paths.is_empty() {
                run(
                    &root,
                    &[
                        "--literal-pathspecs",
                        "add",
                        "--all",
                        "--force",
                        "--pathspec-from-file=-",
                        "--pathspec-file-nul",
                    ],
                    &paths,
                    Some(&source),
                    true,
                )
                .await?;
            }
            ensure!(
                entries == run(&source, &["ls-files", "--stage", "-z"], &[], None, false).await?
                    && source_head == resolve(&source, "HEAD").await?,
                "The source index or HEAD changed during capture; retry the scan"
            );
            let tree = text(&root, &["write-tree"]).await?;
            // Deterministic identity/date: this object exists only in the temporary store.
            let commit = format!(
                "tree {tree}\nparent {source_head}\nauthor MR.DE <radar@example.invalid> 0 +0000\ncommitter MR.DE <radar@example.invalid> 0 +0000\n\nMR.DE working-tree snapshot\n"
            );
            String::from_utf8(
                run(
                    &root,
                    &["hash-object", "-t", "commit", "-w", "--stdin"],
                    commit.as_bytes(),
                    None,
                    true,
                )
                .await?,
            )?
            .trim()
            .into()
        };
        let tree = text(&root, &["rev-parse", &format!("{head}^{{tree}}")]).await?;
        Ok(Self {
            root,
            source,
            source_head,
            head,
            base,
            tree,
            _directory: directory,
        })
    }

    pub async fn file(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let names = run(
            &self.root,
            &["ls-tree", "--name-only", "-z", &self.head, "--", path],
            &[],
            None,
            true,
        )
        .await?;
        if names.is_empty() {
            return Ok(None);
        }
        let bytes = run(
            &self.root,
            &["show", &format!("{}:{path}", self.head)],
            &[],
            None,
            true,
        )
        .await?;
        ensure!(
            bytes.len() <= limit,
            "Snapshot file {path:?} exceeds the size limit"
        );
        Ok(Some(bytes))
    }
}
