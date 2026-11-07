//! Explicit tool preparation. Scoring stays offline; only the setup action may download.
use crate::{
    MrContext,
    config::{Command, Preparation},
};
use anyhow::{Context, Result, ensure};
use mrde_checks::{
    Approval, Check,
    environment::Environment,
    runner::{Network, run_in},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const LIZARD: &str = "1.25.1";
const RUFF: &str = "0.17.4";
static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// Unlock explicitly: another thread may have forked while the descriptor was open.
// Waiting for every inherited copy to close can otherwise cause transient busy errors.
pub(crate) struct FileLock(pub(crate) std::fs::File);
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub fn default_cache() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    #[cfg(target_os = "macos")]
    let cache = home.join("Library/Caches/mrde");
    #[cfg(not(target_os = "macos"))]
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".cache"))
        .join("mrde");
    Some(cache)
}
fn tool_root(cache: &Path, name: &str, version: &str) -> PathBuf {
    cache.join("risk-tools").join(format!("{name}-{version}"))
}
pub fn environment(cache: Option<&Path>) -> Environment {
    let mut env = Environment::default();
    let fallback = default_cache();
    if let Some(cache) = cache.or(fallback.as_deref()) {
        let cache = if cache.is_absolute() {
            cache.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(cache)
        };
        for (name, version) in [("lizard", LIZARD), ("ruff", RUFF)] {
            let root = tool_root(&cache, name, version);
            if root.join("ready").is_file() {
                env.prepend(root.join("bin"));
            }
        }
    }
    env
}
pub(crate) async fn execute(
    id: &str,
    argv: Vec<String>,
    root: &Path,
    env: &Environment,
    network: Network,
) -> Result<()> {
    let result = run_in(
        &Check {
            id: id.into(),
            command: argv,
            approval: Approval::Approved,
        },
        root,
        Duration::from_secs(180),
        env,
        network,
    )
    .await?;
    // Package manager output may contain private registry URLs or credentials.
    ensure!(
        result.code == Some(0),
        "{id} failed (exit {:?}). Check the SDK, lockfile and registry access, then retry setup.",
        result.code
    );
    Ok(())
}
fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}

async fn python_tool(cache: &Path, name: &str, version: &str, env: &mut Environment) -> Result<()> {
    let _guard = INSTALL_LOCK.lock().await;
    let root = tool_root(cache, name, version);
    let managed = root.join("bin").join(name);
    if root.join("ready").is_file()
        && managed.is_file()
        && execute(
            &format!("Verify {name}"),
            vec![managed.to_string_lossy().into_owned(), "--version".into()],
            cache,
            env,
            Network::Offline,
        )
        .await
        .is_ok()
    {
        env.prepend(root.join("bin"));
        return Ok(());
    }
    // Respect a working user installation; repair a broken managed interpreter locally.
    if let Some(program) = env.find(name, cache).filter(|p| !p.starts_with(&root))
        && execute(
            &format!("Verify {name}"),
            vec![program.to_string_lossy().into_owned(), "--version".into()],
            cache,
            env,
            Network::Offline,
        )
        .await
        .is_ok()
    {
        return Ok(());
    }
    let mut host = env.clone();
    host.directories
        .retain(|p| !p.starts_with(cache.join("risk-tools")));
    let python = host.find("python3", cache).context("Python 3 is required to prepare radar tools. Install Python, then retry Detect and install tools.")?;
    std::fs::create_dir_all(&root)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("install.lock"))?;
    // OS locks release on cancellation or process death; a crash cannot leave setup stuck.
    lock.try_lock()
        .context("This tool is already being installed. Retry after that operation finishes.")?;
    let _file_lock = FileLock(lock);
    let _ = std::fs::remove_file(root.join("ready"));
    execute(
        "Create isolated radar environment",
        vec![
            python.to_string_lossy().into_owned(),
            "-I".into(),
            "-m".into(),
            "venv".into(),
            root.to_string_lossy().into_owned(),
        ],
        cache,
        env,
        Network::Setup,
    )
    .await?;
    execute(
        &format!("Install {name} {version}"),
        vec![
            root.join("bin/python").to_string_lossy().into_owned(),
            "-I".into(),
            "-m".into(),
            "pip".into(),
            "--isolated".into(),
            "--disable-pip-version-check".into(),
            "install".into(),
            "--no-input".into(),
            "--only-binary=:all:".into(),
            "--index-url".into(),
            "https://pypi.org/simple".into(),
            format!("{name}=={version}"),
        ],
        cache,
        env,
        Network::Setup,
    )
    .await?;
    execute(
        &format!("Verify {name}"),
        vec![
            root.join("bin").join(name).to_string_lossy().into_owned(),
            "--version".into(),
        ],
        cache,
        env,
        Network::Offline,
    )
    .await?;
    std::fs::write(root.join("ready"), version)?;
    env.prepend(root.join("bin"));
    Ok(())
}

