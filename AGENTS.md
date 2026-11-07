# Repository Guidelines

## Parallel Agents: One Worktree per Task

Several agents (OpenCode, Codex) work on this repository at the same time. They must never edit the same checkout.

- Before changing any file, start the task in its own worktree with the `isolated-worktree` skill (`.agents/skills/isolated-worktree/SKILL.md`): `.agents/skills/isolated-worktree/scripts/agent-worktree.sh start <task>`, then work only in `../mr_de-worktrees/<task>` on branch `agent/<task>`. OpenCode: `/wt-start <task>`. Codex desktop app: a Worktree-mode chat based on `main` is equivalent.
- The main checkout (`main`) only receives merges: `agent-worktree.sh land <task>` after `sync` and the checks. Resolve conflicts in your worktree when possible; touch the main checkout only to finish a merge.
- If the main checkout has uncommitted changes, another agent may still be working there: start with `start <task> --from-checkout` (a snapshot; the checkout is untouched), and ask the user before landing or committing anything in it (`baseline` needs their approval).
- Commit on your own branch; never push `agent/*` branches; never push `main` without the user's approval. Remove the worktree after landing.
- When the workflow surprises you, add a lesson to the skill and improve its script.

## Project Structure & Module Organization

MR.DE is a macOS desktop application built with Rust 2024 and GPUI. Its package, library, and binary are named `mrde`.

- `crates/mrde-app`: GPUI views, theme, settings, and application composition.
- `crates/mrde-core`: domain models, scoring, and review orchestration.
- `crates/mrde-forge`: `Forge` trait and GitLab REST adapter.
- `crates/mrde-git` and `crates/mrde-claude`: worktrees and the Claude CLI. The embedded Ghostty terminal is in `mrde-app` (`terminal.rs`, `threads.rs`).
- `crates/mrde-insights`: read-only worktree analysis (symbols, history, contracts, patterns) and screenshot comparison.
- `crates/mrde-store`: Keychain access, SQLite persistence, and `migrations/`.
- `crates/mrde-checks`: check definitions/results; no execution engine yet.

Tests usually live in inline `#[cfg(test)]` modules; GPUI interaction tests are in `crates/mrde-app/src/ui/tests.rs`. Prompts live in `mrde-claude/prompts/`. Configuration examples are in `config/` and `examples/`; widget assets come from `gpui-component-assets`. User and developer documentation lives in `docs/`.

## Build, Test, and Development Commands

Use the pinned Rust 1.97.1 toolchain and Xcode macOS SDK.

- `cargo run --locked -p mrde`: launch locally.
- `cargo run --locked -p mrde -- --ui-preview`: inspect the UI without accessing saved credentials.
- `cargo build --locked --release -p mrde`: build an optimized executable.
- `cargo check --locked --workspace`: type-check all crates.
- `cargo test --locked --workspace`: run all tests.
- `cargo test --locked -p mrde --lib`: run settings and UI interaction tests.
- `cargo fmt --all --check`: verify formatting.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: enforce lint checks.

## Coding Style & Naming Conventions

Use English throughout. Follow rustfmt, four-space indentation, `snake_case` functions/modules, and `PascalCase` types. Unsafe code is forbidden. Keep domain logic independent of GPUI; run network/subprocess work on Tokio. Use `thiserror`, contextual `anyhow` errors, and secret-free `tracing`. Update `Cargo.lock` with dependency changes.

## Testing Guidelines

Use Rust tests, Tokio tests, Wiremock HTTP mocks, temporary Git repositories, and GPUI click/input tests. Name tests after observable behavior, such as `failed_save_keeps_form_and_displays_keychain_error`. Cover success, failure, and regressions; no numeric coverage threshold is configured. Never test against live approvals or real credentials. Process startup alone does not validate UI interactions.

## Commit & Pull Request Guidelines

Use concise, imperative English commit subjects. PRs should explain behavior changes, reference relevant issues, list validation commands/results, and include screenshots for UI changes when capture is available. Document unverified behavior explicitly.

## Security & Configuration

Store secrets only in Keychain. Configuration lives in `~/.config/mrde/`. Preserve SSH settings and dirty worktrees. Claude requires explicit user action; approval requires confirmation and the reviewed SHA. Keep Test and Save separate, retain masked inputs, and redact debug logs.

## Agent Working Notes

Lessons from previous sessions. Details and the GPUI layout pitfalls are in `docs/development.md`.

### Visual verification loop

