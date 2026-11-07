//! Connection settings: testing performs no writes; saving performs no HTTP requests.
use crate::projects::{CONFIG_LOCK, Projects};
use anyhow::{Context, bail};
use async_trait::async_trait;
use mrde_core::config::{Config, MrRole, QueueSort, config_dir};
use mrde_forge::{Forge, gitlab::GitLab};
use std::{path::PathBuf, sync::Arc};

pub type Progress = Arc<dyn Fn(String) + Send + Sync>;
#[derive(Clone, Default)]
pub struct Draft {
    pub config: Config,
    pub token: String,
    pub cf_id: String,
    pub cf_secret: String,
    pub clear_cloudflare: bool,
}
impl Draft {
    fn normalize(mut self) -> anyhow::Result<Self> {
        self.config.gitlab_url = self
            .config
            .gitlab_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        self.config.name = self.config.name.trim().to_string();
        self.config.group = self.config.group.trim().trim_matches('/').to_string();
        self.config.validate_project()?;
        anyhow::ensure!(
            self.config.refresh_seconds >= 30,
            "Refresh interval must be at least 30 seconds"
        );
        anyhow::ensure!(
            !self.config.claude_path.is_empty(),
            "Enter the Claude executable"
        );
        // Validate before accessing Keychain, including URLs containing credentials.
        GitLab::new(&self.config.gitlab_url, "validation", &[])?;
        if self.config.mr_roles.is_empty() {
            bail!("Select at least one role to choose which merge requests to show.");
        }
        self.config.spec_path = self.config.spec_path.trim().to_string();
        if let Some(spec) = self.config.spec_dir()
            && !spec.is_dir()
        {
            bail!("Specification folder not found: {}", spec.display());
        }
        Ok(self)
    }
    fn client(&self) -> anyhow::Result<GitLab> {
        if self.token.is_empty() {
            bail!("Enter a GitLab token, or save one before leaving this field blank.");
        }
        if self.cf_id.is_empty() != self.cf_secret.is_empty() {
            bail!("Enter both Cloudflare service-token fields, or remove both.");
        }
        let headers = if self.cf_id.is_empty() {
            vec![]
        } else {
            vec![
                ("CF-Access-Client-Id".into(), self.cf_id.clone()),
                ("CF-Access-Client-Secret".into(), self.cf_secret.clone()),
            ]
        };
        Ok(GitLab::new(&self.config.gitlab_url, &self.token, &headers)?
            .with_group(&self.config.group))
    }
}

