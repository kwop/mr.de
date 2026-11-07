//! Queue search, label filters, and merge request references typed by the user.
use crate::models::{Label, MergeRequest};
use crate::scoring::priority;

/// Text and label filters applied to the queue. Every query word must match the MR
/// reference, title, project, author, or a label; every selected label must be present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueFilter {
    pub query: String,
    pub labels: Vec<String>,
}

impl QueueFilter {
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty() || !self.labels.is_empty()
    }

    pub fn matches(&self, mr: &MergeRequest) -> bool {
        if !self
            .labels
            .iter()
            .all(|name| mr.all_labels().any(|label| &label.name == name))
        {
            return false;
        }
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let mut haystack = format!("!{} {} {} {}", mr.id.iid, mr.title, mr.project, mr.author);
        for label in mr.all_labels() {
            haystack.push(' ');
            haystack.push_str(&label.name);
        }
        let haystack = haystack.to_lowercase();
        query.split_whitespace().all(|word| haystack.contains(word))
    }

    /// Adds the label to the filter, or removes it when already selected.
    pub fn toggle_label(&mut self, name: &str) {
        if let Some(index) = self.labels.iter().position(|label| label == name) {
            self.labels.remove(index);
        } else {
            self.labels.push(name.to_owned());
        }
    }
}

/// Labels of the queue (MR and ticket labels) with the number of MRs carrying each:
/// priority labels first, then the most frequent.
pub fn label_facets(queue: &[MergeRequest]) -> Vec<(Label, usize)> {
    let mut facets: Vec<(Label, usize)> = Vec::new();
    for mr in queue {
        let mut seen: Vec<&str> = Vec::new();
        for label in mr.all_labels() {
            if seen.contains(&label.name.as_str()) {
                continue;
            }
            seen.push(&label.name);
            match facets
                .iter_mut()
                .find(|(known, _)| known.name == label.name)
            {
                Some((_, count)) => *count += 1,
                None => facets.push((label.clone(), 1)),
            }
        }
    }
    facets.sort_by(|(a, a_count), (b, b_count)| {
        priority([a])
            .cmp(&priority([b]))
            .then(b_count.cmp(a_count))
            .then_with(|| a.name.cmp(&b.name))
    });
    facets
}

/// A merge request named by its project path and number, from a pasted GitLab URL or a
/// `group/project!123` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrReference {
    pub project: String,
    pub iid: u64,
}

impl MrReference {
    /// URLs must belong to `instance`, so a pasted link never redirects requests elsewhere.
    pub fn parse(input: &str, instance: &str) -> anyhow::Result<Self> {
        let input = input.trim();
        let (project, iid) = if input.starts_with("https://") || input.starts_with("http://") {
            let (host, path) = split_url(input);
            let (instance_host, instance_path) = split_url(instance.trim());
            if instance_host.is_empty() || !host.eq_ignore_ascii_case(instance_host) {
                anyhow::bail!(
                    "This link belongs to another GitLab instance ({host}). Open merge requests from {}.",
                    if instance_host.is_empty() {
                        "the instance configured in Settings"
                    } else {
                        instance_host
                    }
                );
            }
            let path = path.split(['?', '#']).next().unwrap_or_default();
            let path = path.trim_matches('/');
            let prefix = instance_path.trim_matches('/');
            let path = if prefix.is_empty() {
                path
            } else {
                path.strip_prefix(prefix)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .unwrap_or_default()
            };
            let Some((project, rest)) = path.split_once("/-/merge_requests/") else {
                anyhow::bail!("This link is not a merge request.");
            };
            (project, rest.split('/').next().unwrap_or_default())
        } else if let Some((project, iid)) = input.rsplit_once('!') {
            (project, iid)
        } else {
            anyhow::bail!("Paste a merge request link or a reference such as group/project!123.");
        };
        let iid: u64 =
            iid.parse().ok().filter(|iid| *iid > 0).ok_or_else(|| {
                anyhow::anyhow!("The merge request number is missing or invalid.")
            })?;
        let valid = project.contains('/')
            && project.split('/').all(|segment| {
                !segment.is_empty()
                    && segment != "."
                    && segment != ".."
                    && segment
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            });
        if !valid {
            anyhow::bail!("The project path is invalid: use group/project.");
        }
        Ok(Self {
            project: project.to_owned(),
            iid,
        })
    }
}

