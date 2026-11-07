# MR.DE contributor guide

Several agents work on this repository in parallel. Before editing, start your task in its
own worktree with the `isolated-worktree` skill (`.agents/skills/isolated-worktree/`); see
"Parallel Agents" in `AGENTS.md`. The main checkout only receives merges.

## Product

mr.de is a native Rust/GPUI desktop cockpit for reviewing GitLab merge requests.
Cargo package/library/binary: `mrde`. macOS only for the initial release.
All source, documentation, UI copy, prompts, and commit messages are English.
License: MIT.

## Workspace

- `crates/mrde-app`: GPUI views, runtime setup, application composition; package `mrde`.
- `crates/mrde-core`: serializable domain models, deterministic scoring, review workflow ports.
- `crates/mrde-forge`: `Forge` abstraction and GitLab REST API v4 implementation.
- `crates/mrde-git`: dedicated bare repositories, detached worktrees, and read-only Git queries.
- `crates/mrde-insights`: worktree analysis (symbols, impact, file graph, history, contracts,
  dependencies, patterns) and the screenshot comparison; pure functions plus `analyze()`.
- `crates/mrde-claude`: Claude Code CLI bridge, strict response parsing, questions context.
- `crates/mrde-checks`: check definitions, merge rules, standardized results and bounded execution of approved commands.
- `crates/mrde-resolver`: bounded repair orchestration, pipeline polling, persisted Threads sessions.
- `crates/mrde-risk`: local risk scorers and layered configuration.
- `crates/mrde-radar`: public headless scan service, isolated snapshots and text/JSON/SVG CLI; `plugins/mrde` integrates it with Claude Code.
- `crates/mrde-store`: versioned SQLite persistence and macOS Keychain access.

## Invariants

- Claude runs only after an explicit Run review, Check compliance, Generate tour, Ask Claude, or Super Resolver action. No automatic AI analysis. Analyze and Capture screenshots also run only on click.
- Super Resolver shares three repair attempts across conflicts and CI failures. Questions stop it. Push only the source branch with an explicit SHA lease; never auto-resume after restart.
- Stop the headless worker before interactive takeover; hide native terminal surfaces on navigation.
- Each review level has its own cache entry. Compliance, statistics, analysis, tour, and screenshot results apply only to the commit they were computed for.
- The screenshot command comes from the user's configuration only, never from the repository or MR content.
- Analysis uses read-only Git commands on validated SHAs; it never checks out, writes, or runs repository code.
- Use the user's Claude Code installation/subscription, never an Anthropic API key.
- Only exact numeric Px labels (`P0`, `P::0`) influence priority, from the MR or its primary ticket. Lower numbers rank higher; other labels never do.
- Read the MR lists for the configured roles (reviewer and assignee by default; author is
  optional; at least one is required), paginate, deduplicate, and label drafts explicitly.
- Never put MR, issue, or diff text in process arguments; use stdin or a file outside the worktree.
- Tokens and sensitive HTTP header values belong in Keychain, never files, SQLite, URLs, or logs.
- Honor the user's SSH/Cloudflare setup. Do not rewrite SSH configuration.
- Verify source/base SHA before analysis; key caches by context and prompt version as well as SHA.
- Approvals require explicit confirmation and the reviewed SHA; never merge automatically.
- Do not delete dirty worktrees (including ignored files) or overwrite work from an interactive Claude session. Statistics reuse an existing worktree.
- Repository check definitions cannot grant themselves approval. Proposed AI checks stay proposed.
- Unknown metadata is distinct from zero. Surface partial/truncated diffs.

## Engineering

Keep domain logic independent of GPUI and concrete adapters. Use `ReviewServices` for
review orchestration and `Forge` for forge operations. Use typed errors for parsing and
`anyhow` context at boundaries. Use `tracing` without secrets. Run network/subprocess work
on the Tokio runtime and send results back to GPUI; do not block the UI on these operations.
Use `Command` argument arrays. The embedded terminal accepts a command string: shell-quote
only the configured executable and validated session UUID, never MR text. Keep dependencies and
`Cargo.lock` in sync. The compatible GPUI/widget versions are pinned; evaluate upgrades together.

## Commands

```sh
python3 scripts/bootstrap-ghostty.py  # Zig 0.16.0; bootstrap requires Python 3.12+
cargo run --locked -p mrde
cargo run --locked -p mrde-radar -- --help
cargo test --locked -p mrde-radar
claude plugin validate plugins/mrde --strict
cargo fmt --all --check
cargo check --locked --workspace
cargo test --locked --workspace
cargo test --locked --workspace --exclude mrde
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release -p mrde
```

Config: `~/.config/mrde/config.toml`. Global checks: `~/.config/mrde/checks.toml`.
Repository checks: `.mrde.toml`. Cache: `~/Library/Caches/mrde/`.

Use mocked GitLab HTTP tests, temporary Git repositories, and synthetic Claude JSON.
Do not run live approvals, paid/model requests, or credential mutations as part of tests.
Update `docs/` (user guide, architecture, limitations) and the README when behavior or architecture changes.

## UI regression checks

Use `cargo test --locked -p mrde --lib` for real GPUI input/click tests and mocked settings
service tests. Test and Save are separate: Test performs no writes, Save performs no HTTP.
Never clear secret fields before a result. Preserve entered values on both success and failure.
Keep visible busy indicators, inline feedback, and the expandable redacted debug log panel.
Use `--ui-preview` for native visual inspection without loading user configuration or secrets;
it shows demo data only. `MRDE_SCENARIO=<state> MRDE_SNAPSHOT=out.png` captures the app's own
window (see `docs/development.md`), and `scripts/ui-snapshots.sh` captures every scenario. Inspect the PNGs
after any layout change: unit tests do not catch clipped text, squeezed columns, or overflow.
A running process alone is not an interaction test. Do not claim screenshots were inspected
unless captured successfully and viewed. Preview data can hide problems that real data
shows (long titles, ticket labels, wide tables); capture the real application with
`MRDE_SNAPSHOT_REAL=1` only when the user has explicitly agreed, since it uses their saved
credentials (read-only) and the images contain their merge requests. Read the GPUI 0.2.2 layout pitfalls in `docs/development.md` before
writing rows with text: prefer `ellipsis()`, `min_w_0()`, and `flex_shrink_0()` controls.
