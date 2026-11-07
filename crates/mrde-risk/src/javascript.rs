//! Package-manager and workspace ownership, shared by detection and snapshot setup.
use crate::config::Preparation;
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const LOCKFILES: [&str; 5] = [
    "bun.lock",
    "bun.lockb",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
];
const ESLINT: &str = "node_modules/eslint/bin/eslint.js";

pub(crate) fn owns_workspace(package: &Value, relative: &Path) -> bool {
    let workspaces = &package["workspaces"];
    let Some(patterns) = workspaces
        .as_array()
        .or_else(|| workspaces["packages"].as_array())
    else {
        return false;
    };
    let mut included = false;
    for pattern in patterns.iter().filter_map(Value::as_str) {
        let (exclude, pattern) = pattern
            .strip_prefix('!')
            .map_or((false, pattern), |p| (true, p));
        if globset::GlobBuilder::new(pattern.trim_start_matches("./"))
            .literal_separator(true)
            .build()
            .is_ok_and(|glob| glob.compile_matcher().is_match(relative))
        {
            if exclude {
                return false;
            }
            included = true;
        }
    }
    included
}

/// Explicit packageManager wins over leftover lockfiles. Without it, require one manager.
pub(crate) fn resolve(
    directory: &str,
    manifests: &BTreeMap<String, Value>,
    files: &BTreeSet<String>,
) -> Result<(String, Preparation)> {
    let package = Path::new(directory);
    let has = |dir: &Path, name: &str| files.contains(dir.join(name).to_string_lossy().as_ref());
    let has_lock = |dir: &Path| {
        LOCKFILES
            .iter()
            .chain(["yarn.lock"].iter())
            .any(|name| has(dir, name))
    };
    let mut owner = package;
    for parent in package.ancestors().skip(1) {
        if has_lock(owner) {
            break;
        }
        if let Some(manifest) = manifests.get(parent.to_string_lossy().as_ref())
            && owns_workspace(manifest, package.strip_prefix(parent)?)
        {
            owner = parent;
        }
    }
    let label = if owner.as_os_str().is_empty() {
        "repository root"
    } else {
        owner.to_str().unwrap_or("package")
    };
    let mut declared = None;
    for ancestor in package.ancestors() {
        if let Some(value) = manifests
            .get(ancestor.to_string_lossy().as_ref())
            .and_then(|v| v["packageManager"].as_str())
        {
            declared = Some(value.split('@').next().unwrap_or(""));
            break;
        }
        if ancestor == owner {
            break;
        }
    }
    let bun = has(owner, "bun.lock") || has(owner, "bun.lockb");
    let npm = has(owner, "package-lock.json") || has(owner, "npm-shrinkwrap.json");
    let pnpm = has(owner, "pnpm-lock.yaml");
    let yarn = has(owner, "yarn.lock");
    let manager = if let Some(manager) = declared {
        manager
    } else {
        let managers: Vec<_> = [("bun", bun), ("npm", npm), ("pnpm", pnpm), ("yarn", yarn)]
            .into_iter()
            .filter_map(|(manager, present)| present.then_some(manager))
            .collect();
        ensure!(
            managers.len() <= 1,
            "Conflicting package-manager lockfiles in {label}. Set packageManager in package.json to select the intended manager."
        );
        *managers.first().context("No JavaScript lockfile found. Commit bun.lock, bun.lockb or an npm lockfile, or configure a custom lint command in risk.toml")?
    };
    let preparation = match manager {
        "bun" => {
            ensure!(
                bun,
                "Bun is declared in {label}, but bun.lock or bun.lockb is missing. Commit the Bun lockfile; MR.DE will not fall back to npm."
            );
            Preparation::Bun
        }
        "npm" => {
            ensure!(
                npm,
                "npm is declared in {label}, but package-lock.json or npm-shrinkwrap.json is missing. Commit the npm lockfile."
            );
            Preparation::Npm
        }
        "pnpm" | "yarn" => bail!(
            "{manager} detected in {label}. Automatic dependency setup for {manager} is not supported yet; configure its lint command in risk.toml. MR.DE will not substitute npm."
        ),
        _ => bail!(
            "Unsupported packageManager in {label}. Configure an explicit lint command in risk.toml."
        ),
    };
    Ok((owner.to_string_lossy().into_owned(), preparation))
}

pub(crate) fn from_snapshot(root: &Path, directory: &str) -> Result<(String, Preparation)> {
    let mut files = BTreeSet::new();
    let mut manifests = BTreeMap::new();
    for parent in Path::new(directory).ancestors() {
        let path = root.join(parent).join("package.json");
        if path.is_file() {
            ensure!(
                path.metadata()?.len() <= 1024 * 1024,
                "Manifest exceeds the 1 MiB detection limit"
            );
            let value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
            manifests.insert(parent.to_string_lossy().into_owned(), value);
        }
        for name in LOCKFILES.iter().chain(["yarn.lock"].iter()) {
            if root.join(parent).join(name).is_file() {
                files.insert(parent.join(name).to_string_lossy().into_owned());
            }
        }
    }
    resolve(directory, &manifests, &files)
}

/// Installed workspace dependencies may be hoisted. Resolve only the approved ESLint CLI.
pub(crate) fn eslint_argv(command: &crate::config::Command, root: &Path) -> Vec<String> {
    let mut argv = command.command.clone();
    if matches!(
        command.preparation,
        Some(Preparation::Npm | Preparation::Bun)
    ) && command.format == "eslint"
        && argv.get(1).is_some_and(|path| path == ESLINT)
    {
        for parent in Path::new(&command.directory).ancestors() {
            let path = root.join(parent).join(ESLINT);
            if path.is_file() {
                argv[1] = path.to_string_lossy().into_owned();
                break;
            }
        }
        if command.preparation == Some(Preparation::Bun) && argv.first().is_some_and(|v| v == "bun")
        {
            // Cached dependencies must never trigger Bun's runtime auto-installer.
            argv.insert(1, "--no-install".into());
        }
    }
    argv
}
