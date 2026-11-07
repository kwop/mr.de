use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Your relationship to a merge request, used to select which ones appear in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MrRole {
    Reviewer,
    Assignee,
    Author,
}
impl MrRole {
    pub const ALL: [MrRole; 3] = [MrRole::Reviewer, MrRole::Assignee, MrRole::Author];
    pub fn label(self) -> &'static str {
        match self {
            MrRole::Reviewer => "Reviewer",
            MrRole::Assignee => "Assignee",
            MrRole::Author => "Author",
        }
    }
}

/// Queue ordering selected by the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueSort {
    /// Px labels, then pipeline readiness, age, and diff size.
    #[default]
    Priority,
    Newest,
    Oldest,
    Smallest,
    Risk,
}
impl QueueSort {
    pub const ALL: [QueueSort; 5] = [
        QueueSort::Priority,
        QueueSort::Newest,
        QueueSort::Oldest,
        QueueSort::Smallest,
        QueueSort::Risk,
    ];
    pub fn label(self) -> &'static str {
        match self {
            QueueSort::Risk => "Highest risk first",
            QueueSort::Priority => "Priority",
            QueueSort::Newest => "Newest first",
            QueueSort::Oldest => "Oldest first",
            QueueSort::Smallest => "Smallest diff first",
        }
    }
}

/// Provider selection is independent of connection credentials and the UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Gitlab,
    Github,
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gitlab => "GitLab",
            Self::Github => "GitHub",
        }
    }
    pub fn available(self) -> bool {
        self == Self::Gitlab
    }
}

/// Non-secret configuration only. Credentials live in the system keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `default` is reserved for the imported single-connection configuration.
    pub id: String,
    pub name: String,
    pub provider: Provider,
    #[serde(rename = "url", alias = "gitlab_url")]
    pub gitlab_url: String,
    /// GitLab group/subgroup path; empty includes all accessible groups.
    pub group: String,
    pub refresh_seconds: u64,
    pub claude_path: String,
    pub mr_roles: Vec<MrRole>,
    pub mr_sort: QueueSort,
    /// Specification folder used by the compliance check, e.g. `~/code/my-project/specs`.
    pub spec_path: String,
    /// Shell command run in the base and head worktrees to capture screenshots; it writes
    /// PNG files to `$MRDE_SCREENSHOTS_DIR`. Empty disables the visual comparison.
    pub visual_command: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "Default project".into(),
            provider: Provider::Gitlab,
            gitlab_url: String::new(),
            group: String::new(),
            refresh_seconds: 120,
            claude_path: "claude".into(),
            mr_roles: vec![MrRole::Reviewer, MrRole::Assignee],
            mr_sort: QueueSort::Priority,
            spec_path: String::new(),
            visual_command: String::new(),
        }
    }
}
impl Config {
    pub fn validate_project(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.id.is_empty()
                && self.id.len() <= 100
                && self
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Project ID must contain only letters, digits, hyphens or underscores"
        );
        anyhow::ensure!(!self.name.trim().is_empty(), "Enter a project name");
        anyhow::ensure!(
            self.group.is_empty()
                || self.group.split('/').all(|part| !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))),
            "Enter a group path, for example team/platform, without an URL"
        );
        Ok(())
    }
    /// Existing credentials stay reachable; new profiles never inherit a host's token.
    pub fn credential_key(&self) -> String {
        let url = self.gitlab_url.trim_end_matches('/');
        if self.id == "default" && self.provider == Provider::Gitlab {
            url.into()
        } else {
            format!("project:{}:{}:{url}", self.id, self.provider.label())
        }
    }
    /// Scope changes must not restore another group's queue or analysis.
    pub fn namespace(&self) -> String {
        let key = self.credential_key();
        if self.group.is_empty() {
            key
        } else {
            format!("{key}#group={}", self.group)
        }
    }

    /// The specification folder with `~` expanded, or `None` when not configured.
    pub fn spec_dir(&self) -> Option<PathBuf> {
        let path = self.spec_path.trim();
        if path.is_empty() {
            return None;
        }
        Some(match path.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(rest),
            None => PathBuf::from(path),
        })
    }
}

pub fn config_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".config/mrde"))
}

pub fn cache_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join("Library/Caches/mrde"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_config_without_queue_options_keeps_reviewer_and_assignee_by_priority() {
        let config: Config = toml::from_str("gitlab_url = \"https://gitlab.example.com\"").unwrap();
        assert_eq!(config.mr_roles, [MrRole::Reviewer, MrRole::Assignee]);
        assert_eq!(config.mr_sort, QueueSort::Priority);
    }
    #[test]
    fn queue_options_round_trip_through_toml() {
        let config = Config {
            mr_roles: vec![MrRole::Author],
            mr_sort: QueueSort::Newest,
            ..Default::default()
        };
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("mr_roles = [\"author\"]"));
        assert!(text.contains("mr_sort = \"newest\""));
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.mr_roles, [MrRole::Author]);
        assert_eq!(parsed.mr_sort, QueueSort::Newest);
    }
}