pub(crate) fn directory(root: &Path, command: &Command) -> Result<PathBuf> {
    let directory = root
        .join(&command.directory)
        .canonicalize()
        .with_context(|| {
            format!(
                "Package '{}' is absent in one commit; comparable lint results are unavailable",
                command.directory
            )
        })?;
    ensure!(
        directory.starts_with(root.canonicalize()?),
        "Check directory escapes its isolated snapshot"
    );
    Ok(directory)
}

pub(crate) async fn dependencies(
    command: &Command,
    root: &Path,
    env: &Environment,
    network: Network,
) -> Result<()> {
    ensure!(
        command.approved,
        "{} is proposed and needs local approval",
        command.id
    );
    let Some(recipe) = command.preparation else {
        return Ok(());
    };
    let mut package = command.clone();
    if matches!(
        recipe,
        Preparation::Npm | Preparation::Bun | Preparation::JavaScript
    ) {
        let (owner, actual) = crate::javascript::from_snapshot(root, &command.directory)?;
        ensure!(
            actual == recipe,
            "Package manager differs between the reviewed commits; configure explicit comparable lint commands in risk.toml"
        );
        ensure!(
            command
                .preparation_directory
                .as_ref()
                .is_none_or(|expected| expected == &owner),
            "Workspace ownership differs between the reviewed commits; configure an explicit lint command in risk.toml"
        );
        package.directory = owner;
    } else if let Some(directory) = &command.preparation_directory {
        package.directory = directory.clone();
    }
    let root = directory(root, &package)?;
    let online = network == Network::Setup;
    let argv = match recipe {
        Preparation::Cargo => {
            ensure!(
                root.join("Cargo.lock").is_file(),
                "Cargo.lock is missing; configure a custom lint command or commit a lockfile"
            );
            let mut argv = args(&["cargo", "fetch", "--locked"]);
            if !online {
                argv.push("--offline".into());
            }
            argv
        }
        Preparation::Npm => {
            let mut argv = args(&["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund"]);
            if !online {
                argv.push("--offline".into());
            }
            argv
        }
        Preparation::Bun => {
            let mut argv = args(&["bun", "install", "--frozen-lockfile", "--ignore-scripts"]);
            if !online {
                let help = run_in(
                    &Check {
                        id: "Check Bun offline support".into(),
                        command: args(&["bun", "install", "--help"]),
                        approval: Approval::Approved,
                    },
                    &root,
                    Duration::from_secs(10),
                    env,
                    Network::Offline,
                )
                .await?;
                ensure!(
                    help.code == Some(0)
                        && help
                            .stdout
                            .split_whitespace()
                            .chain(help.stderr.split_whitespace())
                            .any(|word| word == "--offline"),
                    "Bun dependencies for this commit are not cached yet. Run Detect and install tools once; subsequent calculations can reuse the prepared dependencies offline."
                );
                argv.push("--offline".into());
            }
            argv
        }
        Preparation::JavaScript => {
            anyhow::bail!("JavaScript package manager could not be resolved; configure risk.toml")
        }
        Preparation::Dart | Preparation::Flutter => {
            ensure!(
                root.join("pubspec.lock").is_file(),
                "pubspec.lock is missing; configure a custom lint command or commit a lockfile"
            );
            let mut argv = args(&[
                if recipe == Preparation::Flutter {
                    "flutter"
                } else {
                    "dart"
                },
                "pub",
                "get",
                "--enforce-lockfile",
            ]);
            if !online {
                argv.push("--offline".into());
            }
            argv
        }
        Preparation::Go => args(&["go", "mod", "download"]),
        Preparation::Ruff => return Ok(()),
    };
    execute(&format!("Prepare {} dependencies", command.id), argv, &root, env, network).await
        .context("Lint dependencies are unavailable. Use Detect and install tools to prepare the approved check")
}