pub trait Secrets: Send + Sync {
    fn read(&self, instance: &str, name: &str) -> anyhow::Result<String>;
    fn write(&self, instance: &str, name: &str, value: &str) -> anyhow::Result<()>;
    fn write_batch(&self, instance: &str, updates: &[(&str, &str)]) -> anyhow::Result<()> {
        let old = updates
            .iter()
            .map(|(name, _)| self.read(instance, name))
            .collect::<anyhow::Result<Vec<_>>>()
            .context("Cannot read existing Keychain values; nothing was changed")?;
        for (index, (name, value)) in updates.iter().enumerate() {
            if let Err(error) = self.write(instance, name, value) {
                for ((name, _), value) in updates[..index].iter().zip(&old) {
                    let _ = self.write(instance, name, value);
                }
                return Err(error);
            }
        }
        Ok(())
    }
}
pub struct Keychain;
impl Secrets for Keychain {
    fn read(&self, instance: &str, name: &str) -> anyhow::Result<String> {
        mrde_store::read_secret(instance, name)
    }
    fn write(&self, instance: &str, name: &str, value: &str) -> anyhow::Result<()> {
        mrde_store::write_secret(instance, name, value)
    }
    fn write_batch(&self, instance: &str, updates: &[(&str, &str)]) -> anyhow::Result<()> {
        mrde_store::write_secrets(instance, updates)
    }
}
#[async_trait]
pub trait ConnectionSettings: Send + Sync {
    async fn test(&self, draft: Draft, progress: Progress) -> anyhow::Result<String>;
    async fn save(&self, draft: Draft, progress: Progress) -> anyhow::Result<Config>;
    async fn activate_project(&self, _id: String) -> anyhow::Result<Config> {
        bail!("Project switching is unavailable in this mode")
    }
    /// Updates only non-secret queue preferences, without resolving credentials.
    async fn save_queue_preferences(
        &self,
        project: Config,
        roles: Vec<MrRole>,
        sort: QueueSort,
    ) -> anyhow::Result<Config>;
}
pub struct SettingsService {
    secrets: Arc<dyn Secrets>,
    directory: PathBuf,
}
impl SettingsService {
    pub fn system() -> anyhow::Result<Self> {
        Ok(Self {
            secrets: Arc::new(Keychain),
            directory: config_dir()?,
        })
    }
    async fn resolve(&self, draft: Draft, progress: &Progress) -> anyhow::Result<Draft> {
        progress("Validating connection settings".into());
        let mut draft = draft.normalize()?;
        progress("Resolving credentials (macOS may request Keychain access)".into());
        let secrets = self.secrets.clone();
        draft = tokio::task::spawn_blocking(move || -> anyhow::Result<Draft> {
            let key = draft.config.credential_key();
            let url = &key;
            if draft.token.is_empty() {
                draft.token = secrets.read(url, "gitlab-token")?;
            }
            if draft.clear_cloudflare {
                draft.cf_id.clear();
                draft.cf_secret.clear();
            } else {
                if draft.cf_id.is_empty() {
                    draft.cf_id = secrets.read(url, "cf-id")?;
                }
                if draft.cf_secret.is_empty() {
                    draft.cf_secret = secrets.read(url, "cf-secret")?;
                }
            }
            Ok(draft)
        })
        .await??;
        if draft.config.provider.available() {
            draft.client()?;
        }
        Ok(draft)
    }
}
#[async_trait]
impl ConnectionSettings for SettingsService {
    async fn activate_project(&self, id: String) -> anyhow::Result<Config> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut catalog = Projects::load(&directory)?;
            catalog.active = id;
            let config = catalog.current()?;
            catalog.save(&directory)?;
            Ok(config)
        })
        .await?
    }

    async fn save_queue_preferences(
        &self,
        project: Config,
        roles: Vec<MrRole>,
        sort: QueueSort,
    ) -> anyhow::Result<Config> {
        if roles.is_empty() {
            bail!("Select at least one role to choose which merge requests to show.");
        }
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut catalog = Projects::load(&directory)?;
            let config = catalog
                .projects
                .iter_mut()
                .find(|p| p.id == project.id)
                .context("Save this project before changing queue preferences")?;
            config.mr_roles = roles;
            config.mr_sort = sort;
            let config = config.clone();
            catalog.save(&directory)?;
            Ok(config)
        })
        .await?
    }
    async fn test(&self, draft: Draft, progress: Progress) -> anyhow::Result<String> {
        if !draft.config.provider.available() {
            bail!("GitHub profiles can be saved, but the GitHub connector is not available yet.");
        }
        let draft = self.resolve(draft, &progress).await?;
        progress("GET /api/v4/user — connecting to GitLab (30s timeout)".into());
        let client = draft.client()?;
        let user = client.test_connection().await?;
        client.test_group().await?;
        progress("GitLab authentication succeeded; nothing has been saved".into());
        Ok(user)
    }
    async fn save(&self, draft: Draft, progress: Progress) -> anyhow::Result<Config> {
        let draft = self.resolve(draft, &progress).await?;
        let secrets = self.secrets.clone();
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            progress("Preparing configuration file".into());
            let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut catalog = Projects::load(&directory)?;
            catalog.upsert(draft.config.clone());
            let staged = catalog.stage(&directory)?;
            progress("Saving credentials to macOS Keychain".into());
            let key = draft.config.credential_key();
            let url = &key;
            let updates = [
                ("gitlab-token", draft.token.as_str()),
                ("cf-id", draft.cf_id.as_str()),
                ("cf-secret", draft.cf_secret.as_str()),
            ];
            if let Err(error) = secrets.write_batch(url, &updates) {
                let _ = std::fs::remove_file(&staged);
                return Err(error).context("Keychain save failed; configuration was not activated");
            }
            progress("Activating saved configuration".into());
            std::fs::rename(staged, directory.join("projects.toml"))
                .context("Cannot activate configuration file")?;
            progress("Settings saved successfully".into());
            Ok(draft.config)
        })
        .await?
    }
}

/// Apply to diagnostics before retaining them in the UI. Never log HTTP headers or bodies.
pub fn redact(message: &str, secrets: &[String]) -> String {
    let mut result = message.to_string();
    for secret in secrets.iter().filter(|v| !v.is_empty()) {
        result = result.replace(secret, "[REDACTED]");
    }
    result
}

