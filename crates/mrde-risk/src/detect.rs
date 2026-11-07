//! Read committed manifests for the code changed by this MR. Never execute CI scripts.
use crate::config::{Command, Preparation};
use anyhow::Result;
use std::{collections::BTreeSet, path::Path};

pub async fn lint_commands(
    worktree: &Path,
    head: &str,
    changed: &[String],
) -> Result<Vec<Command>> {
    let files = mrde_git::list_files(worktree, head).await?;
    let tracked: BTreeSet<_> = files.iter().map(String::as_str).collect();
    let mut roots = BTreeSet::new();
    for file in changed {
        let extension = Path::new(file)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        let manifest = match extension {
            "rs" => "Cargo.toml",
            "dart" => "pubspec.yaml",
            "js" | "jsx" | "ts" | "tsx" | "vue" | "mjs" | "cjs" => "package.json",
            "go" => "go.mod",
            "py" | "pyi" => "pyproject.toml",
            _ => continue,
        };
        let parents: Vec<_> = Path::new(file)
            .parent()
            .into_iter()
            .flat_map(Path::ancestors)
            .collect();
        let found = parents
            .iter()
            .find(|p| tracked.contains(p.join(manifest).to_string_lossy().as_ref()));
        if let Some(root) = found {
            let mut root = *root;
            if manifest == "Cargo.toml" {
                // Only a workspace manifest owns nested Rust packages.
                for parent in parents.iter().filter(|p| root.starts_with(p)) {
                    let candidate = parent.join(manifest).to_string_lossy().into_owned();
                    if !tracked.contains(candidate.as_str()) {
                        continue;
                    }
                    if let Some(text) = mrde_git::show_file(worktree, head, &candidate).await? {
                        anyhow::ensure!(
                            text.len() <= 1024 * 1024,
                            "Manifest exceeds the 1 MiB detection limit"
                        );
                        let value: toml::Value = toml::from_str(&text)?;
                        if value.get("workspace").is_some() {
                            root = parent;
                            break;
                        }
                    }
                }
            }
            if manifest == "package.json" {
                // A workspace can declare ESLint once for all its packages.
                for parent in &parents {
                    if tracked.contains(parent.join(manifest).to_string_lossy().as_ref()) {
                        roots.insert((parent.to_string_lossy().into_owned(), manifest));
                    }
                }
            } else {
                roots.insert((root.to_string_lossy().into_owned(), manifest));
            }
        } else if extension == "py" || extension == "pyi" {
            roots.insert((String::new(), "pyproject.toml"));
        }
    }
    anyhow::ensure!(
        roots.len() <= 32,
        "More than 32 changed packages; configure explicit lint commands in risk.toml"
    );
    let mut commands = Vec::new();
    for (directory, manifest) in roots {
        let path = Path::new(&directory)
            .join(manifest)
            .to_string_lossy()
            .into_owned();
        let text = mrde_git::show_file(worktree, head, &path)
            .await?
            .unwrap_or_default();
        anyhow::ensure!(
            text.len() <= 1024 * 1024,
            "Manifest exceeds the 1 MiB detection limit"
        );
        let spec = match manifest {
            "Cargo.toml" => Some((
                "clippy",
                vec![
                    "cargo",
                    "clippy",
                    "--offline",
                    "--locked",
                    "--message-format=json",
                ],
                "clippy",
                Preparation::Cargo,
            )),
            "pubspec.yaml" => {
                let yaml: serde_yaml::Value = serde_yaml::from_str(&text)?;
                let flutter = yaml["dependencies"]["flutter"]["sdk"].as_str() == Some("flutter");
                Some((
                    "dart-analyze",
                    vec!["dart", "analyze", "--format=machine"],
                    "dart",
                    if flutter {
                        Preparation::Flutter
                    } else {
                        Preparation::Dart
                    },
                ))
            }
            "package.json" => {
                let package: serde_json::Value = serde_json::from_str(&text)?;
                let eslint = ["dependencies", "devDependencies"]
                    .iter()
                    .any(|key| package[key].get("eslint").is_some());
                // Never infer an npm script: it may fix files, run tests, or install packages.
                eslint.then_some((
                    "eslint",
                    vec![
                        "node",
                        "node_modules/eslint/bin/eslint.js",
                        "--format",
                        "json",
                        ".",
                    ],
                    "eslint",
                    Preparation::Npm,
                ))
            }
            "go.mod" => Some((
                "go-vet",
                vec!["go", "vet", "-json", "./..."],
                "go_vet",
                Preparation::Go,
            )),
            "pyproject.toml" => Some((
                "ruff",
                vec!["ruff", "check", "--output-format=json", "."],
                "ruff",
                Preparation::Ruff,
            )),
            _ => None,
        };
        if let Some((id, argv, format, preparation)) = spec {
            if preparation == Preparation::Npm
                && commands.iter().any(|c: &Command| {
                    c.preparation == Some(Preparation::Npm)
                        && Path::new(&directory).starts_with(&c.directory)
                })
            {
                continue;
            }
            commands.push(Command {
                id: if directory.is_empty() {
                    id.into()
                } else {
                    format!("{id}@{directory}")
                },
                command: argv.into_iter().map(str::to_string).collect(),
                format: format.into(),
                directory,
                preparation: Some(preparation),
                approved: false,
                ..Command::default()
            });
        }
    }
    Ok(commands)
}
