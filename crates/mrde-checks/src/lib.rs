//! Check configuration and results. This crate deliberately executes no commands.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    Proposed,
    Approved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub command: Vec<String>,
    pub approval: Approval,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    pub checks: Vec<Check>,
}

/// Repository definitions override global definitions by id.
/// Approval must be granted locally by a future runner, not trusted from repo TOML.
pub fn merge(global: CheckConfig, repository: CheckConfig) -> Vec<Check> {
    let mut checks: BTreeMap<String, Check> = global
        .checks
        .into_iter()
        .map(|c| (c.id.clone(), c))
        .collect();
    for mut check in repository.checks {
        check.approval = Approval::Proposed;
        checks.insert(check.id.clone(), check);
    }
    checks.into_values().collect()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Skipped,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub status: Status,
    pub duration_ms: u64,
    pub artifacts: Vec<String>,
    pub details: serde_json::Value,
}

/// Missing files mean no checks. Invalid files are errors, never silently ignored.
pub fn load(
    global_dir: &std::path::Path,
    repository: &std::path::Path,
) -> anyhow::Result<Vec<Check>> {
    fn read(path: &std::path::Path) -> anyhow::Result<CheckConfig> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(CheckConfig::default())
            }
            Err(error) => Err(error.into()),
        }
    }
    Ok(merge(
        read(&global_dir.join("checks.toml"))?,
        read(&repository.join(".mrde.toml"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_repository_command_needs_approval() {
        let check = |command: &str| Check {
            id: "test".into(),
            command: vec![command.into()],
            approval: Approval::Approved,
        };
        let result = merge(
            CheckConfig {
                checks: vec![check("cargo test")],
            },
            CheckConfig {
                checks: vec![check("another command")],
            },
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].approval, Approval::Proposed);
        assert_eq!(result[0].command, ["another command"]);
    }
}
