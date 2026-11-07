//! Prepared dependencies are immutable. Lint always receives a separate copy.
use crate::{config::Command, setup::FileLock};
use anyhow::{Context, Result, ensure};
use mrde_checks::{
    Approval, Check,
    environment::Environment,
    runner::{Network, run_in},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};

const MAX_ENTRIES: usize = 500_000;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    key: String,
    directories: Vec<PathBuf>,
}

pub(crate) struct BunCache {
    parent: PathBuf,
    entry: PathBuf,
    key: String,
}
impl BunCache {
    pub(crate) async fn new(
        cache: &Path,
        command: &Command,
        sha: &str,
        snapshot: &Path,
        env: &Environment,
    ) -> Result<Self> {
        let result = run_in(
            &Check {
                id: "Identify Bun dependency cache".into(),
                command: vec!["bun".into(), "--version".into()],
                approval: Approval::Approved,
            },
            snapshot,
            Duration::from_secs(10),
            env,
            Network::Offline,
        )
        .await?;
        let version = result.stdout.trim();
        ensure!(
            result.code == Some(0) && !version.is_empty() && version.len() <= 128,
            "Cannot identify the installed Bun version"
        );
        let key = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                "bun-dependencies-v1",
                sha,
                command.key(),
                version,
                std::env::consts::OS,
                std::env::consts::ARCH,
            ))?)
        );
        let parent = std::path::absolute(cache)?.join("risk-tools/bun-dependencies-v1");
        let entry = parent.join(&key);
        Ok(Self { parent, entry, key })
    }

    fn lock(&self, exclusive: bool) -> Result<FileLock> {
        fs::create_dir_all(&self.parent)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.parent.join(format!("{}.lock", self.key)))?;
        if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        }
        .context(
            "Bun dependencies are currently being prepared; retry when that operation finishes",
        )?;
        Ok(FileLock(file))
    }

    pub(crate) async fn restore(&self, snapshot: &Path, env: &Environment) -> Result<bool> {
        let manifest_path = self.entry.join("manifest.json");
        if !manifest_path.is_file() {
            return Ok(false);
        }
        ensure!(
            fs::symlink_metadata(&self.entry)?.is_dir(),
            "Invalid Bun dependency cache entry"
        );
        let _lock = self.lock(false)?;
        let result: Result<()> = async {
            ensure!(
                fs::metadata(&manifest_path)?.len() <= 1024 * 1024,
                "Invalid dependency cache manifest"
            );
            let manifest: Manifest =
                serde_json::from_slice(&tokio::fs::read(manifest_path).await?)?;
            ensure!(manifest.key == self.key, "Dependency cache key mismatch");
            validate_directories(&manifest.directories)?;
            let source = self.entry.clone();
            let directories = manifest.directories.clone();
            let links = tokio::task::spawn_blocking(move || audit(&source, &directories)).await??;
            ensure!(
                links.is_empty(),
                "Dependency cache contains non-relocatable links"
            );
            for directory in manifest.directories {
                create_parents(snapshot, &directory)?;
                let destination = snapshot.join(&directory);
                if let Ok(metadata) = fs::symlink_metadata(&destination) {
                    ensure!(
                        metadata.is_dir() && !metadata.is_symlink(),
                        "Unsafe dependency destination"
                    );
                    tokio::fs::remove_dir_all(&destination).await?;
                }
                copy(&self.entry.join(directory), &destination, snapshot, env).await?;
            }
            Ok(())
        }
        .await;
        result.context("Prepared Bun dependencies could not be restored. Run Detect and install tools to rebuild their cache")?;
        Ok(true)
    }

    pub(crate) async fn store(&self, snapshot: &Path, env: &Environment) -> Result<()> {
        let _lock = self.lock(true)?;
        let source = snapshot.to_path_buf();
        let (directories, links) = tokio::task::spawn_blocking(move || {
            let directories = discover(&source)?;
            let links = audit(&source, &directories)?;
            Ok::<_, anyhow::Error>((directories, links))
        })
        .await??;
        let stage = tempfile::Builder::new()
            .prefix(".prepare-")
            .tempdir_in(&self.parent)?;
        for directory in &directories {
            create_parents(stage.path(), directory)?;
            copy(
                &snapshot.join(directory),
                &stage.path().join(directory),
                snapshot,
                env,
            )
            .await?;
        }
        // Absolute links into the old snapshot become relocatable workspace links.
        for (path, target) in links {
            let path = stage.path().join(path);
            fs::remove_file(&path)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, path)?;
            #[cfg(not(unix))]
            anyhow::bail!("Bun dependency caching requires a Unix platform");
        }
        let manifest = Manifest {
            key: self.key.clone(),
            directories,
        };
        tokio::fs::write(
            stage.path().join("manifest.json"),
            serde_json::to_vec(&manifest)?,
        )
        .await?;
        // Publish only a complete tree. A failed or cancelled copy cannot become a cache hit.
        let retired = tempfile::Builder::new()
            .prefix(".retired-")
            .tempdir_in(&self.parent)?;
        let old = retired.path().join("old");
        let existed = self.entry.try_exists()?;
        if existed {
            fs::rename(&self.entry, &old)?;
        }
        if let Err(error) = fs::rename(stage.path(), &self.entry) {
            if existed {
                fs::rename(old, &self.entry)?;
            }
            return Err(error.into());
        }
        Ok(())
    }
}

