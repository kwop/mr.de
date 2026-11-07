//! Direct dependency changes from manifests, and transitive counts from lockfiles.
use mrde_core::insights::{DependencyChange, DependencyChangeKind};
use std::collections::BTreeMap;

pub fn is_manifest(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "go.mod" | "package.json" | "Cargo.toml" | "requirements.txt"
    )
}

pub fn is_lockfile(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "go.sum"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "Cargo.lock"
            | "poetry.lock"
    )
}

/// name → (version, dev)
type Deps = BTreeMap<String, (String, bool)>;

fn go_mod(text: &str) -> Deps {
    let mut deps = Deps::new();
    let mut block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if line.starts_with("require (") {
            block = true;
            continue;
        }
        if block && line == ")" {
            block = false;
            continue;
        }
        let entry = if block {
            Some(line)
        } else {
            line.strip_prefix("require ")
        };
        if let Some(entry) = entry {
            let mut parts = entry.split_whitespace();
            if let (Some(name), Some(version)) = (parts.next(), parts.next()) {
                deps.insert(name.into(), (version.into(), false));
            }
        }
    }
    deps
}

fn package_json(text: &str) -> Deps {
    let mut deps = Deps::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return deps;
    };
    for (section, dev) in [
        ("dependencies", false),
        ("peerDependencies", false),
        ("optionalDependencies", false),
        ("devDependencies", true),
    ] {
        if let Some(map) = value[section].as_object() {
            for (name, version) in map {
                deps.insert(
                    name.clone(),
                    (version.as_str().unwrap_or_default().into(), dev),
                );
            }
        }
    }
    deps
}

fn cargo_toml(text: &str) -> Deps {
    let mut deps = Deps::new();
    let Ok(value) = text.parse::<toml::Table>() else {
        return deps;
    };
    for (section, dev) in [
        ("dependencies", false),
        ("build-dependencies", false),
        ("dev-dependencies", true),
    ] {
        let tables = [
            value.get(section),
            value.get("workspace").and_then(|w| w.get(section)),
        ];
        for table in tables.into_iter().flatten().filter_map(|t| t.as_table()) {
            for (name, spec) in table {
                let version = match spec {
                    toml::Value::String(v) => v.clone(),
                    toml::Value::Table(t) => t
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or_else(|| t.get("path").map(|_| "path".into()))
                        .or_else(|| t.get("workspace").map(|_| "workspace".into()))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                deps.insert(name.clone(), (version, dev));
            }
        }
    }
    deps
}

fn requirements(text: &str) -> Deps {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty() && !l.starts_with('-'))
        .map(|l| {
            let split = l
                .find(['=', '>', '<', '~', '!', ';', '['])
                .unwrap_or(l.len());
            (
                l[..split].trim().to_lowercase(),
                (l[split..].trim_start_matches(['=', '~']).to_string(), false),
            )
        })
        .collect()
}

fn parse(path: &str, text: &str) -> Deps {
    match path.rsplit('/').next().unwrap_or(path) {
        "go.mod" => go_mod(text),
        "package.json" => package_json(text),
        "Cargo.toml" => cargo_toml(text),
        "requirements.txt" => requirements(text),
        _ => Deps::new(),
    }
}

