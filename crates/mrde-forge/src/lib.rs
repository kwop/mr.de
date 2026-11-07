use async_trait::async_trait;
use mrde_core::{config::MrRole, insights::Overlap, models::*};

pub mod gitlab;
pub mod uploads;

#[async_trait]
pub trait Forge: Send + Sync {
    /// Authenticated image downloads are restricted to this forge's upload API.
    async fn download_image(&self, _url: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("This forge does not support image downloads")
    }
    async fn test_connection(&self) -> anyhow::Result<String>;
    /// Open, non-draft merge requests where the current user holds any of `roles`.
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
