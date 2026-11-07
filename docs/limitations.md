# Known limitations

mr.de is experimental. These limits are known; others certainly exist.

## Platform and packaging

- macOS only. Linux is a possible future target, not a tested platform.
- Development-signed `.app` packaging is available; no notarized distribution, installer, or automatic updates.
- Live GitLab, Cloudflare Access, Keychain prompts, Claude login, and Ghostty must be
  validated with your own setup; automated tests use mocks.

## Projects and providers

- Multiple GitLab projects are supported, including separate accounts on one host and optional
  group scopes. GitHub profiles are configuration-only until its connector is implemented.
- Manually edited TOML requires a restart. Removing a profile is currently done in TOML;
  stored credentials, caches, and worktrees are retained.

## GitLab

- The queue is enriched four merge requests at a time. After a commit's first load, each
  refresh costs three concurrent requests per merge request (details, approvals, related
  issues), plus one label listing per project every ten minutes. HTTP 429 responses are
  reported, not retried.
- GitLab can truncate or omit diffs; an unknown size is shown as `?`, never as zero.
  Truncated diffs are reloaded on each refresh.
- The reader lists changed files; browsing diffs inside mr.de is not implemented.
- The search covers the queue already loaded: merge request numbers, titles, projects,
  authors, and labels, not descriptions or comments. Label filters list only the labels of
  the queue.
- The avatar shows the account name only; GitLab avatars are not downloaded.
- GitLab-specific Markdown extensions (mentions, references as links) are not rendered.
- Approval availability depends on your GitLab edition and project permissions.
- Credentials are read from the Keychain per project and kept in memory until the
  settings are saved again.

## Rendering

- Mermaid uses a pure-Rust renderer, not mermaid.js: only top-level ```` ```mermaid ````
  blocks are drawn, diagrams over 20,000 characters are refused, and unsupported syntax shows
  the error with the source. Diagram layout runs in the background with a loading placeholder.
- Markdown sections over 16 KiB use a 520 pt virtualized viewport with their own scrollbar.
  Very large individual tables or code blocks still require layout as one block.
- The Disktree draws at most 120 tiles; smaller files of each zone share one tile. Its
  estimate needs complete GitLab diffs; otherwise only **Compute statistics** can draw it.

## Analysis

- **Analyze** uses heuristics, not compilers or language servers. Definitions are found with
  line patterns and brace or indentation matching; callers are whole-word `git grep` matches
  (generic or short names are skipped, at most 12 symbols and 25 callers each); imports are
  resolved only for the supported patterns; HTTP links match route literals.
- Contract and pattern rules can report false positives and miss dynamic code.
- Risk levels use fixed thresholds (three fix commits or twenty changes in six months is
  high).
- At most 300 changed files of up to 512 KiB each are read; generated files and lockfiles are
  skipped.
- Overlapping merge requests cover the 30 most recently updated open merge requests and
  those merged in the last 14 days of the same project (first 100 files of each). Without
  GitLab access the analysis still runs and reports overlaps as unknown.

## Claude

- The guided tour and the compliance check are Claude's reading of the change, not verified
  facts. The compliance check reports the specification files, code locations, and tests it
  used so you can check them.
- Quality depends on the Claude Code version and model available to your account.

## Visual comparison and checks

- The visual comparison needs your own screenshot command; mr.de does not start servers or
  seed data. Images of different sizes are reported as changed without a difference image.
- Check definitions are parsed but never executed: there is no runner or approval UI yet.

## Super Resolver and embedded terminal

- Tracks source-commit pipelines. Merged-results or merge-train pipelines with a synthetic
  SHA require manual inspection; they are never mistaken for a passing source pipeline.
- Reads at most five required failed job traces (40 KB tails). Downstream/bridge pipelines
  and runner administration require manual intervention.
- Claude can edit files but cannot execute arbitrary shell tools in automatic repair mode.
  GitLab CI validates published corrections. Missing permissions, SSH authentication, signing
  configuration, quota limits, and ambiguous requirements stop the runner in Threads.
- Session output in SQLite keeps its last 128 KB; full conversation persistence is owned by
  Claude Code. Deleted/missing CLI sessions cannot be reconstructed by MR.DE.
- Resolver worktrees are retained and not yet included in the Worktrees cleanup UI.
- Terminal embedding is verified with a native macOS PTY and synthetic input. Actual Claude
  login, interactive permission prompts, and live GitLab repairs depend on the user's setup.
