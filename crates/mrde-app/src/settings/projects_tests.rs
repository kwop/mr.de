use super::*;
use std::{collections::HashMap, sync::Mutex};
#[derive(Default)]
struct ScopedSecrets(Mutex<HashMap<(String, String), String>>);
impl Secrets for ScopedSecrets {
    fn read(&self, instance: &str, name: &str) -> anyhow::Result<String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(instance.into(), name.into()))
            .cloned()
            .unwrap_or_default())
    }
    fn write(&self, instance: &str, name: &str, value: &str) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert((instance.into(), name.into()), value.into());
        Ok(())
    }
}
#[tokio::test]
async fn profiles_on_the_same_host_never_inherit_tokens_and_preferences_stay_local() {
    let directory = tempfile::tempdir().unwrap();
    let secrets = Arc::new(ScopedSecrets::default());
    let service = SettingsService {
        secrets: secrets.clone(),
        directory: directory.path().into(),
    };
    let first = Draft {
        config: Config {
            gitlab_url: "https://gitlab.example.com".into(),
            ..Default::default()
        },
        token: "first-token".into(),
        ..Default::default()
    };
    service.save(first.clone(), Arc::new(|_| {})).await.unwrap();
    let second = Draft {
        config: Config {
            id: "second".into(),
            name: "Second".into(),
            ..first.config.clone()
        },
        ..Default::default()
    };
    assert!(
        service
            .save(second.clone(), Arc::new(|_| {}))
            .await
            .unwrap_err()
            .to_string()
            .contains("token")
    );
    service
        .save(
            Draft {
                token: "second-token".into(),
                ..second.clone()
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    service
        .save_queue_preferences(
            first.config.clone(),
            vec![MrRole::Author],
            QueueSort::Newest,
        )
        .await
        .unwrap();
    let catalog = Projects::load(directory.path()).unwrap();
    assert_eq!(
        catalog.active, "second",
        "an older preference save must not change the active project"
    );
    assert_eq!(catalog.current().unwrap().mr_sort, QueueSort::Priority);
    assert_eq!(catalog.projects[0].mr_sort, QueueSort::Newest);
    assert_eq!(
        secrets
            .read(&first.config.credential_key(), "gitlab-token")
            .unwrap(),
        "first-token"
    );
    assert_eq!(
        secrets
            .read(&second.config.credential_key(), "gitlab-token")
            .unwrap(),
        "second-token"
    );
    service.activate_project("default".into()).await.unwrap();
    assert_eq!(Projects::load(directory.path()).unwrap().active, "default");
    assert!(service.activate_project("missing".into()).await.is_err());
    assert_eq!(Projects::load(directory.path()).unwrap().active, "default");
    let toml = std::fs::read_to_string(directory.path().join("projects.toml")).unwrap();
    assert!(!toml.contains("first-token") && !toml.contains("second-token"));
}
#[tokio::test]
async fn github_profile_saves_without_http_and_testing_reports_unavailable() {
    let server = wiremock::MockServer::start().await;
    let directory = tempfile::tempdir().unwrap();
    let service = SettingsService {
        secrets: Arc::new(ScopedSecrets::default()),
        directory: directory.path().into(),
    };
    let draft = Draft {
        config: Config {
            id: "community".into(),
            name: "Community".into(),
            provider: mrde_core::config::Provider::Github,
            gitlab_url: server.uri(),
            ..Default::default()
        },
        ..Default::default()
    };
    service.save(draft.clone(), Arc::new(|_| {})).await.unwrap();
    let error = service.test(draft, Arc::new(|_| {})).await.unwrap_err();
    assert!(error.to_string().contains("not available yet"));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        Projects::load(directory.path())
            .unwrap()
            .current()
            .unwrap()
            .provider,
        mrde_core::config::Provider::Github
    );
}
