//! Durable Super Resolver state. The runner and terminal share these records.
use crate::models::MrId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_ATTEMPTS: u8 = 3;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolverState {
    Preparing,
    Rebasing,
    ResolvingConflicts,
    WaitingPipeline,
    FixingPipeline,
    Succeeded,
    NeedsInput,
    Stopped,
    Interrupted,
}
impl ResolverState {
    pub fn active(&self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Rebasing
                | Self::ResolvingConflicts
                | Self::WaitingPipeline
                | Self::FixingPipeline
        )
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::Rebasing => "Rebasing",
            Self::ResolvingConflicts => "Resolving conflicts",
            Self::WaitingPipeline => "Waiting for pipeline",
            Self::FixingPipeline => "Fixing pipeline",
            Self::Succeeded => "Pipeline passed",
            Self::NeedsInput => "Needs your input",
            Self::Stopped => "Stopped",
            Self::Interrupted => "Interrupted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub project: u64,
    pub sha: String,
    pub status: String,
    pub web_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolverTarget {
    pub id: MrId,
    pub source_project: u64,
    pub source_branch: String,
    pub target_branch: String,
    pub source_ssh: String,
    pub target_ssh: String,
    pub head_sha: String,
    pub target_sha: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobLog {
    pub name: String,
    pub trace: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaudeSession {
    pub id: String,
    pub purpose: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadEvent {
    pub at: DateTime<Utc>,
    pub message: String,
}

/// What a thread does: an automated repair loop, or questions to Claude Code only.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadKind {
    /// Super Resolver: rebases, repairs, pushes, and follows the pipeline.
    #[default]
    Resolver,
    /// A read-only Claude Code session about the merge request; never writes or pushes.
    Questions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolverThread {
    pub id: String,
    pub instance: String,
    /// Threads saved before kinds existed are resolver runs.
    #[serde(default)]
    pub kind: ThreadKind,
    pub mr: MrId,
    pub title: String,
    pub context: String,
    pub state: ResolverState,
    pub attempts: u8,
    /// Incremented on Resume/Stop; rejects late writes from an older worker.
    pub generation: u64,
    pub sequence: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub target: Option<ResolverTarget>,
    pub worktree: Option<PathBuf>,
    /// Last observed/pushed source SHA. Pushes use an explicit lease for this value.
    pub remote_sha: String,
    pub pipeline: Option<Pipeline>,
    pub sessions: Vec<ClaudeSession>,
    pub events: Vec<ThreadEvent>,
    pub question: Option<String>,
}
impl ResolverThread {
    /// Only resolver threads run, stop, and resume automation.
    pub fn automated(&self) -> bool {
        self.kind == ThreadKind::Resolver
    }
    pub fn label(&self) -> &'static str {
        match self.kind {
            ThreadKind::Resolver => self.state.label(),
            ThreadKind::Questions => "Questions",
        }
    }
    pub fn event(&mut self, message: impl Into<String>) {
        self.sequence += 1;
        self.updated_at = Utc::now();
        self.events.push(ThreadEvent {
            at: self.updated_at,
            message: message.into(),
        });
        if self.events.len() > 300 {
            self.events.remove(0);
        }
    }
    pub fn block(&mut self, question: impl Into<String>) {
        let question = question.into();
        self.state = ResolverState::NeedsInput;
        self.question = Some(question.clone());
        self.event(question);
    }
    pub fn resume(&mut self) {
        self.generation += 1;
        self.sequence = 0;
        self.attempts = 0;
        self.question = None;
        self.state = ResolverState::Preparing;
        self.event("Resumed: up to three additional repair attempts.");
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepairResult {
    pub ready: bool,
    pub summary: String,
    pub question: Option<String>,
}
