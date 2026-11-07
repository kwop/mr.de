use async_trait::async_trait;
use mrde_core::{config::MrRole, insights::Overlap, models::*};

pub mod github;
pub mod gitlab;
pub mod uploads;

#[async_trait]
pub trait Forge: Send + Sync {
    /// Authenticated image downloads are restricted to this forge's upload API.
    async fn download_image(&self, _url: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("This forge does not support image downloads")
    }
    async fn resolver_target(
        &self,
        _id: &MrId,
    ) -> anyhow::Result<mrde_core::resolver::ResolverTarget> {
        anyhow::bail!("This forge does not support Super Resolver")
    }
    async fn resolver_pipeline(
        &self,
        _target: &mrde_core::resolver::ResolverTarget,
        _sha: &str,
    ) -> anyhow::Result<Option<mrde_core::resolver::Pipeline>> {
        anyhow::bail!("This forge cannot inspect pipelines")
    }
    async fn resolver_logs(
        &self,
        _pipeline: &mrde_core::resolver::Pipeline,
    ) -> anyhow::Result<Vec<mrde_core::resolver::JobLog>> {
        anyhow::bail!("This forge cannot read pipeline logs")
    }
    /// Most recently merged MRs in this repository, ordered by merge time.
    async fn merged_history(&self, _project: u64, _limit: usize) -> anyhow::Result<Vec<MrId>> {
        anyhow::bail!("This forge cannot list merged history")
    }
    async fn test_connection(&self) -> anyhow::Result<String>;
    /// Numeric id of the project at `path` (`group/project`), for pasted MR links.
    async fn project_id(&self, _path: &str) -> anyhow::Result<u64> {
        anyhow::bail!("This forge cannot look up projects by path")
    }
    /// Open merge requests, including drafts, where the current user holds any of `roles`.
    async fn queue(&self, roles: &[MrRole]) -> anyhow::Result<Vec<MergeRequest>>;
    /// Current MR metadata and approvals, without diffs, issues, or project lookups.
    async fn summary(&self, id: &MrId) -> anyhow::Result<MergeRequest>;
    /// `None` when approvals are unavailable to the current user.
    async fn approval_count(&self, id: &MrId) -> anyhow::Result<Option<u64>>;
    async fn detail(&self, id: &MrId) -> anyhow::Result<MrDetail>;
    async fn approve(&self, id: &MrId, sha: &str) -> anyhow::Result<()>;
    /// Open and recently merged MRs of the same project that touch any of `files`.
    async fn related(&self, _id: &MrId, _files: &[String]) -> anyhow::Result<Vec<Overlap>> {
        anyhow::bail!("This forge cannot list related merge requests")
    }
}