/// Leading numeric components, ignoring range operators and a `v` prefix.
fn numbers(version: &str) -> Vec<u64> {
    version
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

/// A major change, or a minor one below 1.0 where semver treats it as breaking.
fn is_major(before: &str, after: &str) -> bool {
    let (a, b) = (numbers(before), numbers(after));
    match (a.first(), b.first()) {
        (Some(0), Some(0)) => a.get(1) != b.get(1),
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}

pub fn compare(path: &str, old: Option<&str>, new: Option<&str>) -> Vec<DependencyChange> {
    let before = old.map(|t| parse(path, t)).unwrap_or_default();
    let after = new.map(|t| parse(path, t)).unwrap_or_default();
    let mut out = Vec::new();
    for (name, (version, dev)) in &after {
        match before.get(name) {
            None => out.push(DependencyChange {
                manifest: path.into(),
                name: name.clone(),
                before: None,
                after: Some(version.clone()),
                kind: DependencyChangeKind::Added,
                major: false,
                dev: *dev,
            }),
            Some((old_version, _)) if old_version != version => {
                let (a, b) = (numbers(old_version), numbers(version));
                out.push(DependencyChange {
                    manifest: path.into(),
                    name: name.clone(),
                    before: Some(old_version.clone()),
                    after: Some(version.clone()),
                    kind: if b < a {
                        DependencyChangeKind::Downgraded
                    } else {
                        DependencyChangeKind::Upgraded
                    },
                    major: is_major(old_version, version),
                    dev: *dev,
                });
            }
            _ => {}
        }
    }
    for (name, (version, dev)) in &before {
        if !after.contains_key(name) {
            out.push(DependencyChange {
                manifest: path.into(),
                name: name.clone(),
                before: Some(version.clone()),
                after: None,
                kind: DependencyChangeKind::Removed,
                major: false,
                dev: *dev,
            });
        }
    }
    out
}

/// Package entries added and removed in a lockfile diff (resolved transitive versions).
pub fn lockfile_counts(path: &str, added: &[&str], removed: &[&str]) -> (usize, usize) {
    let name = path.rsplit('/').next().unwrap_or(path);
    let entry = |line: &str| -> bool {
        let t = line.trim_start();
        match name {
            "go.sum" => !line.trim().is_empty() && !t.contains("/go.mod "),
            "package-lock.json" => t.starts_with("\"node_modules/") && t.ends_with("{"),
            "pnpm-lock.yaml" => {
                line.starts_with("  ")
                    && !line.starts_with("   ")
                    && t.ends_with(':')
                    && t.contains('@')
            }
            "yarn.lock" => !line.starts_with(' ') && line.ends_with(':') && line.contains('@'),
            "Cargo.lock" | "poetry.lock" => t.starts_with("name = "),
            _ => false,
        }
    };
    (
        added.iter().filter(|l| entry(l)).count(),
        removed.iter().filter(|l| entry(l)).count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_manifests_and_flags_major_upgrades() {
        let old = "module example\n\nrequire (\n\tgithub.com/jackc/pgx/v5 v5.5.4\n\tgithub.com/old/lib v1.0.0 // indirect\n)\n";
        let new = "module example\n\nrequire (\n\tgithub.com/jackc/pgx/v5 v5.7.1\n\tgithub.com/teambition/rrule-go v1.8.2\n)\n";
        let changes = compare("backend/go.mod", Some(old), Some(new));
        let summary: Vec<_> = changes
            .iter()
            .map(|c| (c.name.as_str(), c.kind, c.major))
            .collect();
        assert!(summary.contains(&(
            "github.com/jackc/pgx/v5",
            DependencyChangeKind::Upgraded,
            false
        )));
        assert!(summary.contains(&(
            "github.com/teambition/rrule-go",
            DependencyChangeKind::Added,
            false
        )));
        assert!(summary.contains(&("github.com/old/lib", DependencyChangeKind::Removed, false)));
        let pkg = compare(
            "frontend/package.json",
            Some(
                r#"{"dependencies":{"date-fns":"^2.30.0"},"devDependencies":{"vitest":"0.34.0"}}"#,
            ),
            Some(
                r#"{"dependencies":{"date-fns":"^4.1.0","rrule":"2.8.1"},"devDependencies":{"vitest":"0.35.0"}}"#,
            ),
        );
        let date = pkg.iter().find(|c| c.name == "date-fns").unwrap();
        assert!(date.major && date.kind == DependencyChangeKind::Upgraded);
        let vitest = pkg.iter().find(|c| c.name == "vitest").unwrap();
        assert!(vitest.major && vitest.dev, "0.x minor bumps are breaking");
        let cargo = compare(
            "Cargo.toml",
            Some("[dependencies]\nserde = \"1\"\n"),
            Some("[dependencies]\nserde = { version = \"1\" }\nregex = \"1.10\"\n"),
        );
        assert_eq!(cargo.len(), 1);
        assert_eq!(
            lockfile_counts(
                "go.sum",
                &["github.com/a v1 h1:x", "github.com/a v1/go.mod h1:y"],
                &[]
            ),
            (1, 0)
        );
    }
}
