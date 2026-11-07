use anyhow::{Context, Result, ensure};
use mrde_core::risk::{Axis, VerdictThresholds, validate_curve};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RiskConfig {
    pub normalization: BTreeMap<String, Vec<[f64; 2]>>,
    pub sensitive: Vec<SensitiveGlob>,
    pub debt: Vec<DebtRule>,
    pub coverage_path: String,
    pub complexity: Command,
    pub lint: Vec<Command>,
    pub autodetect_lint: bool,
    #[serde(skip)]
    pub detected_lint: Vec<Command>,
    #[serde(skip)]
    pub approvals: HashSet<String>,
    pub baseline_count: usize,
    pub timeout_seconds: u64,
    pub verdict: VerdictThresholds,
    pub exclude: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitiveGlob {
    pub glob: String,
    pub weight: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebtRule {
    pub id: String,
    pub regex: String,
    pub weight: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Command {
    /// Repository-relative package directory; empty means the repository root.
    pub directory: String,
    /// Fixed dependency preparation recipe, covered by the command approval.
    pub preparation: Option<Preparation>,
    pub id: String,
    pub command: Vec<String>,
    /// `json` (standard diagnostics), `eslint`, `clippy`, or `regex`.
    pub format: String,
    pub warning_regex: String,
    #[serde(skip)]
    pub approved: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Preparation {
    Cargo,
    Npm,
    Dart,
    Flutter,
    Go,
    Ruff,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            directory: String::new(),
            preparation: None,
            id: "complexity".into(),
            command: vec!["lizard".into(), "--xml".into()],
            format: "lizard".into(),
            warning_regex: String::new(),
            approved: false,
        }
    }
}
impl Command {
    pub fn key(&self) -> String {
        let mut copy = self.clone();
        copy.approved = false;
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&copy).expect("serializable command"))
        )
    }
}
impl Default for RiskConfig {
    fn default() -> Self {
        let normalization = [
            (
                Axis::DiffSize,
                vec![[0., 0.], [50., 2.], [300., 5.], [1000., 8.], [3000., 10.]],
            ),
            (
                Axis::Files,
                vec![[0., 0.], [3., 2.], [10., 5.], [30., 8.], [80., 10.]],
            ),
            (
                Axis::Complexity,
                vec![[0., 0.], [5., 2.], [20., 5.], [50., 8.], [100., 10.]],
            ),
            (
                Axis::TestGap,
                vec![[0., 0.], [25., 2.], [100., 5.], [300., 8.], [800., 10.]],
            ),
            (
                Axis::SensitivePaths,
                vec![[0., 0.], [0.1, 2.], [0.3, 5.], [0.6, 8.], [1., 10.]],
            ),
            (
                Axis::Coupling,
                vec![[0., 0.], [2., 2.], [5., 5.], [10., 8.], [20., 10.]],
            ),
            (
                Axis::Lint,
                vec![[0., 0.], [2., 2.], [5., 5.], [15., 8.], [30., 10.]],
            ),
            (
                Axis::Debt,
                vec![[0., 0.], [2., 2.], [5., 5.], [15., 8.], [30., 10.]],
            ),
        ]
        .into_iter()
        .map(|(axis, curve)| (axis.id().into(), curve))
        .collect();
        Self {
            normalization,
            sensitive: vec![
                SensitiveGlob {
                    glob: "**/auth/**".into(),
                    weight: 1.,
                },
                SensitiveGlob {
                    glob: "**/migrations/**".into(),
                    weight: 1.,
                },
            ],
            debt: vec![
                DebtRule {
                    id: "markers".into(),
                    regex: r"\b(?:TODO|FIXME|HACK)\b".into(),
                    weight: 1.,
                },
                DebtRule {
                    id: "unwrap".into(),
                    regex: r"\.unwrap\s*\(".into(),
                    weight: 1.,
                },
                DebtRule {
                    id: "any".into(),
                    // TypeScript's `any` type, not the English word or `Iterator::any`.
                    regex: r"(?::\s*any\b|\bas\s+any\b|<any>)".into(),
                    weight: 0.5,
                },
            ],
            coverage_path: String::new(),
            complexity: Command {
                approved: true,
                ..Default::default()
            },
            lint: vec![],
            autodetect_lint: true,
            detected_lint: vec![],
            approvals: HashSet::new(),
            baseline_count: 30,
            timeout_seconds: 60,
            verdict: VerdictThresholds::default(),
            exclude: vec![
                "**/vendor/**".into(),
                "**/node_modules/**".into(),
                "**/target/**".into(),
                "**/*.lock".into(),
            ],
        }
    }
}
fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                if let Some(old) = base.get_mut(&key) {
                    merge(old, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}
fn overlay(value: &mut toml::Value, path: &Path) -> Result<()> {
    match std::fs::read_to_string(path) {
        Ok(text) => merge(
            value,
            toml::from_str(&text)
                .with_context(|| format!("Invalid risk configuration: {}", path.display()))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    };
    Ok(())
}
impl RiskConfig {
    pub fn load(
        global: &Path,
        project: &str,
        repository: &Path,
        approvals: &HashSet<String>,
    ) -> Result<Self> {
        ensure!(
            !project.is_empty()
                && project
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "Invalid project ID"
        );
        let mut value = toml::Value::try_from(Self::default())?;
        overlay(&mut value, &global.join("risk.toml"))?;
        overlay(
            &mut value,
            &global.join("projects").join(project).join("risk.toml"),
        )?;
        let local: Self = value.clone().try_into()?;
        let trusted: HashSet<String> = std::iter::once(&local.complexity)
            .chain(&local.lint)
            .map(Command::key)
            .collect();
        overlay(&mut value, &repository.join("risk.toml"))?;
        let mut config: Self = value.try_into()?;
        for command in std::iter::once(&mut config.complexity).chain(&mut config.lint) {
            command.approved =
                trusted.contains(&command.key()) || approvals.contains(&command.key());
        }
        config.approvals = approvals.clone();
        config.validate()?;
        Ok(config)
    }
    pub fn lint_commands(&self) -> &[Command] {
        if self.lint.is_empty() {
            &self.detected_lint
        } else {
            &self.lint
        }
    }
    pub fn commands_mut(&mut self) -> impl Iterator<Item = &mut Command> {
        let lint = if self.lint.is_empty() {
            &mut self.detected_lint
        } else {
            &mut self.lint
        };
        std::iter::once(&mut self.complexity).chain(lint)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=200).contains(&self.baseline_count),
            "baseline_count must be between 1 and 200"
        );
        ensure!(
            (1..=3600).contains(&self.timeout_seconds),
            "timeout_seconds must be between 1 and 3600"
        );
        self.verdict.validate()?;
        ensure!(
            matches!(self.complexity.format.as_str(), "lizard"),
            "Complexity format must be lizard"
        );
        for command in self.lint_commands() {
            ensure!(
                matches!(
                    command.format.as_str(),
                    "json" | "eslint" | "clippy" | "regex" | "dart" | "go_vet" | "ruff"
                ),
                "Unsupported lint format"
            );
        }
        let coverage = Path::new(&self.coverage_path);
        ensure!(
            !coverage.is_absolute()
                && !coverage
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir)),
            "coverage_path must stay inside the repository"
        );
        ensure!(
            self.normalization.len() == 8,
            "Define only the eight known normalization axes"
        );
        for axis in Axis::ALL {
            validate_curve(
                self.normalization
                    .get(axis.id())
                    .context("Missing normalization axis")?,
            )?;
        }
        for g in &self.sensitive {
            ensure!(
                g.weight.is_finite() && g.weight >= 0.,
                "Invalid sensitive-path weight"
            );
            globset::Glob::new(&g.glob)?;
        }
        for g in &self.exclude {
            globset::Glob::new(g)?;
        }
        for r in &self.debt {
            ensure!(
                r.weight.is_finite() && r.weight >= 0.,
                "Invalid debt weight"
            );
            regex::Regex::new(&r.regex)?;
        }
        let mut ids = HashSet::new();
        for c in std::iter::once(&self.complexity).chain(self.lint_commands()) {
            let directory = Path::new(&c.directory);
            ensure!(
                !directory.is_absolute()
                    && directory
                        .components()
                        .all(|p| !matches!(p, std::path::Component::ParentDir)),
                "Check directory must stay inside the repository"
            );
            ensure!(
                !c.id.is_empty() && ids.insert(&c.id),
                "Check IDs must be nonempty and unique"
            );
            ensure!(
                c.command.first().is_some_and(|s| !s.trim().is_empty()),
                "A check command cannot be empty"
            );
            ensure!(
                c.command.iter().all(|s| !s.contains('\0')),
                "Invalid command argument"
            );
            if c.format == "regex" {
                ensure!(
                    !c.warning_regex.is_empty(),
                    "A regex warning parser needs warning_regex"
                );
                regex::Regex::new(&c.warning_regex)?;
            }
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> String {
        format!(
            // Bumped when scorers change meaning, so cached profiles are measured again.
            "risk-v3:{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("serializable config"))
        )
    }
    pub fn coverage_file(&self, root: &Path) -> Option<PathBuf> {
        (!self.coverage_path.is_empty()).then(|| root.join(&self.coverage_path))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_and_repository_overrides_preserve_unspecified_defaults_and_require_command_approval()
    {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir_all(root.join("projects/team")).unwrap();
        std::fs::write(root.join("risk.toml"), "baseline_count = 20").unwrap();
        std::fs::write(
            root.join("projects/team/risk.toml"),
            "baseline_count = 12\n[normalization]\nfiles = [[0,0],[10,10]]",
        )
        .unwrap();
        std::fs::write(
            repo.join("risk.toml"),
            "[[lint]]\nid = 'lint'\ncommand = ['custom-lint']\nformat = 'json'",
        )
        .unwrap();
        let config = RiskConfig::load(root, "team", &repo, &HashSet::new()).unwrap();
        assert_eq!(config.baseline_count, 12);
        assert_eq!(config.normalization["files"][1], [10., 10.]);
        assert!(!config.lint[0].approved);
        assert!(config.complexity.approved);
        let approved = HashSet::from([config.lint[0].key()]);
        assert!(
            RiskConfig::load(root, "team", &repo, &approved)
                .unwrap()
                .lint[0]
                .approved
        );
        assert!(RiskConfig::load(root, "../escape", &repo, &approved).is_err());
    }
    #[test]
    fn invalid_values_and_unknown_fields_are_errors() {
        let mut config = RiskConfig::default();
        config.validate().unwrap();
        config.timeout_seconds = 0;
        assert!(config.validate().is_err());
        assert!(toml::from_str::<RiskConfig>("basline_count = 30").is_err());
    }
}