/// Restore immutable prepared Bun dependencies into a fresh disposable snapshot.
pub(crate) async fn prepared_dependencies(
    ctx: &MrContext,
    command: &Command,
    root: &Path,
    sha: &str,
    network: Network,
) -> Result<()> {
    ensure!(
        command.approved,
        "{} is proposed and needs local approval",
        command.id
    );
    if command.preparation != Some(Preparation::Bun) {
        return dependencies(command, root, &ctx.environment, network).await;
    }
    let cache = crate::dependency_cache::BunCache::new(
        ctx.tool_cache
            .as_deref()
            .context("Tool cache unavailable; configure the MR.DE cache directory")?,
        command,
        sha,
        root,
        &ctx.environment,
    )
    .await?;
    if network == Network::Offline && cache.restore(root, &ctx.environment).await? {
        return Ok(());
    }
    dependencies(command, root, &ctx.environment, network).await?;
    // Publish before lint runs: a check must never poison the reusable dependency tree.
    cache.store(root, &ctx.environment).await
}

/// Called only by an explicit setup action, never by background refresh or `analyze`.
pub async fn prepare(ctx: &mut MrContext, cache: &Path) {
    let cache = if cache.is_absolute() {
        cache.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(directory) => directory.join(cache),
            Err(error) => {
                ctx.setup_messages
                    .push(format!("Tool cache unavailable: {error}"));
                return;
            }
        }
    };
    let cache = cache.as_path();
    ctx.set_tool_cache(cache);
    if let Err(error) = std::fs::create_dir_all(cache) {
        ctx.setup_messages
            .push(format!("Tool cache unavailable: {error}"));
        return;
    }
    let complexity = ctx.config.complexity.clone();
    if complexity.approved && complexity.command.first().map(String::as_str) == Some("lizard") {
        match python_tool(cache, "lizard", LIZARD, &mut ctx.environment).await {
            Ok(()) => ctx
                .setup_messages
                .push("Lizard is ready for complexity measurements.".into()),
            Err(error) => {
                let message = format!("Lizard setup: {error:#}");
                ctx.check_errors.insert(complexity.key(), message.clone());
                ctx.setup_messages.push(message);
            }
        }
    }
    for command in ctx.config.lint_commands().to_vec() {
        if !command.approved {
            ctx.setup_messages.push(format!("{} detected. Allow this command to install its prerequisites and compare the two commits.", command.id));
            continue;
        }
        let result: Result<()> = async {
            if command.preparation == Some(Preparation::Ruff) {
                python_tool(cache, "ruff", RUFF, &mut ctx.environment).await?;
            }
            let (base, head) = ctx.snapshot_paths(&command).await?;
            if command.preparation == Some(Preparation::Cargo) {
                let root = directory(&head, &command)?;
                let available = execute(
                    "Check Clippy",
                    args(&["cargo", "clippy", "--version"]),
                    &root,
                    &ctx.environment,
                    Network::Offline,
                )
                .await;
                if available.is_err() {
                    execute(
                        "Install Clippy for the installed Rust toolchain",
                        args(&["rustup", "component", "add", "clippy"]),
                        &root,
                        &ctx.environment,
                        Network::Setup,
                    )
                    .await?;
                }
            }
            for (root, sha) in [(&base, &ctx.base_sha), (&head, &ctx.head_sha)] {
                prepared_dependencies(ctx, &command, root, sha, Network::Setup).await?;
            }
            ctx.prepared_checks.insert(command.key());
            Ok(())
        }
        .await;
        match result {
            Ok(()) => ctx
                .setup_messages
                .push(format!("{} prerequisites are ready.", command.id)),
            Err(error) => {
                let message = format!("{} setup: {error:#}", command.id);
                ctx.check_errors.insert(command.key(), message.clone());
                ctx.setup_messages.push(message);
            }
        }
    }
    if ctx.config.lint_commands().is_empty() {
        ctx.setup_messages.push("No supported lint tool detected for the changed code. Add a [[lint]] command in risk.toml for this project.".into());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn isolated_installation_is_reusable_and_failed_installations_can_retry() {
        let temp = tempfile::tempdir().unwrap();
        let python = temp.path().join("python3");
        let script = |fail: bool| {
            format!(
                r#"#!/bin/sh
case "$3" in
venv)
 /bin/mkdir -p "$4/bin"
 /bin/cp "$0" "$4/bin/python"
 ;;
pip)
 {}
 directory=$(/usr/bin/dirname "$0")
 /usr/bin/printf '#!/bin/sh\nexit 0\n' > "$directory/lizard"
 /bin/chmod 755 "$directory/lizard"
 /usr/bin/printf 'installed\n' >> "$directory/install-count"
 ;;
esac
"#,
                if fail { "exit 7" } else { "true" }
            )
        };
        std::fs::write(&python, script(true)).unwrap();
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = Environment {
            directories: vec![temp.path().into()],
        };
        let cache = temp.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        assert!(
            python_tool(&cache, "lizard", LIZARD, &mut env)
                .await
                .is_err()
        );
        let root = tool_root(&cache, "lizard", LIZARD);
        assert!(!root.join("ready").exists());
        std::fs::write(&python, script(false)).unwrap();
        python_tool(&cache, "lizard", LIZARD, &mut env)
            .await
            .unwrap();
        python_tool(&cache, "lizard", LIZARD, &mut env)
            .await
            .unwrap();
        assert!(root.join("ready").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join("bin/install-count")).unwrap(),
            "installed\n"
        );
        assert_eq!(env.find("lizard", &cache), Some(root.join("bin/lizard")));
        // A tool left behind with a broken launcher is repaired on the next setup.
        std::fs::write(root.join("bin/lizard"), "#!/bin/sh\nexit 7\n").unwrap();
        python_tool(&cache, "lizard", LIZARD, &mut env)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("bin/install-count")).unwrap(),
            "installed\ninstalled\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_preparation_preserves_lockfiles_skips_scripts_and_never_silently_goes_online() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let bin = root.join("tools");
        std::fs::create_dir(&bin).unwrap();
        std::fs::create_dir_all(root.join("packages/web")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"workspaces":["packages/*"],"packageManager":"bun@1.3.3"}"#,
        )
        .unwrap();
        std::fs::write(root.join("packages/web/package.json"), "{}").unwrap();
        std::fs::write(root.join("bun.lock"), "original lockfile").unwrap();
        let script = |offline: bool| {
            format!(
                "#!/bin/sh\nif [ \"$2\" = --help ]; then echo '{}'; exit 0; fi\nprintf '%s\\n' \"$*\" >> bun-invocations\n",
                if offline {
                    "--offline"
                } else {
                    "--prefer-offline"
                }
            )
        };
        let bun = bin.join("bun");
        std::fs::write(&bun, script(false)).unwrap();
        std::fs::set_permissions(&bun, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = Environment {
            directories: vec![bin],
        };
        let mut command = Command {
            id: "eslint@packages/web".into(),
            directory: "packages/web".into(),
            preparation: Some(Preparation::Bun),
            preparation_directory: Some(String::new()),
            approved: false,
            ..Command::default()
        };
        assert!(
            dependencies(&command, root, &env, Network::Setup)
                .await
                .unwrap_err()
                .to_string()
                .contains("approval")
        );
        assert!(!root.join("bun-invocations").exists());
        command.approved = true;
        dependencies(&command, root, &env, Network::Setup)
            .await
            .unwrap();
        let log = root.join("bun-invocations");
        let first = std::fs::read_to_string(&log).unwrap();
        assert_eq!(first, "install --frozen-lockfile --ignore-scripts\n");
        assert!(!root.join("packages/web/bun-invocations").exists());
        let error = dependencies(&command, root, &env, Network::Offline)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not cached yet"));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), first);
        std::fs::write(&bun, script(true)).unwrap();
        dependencies(&command, root, &env, Network::Offline)
            .await
            .unwrap();
        assert!(
            std::fs::read_to_string(&log)
                .unwrap()
                .ends_with("--ignore-scripts --offline\n")
        );
        assert_eq!(
            std::fs::read_to_string(root.join("bun.lock")).unwrap(),
            "original lockfile"
        );
    }
}
