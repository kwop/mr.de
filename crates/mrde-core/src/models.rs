use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MrId {
    pub project: u64,
    pub iid: u64,
}

/// A forge label, including its own presentation colors. Old cached queues used strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "LabelRepr")]
pub struct Label {
    pub name: String,
    pub color: Option<String>,
    pub text_color: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LabelRepr {
    Name(String),
    Detailed {
        name: String,
        color: Option<String>,
        text_color: Option<String>,
    },
}
impl From<LabelRepr> for Label {
    fn from(value: LabelRepr) -> Self {
        match value {
            LabelRepr::Name(name) => name.into(),
            LabelRepr::Detailed {
                name,
                color,
                text_color,
            } => Self {
                name,
                color,
                text_color,
            },
        }
    }
}
impl From<String> for Label {
    fn from(name: String) -> Self {
        Self {
            name,
            color: None,
            text_color: None,
        }
    }
}
impl From<&str> for Label {
    fn from(name: &str) -> Self {
        name.to_owned().into()
    }
}
impl Label {
    pub fn scope(&self) -> Option<(&str, &str)> {
        self.name
            .split_once("::")
            .filter(|(scope, value)| !scope.is_empty() && !value.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRequest {
    /// Provider draft state; older cached requests default to ready.
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub risk: Option<crate::risk::RiskSummary>,
    pub id: MrId,
    pub title: String,
    pub description: String,
    pub project: String,
    pub author: String,
    pub web_url: String,
    pub sha: String,
    pub base_sha: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    /// Oldest primary ticket, independent of the merge request's creation date.
    #[serde(default)]
    pub ticket_created_at: Option<DateTime<Utc>>,
    pub labels: Vec<Label>,
    /// Labels of the linked tickets (related issues) not already on the MR. Teams often
    /// label tickets rather than merge requests.
    #[serde(default)]
    pub ticket_labels: Vec<Label>,
    pub pipeline: Option<String>,
    pub approvals: Option<u64>,
    pub changed_lines: Option<u64>,
}

/// Issue references in text: `#12`, `project#12`, or `group/project#12`.
fn issue_references(text: &str) -> Vec<(String, u64)> {
    let bytes = text.as_bytes();
    let mut references = Vec::new();
    for (index, _) in text.match_indices('#') {
        let digits: String = text[index + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let Ok(iid) = digits.parse() else { continue };
        let mut start = index;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric() || b"_-./".contains(&bytes[start - 1]))
        {
            start -= 1;
        }
        references.push((text[start..index].trim_matches('/').to_string(), iid));
    }
    references
}

/// The tickets an MR implements, among its related issues: those referenced in the title,
/// else in the description, else the only related issue. Merely mentioned issues would
/// add unrelated labels (other statuses and priorities).
pub fn primary_tickets<'a>(mr: &MergeRequest, issues: &'a [Issue]) -> Vec<&'a Issue> {
    // GitLab: `<project>/-/merge_requests/1`; GitHub: `<project>/pull/1`.
    let own_project = mr
        .web_url
        .split("/-/")
        .next()
        .and_then(|url| url.split("/pull/").next())
        .unwrap_or_default();
    let matches = |reference: &(String, u64), issue: &Issue| {
        // `/-/issues/12`, `/-/work_items/12` on recent GitLab versions, or `/issues/12` on
        // GitHub.
        let Some((project, rest)) = issue.web_url.split_once("/-/").or_else(|| {
            issue
                .web_url
                .rsplit_once("/issues/")
                .map(|(p, _)| (p, &issue.web_url[p.len() + 1..]))
        }) else {
            return false;
        };
        let iid = rest
            .split(['/', '#', '?'])
            .nth(1)
            .filter(|_| rest.starts_with("issues/") || rest.starts_with("work_items/"))
            .unwrap_or_default();
        iid.parse() == Ok(reference.1)
            && if reference.0.is_empty() {
                project == own_project
            } else {
                project.ends_with(&format!("/{}", reference.0))
            }
    };
    for text in [&mr.title, &mr.description] {
        let references = issue_references(text);
        let found: Vec<&Issue> = issues
            .iter()
            .filter(|issue| references.iter().any(|r| matches(r, issue)))
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    if issues.len() == 1 {
        issues.iter().collect()
    } else {
        Vec::new()
    }
}

/// The oldest known primary ticket is the most likely to need its requirements rechecked.
pub fn ticket_created_at(mr: &MergeRequest, issues: &[Issue]) -> Option<DateTime<Utc>> {
    primary_tickets(mr, issues)
        .into_iter()
        .filter_map(|issue| issue.created_at)
        .min()
}

impl MergeRequest {
    /// MR labels first, then ticket labels; used for display and priority.
    pub fn all_labels(&self) -> impl Iterator<Item = &Label> {
        self.labels.iter().chain(&self.ticket_labels)
    }
}

/// Labels of `issues` absent from `own`, deduplicated by name, in first-seen order.
pub fn ticket_labels<'a>(own: &[Label], issues: impl IntoIterator<Item = &'a Issue>) -> Vec<Label> {
    let mut labels: Vec<Label> = Vec::new();
    for label in issues.into_iter().flat_map(|issue| &issue.labels) {
        if !own
            .iter()
            .chain(&labels)
            .any(|known| known.name == label.name)
        {
            labels.push(label.clone());
        }
    }
    labels
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    pub old_path: String,
    pub new_path: String,
    pub diff: String,
    #[serde(default)]
    pub too_large: bool,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    pub title: String,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub description: String,
    pub web_url: String,
    /// GitLab returns names only for related issues; the forge adds project colors.
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub project_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrDetail {
    pub mr: MergeRequest,
    pub files: Vec<ChangedFile>,
    pub issues: Vec<Issue>,
    pub ssh_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub summary: String,
    pub intention: String,
    pub risks: Vec<Risk>,
    pub priority_files: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Risk {
    pub severity: Severity,
    pub description: String,
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

/// Depth of a Claude pre-review. Each level has its own cache entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLevel {
    Quick,
    #[default]
    Standard,
    Deep,
}
impl ReviewLevel {
    pub const ALL: [ReviewLevel; 3] =
        [ReviewLevel::Quick, ReviewLevel::Standard, ReviewLevel::Deep];
    pub fn label(self) -> &'static str {
        match self {
            ReviewLevel::Quick => "Quick",
            ReviewLevel::Standard => "Standard",
            ReviewLevel::Deep => "Deep",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            ReviewLevel::Quick => "Blocking issues only, changed files only.",
            ReviewLevel::Standard => "Risks in the change and its immediate context.",
            ReviewLevel::Deep => "Callers, tests, edge cases, security, and data migrations.",
        }
    }
}

/// Lines changed in one file. Binary files have no line counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStat {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    pub binary: bool,
}
impl FileStat {
    pub fn changed(&self) -> u64 {
        self.added + self.removed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsSource {
    /// Estimated from the GitLab diff API; unavailable when GitLab truncates a diff.
    GitLab,
    /// Exact `git diff --numstat` between the base and head commits in the worktree.
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    pub sha: String,
    pub source: StatsSource,
    pub files: Vec<FileStat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceVerdict {
    Compliant,
    Partial,
    NonCompliant,
    NoSpecFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Met,
    Partial,
    Missing,
    Unclear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplianceFinding {
    pub requirement: String,
    pub status: RequirementStatus,
    pub evidence: String,
    pub spec_file: Option<String>,
    /// Code locations implementing the requirement, as `path:line`.
    #[serde(default)]
    pub code: Vec<String>,
    /// Tests exercising the requirement, as `path` or `path::name`.
    #[serde(default)]
    pub tests: Vec<String>,
}

/// Whether the MR's ticket and implementation follow the project specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compliance {
    pub verdict: ComplianceVerdict,
    pub summary: String,
    pub spec_files: Vec<String>,
    pub findings: Vec<ComplianceFinding>,
    pub confidence: f64,
}

fn null_to_empty<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mr(title: &str, description: &str) -> MergeRequest {
        MergeRequest {
            draft: false,
            risk: None,
            id: MrId {
                project: 8,
                iid: 97,
            },
            title: title.into(),
            description: description.into(),
            project: "team/platform".into(),
            author: "alice".into(),
            web_url: "https://gitlab.example.com/team/platform/-/merge_requests/97".into(),
            sha: "a".into(),
            base_sha: "b".into(),
            created_at: Utc::now(),
            labels: vec![],
            merged_at: None,
            ticket_created_at: None,
            ticket_labels: vec![],
            pipeline: None,
            approvals: None,
            changed_lines: None,
        }
    }
    fn issue(project: &str, iid: u64, labels: &[&str]) -> Issue {
        Issue {
            created_at: None,
            title: format!("{project}#{iid}"),
            description: String::new(),
            web_url: format!("https://gitlab.example.com/team/{project}/-/issues/{iid}"),
            labels: labels.iter().map(|l| (*l).into()).collect(),
            project_id: None,
        }
    }

    #[test]
    fn ticket_age_uses_the_primary_issue_instead_of_mr_age_or_mentions() {
        let request = mr("Implement web-app#12 and web-app#13", "Also see web-app#99");
        let mut issues = vec![
            issue("web-app", 12, &[]),
            issue("web-app", 13, &[]),
            issue("web-app", 99, &[]),
        ];
        issues[0].created_at = Some("2026-06-01T00:00:00Z".parse().unwrap());
        issues[1].created_at = Some("2026-05-01T00:00:00Z".parse().unwrap());
        issues[2].created_at = Some("2020-01-01T00:00:00Z".parse().unwrap());
        assert_eq!(ticket_created_at(&request, &issues), issues[1].created_at);
        assert_eq!(ticket_created_at(&request, &[]), None);
    }

    #[test]
    fn primary_tickets_come_from_the_title_then_the_description() {
        let mut work_item = issue("web-app", 3436, &["P::0"]);
        work_item.web_url = work_item.web_url.replace("/-/issues/", "/-/work_items/");
        let issues = [
            work_item,
            issue("web-app", 3000, &["P::2"]),
            issue("platform", 12, &["Type::Bug"]),
        ];
        let titles = |found: Vec<&Issue>| found.iter().map(|i| i.title.clone()).collect::<Vec<_>>();
        let titled = mr(
            "Clarify review instructions, fix web-app#3436",
            "Voir aussi #12",
        );
        assert_eq!(titles(primary_tickets(&titled, &issues)), ["web-app#3436"]);
        let described = mr(
            "Clarify review instructions",
            "Ticket : #12, related to team/web-app#3000",
        );
        assert_eq!(
            titles(primary_tickets(&described, &issues)),
            ["web-app#3000", "platform#12"]
        );
        let unreferenced = mr("Clarify", "");
        assert!(
            primary_tickets(&unreferenced, &issues).is_empty(),
            "mentions only"
        );
        assert_eq!(
            primary_tickets(&unreferenced, &issues[..1]).len(),
            1,
            "single related issue"
        );
    }

    #[test]
    fn github_issue_links_match_references_in_pull_requests() {
        let github = |project: &str, iid: u64| {
            let mut found = issue(project, iid, &[]);
            found.web_url = format!("https://github.com/acme/{project}/issues/{iid}");
            found
        };
        let issues = [
            github("platform", 3),
            github("platform", 4),
            github("web", 3),
        ];
        let mut pull = mr("Fix #3, see web#3", "");
        pull.web_url = "https://github.com/acme/platform/pull/9".into();
        let titles: Vec<String> = primary_tickets(&pull, &issues)
            .iter()
            .map(|i| i.title.clone())
            .collect();
        assert_eq!(titles, ["platform#3", "web#3"]);
    }
}
