use crate::Forge;
use anyhow::{Context, bail};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use mrde_core::{
    config::{MrRole, QueueSort},
    insights::Overlap,
    models::*,
    scoring::sort_queue,
};
use reqwest::{
    Client, StatusCode,
    header::{HeaderMap, HeaderValue},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;

/// Project label colors are reused for a while: refreshes would otherwise list every
/// project's labels again.
const LABEL_COLORS_TTL: Duration = Duration::from_secs(600);

type LabelColors = Arc<HashMap<String, Label>>;

#[derive(Clone)]
pub struct GitLab {
    client: Client,
    base: Url,
    label_colors: Arc<Mutex<HashMap<u64, (Instant, LabelColors)>>>,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    username: String,
}
#[derive(Deserialize)]
struct RawMr {
    iid: u64,
    project_id: u64,
    title: String,
    description: Option<String>,
    web_url: String,
    sha: Option<String>,
    created_at: chrono::DateTime<Utc>,
    labels: Vec<Label>,
    author: User,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    work_in_progress: bool,
    head_pipeline: Option<Value>,
    diff_refs: Option<Value>,
}
impl RawMr {
    fn into_model(self) -> MergeRequest {
        let project = self
            .web_url
            .split("/-/")
            .next()
            .unwrap_or(&self.web_url)
            .to_string();
        MergeRequest {
            id: MrId {
                project: self.project_id,
                iid: self.iid,
            },
            title: self.title,
            description: self.description.unwrap_or_default(),
            project,
            author: self.author.username,
            web_url: self.web_url,
            sha: self.sha.unwrap_or_default(),
            base_sha: self
                .diff_refs
                .as_ref()
                .and_then(|v| v["base_sha"].as_str())
                .unwrap_or_default()
                .into(),
            created_at: self.created_at,
            labels: self.labels,
            ticket_labels: Vec::new(),
            pipeline: self
                .head_pipeline
                .and_then(|v| v["status"].as_str().map(str::to_string)),
            approvals: None,
            changed_lines: None,
        }
    }
}

/// Unknown when GitLab truncated any diff.
fn changed_lines(files: &[ChangedFile]) -> Option<u64> {
    mrde_core::stats::gitlab_stats(files).map(|stats| stats.iter().map(|f| f.changed()).sum())
}

impl GitLab {
    pub fn new(url: &str, token: &str, headers: &[(String, String)]) -> anyhow::Result<Self> {
        let mut base = Url::parse(url).context("Invalid GitLab URL")?;
        if base.scheme() != "https"
            && !(base.scheme() == "http"
                && matches!(base.host_str(), Some("127.0.0.1" | "localhost")))
        {
            bail!("GitLab requires HTTPS (HTTP is allowed only on localhost for tests)");
        }
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            bail!("GitLab URL must not contain credentials, a query, or a fragment");
        }
        base.set_path(&format!("{}/api/v4/", base.path().trim_end_matches('/')));
        let mut defaults = HeaderMap::new();
        let mut token = HeaderValue::from_str(token).context("Invalid token")?;
        token.set_sensitive(true);
        defaults.insert("private-token", token);
        for (name, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .context("Invalid header name")?;
            if matches!(
                name.as_str(),
                "host" | "private-token" | "authorization" | "cookie"
            ) {
                bail!("Reserved header name");
            }
            let mut value = HeaderValue::from_str(value).context("Invalid header value")?;
            value.set_sensitive(true);
            defaults.insert(name, value);
        }
        let client = Client::builder()
            .default_headers(defaults)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            base,
            label_colors: Arc::default(),
        })
    }

    /// Labels of a project and its parent groups, by name, with their colors.
    async fn label_colors(&self, project: u64) -> anyhow::Result<LabelColors> {
        let cached = {
            let cache = self.label_colors.lock().unwrap_or_else(|e| e.into_inner());
            cache
                .get(&project)
                .filter(|(at, _)| at.elapsed() < LABEL_COLORS_TTL)
                .map(|(_, colors)| colors.clone())
        };
        if let Some(colors) = cached {
            return Ok(colors);
        }
        let labels: Vec<Label> = self
            .pages(
                &format!("projects/{project}/labels"),
                &[("include_ancestor_groups", "true".into())],
            )
            .await?;
        let colors: LabelColors = Arc::new(
            labels
                .into_iter()
                .map(|label| (label.name.clone(), label))
                .collect(),
        );
        self.label_colors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project, (Instant::now(), colors.clone()));
        Ok(colors)
    }

    /// Single-MR and related-issue responses name labels without colors, even with
    /// `with_labels_details`; take them from the project's labels. Failures keep names.
    async fn colorize(&self, project: u64, labels: &mut [Label]) {
        if labels.iter().all(|label| label.color.is_some()) {
            return;
        }
        match self.label_colors(project).await {
            Ok(colors) => {
                for label in labels.iter_mut().filter(|label| label.color.is_none()) {
                    if let Some(known) = colors.get(&label.name) {
                        label.color = known.color.clone();
                        label.text_color = known.text_color.clone();
                    }
                }
            }
            Err(error) => tracing::debug!(%error, project, "Label colors unavailable"),
        }
    }

    /// MR metadata, approvals, and linked tickets, fetched concurrently, with colored
    /// MR and ticket labels.
    async fn mr_and_issues(&self, id: &MrId) -> anyhow::Result<(MergeRequest, Vec<Issue>)> {
        let path = format!("projects/{}/merge_requests/{}", id.project, id.iid);
        let issues_path = format!("{path}/related_issues");
        let (raw, approvals, issues) = futures_util::future::join3(
            self.get::<RawMr>(&path),
            self.approvals(id),
            self.pages::<Issue>(&issues_path, &[]),
        )
        .await;
        let mut mr = raw?.into_model();
        mr.approvals = approvals?;
        let mut issues = issues.unwrap_or_else(|error| {
            tracing::warn!(%error, "Linked tickets unavailable; their labels are not shown");
            Vec::new()
        });
        self.colorize(id.project, &mut mr.labels).await;
        for issue in &mut issues {
            let project = issue.project_id.unwrap_or(id.project);
            self.colorize(project, &mut issue.labels).await;
        }
        mr.ticket_labels = ticket_labels(&mr.labels, primary_tickets(&mr, &issues));
        Ok((mr, issues))
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let response = self.client.get(self.base.join(path)?).send().await?;
        if !response.status().is_success() {
            bail!("GitLab request failed: HTTP {}", response.status());
        }
        response.json().await.context("Invalid GitLab response")
    }

    async fn pages<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        filters: &[(&str, String)],
    ) -> anyhow::Result<Vec<T>> {
        let mut result = Vec::new();
        for page in 1..=10_000 {
            let response = self
                .client
                .get(self.base.join(path)?)
                .query(filters)
                .query(&[("per_page", "100".to_string()), ("page", page.to_string())])
                .send()
                .await?;
            if !response.status().is_success() {
                bail!("GitLab request failed: HTTP {}", response.status());
            }
            let next = response
                .headers()
                .get("x-next-page")
                .and_then(|h| h.to_str().ok())
                .map(str::to_string);
            let batch: Vec<T> = response.json().await.context("Invalid GitLab page")?;
            let full = batch.len() == 100;
            result.extend(batch);
            if next.as_ref().is_some_and(|v| v.is_empty()) || (next.is_none() && !full) {
                return Ok(result);
            }
        }
        bail!("GitLab pagination exceeded the safety limit")
    }

    async fn approvals(&self, id: &MrId) -> anyhow::Result<Option<u64>> {
        let response = self
            .client
            .get(self.base.join(&format!(
                "projects/{}/merge_requests/{}/approvals",
                id.project, id.iid
            ))?)
            .send()
            .await?;
        if matches!(
            response.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ) {
            return Ok(None);
        }
        if !response.status().is_success() {
            bail!("Approval lookup failed: HTTP {}", response.status());
        }
        let value: Value = response.json().await?;
        Ok(value["approved_by"].as_array().map(|v| v.len() as u64))
    }
}