/// `https://host:port/path` as (`host:port`, `/path`).
fn split_url(url: &str) -> (&str, &str) {
    let rest = url.split_once("://").map_or("", |(_, rest)| rest);
    match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MrId;

    fn mr(iid: u64, title: &str, labels: &[&str], ticket: &[&str]) -> MergeRequest {
        MergeRequest {
            risk: None,
            id: MrId { project: 1, iid },
            title: title.into(),
            description: "Mentions billing only in the description".into(),
            project: "acme/platform".into(),
            author: "alice".into(),
            web_url: String::new(),
            sha: String::new(),
            base_sha: String::new(),
            created_at: chrono::Utc::now(),
            labels: labels.iter().map(|&name| name.into()).collect(),
            ticket_labels: ticket.iter().map(|&name| name.into()).collect(),
            pipeline: None,
            approvals: None,
            changed_lines: None,
        }
    }

    #[test]
    fn every_query_word_must_match_reference_title_author_or_label() {
        let queue = [
            mr(231, "Create recurring invoices", &["P::1"], &["billing"]),
            mr(238, "Cancel an invoice", &["Type::Bug"], &[]),
        ];
        let matching = |query: &str| -> Vec<u64> {
            let filter = QueueFilter {
                query: query.into(),
                labels: vec![],
            };
            queue
                .iter()
                .filter(|mr| filter.matches(mr))
                .map(|mr| mr.id.iid)
                .collect()
        };
        assert_eq!(matching(""), [231, 238]);
        assert_eq!(matching("  INVOICE "), [231, 238]);
        assert_eq!(matching("invoice recurring"), [231]);
        assert_eq!(matching("!238"), [238]);
        assert_eq!(matching("billing"), [231], "ticket labels are searchable");
        assert_eq!(matching("type::bug alice"), [238]);
        assert!(
            matching("description").is_empty(),
            "descriptions are not searched"
        );
    }

    #[test]
    fn selected_labels_must_all_be_present_and_toggle_off() {
        let queue = [
            mr(1, "a", &["P::1", "backend"], &[]),
            mr(2, "b", &["backend"], &["P::1"]),
            mr(3, "c", &["backend"], &[]),
        ];
        let mut filter = QueueFilter::default();
        assert!(!filter.is_active());
        filter.toggle_label("P::1");
        filter.toggle_label("backend");
        assert!(filter.is_active());
        let ids: Vec<u64> = queue
            .iter()
            .filter(|mr| filter.matches(mr))
            .map(|mr| mr.id.iid)
            .collect();
        assert_eq!(ids, [1, 2]);
        filter.toggle_label("P::1");
        assert_eq!(filter.labels, ["backend"]);
    }

    #[test]
    fn facets_count_each_mr_once_and_list_priorities_first() {
        let queue = [
            mr(1, "a", &["backend", "frontend"], &["backend"]),
            mr(2, "b", &["backend", "P::2"], &[]),
            mr(3, "c", &["frontend", "P0"], &[]),
            mr(4, "d", &["backend"], &[]),
        ];
        let facets: Vec<(String, usize)> = label_facets(&queue)
            .into_iter()
            .map(|(label, count)| (label.name, count))
            .collect();
        assert_eq!(
            facets,
            [
                ("P0".to_string(), 1),
                ("P::2".to_string(), 1),
                ("backend".to_string(), 3),
                ("frontend".to_string(), 2),
            ]
        );
    }

    #[test]
    fn references_accept_instance_links_and_short_forms() {
        let instance = "https://gitlab.example.com";
        for (input, project, iid) in [
            (
                "https://gitlab.example.com/acme/platform/-/merge_requests/231",
                "acme/platform",
                231,
            ),
            (
                " https://GitLab.example.com/acme/sub/web-app/-/merge_requests/7/diffs?view=inline#note_1 ",
                "acme/sub/web-app",
                7,
            ),
            ("acme/platform!42", "acme/platform", 42),
        ] {
            assert_eq!(
                MrReference::parse(input, instance).unwrap(),
                MrReference {
                    project: project.into(),
                    iid
                },
                "{input}"
            );
        }
        let prefixed = MrReference::parse(
            "https://example.com/gitlab/acme/platform/-/merge_requests/3",
            "https://example.com/gitlab/",
        )
        .unwrap();
        assert_eq!(prefixed.project, "acme/platform");
    }

    #[test]
    fn references_reject_other_hosts_and_unsafe_or_incomplete_input() {
        let instance = "https://gitlab.example.com";
        let error = |input: &str| MrReference::parse(input, instance).unwrap_err().to_string();
        assert!(
            error("https://gitlab.example.com.evil.invalid/a/b/-/merge_requests/1")
                .contains("another GitLab instance")
        );
        assert!(error("https://gitlab.example.com/a/b/-/issues/1").contains("not a merge request"));
        assert!(error("acme/platform!").contains("number"));
        assert!(error("acme/platform!0").contains("number"));
        assert!(error("platform!12").contains("project path"));
        assert!(error("acme/../admin!12").contains("project path"));
        assert!(error("acme/plat%2Fform!12").contains("project path"));
        assert!(error("231").contains("Paste"));
        assert!(MrReference::parse("acme/platform!1", "").is_ok());
        assert!(
            MrReference::parse("https://gitlab.example.com/a/b/-/merge_requests/1", "")
                .unwrap_err()
                .to_string()
                .contains("configured in Settings")
        );
    }
}
