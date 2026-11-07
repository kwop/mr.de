//! Pre-review orchestration independent of UI, network, Git and Claude adapters.
use crate::models::{MrDetail, Review, ReviewLevel};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

#[async_trait]
pub trait ReviewServices: Send + Sync {
    async fn latest(&self, selected: &MrDetail) -> anyhow::Result<MrDetail>;
    async fn cached(&self, detail: &MrDetail, level: ReviewLevel)
    -> anyhow::Result<Option<Review>>;
    async fn prepare(&self, detail: &MrDetail) -> anyhow::Result<PathBuf>;
    async fn analyze(
        &self,
        directory: &Path,
        detail: &MrDetail,
        level: ReviewLevel,
    ) -> anyhow::Result<Review>;
    async fn save(
        &self,
        detail: &MrDetail,
        level: ReviewLevel,
        review: &Review,
    ) -> anyhow::Result<()>;
}

pub async fn pre_review(
    services: &impl ReviewServices,
    selected: &MrDetail,
    level: ReviewLevel,
) -> anyhow::Result<Review> {
    let current = services.latest(selected).await?;
    if current.mr.sha != selected.mr.sha || current.mr.base_sha != selected.mr.base_sha {
        anyhow::bail!("MR changed. Select it again before starting the review.");
    }
    if let Some(review) = services.cached(&current, level).await? {
        return Ok(review);
    }
    let directory = services.prepare(&current).await?;
    let review = services.analyze(&directory, &current, level).await?;
    services.save(&current, level, &review).await?;
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MergeRequest, MrId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn detail() -> MrDetail {
        MrDetail {
            mr: MergeRequest {
                draft: false,
                risk: None,
                id: MrId { project: 1, iid: 2 },
                title: "Title".into(),
                description: String::new(),
                project: "team/project".into(),
                author: "alice".into(),
                web_url: "https://example.com/mr/2".into(),
                sha: "a".repeat(40),
                base_sha: "b".repeat(40),
                created_at: chrono::Utc::now(),
                labels: vec![],
                ticket_labels: vec![],
                pipeline: None,
                approvals: None,
                changed_lines: None,
            },
            files: vec![],
            issues: vec![],
            ssh_url: "git@example.com:team/project.git".into(),
        }
    }
    fn review() -> Review {
        Review {
            summary: "Summary".into(),
            intention: "Intent".into(),
            risks: vec![],
            priority_files: vec![],
            confidence: 0.8,
        }
    }
    struct Services {
        cached: bool,
        changed: bool,
        prepared: AtomicUsize,
        analyzed: AtomicUsize,
        saved: AtomicUsize,
    }
    impl Services {
        fn new(cached: bool, changed: bool) -> Self {
            Self {
                cached,
                changed,
                prepared: 0.into(),
                analyzed: 0.into(),
                saved: 0.into(),
            }
        }
    }
    #[async_trait]
    impl ReviewServices for Services {
        async fn latest(&self, selected: &MrDetail) -> anyhow::Result<MrDetail> {
            let mut detail = selected.clone();
            if self.changed {
                detail.mr.sha = "c".repeat(40);
            }
            Ok(detail)
        }
        async fn cached(&self, _: &MrDetail, _: ReviewLevel) -> anyhow::Result<Option<Review>> {
            Ok(self.cached.then(review))
        }
        async fn prepare(&self, _: &MrDetail) -> anyhow::Result<PathBuf> {
            self.prepared.fetch_add(1, Ordering::SeqCst);
            Ok("/fixture".into())
        }
        async fn analyze(&self, _: &Path, _: &MrDetail, _: ReviewLevel) -> anyhow::Result<Review> {
            self.analyzed.fetch_add(1, Ordering::SeqCst);
            Ok(review())
        }
        async fn save(&self, _: &MrDetail, _: ReviewLevel, _: &Review) -> anyhow::Result<()> {
            self.saved.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[tokio::test]
    async fn cache_hit_never_prepares_or_runs_claude() {
        let services = Services::new(true, false);
        pre_review(&services, &detail(), ReviewLevel::Standard)
            .await
            .unwrap();
        assert_eq!(services.prepared.load(Ordering::SeqCst), 0);
        assert_eq!(services.analyzed.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn changed_sha_is_rejected_before_cache_or_execution() {
        let services = Services::new(true, true);
        assert!(
            pre_review(&services, &detail(), ReviewLevel::Standard)
                .await
                .is_err()
        );
        assert_eq!(services.prepared.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn cache_miss_runs_once_and_saves() {
        let services = Services::new(false, false);
        pre_review(&services, &detail(), ReviewLevel::Standard)
            .await
            .unwrap();
        assert_eq!(services.prepared.load(Ordering::SeqCst), 1);
        assert_eq!(services.analyzed.load(Ordering::SeqCst), 1);
        assert_eq!(services.saved.load(Ordering::SeqCst), 1);
    }
}