#[async_trait]
impl Forge for GitLab {
    async fn download_image(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let instance = self.base.as_str().trim_end_matches("/api/v4/");
        if !crate::uploads::is_upload_url(instance, url) {
            bail!("Authenticated image URL is outside the GitLab upload API");
        }
        crate::uploads::image_bytes(
            self.client
                .get(url)
                .send()
                .await
                .map_err(|error| error.without_url())
                .context("Cannot download GitLab image")?,
        )
        .await
    }

    async fn test_connection(&self) -> anyhow::Result<String> {
        Ok(self.get::<User>("user").await?.username)
    }
    async fn queue(&self, roles: &[MrRole]) -> anyhow::Result<Vec<MergeRequest>> {
        if roles.is_empty() {
            bail!("Select at least one merge request role in Settings");
        }
        let user: User = self.get("user").await?;
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for role in roles {
            let role = match role {
                MrRole::Reviewer => "reviewer_id",
                MrRole::Assignee => "assignee_id",
                MrRole::Author => "author_id",
            };
            let filters = [
                ("scope", "all".into()),
                ("state", "opened".into()),
                ("with_labels_details", "true".into()),
                (role, user.id.to_string()),
            ];
            for raw in self.pages::<RawMr>("merge_requests", &filters).await? {
                if raw.draft || raw.work_in_progress {
                    continue;
                }
                let mr = raw.into_model();
                if seen.insert(mr.id.clone()) {
                    result.push(mr);
                }
            }
        }
        sort_queue(&mut result, QueueSort::Priority, Utc::now());
        Ok(result)
    }
    async fn summary(&self, id: &MrId) -> anyhow::Result<MergeRequest> {
        Ok(self.mr_and_issues(id).await?.0)
    }
    async fn approval_count(&self, id: &MrId) -> anyhow::Result<Option<u64>> {
        self.approvals(id).await
    }
    async fn detail(&self, id: &MrId) -> anyhow::Result<MrDetail> {
        let path = format!("projects/{}/merge_requests/{}", id.project, id.iid);
        let (mut mr, issues) = self.mr_and_issues(id).await?;
        let files: Vec<ChangedFile> = self.pages(&format!("{path}/diffs"), &[]).await?;
        mr.changed_lines = changed_lines(&files);
        let project: Value = self.get(&format!("projects/{}", id.project)).await?;
        mr.project = project["path_with_namespace"]
            .as_str()
            .unwrap_or(&mr.project)
            .into();
        let ssh_url = project["ssh_url_to_repo"]
            .as_str()
            .context("Project has no SSH clone URL")?
            .to_string();
        Ok(MrDetail {
            mr,
            files,
            issues,
            ssh_url,
        })
    }
    async fn related(&self, id: &MrId, files: &[String]) -> anyhow::Result<Vec<Overlap>> {
        #[derive(Deserialize)]
        struct Listed {
            iid: u64,
            title: String,
            web_url: String,
            updated_at: String,
            #[serde(default)]
            merged_at: Option<String>,
        }
        #[derive(Deserialize)]
        struct Path {
            old_path: String,
            new_path: String,
        }
        let list = |query: Vec<(&'static str, String)>| async move {
            let response = self
                .client
                .get(
                    self.base
                        .join(&format!("projects/{}/merge_requests", id.project))?,
                )
                .query(&query)
                .send()
                .await?;
            if !response.status().is_success() {
                bail!("Listing merge requests failed: HTTP {}", response.status());
            }
            response
                .json::<Vec<Listed>>()
                .await
                .context("Invalid GitLab response")
        };
        let since = (Utc::now() - chrono::Duration::days(14)).to_rfc3339();
        let mut candidates = list(vec![
            ("state", "opened".into()),
            ("order_by", "updated_at".into()),
            ("per_page", "30".into()),
        ])
        .await?;
        candidates.extend(
            list(vec![
                ("state", "merged".into()),
                ("updated_after", since),
                ("order_by", "updated_at".into()),
                ("per_page", "20".into()),
            ])
            .await?,
        );
        candidates.retain(|mr| mr.iid != id.iid);
        let wanted: HashSet<&str> = files.iter().map(String::as_str).collect();
        let results: Vec<anyhow::Result<Option<Overlap>>> = futures_util::stream::iter(candidates)
            .map(|mr| {
                let wanted = &wanted;
                async move {
                    let response = self
                        .client
                        .get(self.base.join(&format!(
                            "projects/{}/merge_requests/{}/diffs",
                            id.project, mr.iid
                        ))?)
                        .query(&[("per_page", "100")])
                        .send()
                        .await?;
                    if !response.status().is_success() {
                        return Ok(None);
                    }
                    let paths: Vec<Path> = response.json().await.context("Invalid GitLab diffs")?;
                    let mut common: Vec<String> = paths
                        .into_iter()
                        .flat_map(|p| [p.new_path, p.old_path])
                        .filter(|p| wanted.contains(p.as_str()))
                        .collect();
                    common.sort();
                    common.dedup();
                    Ok((!common.is_empty()).then(|| Overlap {
                        iid: mr.iid,
                        title: mr.title,
                        merged: mr.merged_at.is_some(),
                        web_url: mr.web_url,
                        files: common,
                        updated_at: mr.updated_at,
                    }))
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        let mut overlaps: Vec<Overlap> = results
            .into_iter()
            .filter_map(|r| r.ok().flatten())
            .collect();
        overlaps.sort_by(|a, b| {
            b.files
                .len()
                .cmp(&a.files.len())
                .then_with(|| a.iid.cmp(&b.iid))
        });
        Ok(overlaps)
    }
    async fn approve(&self, id: &MrId, sha: &str) -> anyhow::Result<()> {
        if sha.is_empty() {
            bail!("Cannot approve without a commit SHA");
        }
        let response = self
            .client
            .post(self.base.join(&format!(
                "projects/{}/merge_requests/{}/approve",
                id.project, id.iid
            ))?)
            .json(&serde_json::json!({"sha": sha}))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!(
                "Approval failed: HTTP {}. Refresh the MR before retrying.",
                response.status()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    #[tokio::test]
    async fn upload_images_include_authentication_and_never_follow_redirects() {
        let server = MockServer::start().await;
        let external = MockServer::start().await;
        let secret = "5f4dcc3b5aa765d61d8327deb882cf99";
        let endpoint =
            format!("/api/v4/projects/team%2Fapp/uploads/{secret}/01-schedule-sheet.png");
        Mock::given(method("GET"))
            .and(path(&endpoint))
            .and(header("private-token", "test-token"))
            .and(header("cf-access-client-id", "test-id"))
            .and(header("cf-access-client-secret", "test-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1, 2, 3], "image/png"))
            .expect(1)
            .mount(&server)
            .await;
        let forge = GitLab::new(
            &server.uri(),
            "test-token",
            &[
                ("CF-Access-Client-Id".into(), "test-id".into()),
                ("CF-Access-Client-Secret".into(), "test-secret".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            forge
                .download_image(&format!("{}{endpoint}", server.uri()))
                .await
                .unwrap(),
            [1, 2, 3]
        );
        assert!(
            forge
                .download_image(&format!("{}{endpoint}", external.uri()))
                .await
                .is_err()
        );
        let redirect_path = format!("/api/v4/projects/1/uploads/{secret}/redirect.png");
        Mock::given(path(&redirect_path))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/stolen.png", external.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = forge
            .download_image(&format!("{}{redirect_path}", server.uri()))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("302"));
        assert!(!error.to_string().contains("test-token"));
        assert!(external.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn public_images_have_no_credentials_and_login_pages_are_rejected() {
        let server = MockServer::start().await;
        Mock::given(path("/public.png"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1, 2], "image/png"))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            crate::uploads::download_public_image(&format!("{}/public.png", server.uri()))
                .await
                .unwrap(),
            [1, 2]
        );
        let requests = server.received_requests().await.unwrap();
        for header in [
            "private-token",
            "cf-access-client-id",
            "cf-access-client-secret",
        ] {
            assert!(!requests[0].headers.contains_key(header));
        }
        let endpoint = "/api/v4/projects/1/uploads/5f4dcc3b5aa765d61d8327deb882cf99/login.png";
        Mock::given(path(endpoint))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("<html>Sign in</html>", "text/html"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let forge = GitLab::new(&server.uri(), "test-token", &[]).unwrap();
        assert!(
            forge
                .download_image(&format!("{}{endpoint}", server.uri()))
                .await
                .unwrap_err()
                .to_string()
                .contains("content type")
        );
    }
    #[tokio::test]
    async fn connection_includes_gitlab_and_cloudflare_credentials() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .and(header("private-token", "token"))
            .and(header("cf-access-client-id", "client"))
            .and(header("cf-access-client-secret", "secret"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 7, "username": "reviewer"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let forge = GitLab::new(
            &server.uri(),
            "token",
            &[
                ("CF-Access-Client-Id".into(), "client".into()),
                ("CF-Access-Client-Secret".into(), "secret".into()),
            ],
        )
        .unwrap();
        assert_eq!(forge.test_connection().await.unwrap(), "reviewer");
    }
    #[tokio::test]
    async fn approval_sends_reviewed_sha_and_reports_conflict() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/1/merge_requests/2/approve"))
            .and(body_json(serde_json::json!({"sha":"abc"})))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;
        let forge = GitLab::new(&server.uri(), "token", &[]).unwrap();
        assert!(
            forge
                .approve(&MrId { project: 1, iid: 2 }, "abc")
                .await
                .unwrap_err()
                .to_string()
                .contains("409")
        );
    }
    #[tokio::test]
    async fn queue_unions_roles_deduplicates_skips_drafts_and_follows_pages() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id":7,"username":"me"})),
            )
            .mount(&server)
            .await;
        let mr = |iid: u64, draft: bool| {
            serde_json::json!({
                "iid":iid,"project_id":1,"title":"Review","description":null,
                "web_url":"https://gitlab.example.com/team/project/-/merge_requests/1",
                "sha":"abc","created_at":"2026-01-01T00:00:00Z","labels":[],
                "author":{"id":8,"username":"author"},"draft":draft
            })
        };
        for (role, page, next, body) in [
            ("reviewer_id", "1", "2", vec![mr(1, false), mr(2, true)]),
            ("reviewer_id", "2", "", vec![mr(3, false)]),
            ("assignee_id", "1", "", vec![mr(1, false), mr(4, false)]),
        ] {
            Mock::given(path("/api/v4/merge_requests"))
                .and(query_param(role, "7"))
                .and(query_param("page", page))
                .and(query_param("state", "opened"))
                .and(query_param("scope", "all"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("x-next-page", next)
                        .set_body_json(body),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let queue = GitLab::new(&server.uri(), "token", &[])
            .unwrap()
            .queue(&[MrRole::Reviewer, MrRole::Assignee])
            .await
            .unwrap();
        assert_eq!(
            queue.iter().map(|mr| mr.id.iid).collect::<Vec<_>>(),
            [1, 3, 4]
        );
    }
    #[tokio::test]
    async fn queue_requests_only_the_selected_roles() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id":7,"username":"me"})),
            )
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/merge_requests"))
            .and(query_param("author_id", "7"))
            .and(query_param("state", "opened"))
            .and(query_param("scope", "all"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                    "iid":5,"project_id":1,"title":"Mine","description":null,
                    "web_url":"https://gitlab.example.com/team/project/-/merge_requests/5",
                    "sha":"abc","created_at":"2026-01-01T00:00:00Z","labels":[],
                    "author":{"id":7,"username":"me"}
                }])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let forge = GitLab::new(&server.uri(), "token", &[]).unwrap();
        let queue = forge.queue(&[MrRole::Author]).await.unwrap();
        assert_eq!(queue.iter().map(|mr| mr.id.iid).collect::<Vec<_>>(), [5]);
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().all(|request| {
            let query = request.url.query().unwrap_or_default();
            !query.contains("reviewer_id") && !query.contains("assignee_id")
        }));
        assert!(forge.queue(&[]).await.is_err());
    }
    #[tokio::test]
    async fn label_colors_survive_queue_detail_and_cache_serialization() {
        let server = MockServer::start().await;
        let raw = |labels: serde_json::Value| {
            serde_json::json!({
                "iid":1,"project_id":1,"title":"Labels","description":null,
                "web_url":"https://gitlab.example.com/team/project/-/merge_requests/1",
                "sha":"abc","created_at":"2026-01-01T00:00:00Z",
                "labels":labels, "author":{"id":7,"username":"me"}
            })
        };
        Mock::given(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id":7,"username":"me"})),
            )
            .mount(&server)
            .await;
        // Only list endpoints honor with_labels_details.
        Mock::given(path("/api/v4/merge_requests"))
            .and(query_param("with_labels_details", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([raw(
                serde_json::json!([{"name":"Review::Dev","color":"#428BCA","text_color":"#FFFFFF"}, "legacy"])
            )])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/projects/1/merge_requests/1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(raw(serde_json::json!(["Review::Dev", "legacy"]))),
            )
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/projects/1/merge_requests/1/approvals"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/projects/1/merge_requests/1/related_issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "title":"Ticket","description":null,"web_url":"https://gitlab.example.com/front/-/issues/3",
                "project_id":2,"labels":["P::0","Review::Dev"]
            }])))
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/projects/1/labels"))
            .and(query_param("include_ancestor_groups", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id":1,"name":"Review::Dev","color":"#428BCA","text_color":"#FFFFFF","description":null}
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/projects/2/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id":2,"name":"P::0","color":"#dc143c","text_color":"#FFFFFF"}
            ])))
            .expect(1)
            .mount(&server)
            .await;
        let forge = GitLab::new(&server.uri(), "token", &[]).unwrap();
        let queue = forge.queue(&[MrRole::Reviewer]).await.unwrap();
        let summary = forge.summary(&queue[0].id).await.unwrap();
        assert_eq!(queue[0].labels, summary.labels);
        assert_eq!(summary.labels[0].color.as_deref(), Some("#428BCA"));
        assert_eq!(summary.labels[0].text_color.as_deref(), Some("#FFFFFF"));
        assert_eq!(summary.labels[0].scope(), Some(("Review", "Dev")));
        assert_eq!(
            summary.labels[1],
            Label::from("legacy"),
            "unknown labels keep their name"
        );
        let tickets: Vec<_> = summary
            .ticket_labels
            .iter()
            .map(|l| (l.name.as_str(), l.color.as_deref()))
            .collect();
        assert_eq!(
            tickets,
            [("P::0", Some("#dc143c"))],
            "ticket project colors, no duplicate"
        );
        // Project labels are cached: a second summary lists no labels again.
        let again = forge.summary(&queue[0].id).await.unwrap();
        assert_eq!(again.ticket_labels, summary.ticket_labels);
        let restored: MergeRequest =
            serde_json::from_str(&serde_json::to_string(&summary).unwrap()).unwrap();
        assert_eq!(restored.labels, summary.labels);
        assert_eq!(restored.ticket_labels, summary.ticket_labels);
    }

