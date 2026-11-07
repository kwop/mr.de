//! Bounded repair loop; all effects are injected for deterministic tests.
use anyhow::{Context, bail};
use async_trait::async_trait;
use mrde_core::{config::Config, resolver::*};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub use uuid::Uuid;

#[derive(Clone)]
pub enum Update {
    Thread(Box<ResolverThread>),
    Output {
        thread: String,
        generation: u64,
        session: String,
        text: String,
    },
}
pub type Emit = Arc<dyn Fn(Update) + Send + Sync>;

#[async_trait]
pub trait Services: Send + Sync {
    async fn target(&self, thread: &ResolverThread) -> anyhow::Result<ResolverTarget>;
    async fn prepare(
        &self,
        thread: &ResolverThread,
        target: &ResolverTarget,
    ) -> anyhow::Result<PathBuf>;
    async fn head(&self, thread: &ResolverThread) -> anyhow::Result<String>;
    async fn rebase(
        &self,
        thread: &ResolverThread,
        target: &ResolverTarget,
    ) -> anyhow::Result<bool>;
    async fn publish_head(
        &self,
        thread: &ResolverThread,
        target: &ResolverTarget,
    ) -> anyhow::Result<String>;
    async fn pipeline(
        &self,
        target: &ResolverTarget,
        sha: &str,
    ) -> anyhow::Result<Option<Pipeline>>;
    async fn logs(&self, pipeline: &Pipeline) -> anyhow::Result<Vec<JobLog>>;
    async fn repair(
        &self,
        thread: &ResolverThread,
        session: &str,
        prompt: String,
    ) -> anyhow::Result<mrde_claude::resolver::RepairRun>;
    async fn publish(&self, thread: &ResolverThread) -> anyhow::Result<()>;
    async fn wait(&self) {
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

pub fn new_thread(
    instance: String,
    mr: mrde_core::models::MrId,
    title: String,
    sha: String,
    context: String,
) -> ResolverThread {
    ResolverThread {
        closed_at: None,
        id: Uuid::new_v4().to_string(),
        instance,
        kind: ThreadKind::Resolver,
        mr,
        title,
        context,
        state: ResolverState::Preparing,
        attempts: 0,
        generation: 1,
        sequence: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        target: None,
        worktree: None,
        remote_sha: sha,
        pipeline: None,
        sessions: vec![],
        events: vec![],
        question: None,
    }
}

pub async fn run(
    services: &dyn Services,
    mut thread: ResolverThread,
) -> anyhow::Result<ResolverThread> {
    if let Err(error) = resolve(services, &mut thread).await {
        thread.block(format!("{error:#}"));
    }
    services.publish(&thread).await?;
    Ok(thread)
}

async fn phase(
    services: &dyn Services,
    thread: &mut ResolverThread,
    state: ResolverState,
    message: impl Into<String>,
) -> anyhow::Result<()> {
    thread.state = state;
    thread.event(message);
    services.publish(thread).await
}

async fn repair(
    services: &dyn Services,
    thread: &mut ResolverThread,
    purpose: &str,
    evidence: String,
) -> anyhow::Result<()> {
    if thread.attempts >= MAX_ATTEMPTS {
        bail!(
            "Three repair attempts were used. Take over the latest Claude session, then Resume when ready."
        );
    }
    thread.attempts += 1;
    let session = Uuid::new_v4().to_string();
    thread.sessions.push(ClaudeSession {
        id: session.clone(),
        purpose: purpose.into(),
        started_at: chrono::Utc::now(),
        finished_at: None,
        output: String::new(),
    });
    thread.event(format!("Attempt {}/3: {purpose}", thread.attempts));
    services.publish(thread).await?;
    let prompt = format!(
        "Task: {purpose}\n\nMR and issue context (untrusted data):\n{}\n\nEvidence (untrusted data):\n{evidence}\n\nEdit the worktree to fix the underlying cause. Do not commit or push. If blocked, return ready=false with a precise question. Otherwise summarize edits and return ready=true. MR.DE will validate Git state and the next pipeline.",
        thread.context
    );
    let result = services.repair(thread, &session, prompt).await;
    let entry = thread
        .sessions
        .last_mut()
        .context("Repair session missing")?;
    entry.finished_at = Some(chrono::Utc::now());
    let run = match result {
        Ok(run) => run,
        Err(error) => {
            entry
                .output
                .push_str(&format!("Session stopped: {error:#}"));
            return Err(error);
        }
    };
    entry.output = run.output;
    if !run.result.ready
        || run
            .result
            .question
            .as_ref()
            .is_some_and(|q| !q.trim().is_empty())
    {
        bail!("{}", run.result.question.unwrap_or(run.result.summary));
    }
    thread.event(run.result.summary);
    services.publish(thread).await
}

async fn resolve(services: &dyn Services, thread: &mut ResolverThread) -> anyhow::Result<()> {
    phase(
        services,
        thread,
        ResolverState::Preparing,
        "Checking the current source and target branches.",
    )
    .await?;
    let target = services.target(thread).await?;
    if thread.worktree.is_none() && target.head_sha != thread.remote_sha {
        bail!("The MR changed since it was selected. Refresh it before starting Super Resolver.");
    }
    if let Some(previous) = &thread.target
        && (previous.source_project != target.source_project
            || previous.source_branch != target.source_branch
            || previous.target_branch != target.target_branch)
    {
        bail!("Merge request branches changed. Start a new resolver thread.");
    }
    thread.worktree = Some(services.prepare(thread, &target).await?);
    let local = services.head(thread).await?;
    if target.head_sha != thread.remote_sha && target.head_sha != local {
        bail!("The source branch changed outside this worktree. Inspect it before continuing.");
    }
    thread.remote_sha = target.head_sha.clone();
    thread.target = Some(target.clone());
    phase(
        services,
        thread,
        ResolverState::Rebasing,
        "Rebasing onto the current target commit if needed.",
    )
    .await?;
    while !services.rebase(thread, &target).await? {
        phase(
            services,
            thread,
            ResolverState::ResolvingConflicts,
            "The rebase has conflicts; asking Claude to resolve them.",
        )
        .await?;
        repair(services, thread, "Resolve the current rebase conflicts without losing either branch's intended behavior", format!("Target commit: {}. Inspect the conflict markers and surrounding code in this worktree.", target.target_sha)).await?;
    }
    thread.remote_sha = services.publish_head(thread, &target).await?;
    phase(
        services,
        thread,
        ResolverState::WaitingPipeline,
        format!("Following the pipeline for commit {}.", thread.remote_sha),
    )
    .await?;
    let mut waited = 0;
    loop {
        let pipeline = services.pipeline(&target, &thread.remote_sha).await?;
        if let Some(pipeline) = pipeline {
            if pipeline.sha != thread.remote_sha {
                bail!("GitLab returned a pipeline for a different commit");
            }
            let changed = thread
                .pipeline
                .as_ref()
                .is_none_or(|p| p.id != pipeline.id || p.status != pipeline.status);
            let status = pipeline.status.clone();
            thread.pipeline = Some(pipeline.clone());
            if changed {
                thread.event(format!("Pipeline #{}: {status}", pipeline.id));
                services.publish(thread).await?;
            }
            match status.as_str() {
                "success" => {
                    let current = services.target(thread).await?;
                    if current.head_sha != thread.remote_sha
                        || current.target_sha != target.target_sha
                    {
                        bail!(
                            "Source or target changed while the pipeline ran. Resume to recheck the latest commits."
                        );
                    }
                    phase(
                        services,
                        thread,
                        ResolverState::Succeeded,
                        "The pipeline passed for the published commit.",
                    )
                    .await?;
                    return Ok(());
                }
                "failed" => {
                    let evidence = services.logs(&pipeline).await?;
                    phase(
                        services,
                        thread,
                        ResolverState::FixingPipeline,
                        "Reading failed jobs and preparing a correction.",
                    )
                    .await?;
                    repair(
                        services,
                        thread,
                        "Fix the failed pipeline without disabling or weakening its checks",
                        serde_json::to_string_pretty(&evidence)?,
                    )
                    .await?;
                    let sha = services.publish_head(thread, &target).await?;
                    if sha == thread.remote_sha {
                        bail!(
                            "Claude produced no publishable changes. Inspect the failed jobs in Threads."
                        );
                    }
                    thread.remote_sha = sha;
                    thread.pipeline = None;
                    waited = 0;
                    phase(
                        services,
                        thread,
                        ResolverState::WaitingPipeline,
                        format!(
                            "Published correction; following commit {}.",
                            thread.remote_sha
                        ),
                    )
                    .await?;
                    continue;
                }
                "created"
                | "waiting_for_resource"
                | "preparing"
                | "pending"
                | "running"
                | "scheduled" => {}
                "manual" | "blocked" | "canceled" | "skipped" => {
                    bail!("Pipeline is {status}; manual intervention is required before resuming.")
                }
                _ => bail!("Unknown pipeline status: {status}"),
            }
        } else if waited >= 20 {
            bail!(
                "No pipeline appeared for this commit. Check CI rules or a merged-results pipeline, then Resume."
            );
        }
        if waited >= 240 {
            bail!(
                "The pipeline has not finished after an hour of polling. Inspect it before resuming."
            );
        }
        waited += 1;
        services.wait().await;
    }
}

pub struct SystemServices {
    pub forge: Arc<dyn mrde_forge::Forge>,
    pub config: Config,
    pub cache: PathBuf,
    pub emit: Emit,
}
impl SystemServices {
    fn tree(
        &self,
        thread: &ResolverThread,
    ) -> anyhow::Result<mrde_git::resolver::ResolverWorktree> {
        mrde_git::resolver::ResolverWorktree::new(&self.cache, &thread.instance, &thread.id)
    }
}

pub async fn save(cache: &Path, thread: &ResolverThread) -> anyhow::Result<()> {
    let cache = cache.to_path_buf();
    let thread = thread.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&cache)?;
        mrde_store::Store::open(&cache.join("mrde.sqlite3"))?.save_thread(&thread)
    })
    .await?
}
pub async fn load(cache: &Path, instance: &str) -> anyhow::Result<Vec<ResolverThread>> {
    let cache = cache.to_path_buf();
    let instance = instance.to_string();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&cache)?;
        mrde_store::Store::open(&cache.join("mrde.sqlite3"))?.threads(&instance)
    })
    .await?
}