- Tests do not catch clipped text, overflow, or overlaps. After any UI change: `cargo build -p mrde`, then `MRDE_SCENARIO=focus:risks MRDE_SNAPSHOT=<tmp>/risks.png timeout 40 ./target/debug/mrde --ui-preview`, and open the PNG with the image-reading tool. Write captures to the agent temp directory, never into the repository.
- `screencapture` fails in agent sessions ("could not create image from display"). The in-app capture (`src/snapshot.rs`, CoreGraphics, own window only) works without the Screen Recording permission.
- Scenario code: `src/ui/preview.rs` (states), `src/demo.rs` (data), `scripts/ui-snapshots.sh` (all states). Useful variables: `MRDE_SCENARIO_MR`, `MRDE_SCENARIO_FRESH`, `MRDE_SCENARIO_RISK`, `MRDE_SCENARIO_SCROLL`.
- The window manager overrides the requested size: captures came out 1000 to 1175 pt wide whatever `MRDE_WINDOW` said. The minimum window width is 1100 pt; to test that width, use a GPUI test with `cx.simulate_resize`.
- The review context panel docks only from 1360 pt (`ui/context.rs`), wider than any capture: pin its state with `MRDE_SCENARIO_CONTEXT=docked|overlay|rail|closed`. `MRDE_SCENARIO_SEARCH`, `MRDE_SCENARIO_LABEL`, and `MRDE_SCENARIO=open` show the search, label filters, and "Open MR…" dialog.
- Rounded sheets do not clip their children: a child with a background must round the corners it touches (see `docs/development.md`). Radii come from `theme.rs` (`RADIUS_SHEET`, `RADIUS_CARD`, `RADIUS_CONTROL`, `RADIUS_CHIP`).
- Demo data must reproduce real-world shapes: long titles that wrap, labels on linked tickets, `/-/work_items/` links, tables with long cells, queue sorted by priority. Data that fits on one line hid real bugs.
- Real application captures (`MRDE_SNAPSHOT_REAL=1`, `src/ui/automation.rs`) only with the user's explicit consent: they use the Keychain token for read-only requests and show private merge requests. Example: `MRDE_SNAPSHOT_REAL=1 MRDE_SCENARIO=focus:description MRDE_SCENARIO_INDEX=0 MRDE_SNAPSHOT=<tmp>/real.png timeout 240 ./target/debug/mrde`. The startup refresh took about 45 s on a real instance; the capture waits for the scenario, then `MRDE_SNAPSHOT_DELAY_MS`. Supported states are `queue`, `detail:<tab>`, `focus:<tab>`, `worktrees`, and `settings`.
- To inspect real data without credentials, read the cached queue: `sqlite3 ~/Library/Caches/mrde/mrde.sqlite3 "SELECT payload FROM queues"` (JSON). Print structure and counts, not private content.

### Debugging layout

- Add `.debug_selector(|| "name".into())` and a temporary GPUI test: `cx.simulate_resize(size(px(1100.), px(1109.)))`, `draw(&root, cx)`, then print `cx.debug_bounds("name")`. Remove the test afterwards. The test `draw()` helper renders at 1280×900.
- The test platform can lay out correctly while the real app does not, because GPUI text measurement depends on call order. When a screenshot contradicts a passing test, temporarily print real bounds with `.on_children_prepainted(|b, _, _| eprintln!(...))` behind an environment variable, read stderr from a real or preview run, then remove it.
- `debug_bounds` keeps selectors from earlier frames: assert absence only for elements never rendered.
- In GPUI tests, a wheel event over the reader scrolls it twice, so `click_action` can overshoot long tabs. Set `detail_scroll` directly instead (`reveal` in `ui/tests/risk.rs`). `MRDE_SCENARIO_SCROLL` is in points; captures are 2× pixels.
- Backend tests get their settings directory from `Env::settings`: never read the real `~/.config/mrde`, where a user `risk.toml` could run lint commands.
- Tests never attach native Ghostty surfaces: under `cfg(test)`, `Threads::launched` records the terminal commands instead. The preview always runs a local shell (`use_preview_terminal`), never Claude Code; `MRDE_SCENARIO=ask` shows a questions thread.
- `finish()` in `ui/tests.rs` waits for the background job. `FakeExecutor` bails on unknown actions: add an arm for every new `Action`.
- In modules that `use gpui::{prelude::*, *}`, unit tests need `#[::core::prelude::v1::test]`, because GPUI exports its own `test` attribute.
- Window chrome lives in `ui/chrome.rs` (title bar, sidebar, shortcuts, "Open MR…" dialog) and the right panel in `ui/context.rs`. Shortcuts are App-level action handlers that `cx.defer` their work, so they also fire when nothing is focused. UI tests can live in `ui/tests/<name>.rs` submodules of `ui/tests.rs`, which share its fakes.
- Building blocks: `ellipsis()` and `one_line()` for single-line text, `paragraph()` for wrapping titles, `Fitted` in `mermaid.rs` for images that keep their aspect ratio, and `widgets.rs` (`heading`, `card`, `badge`, `path_line`, `count`) for analysis tabs.

### GitLab behaviors

- Only list endpoints honor `with_labels_details`. Single-MR and related-issue responses name labels without colors; the adapter colors them from `projects/:id/labels?include_ancestor_groups=true`.
- Teams may label tickets rather than merge requests. `related_issues` also returns issues merely mentioned, so ticket labels come only from the primary ticket (`primary_tickets` in `mrde-core/src/models.rs`).
- Wiremock fixtures must mimic real responses, such as names-only labels on single-MR endpoints. A fixture that was too generous once let tests pass while the real app lost every label color.

### Repository practice

- Another OpenCode session may edit this tree concurrently. Check modification times (`ls -lT`) before large edits and before committing; ask the user when unexpected changes appear.
- Changes to `vendor/gpui-component` must be listed in `vendor/gpui-component/MRDE-PATCH.txt`.
- The repository is public (github.com/kwop/mr.de). Keep demo data, tests, docs, and screenshots generic: no employer or product names, real merge request titles, colleague usernames, real hostnames, or GitLab upload secrets. `docs/design/` stays excluded through `.git/info/exclude`. Grep staged files for secrets and such names before every push.
- The first build compiles dependencies with `opt-level = 3` and takes several minutes. When adding a dependency, run once without `--locked` to update `Cargo.lock`.
- rustfmt rewraps code after edits; re-read the current text before scripted replacements.