/// Explicit preview mode for native visual inspection, never used in normal operation.
pub struct PreviewSettings;
#[async_trait]
impl ConnectionSettings for PreviewSettings {
    async fn activate_project(&self, id: String) -> anyhow::Result<Config> {
        let github = id == "community";
        Ok(Config {
            id,
            name: if github { "Community" } else { "Platform" }.into(),
            provider: if github {
                mrde_core::config::Provider::Github
            } else {
                mrde_core::config::Provider::Gitlab
            },
            gitlab_url: if github {
                "https://github.com"
            } else {
                crate::demo::INSTANCE
            }
            .into(),
            ..Default::default()
        })
    }

    async fn save_queue_preferences(
        &self,
        _: Config,
        _: Vec<MrRole>,
        _: QueueSort,
    ) -> anyhow::Result<Config> {
        bail!("Preview mode does not save preferences.")
    }
    async fn test(&self, _: Draft, progress: Progress) -> anyhow::Result<String> {
        progress("PREVIEW  Simulating a connection test; no request sent".into());
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        Ok("preview-user".into())
    }
    async fn save(&self, _: Draft, _: Progress) -> anyhow::Result<Config> {
        bail!(
            "Preview mode does not save credentials. Relaunch without --ui-preview to configure GitLab."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    #[derive(Default)]
    struct MemorySecrets {
        values: Mutex<std::collections::HashMap<String, String>>,
        writes: AtomicUsize,
        fail: bool,
        fail_on: Option<&'static str>,
    }
    impl Secrets for MemorySecrets {
        fn read(&self, _: &str, name: &str) -> anyhow::Result<String> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .unwrap_or_default())
        }
        fn write(&self, _: &str, name: &str, value: &str) -> anyhow::Result<()> {
            if self.fail || self.fail_on == Some(name) {
                bail!("Keychain access denied");
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.values
                .lock()
                .unwrap()
                .insert(name.into(), value.into());
            Ok(())
        }
    }
    fn draft(url: &str) -> Draft {
        Draft {
            config: Config {
                gitlab_url: url.into(),
                ..Default::default()
            },
            token: "synthetic-token".into(),
            clear_cloudflare: true,
            ..Default::default()
        }
    }
    #[tokio::test]
    async fn test_authenticates_but_never_saves_and_failed_test_preserves_existing_config() {
        let server = MockServer::start().await;
        let temp = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let service = SettingsService {
            secrets: secrets.clone(),
            directory: temp.path().to_path_buf(),
        };
        Mock::given(path("/api/v4/user"))
            .and(header("private-token", "synthetic-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id":1,"username":"reviewer"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            service
                .test(draft(&server.uri()), Arc::new(|_| {}))
                .await
                .unwrap(),
            "reviewer"
        );
        assert_eq!(secrets.writes.load(Ordering::SeqCst), 0);
        assert!(!temp.path().join("config.toml").exists());
        server.reset().await;
        Mock::given(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        std::fs::write(temp.path().join("config.toml"), "existing configuration").unwrap();
        assert!(
            service
                .test(draft(&server.uri()), Arc::new(|_| {}))
                .await
                .unwrap_err()
                .to_string()
                .contains("401")
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
            "existing configuration"
        );
        assert_eq!(secrets.writes.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn save_persists_without_http_and_reports_keychain_failure() {
        let server = MockServer::start().await;
        let temp = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let service = SettingsService {
            secrets: secrets.clone(),
            directory: temp.path().to_path_buf(),
        };
        service
            .save(draft(&server.uri()), Arc::new(|_| {}))
            .await
            .unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(secrets.read("", "gitlab-token").unwrap(), "synthetic-token");
        let config = std::fs::read_to_string(temp.path().join("projects.toml")).unwrap();
        assert!(!config.contains("synthetic-token"));
        let failed = SettingsService {
            secrets: Arc::new(MemorySecrets {
                fail: true,
                ..Default::default()
            }),
            directory: temp.path().to_path_buf(),
        };
        assert!(
            failed
                .save(draft("https://other.example.com"), Arc::new(|_| {}))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("projects.toml")).unwrap(),
            config
        );
    }
    #[tokio::test]
    async fn save_persists_queue_roles_and_sort_and_rejects_empty_roles() {
        let temp = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let service = SettingsService {
            secrets: secrets.clone(),
            directory: temp.path().to_path_buf(),
        };
        let mut chosen = draft("https://gitlab.example.com");
        chosen.config.mr_roles = vec![MrRole::Author, MrRole::Reviewer];
        chosen.config.mr_sort = QueueSort::Oldest;
        let saved = service.save(chosen, Arc::new(|_| {})).await.unwrap();
        assert_eq!(saved.mr_roles, [MrRole::Author, MrRole::Reviewer]);
        let reloaded = Projects::load(temp.path()).unwrap().current().unwrap();
        assert_eq!(reloaded.mr_roles, [MrRole::Author, MrRole::Reviewer]);
        assert_eq!(reloaded.mr_sort, QueueSort::Oldest);
        let mut empty = draft("https://other.example.com");
        empty.config.mr_roles.clear();
        let writes = secrets.writes.load(Ordering::SeqCst);
        assert!(
            service
                .save(empty, Arc::new(|_| {}))
                .await
                .unwrap_err()
                .to_string()
                .contains("at least one role")
        );
        assert_eq!(secrets.writes.load(Ordering::SeqCst), writes);
        assert_eq!(reloaded.gitlab_url, "https://gitlab.example.com");
    }
    #[tokio::test]
    async fn queue_preferences_preserve_connection_and_never_access_keychain() {
        struct InaccessibleSecrets;
        impl Secrets for InaccessibleSecrets {
            fn read(&self, _: &str, _: &str) -> anyhow::Result<String> {
                panic!("Queue preferences must not read secrets")
            }
            fn write(&self, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
                panic!("Queue preferences must not write secrets")
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let service = SettingsService {
            directory: temp.path().to_path_buf(),
            secrets: Arc::new(InaccessibleSecrets),
        };
        let original = Config {
            gitlab_url: "https://gitlab.example.com".into(),
            refresh_seconds: 300,
            claude_path: "/custom/bin/claude".into(),
            ..Default::default()
        };
        let path = temp.path().join("projects.toml");
        std::fs::write(
            &path,
            toml::to_string(&Projects::single(original.clone())).unwrap(),
        )
        .unwrap();
        service
            .save_queue_preferences(original.clone(), vec![MrRole::Author], QueueSort::Newest)
            .await
            .unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        let config = toml::from_str::<Projects>(&saved)
            .unwrap()
            .current()
            .unwrap();
        assert_eq!(config.gitlab_url, original.gitlab_url);
        assert_eq!(config.refresh_seconds, 300);
        assert_eq!(config.claude_path, original.claude_path);
        assert_eq!(config.mr_roles, [MrRole::Author]);
        assert_eq!(config.mr_sort, QueueSort::Newest);
        assert!(
            service
                .save_queue_preferences(original.clone(), vec![], QueueSort::Oldest)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), saved);
    }
    #[tokio::test]
    async fn partial_keychain_failure_restores_previous_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets {
            fail_on: Some("cf-secret"),
            ..Default::default()
        });
        for (name, value) in [
            ("gitlab-token", "old-token"),
            ("cf-id", "old-id"),
            ("cf-secret", "old-secret"),
        ] {
            secrets
                .values
                .lock()
                .unwrap()
                .insert(name.into(), value.into());
        }
        let service = SettingsService {
            secrets: secrets.clone(),
            directory: temp.path().to_path_buf(),
        };
        let mut changed = draft("https://gitlab.example.com");
        changed.token = "new-token".into();
        changed.clear_cloudflare = false;
        changed.cf_id = "new-id".into();
        changed.cf_secret = "new-secret".into();
        assert!(service.save(changed, Arc::new(|_| {})).await.is_err());
        assert_eq!(secrets.read("", "gitlab-token").unwrap(), "old-token");
        assert_eq!(secrets.read("", "cf-id").unwrap(), "old-id");
        assert!(!temp.path().join("config.toml").exists());
        assert!(!temp.path().join("projects.toml.tmp").exists());
    }
    #[tokio::test]
    async fn save_rejects_a_missing_specification_folder() {
        let temp = tempfile::tempdir().unwrap();
        let service = SettingsService {
            secrets: Arc::new(MemorySecrets::default()),
            directory: temp.path().to_path_buf(),
        };
        let mut missing = draft("https://gitlab.example.com");
        missing.config.spec_path = temp.path().join("missing").display().to_string();
        let error = service.save(missing, Arc::new(|_| {})).await.unwrap_err();
        assert!(error.to_string().contains("Specification folder not found"));
        let mut present = draft("https://gitlab.example.com");
        present.config.spec_path = format!("  {}  ", temp.path().display());
        let saved = service.save(present, Arc::new(|_| {})).await.unwrap();
        assert_eq!(saved.spec_path, temp.path().display().to_string());
    }
    #[test]
    fn redacts_all_supplied_credentials() {
        assert_eq!(
            redact(
                "token=a-secret cf=b-secret",
                &["a-secret".into(), "b-secret".into()]
            ),
            "token=[REDACTED] cf=[REDACTED]"
        );
    }
}

#[cfg(test)]
mod projects_tests;
