# Merge request risk radar

Open **Changes** and select **Calculate risk**. MR.DE prepares the MR worktree when needed,
then measures the base-to-head change locally. It never starts Claude for this calculation.
The experimental tab keeps the existing Files, Statistics, and History tabs available.
Click a disktree file or a file row to see its recent commits, line ownership percentages
from `git blame`, and related risk findings. Click an axis for evidence and **Ask Claude
about this axis** to open an explicit, interactive, read-only discussion in Threads.

## Configuration

Copy [the commented example](../config/risk.example.toml) to `~/.config/mrde/risk.toml`.
Overrides merge in this order:

1. Built-in defaults.
2. `~/.config/mrde/risk.toml`.
3. `~/.config/mrde/projects/<project-id>/risk.toml`.
4. `risk.toml` in the MR repository.

Tables merge; arrays replace the earlier array. Unknown fields, invalid curves, globs,
regexes, and verdict thresholds are rejected. Coverage paths are repository-relative.
Project IDs are the IDs in `projects.toml`, not GitLab repository IDs.

Local command definitions are trusted. Commands introduced or changed by repository
configuration remain **Proposed** until **Allow and calculate** approves their exact
argument vector and parser. Approval is scoped to the MR.DE project. Programs receive
arguments directly, without an implicit shell. Install Lizard yourself or configure its
absolute executable path; a missing executable leaves complexity unavailable.

Each external tool runs through `mrde-checks` in its own disposable base/head snapshots.
Timeouts and cancellation kill its process group. Output is bounded; package-manager
offline flags are set. No packages are installed automatically. Git, HTTP, disk reads,
and subprocesses stay off the GPUI thread. Scoring itself has no network client or Git
fetch; initial worktree preparation uses the existing SSH workflow.

## Reading the measurements

Every axis runs from 0 to 10: higher means riskier. Piecewise-linear normalization clamps
outside the configured endpoints. The default review depth is **Deep** when any known
axis reaches 8 or the known-axis mean reaches 6; **Standard** starts at 3.5, otherwise
**Quick**. Missing axes stay **n/a**, are excluded from means, and make the verdict partial.
These measurements guide review depth; they do not establish correctness or safety.

| Axis | Measurement and limits |
| --- | --- |
| Diff size / files | Merge-base-to-head `git diff --numstat`; renames count once. Binary files have no line counts. |
| Complexity | Net cyclomatic complexity delta from Lizard XML; negative deltas normalize to zero. Missing reports for any changed source file make this axis unavailable. |
| Test gap | Uncovered changed executable lines from LCOV/Cobertura. Report freshness cannot be proven. Without a report, an explicitly marked test-file/source-file ratio heuristic is used. |
| Sensitive paths | Changed-line share weighted by the highest matching glob, without counting overlapping rules twice. |
| Coupling | Added Cargo/npm/Dart/Go dependencies plus new textual import targets; explicitly heuristic, not a compiler-resolved dependency graph. |
| Conventions | New diagnostics by path, rule and message compared with base. Line shifts alone do not add warnings. JSON, Clippy, ESLint and regex output are supported. |
| Debt | Weighted regex matches on added lines; matches may include comments and strings. |

Click **Update team median** to request the most recent merged MRs from GitLab, ordered
by merge time (GitLab versions supporting `order_by=merged_at`). The default sample is 30,
scoped to the GitLab repository and MR.DE project. Only commits already present in the
local object database are measured; missing commits are reported in the debug log and
excluded per axis. The radar shows each axis's sample count. Historical test gaps use
the file-ratio heuristic because a current coverage report cannot describe old commits.
A sparse baseline is not representative of the whole team.

The blue polygon describes this MR; dashed gray is the median. Missing axes leave gaps,
and a partial profile has no filled polygon. Queue miniatures use cached scores only;
**Risk** sorts measured MRs from highest mean to lowest, with unavailable scores last.
SQLite stores full profiles by project, repository, MR, base/head SHA, and configuration
fingerprint. **Calculate risk** explicitly recomputes when local reports or tools change.

## Validation

`cargo test --locked -p mrde-risk` includes diff fixtures, all scorer types, report parsers,
and a two-commit repository with a dirty worktree and a timed-out tool.
`cargo test --locked -p mrde --lib` covers axis selection, calculation, file history and
image/reader scrolling. Preview the panel with `MRDE_SCENARIO=focus:changes cargo run
--locked -p mrde -- --ui-preview`. The scorer crate and native drawing use portable APIs;
the complete desktop application's existing macOS integrations are not Linux-validated.

Preview variations: `MRDE_SCENARIO_FILE=first` opens file ownership/history;
`MRDE_SCENARIO_AXIS=lint` shows an unavailable axis; `MRDE_SCENARIO_THEME=light`
checks the radar component's light palette. `MRDE_SCENARIO_RISK_FILE=<report.json>`
loads a locally measured report in preview mode only, without credentials.

```mermaid
flowchart LR
  WT[Local MR commits] --> S[Eight isolated scorers]
  CFG[risk.toml] --> S
  S --> N[Piecewise normalization]
  N --> P[RiskProfile]
  P --> DB[(SQLite cache)]
  H[GitLab merged MR metadata] --> B[Local historical profiles]
  B --> M[Per-axis median and sample count]
  P --> R[Native GPUI radar and evidence]
  M --> R
  P --> Q[Verdict and risk queue order]
```
