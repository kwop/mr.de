use crate::{
    config::QueueSort,
    models::{Label, MergeRequest},
};
use chrono::{DateTime, Utc};

/// Lower Px values rank first. Unlabelled requests follow explicit priorities.
/// The tuple keeps age and size from accidentally outranking urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score {
    pub priority: std::cmp::Reverse<u32>,
    pub readiness: u8,
    pub age_hours: i64,
    pub small_diff: u64,
}

pub fn priority<'a>(labels: impl IntoIterator<Item = &'a Label>) -> u32 {
    labels
        .into_iter()
        .filter_map(|label| {
            let number = label
                .name
                .strip_prefix("P::")
                .or_else(|| label.name.strip_prefix('P'))?;
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            number.parse().ok()
        })
        .min()
        .unwrap_or(u32::MAX)
}

pub fn score(mr: &MergeRequest, now: DateTime<Utc>) -> Score {
    Score {
        priority: std::cmp::Reverse(priority(mr.all_labels())),
        readiness: match mr.pipeline.as_deref() {
            Some("success") => 3,
            Some("running" | "pending") => 2,
            None => 1,
            _ => 0,
        },
        age_hours: (now - mr.created_at).num_hours().clamp(0, 24 * 30),
        small_diff: mr
            .changed_lines
            .map(|n| 10_000 - n.min(10_000))
            .unwrap_or(0),
    }
}

pub fn sort_queue(queue: &mut [MergeRequest], sort: QueueSort, now: DateTime<Utc>) {
    queue.sort_by(|a, b| {
        match sort {
            QueueSort::Risk => {
                let risk = |mr: &MergeRequest| mr.risk.as_ref().filter(|r| r.head_sha == mr.sha).and_then(|r| r.mean).filter(|n| n.is_finite());
                match (risk(a), risk(b)) { (Some(a),Some(b)) => b.total_cmp(&a), (Some(_),None) => std::cmp::Ordering::Less, (None,Some(_)) => std::cmp::Ordering::Greater, _ => score(b,now).cmp(&score(a,now)) }
            }
            QueueSort::Priority => score(b, now).cmp(&score(a, now)),
            QueueSort::Newest => b.created_at.cmp(&a.created_at),
            QueueSort::Oldest => a.created_at.cmp(&b.created_at),
            // Unknown sizes go last rather than being treated as empty diffs.
            QueueSort::Smallest => a
                .changed_lines
                .unwrap_or(u64::MAX)
                .cmp(&b.changed_lines.unwrap_or(u64::MAX)),
        }
        .then_with(|| a.id.project.cmp(&b.id.project))
        .then_with(|| a.id.iid.cmp(&b.id.iid))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_and_scoped_priority_labels_count() {
        assert_eq!(priority(&["backend".into(), "P10".into(), "P2".into()]), 2);
        assert_eq!(
            priority(&["backend".into(), "P-1".into(), "P0 urgent".into()]),
            u32::MAX
        );
        assert_eq!(priority(&["P1".into(), "P0".into()]), 0);
        assert_eq!(priority(&["P::0".into(), "P1".into()]), 0);
        assert_eq!(
            priority(&["Type::P0".into(), "P::".into(), "P::1::2".into()]),
            u32::MAX
        );
    }
    #[test]
    fn ticket_labels_set_priority_when_the_mr_has_none() {
        let issue = |labels: &[&str]| crate::models::Issue {
            title: "Ticket".into(),
            description: String::new(),
            web_url: "https://example.invalid/issues/1".into(),
            labels: labels.iter().map(|l| (*l).into()).collect(),
            project_id: Some(2),
        };
        let own: Vec<Label> = vec!["Review::Dev".into()];
        let tickets = crate::models::ticket_labels(
            &own,
            &[
                issue(&["P::0", "Review::Dev"]),
                issue(&["P::0", "Type::Feature"]),
            ],
        );
        assert!(crate::models::ticket_labels(&own, &[]).is_empty());
        let names: Vec<_> = tickets.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            ["P::0", "Type::Feature"],
            "deduplicated, MR labels excluded"
        );
        assert_eq!(priority(own.iter().chain(&tickets)), 0);
    }
    #[test]
    fn urgency_dominates_other_signals() {
        let urgent = Score {
            priority: std::cmp::Reverse(0),
            readiness: 0,
            age_hours: 0,
            small_diff: 0,
        };
        let ready = Score {
            priority: std::cmp::Reverse(1),
            readiness: 3,
            age_hours: 720,
            small_diff: 10_000,
        };
        assert!(urgent > ready);
    }
    #[test]
    fn user_selected_sort_orders_queue() {
        let mr = |iid: u64, day: u32, lines: Option<u64>, labels: &[&str]| MergeRequest {
            risk: None,
            id: crate::models::MrId { project: 1, iid },
            title: String::new(),
            description: String::new(),
            project: String::new(),
            author: String::new(),
            web_url: String::new(),
            sha: String::new(),
            base_sha: String::new(),
            created_at: format!("2026-01-{day:02}T00:00:00Z").parse().unwrap(),
            labels: labels.iter().map(|l| (*l).into()).collect(),
            ticket_labels: vec![],
            pipeline: None,
            approvals: None,
            changed_lines: lines,
        };
        let now = "2026-02-01T00:00:00Z".parse().unwrap();
        let base = [
            mr(1, 10, Some(500), &[]),
            mr(2, 20, None, &["P0"]),
            mr(3, 5, Some(20), &[]),
        ];
        let order = |sort| {
            let mut queue = base.clone();
            sort_queue(&mut queue, sort, now);
            queue.iter().map(|mr| mr.id.iid).collect::<Vec<_>>()
        };
        assert_eq!(order(QueueSort::Priority), [2, 3, 1]);
        assert_eq!(order(QueueSort::Newest), [2, 1, 3]);
        assert_eq!(order(QueueSort::Oldest), [3, 1, 2]);
        assert_eq!(order(QueueSort::Smallest), [3, 1, 2]);
    }
}
