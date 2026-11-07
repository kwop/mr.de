//! Results of the worktree analysis that helps understand complex merge requests.
use crate::models::DiffStats;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Insights {
    pub sha: String,
    pub stats: Option<DiffStats>,
    pub symbols: Vec<SymbolChange>,
    pub impacts: Vec<Impact>,
    pub graph: FileGraph,
    pub removed: Vec<RemovedBlock>,
    pub history: Vec<FileHistory>,
    pub experts: Vec<Expert>,
    pub contracts: Vec<ContractChange>,
    pub dependencies: Vec<DependencyChange>,
    pub lockfiles: Vec<LockfileChange>,
    pub patterns: Vec<PatternFinding>,
    /// `None` when GitLab could not be queried; empty when nothing overlaps.
    pub overlaps: Option<Vec<Overlap>>,
    /// Steps that failed without invalidating the rest of the analysis.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Type,
    Class,
    Constant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolChangeKind {
    SignatureChanged,
    Modified,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolChange {
    pub path: String,
    pub name: String,
    pub kind: SymbolKind,
    pub change: SymbolChangeKind,
    pub before: Option<String>,
    pub after: Option<String>,
    pub added: u32,
    pub removed: u32,
    /// Line of the definition in the new version, or the old one when removed.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSite {
    pub path: String,
    pub line: u32,
    pub text: String,
    pub in_mr: bool,
    pub test: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Impact {
    pub symbol: String,
    pub path: String,
    pub change: SymbolChangeKind,
    pub callers: Vec<CallSite>,
    /// Matches beyond the displayed callers.
    pub more: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileGraph {
    pub nodes: Vec<String>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
    /// `import`, or `http` for a call to an endpoint declared in the MR.
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameCommit {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub summary: String,
    /// `#123` issues and `!45` merge requests mentioned in the commit summary.
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedBlock {
    pub path: String,
    /// First removed line in the base version.
    pub line: u32,
    pub lines: Vec<String>,
    /// Nothing was added in place of the removed lines.
    pub pure_deletion: bool,
    pub origin: Option<BlameCommit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHistory {
    pub path: String,
    pub commits: u32,
    pub fixes: u32,
    pub authors: u32,
    pub last_fix: Option<String>,
    pub risk: Level,
    pub new_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expert {
    pub zone: String,
    /// Authors with their share of the zone's recent commits, in percent.
    pub people: Vec<(String, u32)>,
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractCategory {
    Database,
    Http,
    Configuration,
    Permissions,
    Schema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractSeverity {
    Breaking,
    Review,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractChange {
    pub category: ContractCategory,
    pub severity: ContractSeverity,
    pub title: String,
    pub detail: String,
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyChangeKind {
    Added,
    Removed,
    Upgraded,
    Downgraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyChange {
    pub manifest: String,
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub kind: DependencyChangeKind,
    pub major: bool,
    pub dev: bool,
}

/// Resolved (mostly transitive) packages added and removed in a lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockfileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternFinding {
    pub level: Level,
    pub rule: String,
    pub title: String,
    pub path: String,
    pub line: u32,
    pub snippet: String,
    pub advice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlap {
    pub iid: u64,
    pub title: String,
    pub merged: bool,
    pub web_url: String,
    pub files: Vec<String>,
    pub updated_at: String,
}

/// Claude's guided tour of the change, with a Mermaid diagram of the main flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tour {
    pub minutes: u32,
    pub steps: Vec<TourStep>,
    #[serde(default)]
    pub outside: Vec<String>,
    #[serde(default)]
    pub diagram: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TourStep {
    pub title: String,
    pub summary: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub core: bool,
}

/// Before/after screenshots produced by the configured visual command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisualReport {
    pub screens: Vec<VisualScreen>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualScreen {
    pub name: String,
    pub before: Option<std::path::PathBuf>,
    pub after: Option<std::path::PathBuf>,
    pub diff: Option<std::path::PathBuf>,
    /// Share of pixels that differ, from 0 to 1; `None` when a side is missing.
    pub changed: Option<f64>,
}
