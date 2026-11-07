//! GitHub adapter: pull requests become merge requests with the same review workflow.
//!
//! The queue, summaries, and details use GraphQL, which returns a pull request with its
//! labels, latest reviews, and check status in one request; files, merge bases, overlaps,
//! and approvals use REST. Repositories are addressed by their numeric ID
//! (`/repositories/{id}`), so a renamed repository keeps working and `MrId.project` stays
//! numeric as with GitLab.
use crate::Forge;
use anyhow::{Context, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;

/// Pull requests listed by the queue answer the summaries requested right after it.
const SUMMARY_TTL: Duration = Duration::from_secs(60);
/// GitHub search returns at most 1,000 results.
const SEARCH_PAGES: usize = 20;
/// Pull request files are listed 100 at a time, up to GitHub's limit of 3,000.
const FILE_PAGES: usize = 30;

const PULL_FIELDS: &str = "number title body url createdAt mergedAt isDraft additions deletions
    author { login }
    headRefOid baseRefOid
    repository { databaseId nameWithOwner sshUrl }
    labels(first: 30) { nodes { name color } }
    ticketDates: closingIssuesReferences(first: 10) { nodes { createdAt } }
    latestOpinionatedReviews(first: 100) { nodes { state } }
    commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }";

#[derive(Clone)]
pub struct GitHub {
    client: Client,
    api: Url,
    graphql: Url,
    group: String,
    names: Arc<Mutex<HashMap<u64, String>>>,
    recent: Arc<Mutex<HashMap<MrId, (Instant, MergeRequest)>>>,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}