async fn copy(source: &Path, destination: &Path, snapshot: &Path, env: &Environment) -> Result<()> {
    crate::setup::execute(
        "Copy prepared Bun dependencies",
        vec![
            "/bin/cp".into(),
            "-R".into(),
            "-P".into(),
            source.to_string_lossy().into_owned(),
            destination.to_string_lossy().into_owned(),
        ],
        snapshot,
        env,
        Network::Offline,
    )
    .await
}

fn validate_directories(directories: &[PathBuf]) -> Result<()> {
    ensure!(
        !directories.is_empty() && directories.len() <= 256,
        "Invalid dependency directory count"
    );
    for (index, path) in directories.iter().enumerate() {
        ensure!(
            path.file_name().is_some_and(|p| p == "node_modules")
                && path.components().all(|p| matches!(p, Component::Normal(_))),
            "Invalid dependency directory"
        );
        ensure!(
            !directories[..index]
                .iter()
                .any(|other| path.starts_with(other) || other.starts_with(path)),
            "Overlapping dependency directories"
        );
    }
    Ok(())
}
fn discover(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![PathBuf::new()];
    let mut directories = Vec::new();
    let mut count = 0;
    while let Some(relative) = pending.pop() {
        for entry in fs::read_dir(root.join(&relative))? {
            count += 1;
            ensure!(
                count <= MAX_ENTRIES,
                "Dependency snapshot has too many files"
            );
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = relative.join(entry.file_name());
            if entry.file_name() == "node_modules" {
                directories.push(path);
            } else {
                pending.push(path);
            }
        }
    }
    directories.sort();
    validate_directories(&directories)?;
    Ok(directories)
}
/// Inspect without following links; keep every link inside the disposable snapshot.
fn audit(root: &Path, directories: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let canonical = root.canonicalize()?;
    let mut pending = directories.to_vec();
    let mut links = Vec::new();
    let mut count = 0;
    let mut bytes = 0;
    while let Some(relative) = pending.pop() {
        count += 1;
        ensure!(
            count <= MAX_ENTRIES && relative.components().count() <= 128,
            "Dependency cache is too large or deeply nested"
        );
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_symlink() {
            let target = fs::read_link(&path)?;
            let parent = relative
                .parent()
                .context("Missing dependency link parent")?;
            if target.is_absolute() {
                let target = target
                    .strip_prefix(root)
                    .or_else(|_| target.strip_prefix(&canonical))
                    .context("Dependency links outside its snapshot cannot be cached")?;
                links.push((
                    relative.clone(),
                    relative_link(parent, &normalize(target)?)?,
                ));
            } else {
                normalize(&parent.join(target))?;
            }
        } else if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(relative.join(entry?.file_name()));
            }
        } else {
            ensure!(metadata.is_file(), "Unsupported file in dependency cache");
            bytes += metadata.len();
            ensure!(
                bytes <= MAX_BYTES,
                "Prepared dependencies exceed the 2 GiB cache entry limit"
            );
        }
    }
    Ok(links)
}
fn normalize(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => (),
            Component::ParentDir => {
                ensure!(normalized.pop(), "Dependency link escapes its snapshot")
            }
            _ => anyhow::bail!("Invalid dependency link"),
        }
    }
    Ok(normalized)
}
fn relative_link(parent: &Path, target: &Path) -> Result<PathBuf> {
    let parent = normalize(parent)?;
    let a: Vec<_> = parent.components().collect();
    let b: Vec<_> = target.components().collect();
    let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let mut link = PathBuf::new();
    for _ in common..a.len() {
        link.push("..");
    }
    for component in &b[common..] {
        link.push(component);
    }
    if link.as_os_str().is_empty() {
        link.push(".");
    }
    Ok(link)
}
fn create_parents(root: &Path, relative: &Path) -> Result<()> {
    let mut parent = root.to_path_buf();
    for part in relative
        .parent()
        .context("Missing dependency parent")?
        .components()
    {
        ensure!(
            matches!(part, Component::Normal(_)),
            "Invalid dependency path"
        );
        parent.push(part);
        match fs::symlink_metadata(&parent) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.is_symlink(),
                "Dependency destination traverses a link"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&parent)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::Preparation;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn write(root: &Path, path: &str, text: &str) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    fn bun(bin: &Path, version: &str) -> Environment {
        fs::create_dir_all(bin).unwrap();
        write(
            bin,
            "bun",
            &format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n"),
        );
        fs::set_permissions(bin.join("bun"), fs::Permissions::from_mode(0o755)).unwrap();
        Environment {
            directories: vec![bin.into()],
        }
    }
    fn command() -> Command {
        Command {
            preparation: Some(Preparation::Bun),
            approved: true,
            ..Command::default()
        }
    }
    fn source(root: &Path) {
        write(root, "node_modules/eslint/bin/eslint.js", "pristine");
        fs::set_permissions(
            root.join("node_modules/eslint/bin/eslint.js"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        write(root, "packages/web/index.js", "workspace source");
        fs::create_dir_all(root.join("packages/web/node_modules")).unwrap();
        symlink(
            "../../../node_modules/eslint",
            root.join("packages/web/node_modules/eslint"),
        )
        .unwrap();
        symlink(root.join("packages/web"), root.join("node_modules/web")).unwrap();
        write(root, ".npmrc", "not part of the dependency cache");
    }

    #[tokio::test]
    async fn restores_after_snapshot_deletion_without_sharing_mutable_files_or_absolute_links() {
        let temp = tempfile::tempdir().unwrap();
        let env = bun(&temp.path().join("bin"), "1.3.3");
        let original = tempfile::tempdir().unwrap();
        source(original.path());
        let cache = BunCache::new(
            &temp.path().join("cache with spaces"),
            &command(),
            "commit-a",
            original.path(),
            &env,
        )
        .await
        .unwrap();
        cache.store(original.path(), &env).await.unwrap();
        assert!(!cache.entry.join(".npmrc").exists());
        assert!(!cache.entry.join("packages/web/index.js").exists());
        drop(original);
        for _ in 0..2 {
            let fresh = tempfile::tempdir().unwrap();
            write(fresh.path(), "packages/web/index.js", "workspace source");
            assert!(cache.restore(fresh.path(), &env).await.unwrap());
            assert_eq!(
                fs::read_to_string(
                    fresh
                        .path()
                        .join("packages/web/node_modules/eslint/bin/eslint.js")
                )
                .unwrap(),
                "pristine"
            );
            assert_eq!(
                fs::read_to_string(fresh.path().join("node_modules/web/index.js")).unwrap(),
                "workspace source"
            );
            assert!(
                !fs::read_link(fresh.path().join("node_modules/web"))
                    .unwrap()
                    .is_absolute()
            );
            assert_ne!(
                fs::metadata(fresh.path().join("node_modules/eslint/bin/eslint.js"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
            // Even a destructive linter can only change this disposable copy.
            fs::write(
                fresh.path().join("node_modules/eslint/bin/eslint.js"),
                "modified by lint",
            )
            .unwrap();
        }
        let fresh = tempfile::tempdir().unwrap();
        let changed = BunCache::new(
            &temp.path().join("cache with spaces"),
            &command(),
            "commit-b",
            fresh.path(),
            &env,
        )
        .await
        .unwrap();
        assert!(!changed.restore(fresh.path(), &env).await.unwrap());
        let mut different = command();
        different.directory = "packages/web".into();
        let changed = BunCache::new(
            &temp.path().join("cache with spaces"),
            &different,
            "commit-a",
            fresh.path(),
            &env,
        )
        .await
        .unwrap();
        assert!(!changed.restore(fresh.path(), &env).await.unwrap());
        let env = bun(&temp.path().join("bin"), "1.4.4");
        let changed = BunCache::new(
            &temp.path().join("cache with spaces"),
            &command(),
            "commit-a",
            fresh.path(),
            &env,
        )
        .await
        .unwrap();
        assert!(!changed.restore(fresh.path(), &env).await.unwrap());
    }

    #[tokio::test]
    async fn failed_publication_keeps_previous_cache_and_setup_repairs_invalid_entries() {
        let temp = tempfile::tempdir().unwrap();
        let env = bun(&temp.path().join("bin"), "1.3.3");
        let original = tempfile::tempdir().unwrap();
        source(original.path());
        let cache = BunCache::new(
            &temp.path().join("cache"),
            &command(),
            "commit-a",
            original.path(),
            &env,
        )
        .await
        .unwrap();
        cache.store(original.path(), &env).await.unwrap();
        symlink("../../outside", original.path().join("node_modules/escape")).unwrap();
        assert!(cache.store(original.path(), &env).await.is_err());
        let fresh = tempfile::tempdir().unwrap();
        write(fresh.path(), "packages/web/index.js", "workspace source");
        assert!(cache.restore(fresh.path(), &env).await.unwrap());
        fs::remove_file(original.path().join("node_modules/escape")).unwrap();
        // Corruption is actionable and never turns into a partial successful restore.
        fs::write(
            cache.entry.join("manifest.json"),
            serde_json::to_vec(&Manifest {
                key: cache.key.clone(),
                directories: vec!["../outside/node_modules".into()],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(
            cache
                .restore(fresh.path(), &env)
                .await
                .unwrap_err()
                .to_string()
                .contains("Detect and install tools")
        );
        cache.store(original.path(), &env).await.unwrap();
        assert!(cache.restore(fresh.path(), &env).await.unwrap());
        assert!(!temp.path().join("outside").exists());
    }
}