    #[test]
    fn changed_lines_count_removed_dash_lines_and_keep_truncation_unknown() {
        let file = |diff: &str, too_large: bool| ChangedFile {
            old_path: "a.sql".into(),
            new_path: "a.sql".into(),
            diff: diff.into(),
            too_large,
            collapsed: false,
        };
        let sql = file(
            "@@ -1,2 +1,1 @@\n--- old comment\n+++ new comment\n context\n",
            false,
        );
        assert_eq!(changed_lines(std::slice::from_ref(&sql)), Some(2));
        let headed = file("--- a/a.sql\n+++ b/a.sql\n@@ -1 +1 @@\n-a\n+b\n", false);
        assert_eq!(changed_lines(&[headed]), Some(2));
        assert_eq!(changed_lines(&[sql, file("", true)]), None);
    }
    #[tokio::test]
    async fn related_merge_requests_share_files_with_the_selected_one() {
        let server = MockServer::start().await;
        let listed = |iid: u64, merged: bool| {
            serde_json::json!({
                "iid": iid, "title": format!("MR {iid}"), "web_url": format!("https://g/mr/{iid}"),
                "updated_at": "2026-11-05T10:00:00Z", "merged_at": if merged { serde_json::json!("2026-11-04T10:00:00Z") } else { serde_json::Value::Null }
            })
        };
        Mock::given(path("/api/v4/projects/1/merge_requests"))
            .and(query_param("state", "opened"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                listed(231, false),
                listed(238, false),
                listed(241, false)
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v4/projects/1/merge_requests"))
            .and(query_param("state", "merged"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([listed(229, true)])),
            )
            .expect(1)
            .mount(&server)
            .await;
        for (iid, files) in [
            (238, vec!["src/create.go", "src/other.go"]),
            (241, vec!["ui/chat.ts"]),
            (229, vec!["src/invoice.go"]),
        ] {
            let body: Vec<_> = files
                .iter()
                .map(|f| serde_json::json!({"old_path": f, "new_path": f}))
                .collect();
            Mock::given(path(format!(
                "/api/v4/projects/1/merge_requests/{iid}/diffs"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        }
        let forge = GitLab::new(&server.uri(), "token", &[]).unwrap();
        let overlaps = forge
            .related(
                &MrId {
                    project: 1,
                    iid: 231,
                },
                &["src/create.go".into(), "src/invoice.go".into()],
            )
            .await
            .unwrap();
        let found: Vec<_> = overlaps
            .iter()
            .map(|o| (o.iid, o.merged, o.files.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (229, true, vec!["src/invoice.go".to_string()]),
                (238, false, vec!["src/create.go".to_string()])
            ]
        );
    }
    #[tokio::test]
    async fn credentials_are_not_forwarded_to_redirects() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "https://other.example.com/user"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = GitLab::new(&server.uri(), "token", &[])
            .unwrap()
            .test_connection()
            .await
            .unwrap_err();
        assert!(error.to_string().contains("302"));
    }
}