#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<Option<T>>,
}
impl<T> Nodes<T> {
    fn items(self) -> impl Iterator<Item = T> {
        self.nodes.into_iter().flatten()
    }
}
#[derive(Deserialize)]
struct GqlLabel {
    name: String,
    color: Option<String>,
}
impl From<GqlLabel> for Label {
    fn from(label: GqlLabel) -> Self {
        Self {
            name: label.name,
            color: label.color.map(|color| format!("#{color}")),
            text_color: None,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRepository {
    database_id: u64,
    name_with_owner: String,
    ssh_url: String,
}
#[derive(Deserialize)]
struct GqlState {
    state: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlCommit {
    status_check_rollup: Option<GqlState>,
}
#[derive(Deserialize)]
struct GqlCommitNode {
    commit: GqlCommit,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlIssueRepository {
    database_id: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlIssue {
    created_at: Option<DateTime<Utc>>,
    title: String,
    body: Option<String>,
    url: String,
    repository: GqlIssueRepository,
    labels: Option<Nodes<GqlLabel>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlTicketDate {
    created_at: Option<DateTime<Utc>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPull {
    number: u64,
    title: String,
    body: Option<String>,
    url: String,
    created_at: DateTime<Utc>,
    merged_at: Option<DateTime<Utc>>,
    is_draft: bool,
    additions: Option<u64>,
    deletions: Option<u64>,
    author: Option<Login>,
    head_ref_oid: String,
    base_ref_oid: String,
    repository: GqlRepository,
    labels: Option<Nodes<GqlLabel>>,
    latest_opinionated_reviews: Option<Nodes<GqlState>>,
    commits: Option<Nodes<GqlCommitNode>>,
    #[serde(default)]
    closing_issues_references: Option<Nodes<GqlIssue>>,
    ticket_dates: Option<Nodes<GqlTicketDate>>,
}

/// GitHub's combined check state, named like GitLab pipeline statuses.
fn pipeline(state: &str) -> Option<String> {
    Some(
        match state {
            "SUCCESS" => "success",
            "FAILURE" | "ERROR" => "failed",
            "PENDING" => "running",
            "EXPECTED" => "pending",
            _ => return None,
        }
        .into(),
    )
}

impl GqlPull {
    fn into_model(self) -> (MergeRequest, Vec<Issue>, String) {
        let approvals = self
            .latest_opinionated_reviews
            .map(|reviews| reviews.items().filter(|r| r.state == "APPROVED").count() as u64);
        let pipeline = self
            .commits
            .and_then(|commits| commits.items().last())
            .and_then(|node| node.commit.status_check_rollup)
            .and_then(|rollup| pipeline(&rollup.state));
        let issues: Vec<Issue> = self
            .closing_issues_references
            .map(|issues| {
                issues
                    .items()
                    .map(|issue| Issue {
                        created_at: issue.created_at,
                        title: issue.title,
                        description: issue.body.unwrap_or_default(),
                        web_url: issue.url,
                        labels: issue
                            .labels
                            .map(|labels| labels.items().map(Label::from).collect())
                            .unwrap_or_default(),
                        project_id: Some(issue.repository.database_id),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut mr = MergeRequest {
            draft: self.is_draft,
            risk: None,
            id: MrId {
                project: self.repository.database_id,
                iid: self.number,
            },
            title: self.title,
            description: self.body.unwrap_or_default(),
            project: self.repository.name_with_owner,
            author: self
                .author
                .map(|author| author.login)
                .unwrap_or_else(|| "ghost".into()),
            web_url: self.url,
            sha: self.head_ref_oid,
            base_sha: self.base_ref_oid,
            created_at: self.created_at,
            labels: self
                .labels
                .map(|labels| labels.items().map(Label::from).collect())
                .unwrap_or_default(),
            merged_at: self.merged_at,
            ticket_created_at: None,
            ticket_labels: Vec::new(),
            pipeline,
            approvals,
            changed_lines: self.additions.zip(self.deletions).map(|(a, d)| a + d),
        };
        // Issues a pull request closes are its tickets.
        mr.ticket_labels = ticket_labels(&mr.labels, &issues);
        mr.ticket_created_at = self
            .ticket_dates
            .into_iter()
            .flat_map(Nodes::items)
            .filter_map(|issue| issue.created_at)
            .chain(issues.iter().filter_map(|issue| issue.created_at))
            .min();
        (mr, issues, self.repository.ssh_url)
    }
}

#[derive(Deserialize)]
struct RestFile {
    filename: String,
    previous_filename: Option<String>,
    patch: Option<String>,
    changes: u64,
}
impl From<RestFile> for ChangedFile {
    fn from(file: RestFile) -> Self {
        // GitHub omits patches for binary and very large files.
        let too_large = file.patch.is_none() && file.changes > 0;
        Self {
            old_path: file
                .previous_filename
                .unwrap_or_else(|| file.filename.clone()),
            new_path: file.filename,
            diff: file.patch.unwrap_or_default(),
            too_large,
            collapsed: false,
        }
    }
}

#[derive(Deserialize)]
struct RestPull {
    number: u64,
    title: String,
    html_url: String,
    updated_at: String,
    merged_at: Option<String>,
}

impl GitHub {
    /// `url` is `https://github.com`, or a GitHub Enterprise Server address whose API lives
    /// under `/api/v3` and `/api/graphql`.
    pub fn new(url: &str, token: &str) -> anyhow::Result<Self> {
        let base = Url::parse(url.trim()).context("Invalid GitHub URL")?;
        let local = matches!(base.host_str(), Some("127.0.0.1" | "localhost"));
        if base.scheme() != "https" && !(base.scheme() == "http" && local) {
            bail!("GitHub requires HTTPS (HTTP is allowed only on localhost for tests)");
        }
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            bail!("GitHub URL must not contain credentials, a query, or a fragment");
        }
        let (api, graphql) = match base.host_str() {
            Some("github.com" | "api.github.com" | "www.github.com") => (
                Url::parse("https://api.github.com/")?,
                Url::parse("https://api.github.com/graphql")?,
            ),
            _ => {
                let root = base.as_str().trim_end_matches('/');
                (
                    Url::parse(&format!("{root}/api/v3/"))?,
                    Url::parse(&format!("{root}/api/graphql"))?,
                )
            }
        };
        let mut defaults = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).context("Invalid token")?;
        authorization.set_sensitive(true);
        defaults.insert(reqwest::header::AUTHORIZATION, authorization);
        defaults.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        defaults.insert(
            "x-github-api-version",
            HeaderValue::from_static("2022-11-28"),
        );
        let client = Client::builder()
            .default_headers(defaults)
            .user_agent(concat!("mrde/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            api,
            graphql,
            group: String::new(),
            names: Arc::default(),
            recent: Arc::default(),
        })
    }

    /// Limits the queue to an owner (`acme`) or one repository (`acme/platform`).
    pub fn with_group(mut self, group: &str) -> Self {
        self.group = group.trim().trim_matches('/').into();
        self
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let response = self.client.get(self.api.join(path)?).send().await?;
        check(&response)?;
        response.json().await.context("Invalid GitHub response")
    }

    /// Follows `Link: <…>; rel="next"` up to `max_pages` pages of 100 items.
    async fn pages<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        max_pages: usize,
    ) -> anyhow::Result<Vec<T>> {
        let mut result = Vec::new();
        for page in 1..=max_pages {
            let response = self
                .client
                .get(self.api.join(path)?)
                .query(query)
                .query(&[("per_page", "100".to_string()), ("page", page.to_string())])
                .send()
                .await?;
            check(&response)?;
            let next = response
                .headers()
                .get(reqwest::header::LINK)
                .and_then(|link| link.to_str().ok())
                .is_some_and(|link| link.contains("rel=\"next\""));
            let batch: Vec<T> = response.json().await.context("Invalid GitHub page")?;
            result.extend(batch);
            if !next {
                return Ok(result);
            }
        }
        Ok(result)
    }

    async fn graphql(&self, query: &str, variables: Value) -> anyhow::Result<Value> {
        let response = self
            .client
            .post(self.graphql.clone())
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .await?;
        check(&response)?;
        let mut body: Value = response.json().await.context("Invalid GitHub response")?;
        if let Some(errors) = body["errors"].as_array().filter(|e| !e.is_empty()) {
            let messages: Vec<&str> = errors
                .iter()
                .filter_map(|error| error["message"].as_str())
                .collect();
            bail!("GitHub GraphQL error: {}", messages.join("; "));
        }
        Ok(body["data"].take())
    }

    /// `owner/name` of a repository, from earlier responses or `/repositories/{id}`.
    async fn name(&self, project: u64) -> anyhow::Result<String> {
        if let Some(name) = self.names().get(&project) {
            return Ok(name.clone());
        }
        #[derive(Deserialize)]
        struct Repository {
            full_name: String,
        }
        let repository: Repository = self.get(&format!("repositories/{project}")).await?;
        self.names().insert(project, repository.full_name.clone());
        Ok(repository.full_name)
    }
    fn names(&self) -> std::sync::MutexGuard<'_, HashMap<u64, String>> {
        self.names.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn remember(&self, mr: &MergeRequest) {
        self.names().insert(mr.id.project, mr.project.clone());
        self.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(mr.id.clone(), (Instant::now(), mr.clone()));
    }

    async fn pull(
        &self,
        id: &MrId,
        detail: bool,
    ) -> anyhow::Result<(MergeRequest, Vec<Issue>, String)> {
        let name = self.name(id.project).await?;
        let (owner, repository) = name
            .split_once('/')
            .context("Invalid GitHub repository name")?;
        let issues = if detail {
            "closingIssuesReferences(first: 10) { nodes { title body url createdAt repository { databaseId } labels(first: 30) { nodes { name color } } } }"
        } else {
            ""
        };
        let query = format!(
            "query($owner: String!, $name: String!, $number: Int!) {{ repository(owner: $owner, name: $name) {{ pullRequest(number: $number) {{ {PULL_FIELDS} {issues} }} }} }}"
        );
        let mut data = self
            .graphql(
                &query,
                json!({"owner": owner, "name": repository, "number": id.iid}),
            )
            .await?;
        let pull = data["repository"]["pullRequest"].take();
        if pull.is_null() {
            bail!("Pull request {name}#{} not found", id.iid);
        }
        let pull: GqlPull = serde_json::from_value(pull).context("Invalid GitHub pull request")?;
        let result = pull.into_model();
        self.remember(&result.0);
        Ok(result)
    }

    /// Search qualifier limiting the queue: a repository, an organization, or a user.
    async fn scope(&self) -> anyhow::Result<String> {
        if self.group.is_empty() {
            return Ok(String::new());
        }
        if self.group.contains('/') {
            return Ok(format!("repo:{}", self.group));
        }
        #[derive(Deserialize)]
        struct Owner {
            #[serde(rename = "type")]
            kind: String,
        }
        let owner: Owner = self
            .get(&format!("users/{}", self.group))
            .await
            .with_context(|| format!("Cannot find the GitHub owner {}", self.group))?;
        Ok(if owner.kind == "Organization" {
            format!("org:{}", self.group)
        } else {
            format!("user:{}", self.group)
        })
    }

    /// Checks the owner or repository that limits the queue.
    pub async fn test_group(&self) -> anyhow::Result<()> {
        if self.group.contains('/') {
            let _: Value = self
                .get(&format!("repos/{}", self.group))
                .await
                .with_context(|| format!("Cannot open the repository {}", self.group))?;
        } else {
            self.scope().await?;
        }
        Ok(())
    }

    async fn search(&self, query: &str) -> anyhow::Result<Vec<MergeRequest>> {
        let graphql = format!(
            "query($q: String!, $after: String) {{ search(query: $q, type: ISSUE, first: 50, after: $after) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ ... on PullRequest {{ {PULL_FIELDS} }} }} }} }}"
        );
        let mut result = Vec::new();
        let mut after = Value::Null;
        for _ in 0..SEARCH_PAGES {
            let mut data = self
                .graphql(&graphql, json!({"q": query, "after": after}))
                .await?;
            let search = data["search"].take();
            for node in search["nodes"].as_array().into_iter().flatten() {
                // Other result types are empty objects.
                if let Ok(pull) = serde_json::from_value::<GqlPull>(node.clone()) {
                    result.push(pull.into_model().0);
                }
            }
            if search["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = search["pageInfo"]["endCursor"].clone();
        }
        Ok(result)
    }

    async fn files(&self, id: &MrId) -> anyhow::Result<Vec<ChangedFile>> {
        let files: Vec<RestFile> = self
            .pages(
                &format!("repositories/{}/pulls/{}/files", id.project, id.iid),
                &[],
                FILE_PAGES,
            )
            .await?;
        Ok(files.into_iter().map(ChangedFile::from).collect())
    }

    /// The commit both branches share, which analyses diff against: GitHub's `base` is the
    /// tip of the target branch, possibly ahead of the merge base.
    async fn merge_base(&self, project: u64, base: &str, head: &str) -> anyhow::Result<String> {
        let compare: Value = self
            .get(&format!(
                "repositories/{project}/compare/{base}...{head}?per_page=1"
            ))
            .await?;
        compare["merge_base_commit"]["sha"]
            .as_str()
            .map(str::to_string)
            .context("GitHub returned no merge base")
    }
}

fn check(response: &reqwest::Response) -> anyhow::Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let remaining = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok());
    if matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    ) && remaining == Some("0")
    {
        bail!("GitHub rate limit reached: HTTP {status}. Retry later.");
    }
    bail!("GitHub request failed: HTTP {status}")
}

#[async_trait]
impl Forge for GitHub {
    async fn test_connection(&self) -> anyhow::Result<String> {
        Ok(self.get::<Login>("user").await?.login)
    }

    async fn project_id(&self, path: &str) -> anyhow::Result<u64> {
        #[derive(Deserialize)]
        struct Repository {
            id: u64,
            full_name: String,
        }
        let repository: Repository = self
            .get(&format!("repos/{path}"))
            .await
            .with_context(|| format!("Cannot open repository {path}"))?;
        self.names().insert(repository.id, repository.full_name);
        Ok(repository.id)
    }

    async fn queue(&self, roles: &[MrRole]) -> anyhow::Result<Vec<MergeRequest>> {
        if roles.is_empty() {
            bail!("Select at least one pull request role in Settings");
        }
        let user = self.get::<Login>("user").await?.login;
        let scope = self.scope().await?;
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for role in roles {
            // A review request disappears once the review is submitted: reviewed pull
            // requests stay in the queue until they close.
            let filters: &[&str] = match role {
                MrRole::Reviewer => &["review-requested", "reviewed-by"],
                MrRole::Assignee => &["assignee"],
                MrRole::Author => &["author"],
            };
            for filter in filters {
                let query = format!("is:pr is:open archived:false {filter}:{user} {scope}");
                for mr in self.search(query.trim()).await? {
                    if seen.insert(mr.id.clone()) {
                        self.remember(&mr);
                        result.push(mr);
                    }
                }
            }
        }
        sort_queue(&mut result, QueueSort::Priority, Utc::now());
        Ok(result)
    }

    async fn summary(&self, id: &MrId) -> anyhow::Result<MergeRequest> {
        let recent = self
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .filter(|(at, _)| at.elapsed() < SUMMARY_TTL)
            .map(|(_, mr)| mr.clone());
        match recent {
            Some(mr) => Ok(mr),
            None => Ok(self.pull(id, false).await?.0),
        }
    }

    async fn approval_count(&self, id: &MrId) -> anyhow::Result<Option<u64>> {
        Ok(self.pull(id, false).await?.0.approvals)
    }

    async fn detail(&self, id: &MrId) -> anyhow::Result<MrDetail> {
        let ((mut mr, issues, ssh_url), files) =
            futures_util::try_join!(self.pull(id, true), self.files(id))?;
        mr.base_sha = self.merge_base(id.project, &mr.base_sha, &mr.sha).await?;
        if mr.changed_lines.is_none() {
            mr.changed_lines = mrde_core::stats::gitlab_stats(&files)
                .map(|stats| stats.iter().map(|f| f.changed()).sum());
        }
        Ok(MrDetail {
            mr,
            files,
            issues,
            ssh_url,
        })
    }

    async fn approve(&self, id: &MrId, sha: &str) -> anyhow::Result<()> {
        if sha.is_empty() {
            bail!("Cannot approve without a commit SHA");
        }
        // GitHub accepts reviews of older commits; refuse rather than approve unseen code.
        let current = self
            .get::<Value>(&format!("repositories/{}/pulls/{}", id.project, id.iid))
            .await?;
        if current["head"]["sha"].as_str() != Some(sha) {
            bail!(
                "The pull request changed since this commit was reviewed. Refresh it before approving."
            );
        }
        let response = self
            .client
            .post(self.api.join(&format!(
                "repositories/{}/pulls/{}/reviews",
                id.project, id.iid
            ))?)
            .json(&json!({"commit_id": sha, "event": "APPROVE"}))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!(
                "Approval failed: HTTP {}. Refresh the pull request before retrying.",
                response.status()
            );
        }
        Ok(())
    }

    async fn related(&self, id: &MrId, files: &[String]) -> anyhow::Result<Vec<Overlap>> {
        let list = |state: &'static str, count: &'static str| {
            let path = format!("repositories/{}/pulls", id.project);
            async move {
                let response = self
                    .client
                    .get(self.api.join(&path)?)
                    .query(&[
                        ("state", state),
                        ("sort", "updated"),
                        ("direction", "desc"),
                        ("per_page", count),
                    ])
                    .send()
                    .await?;
                check(&response)?;
                response
                    .json::<Vec<RestPull>>()
                    .await
                    .context("Invalid GitHub response")
            }
        };
        let since = Utc::now() - chrono::Duration::days(14);
        let mut candidates = list("open", "30").await?;
        candidates.extend(list("closed", "20").await?.into_iter().filter(|pull| {
            pull.merged_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at > since)
        }));
        candidates.retain(|pull| pull.number != id.iid);
        let wanted: HashSet<&str> = files.iter().map(String::as_str).collect();
        let results: Vec<anyhow::Result<Option<Overlap>>> = futures_util::stream::iter(candidates)
            .map(|pull| {
                let wanted = &wanted;
                async move {
                    let changed = self
                        .files(&MrId {
                            project: id.project,
                            iid: pull.number,
                        })
                        .await?;
                    let mut common: Vec<String> = changed
                        .into_iter()
                        .flat_map(|file| [file.new_path, file.old_path])
                        .filter(|path| wanted.contains(path.as_str()))
                        .collect();
                    common.sort();
                    common.dedup();
                    Ok((!common.is_empty()).then(|| Overlap {
                        iid: pull.number,
                        title: pull.title,
                        merged: pull.merged_at.is_some(),
                        web_url: pull.html_url,
                        files: common,
                        updated_at: pull.updated_at,
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

    async fn merged_history(&self, project: u64, limit: usize) -> anyhow::Result<Vec<MrId>> {
        anyhow::ensure!((1..=200).contains(&limit), "Baseline size must be 1–200");
        let pulls: Vec<RestPull> = self
            .pages(
                &format!("repositories/{project}/pulls"),
                &[
                    ("state", "closed".into()),
                    ("sort", "updated".into()),
                    ("direction", "desc".into()),
                ],
                limit.div_ceil(50).max(1),
            )
            .await?;
        let mut merged: Vec<(String, u64)> = pulls
            .into_iter()
            .filter_map(|pull| Some((pull.merged_at?, pull.number)))
            .collect();
        merged.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(merged
            .into_iter()
            .take(limit)
            .map(|(_, iid)| MrId { project, iid })
            .collect())
    }
}

#[cfg(test)]
mod tests;