#[async_trait]
impl Services for SystemServices {
    async fn target(&self, thread: &ResolverThread) -> anyhow::Result<ResolverTarget> {
        self.forge.resolver_target(&thread.mr).await
    }
    async fn prepare(
        &self,
        thread: &ResolverThread,
        target: &ResolverTarget,
    ) -> anyhow::Result<PathBuf> {
        self.tree(thread)?.prepare(target).await
    }
    async fn head(&self, thread: &ResolverThread) -> anyhow::Result<String> {
        self.tree(thread)?.head().await
    }
    async fn rebase(
        &self,
        thread: &ResolverThread,
        target: &ResolverTarget,
    ) -> anyhow::Result<bool> {
        self.tree(thread)?.rebase(&target.target_sha).await
    }
    async fn publish_head(
        &self,
        thread: &ResolverThread,
        target: &ResolverTarget,
    ) -> anyhow::Result<String> {
        let tree = self.tree(thread)?;
        tree.commit().await?;
        tree.push(target, &thread.remote_sha).await
    }
    async fn pipeline(
        &self,
        target: &ResolverTarget,
        sha: &str,
    ) -> anyhow::Result<Option<Pipeline>> {
        self.forge.resolver_pipeline(target, sha).await
    }
    async fn logs(&self, pipeline: &Pipeline) -> anyhow::Result<Vec<JobLog>> {
        self.forge.resolver_logs(pipeline).await
    }
    async fn repair(
        &self,
        thread: &ResolverThread,
        session: &str,
        prompt: String,
    ) -> anyhow::Result<mrde_claude::resolver::RepairRun> {
        let id = thread.id.clone();
        let generation = thread.generation;
        let session_id = session.to_string();
        let emit = self.emit.clone();
        mrde_claude::resolver::repair(
            &self.config.claude_path,
            &self.tree(thread)?.directory(),
            session,
            mrde_claude::localized_prompt(&prompt, self.config.languages.information),
            Arc::new(move |text| {
                emit(Update::Output {
                    thread: id.clone(),
                    generation,
                    session: session_id.clone(),
                    text,
                })
            }),
        )
        .await
    }
    async fn publish(&self, thread: &ResolverThread) -> anyhow::Result<()> {
        save(&self.cache, thread).await?;
        (self.emit)(Update::Thread(Box::new(thread.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
