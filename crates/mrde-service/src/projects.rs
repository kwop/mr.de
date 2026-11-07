//! Atomic, non-secret project catalog. Legacy config.toml is kept as a backup.
use anyhow::{Context, ensure};
use mrde_core::config::Config;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, sync::Mutex};

pub(crate) static CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projects {
    pub version: u32,
    pub active: String,
    pub projects: Vec<Config>,
    #[serde(default)]
    pub languages: mrde_core::language::Languages,
}
impl Projects {
    pub fn single(config: Config) -> Self {
        Self {
            languages: config.languages,
            version: 1,
            active: config.id.clone(),
            projects: vec![config],
        }
    }
    pub fn load(directory: &Path) -> anyhow::Result<Self> {
        let path = directory.join("projects.toml");
        let catalog = if path.exists() {
            toml::from_str(&std::fs::read_to_string(path)?).context("Cannot parse projects.toml")?
        } else {
            let legacy = directory.join("config.toml");
            Self::single(if legacy.exists() {
                toml::from_str(&std::fs::read_to_string(legacy)?)
                    .context("Cannot parse legacy config.toml")?
            } else {
                Config::default()
            })
        };
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.version == 1, "Unsupported projects.toml version");
        let mut ids = HashSet::new();
        for project in &self.projects {
            project.validate_project()?;
            ensure!(
                ids.insert(&project.id),
                "Duplicate project ID: {}",
                project.id
            );
        }
        self.current()?;
        Ok(())
    }
    pub fn current(&self) -> anyhow::Result<Config> {
        self.projects
            .iter()
            .find(|p| p.id == self.active)
            .cloned()
            .map(|mut config| {
                config.languages = self.languages;
                config
            })
            .context("The active project does not exist in projects.toml")
    }
    pub fn upsert(&mut self, config: Config) {
        self.active = config.id.clone();
        if let Some(old) = self.projects.iter_mut().find(|p| p.id == config.id) {
            *old = config;
        } else {
            self.projects.push(config);
        }
    }
    pub fn stage(&self, directory: &Path) -> anyhow::Result<std::path::PathBuf> {
        self.validate()?;
        std::fs::create_dir_all(directory)?;
        let staged = directory.join("projects.toml.tmp");
        std::fs::write(&staged, toml::to_string_pretty(self)?)
            .context("Cannot write project configuration")?;
        Ok(staged)
    }
    pub fn save(&self, directory: &Path) -> anyhow::Result<()> {
        let staged = self.stage(directory)?;
        std::fs::rename(staged, directory.join("projects.toml"))
            .context("Cannot activate project configuration")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_import_preserves_credentials_preferences_and_the_original_file() {
        let temp = tempfile::tempdir().unwrap();
        let text = "gitlab_url = 'https://gitlab.example.com'\nmr_roles = ['author']\nrefresh_seconds = 300\n";
        std::fs::write(temp.path().join("config.toml"), text).unwrap();
        let mut catalog = Projects::load(temp.path()).unwrap();
        let config = catalog.current().unwrap();
        assert_eq!(config.credential_key(), "https://gitlab.example.com");
        assert_eq!(config.namespace(), "https://gitlab.example.com");
        assert_eq!(config.refresh_seconds, 300);
        catalog.upsert(Config {
            id: "second".into(),
            name: "Community".into(),
            ..config.clone()
        });
        catalog.save(temp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
            text
        );
        let reloaded = Projects::load(temp.path()).unwrap();
        assert_eq!(reloaded.projects.len(), 2);
        assert_eq!(reloaded.current().unwrap().id, "second");
        assert_ne!(
            reloaded.current().unwrap().credential_key(),
            config.credential_key()
        );
    }
    #[test]
    fn invalid_catalogs_fail_without_replacing_the_saved_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let mut catalog = Projects::single(Config::default());
        catalog.save(temp.path()).unwrap();
        let before = std::fs::read(temp.path().join("projects.toml")).unwrap();
        catalog.projects.push(Config::default());
        assert!(catalog.save(temp.path()).is_err());
        catalog.projects.pop();
        catalog.active = "missing".into();
        assert!(catalog.save(temp.path()).is_err());
        assert_eq!(
            std::fs::read(temp.path().join("projects.toml")).unwrap(),
            before
        );
        assert!(
            toml::from_str::<Projects>(
                "version = 1\nactive = 'one'\n[[projects]]\nid = 'one'\ntoken = 'forbidden'"
            )
            .is_err()
        );
    }
    #[test]
    fn project_names_do_not_change_storage_but_provider_host_and_group_do() {
        let mut project = Config {
            id: "one".into(),
            gitlab_url: "https://gitlab.example.com".into(),
            ..Default::default()
        };
        let key = project.credential_key();
        let ns = project.namespace();
        project.name = "Renamed".into();
        assert_eq!(key, project.credential_key());
        assert_eq!(ns, project.namespace());
        project.group = "team/platform".into();
        assert_eq!(key, project.credential_key());
        assert_ne!(ns, project.namespace());
        project.provider = mrde_core::config::Provider::Github;
        assert_ne!(key, project.credential_key());
        project.id = "../../escape".into();
        assert!(project.validate_project().is_err());
    }
}
