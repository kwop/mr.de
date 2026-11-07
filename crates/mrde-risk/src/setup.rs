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
        for (name, version) in [("lizard", LIZARD), ("ruff", RUFF)] {
            let root = tool_root(cache, name, version);
            if root.join("ready").is_file() {
                env.prepend(root.join("bin"));
            }
        }
    }
    env
}
async fn execute(
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

struct InstallFile(PathBuf);
impl Drop for InstallFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn python_tool(cache: &Path, name: &str, version: &str, env: &mut Environment) -> Result<()> {
    let _guard = INSTALL_LOCK.lock().await;
    let root = tool_root(cache, name, version);
    if root.join("ready").is_file() && root.join("bin").join(name).is_file() {
        env.prepend(root.join("bin"));
        return Ok(());
    }
    // Respect an already-installed user executable, including tools invisible to Finder.
    if env.find(name, cache).is_some() {
        return Ok(());
    }
    let python = env.find("python3", cache).context("Python 3 is required to prepare radar tools. Install Python, then retry Detect and install tools.")?;
    std::fs::create_dir_all(&root)?;
    let lock = root.join("install.lock");
    std::fs::OpenOptions::new().write(true).create_new(true).open(&lock)
        .context("This tool is already being installed. Retry after that operation finishes; after a crash, remove its install.lock in the risk-tools cache.")?;
    let _file = InstallFile(lock);
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
    let Some(recipe) = command.preparation else {
        return Ok(());
    };
    let root = directory(root, command)?;
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
            ensure!(
                root.join("package-lock.json").is_file()
                    || root.join("npm-shrinkwrap.json").is_file(),
                "No npm lockfile in this package. Configure risk.toml for its package manager; MR.DE will not choose new dependency versions."
            );
            let mut argv = args(&["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund"]);
            if !online {
                argv.push("--offline".into());
            }
            argv
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

/// Called only by an explicit setup action, never by background refresh or `analyze`.
pub async fn prepare(ctx: &mut MrContext, cache: &Path) {
    ctx.environment = environment(Some(cache));
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
            for root in [&base, &head] {
                dependencies(&command, root, &ctx.environment, Network::Setup).await?;
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
        assert!(!root.join("install.lock").exists());
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
    }
}
