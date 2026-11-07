# Development

## Building and checking

```sh
python3 scripts/bootstrap-ghostty.py      # Python 3.12+, or install Zig 0.16.0 yourself
cargo run --locked -p mrde                 # run the application
cargo fmt --all --check
cargo check --locked --workspace
cargo test --locked --workspace
cargo test --locked --workspace --exclude mrde   # backend crates only, no GPUI linking
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release -p mrde
```

The toolchain is pinned to Rust 1.97.1. `Cargo.lock` is part of the source; update it with
any dependency change. GPUI 0.2.2 and `gpui-component` 0.5.0 are pinned as a compatible pair;
evaluate upgrades together. GPUI's `runtime_shaders` feature compiles Metal shaders at
startup, so the separate Metal toolchain download is not needed, but the macOS SDK is.

The embedded terminal pins `gpui-libghostty` 0.3.1, which builds its native Ghostty library
with Zig 0.16.0. `.cargo/config.toml` selects `scripts/zig.sh`; it prefers the ignored local
`target/tools/zig/zig`, then a compatible system Zig. An explicit `ZIG` environment variable
overrides the wrapper. The first build downloads/builds the pinned Ghostty sources and needs
network access. The adapter's MIT license and compatibility notes live in
`vendor/mrde-ghostty-adapter/`.

The `dev` profile builds dependencies with `opt-level = 3`: unoptimized GPUI layout and
Markdown rendering made scrolling lag by hundreds of milliseconds per frame. The first build
takes a few minutes longer; incremental builds of workspace crates stay fast. GPUI re-renders
the whole view on every scroll frame, so render code must not parse, compute statistics, or
allocate per document: Markdown preparation, the Disktree, and file statistics are cached
per document and per commit.

Conventions for contributors and coding agents are in [`AGENTS.md`](../AGENTS.md) and
[`CLAUDE.md`](../CLAUDE.md): English everywhere, no `unsafe`, domain logic independent of
GPUI, `thiserror` and contextual `anyhow` errors, secret-free `tracing`.

## Tests

- Unit tests live in inline `#[cfg(test)]` modules.
- The GitLab adapter is tested against [Wiremock](https://docs.rs/wiremock) servers.
- Git code runs against temporary repositories; the analysis crate has an end-to-end test on
  a real repository built in a temporary directory.
- Claude integration uses a fake `claude` script returning synthetic JSON.
- GPUI interaction tests (`crates/mrde-app/src/ui/tests.rs`) type into real inputs and click
  rendered buttons through an injected fake executor: settings test/save separation, secret
  retention, loading states, approval confirmation with the displayed SHA, tabs, analysis,
  tour, screenshots, and layout regressions such as wrapping titles and tab overflow.

Tests never contact a real GitLab instance, launch Claude, use the Keychain, or modify SSH
configuration. In modules importing `gpui::*`, use `#[::core::prelude::v1::test]` for plain
unit tests, because GPUI exports its own `test` attribute.

## UI preview and screenshots

`--ui-preview` opens the native UI with synthetic merge requests, without loading saved
configuration or credentials and without contacting GitLab, Git, or Claude. Every action
answers with demo data after a short delay; approval is simulated and saving is refused.

Environment variables put the preview in a given state, and `MRDE_SNAPSHOT` saves a PNG of
the application's own window, then quits. A process may capture its own windows without the
Screen Recording permission, so this works from a terminal or an agent.

| Variable | Effect |
|---|---|
| `MRDE_SCENARIO` | `queue`, `detail:<tab>`, `focus:<tab>` (queue hidden), `confirm`, `worktrees`, `settings`, `logs`, `threads`, `threads:terminal`, `ask` (a questions thread with the local preview shell), `settings:github` (a GitHub project's settings), `open` (the "Open MR…" dialog), `image`, `image:zoom` (4×), `image:max` (8×). Image scenarios position a preview pointer over the image for cursor inspection. Tabs: `description`, `structure`, `files`, `review`, `statistics`, `history`, `risks`, `checks`, `visual`. |
| `MRDE_SCENARIO_MR` | Demo merge request to open, for example `244` (180 files). Default `231`. |
| `MRDE_SCENARIO_FRESH` | Show the tabs before any analysis, tour, or screenshots. |
| `MRDE_SCENARIO_RISK` | Color the Disktree by risk. |
| `MRDE_SCENARIO_SCROLL` | Scroll the reader down by this many points. |
| `MRDE_SCENARIO_CONTEXT` | Pin the review context panel: `docked`, `overlay`, `rail`, or `closed`, whatever the window width. |
| `MRDE_SCENARIO_SEARCH` | Text typed in the title bar search. |
| `MRDE_SCENARIO_LABEL` | A label selected in the sidebar filters, for example `billing`. |
| `MRDE_SNAPSHOT` | Output PNG path; captured after `MRDE_SNAPSHOT_DELAY_MS` (default 2500). |
| `MRDE_WINDOW` | Window size such as `1400x900` (minimum 1100×720; a window manager may override it). |

```sh
MRDE_SCENARIO=focus:risks MRDE_SNAPSHOT=/tmp/risks.png cargo run -p mrde -- --ui-preview
scripts/ui-snapshots.sh target/ui-snapshots    # every scenario
```

The demo data mirrors real-world merge requests on purpose: long titles with ticket
references, labels on tickets, work-item links, and tables with long cells. Demo data that
fits on one line hides wrapping bugs.

To compare with real data, the application itself can be captured with the explicit
`MRDE_SNAPSHOT_REAL=1` opt-in (without `--ui-preview`). It uses the saved credentials for
read-only requests: it waits for the startup refresh, opens the merge request at
`MRDE_SCENARIO_INDEX` (default 0) and selects the tab, or opens `worktrees` or `settings`,
then captures once that screen is ready. It never starts reviews, analyses, Ask Claude,
approvals, or saves. These captures contain real merge requests: keep them private.

Inspect screenshots after any layout change: unit tests do not catch clipped text, squeezed
columns, or overflow.

## GPUI 0.2.2 layout pitfalls

Found with the screenshot harness, with the workarounds used in this code base:

- `truncate()` and `line_clamp(1)` paint stale measurements: text is clipped mid-glyph or
  after one word. Single-line text uses `ellipsis()` (`src/ellipsis.rs`) in a `one_line()`
  container.
- Wrapped text squeezed inside a row can be painted from its min-content measurement, one
  glyph per line. Text that may shrink in a row is either `flex_1().min_w_0()` or an
  ellipsized single line; fixed controls beside it are `flex_shrink_0()`.
- A block container lets descendants' percentage widths resolve against the window, so
  content overflowed the reading column. Tab content is a stretched flex column; prefer
  stretch with `min_w_0()` over `w_full()`.
- `w_full().max_w(..).mx_auto()` measured a column's children at the parent's width before
  the maximum applied: a title wrapping only at the maximum kept a one-line height and
  overlapped the next line. `flex_1().max_w(..)` also undercounted wrapped-title height,
  leaving History's last items beyond the scroll limit. The centered reading column uses
  `w(px(900.)).max_w_full()` to measure at its final width; vertical padding belongs to
  the scrolling content. Long titles use `paragraph()` (`src/paragraph.rs`). Regression
  tests check the last History item at multiple widths, with and without the queue.
- `img` replaces an `auto` height with the image's natural height when its width shrinks.
  Mermaid diagrams use a measured element that fits the width and keeps the aspect ratio.
- `overflow_hidden()` clips children to a rectangle, not to rounded corners: a child with
  its own background paints over its parent's rounded corners. Children touching a corner
  of a rounded sheet round themselves (`rounded_t`, `rounded_b`), and scrolled rows stop a
  few points above the bottom corners.
- `window.window_handle().update(..)` fails inside an action dispatched by a keystroke,
  because the window is busy: global shortcut handlers use `cx.defer`.
- A single-line `Input` propagates **Enter**; an unhandled keystroke then reaches the input
  handler, and the test platform inserts a newline that panics text shaping. Handle
  `input::Enter` on an ancestor instead of only subscribing to `PressEnter`.
- In tests, `debug_bounds` keeps selectors from earlier frames: assert absence only for
  elements never rendered, otherwise check the state behind them.
- To debug a layout, add a temporary GPUI test that calls `cx.debug_bounds(selector)`, or
  print `on_children_prepainted` bounds in the real application.

## Vendored gpui-component

[`vendor/gpui-component`](../vendor/gpui-component) is the crates.io source of
`gpui-component` 0.5.0 (Apache-2.0), used through `[patch.crates-io]`. Local changes, listed
in [`MRDE-PATCH.txt`](../vendor/gpui-component/MRDE-PATCH.txt):

- initial Markdown parsing/highlighting runs in the background, with ordered updates;
- an opt-in image-click callback for Markdown images;
- list items keep their width (upstream rendered empty bullets);
- table cells share the width and wrap instead of being clipped;
- Markdown blocks keep their spacing, so headings no longer touch the paragraph above.

Remove these patches once a compatible upstream release fixes the same problems.

## Resolver and terminal verification

Resolver tests use injected effects, local temporary Git repositories, Wiremock job traces,
and a fake streaming Claude executable. They cover conflicts, lease rejection, exact-SHA
pipeline tracking, questions, the three-attempt cap, Stop/Resume, stale updates, and relaunch.

`MRDE_SCENARIO=threads:terminal` opens a real embedded PTY with a harmless local shell.
Set `MRDE_TERMINAL_INPUT=1` to dispatch `ok` and Enter through GPUI; a native capture should
show `Received: ok`. `MRDE_TERMINAL_NAVIGATE=1` switches to Settings after input, allowing a
second capture to check that the native terminal does not cover the new page. Both flags
operate only in UI preview. No Claude installation or GitLab credentials are used.

## Product contracts and UI reference

Behavioral contracts live in [`specs/features/`](../specs/features/), with stable scenario IDs
and a [map to existing Rust tests](../specs/coverage.md). They are specifications awaiting
step bindings, not an additional passing test suite. Install `specs/requirements.txt` in a
development virtual environment and run `python scripts/check_specs.py` to validate them.

The [native story catalog](../specs/ui/README.md) reuses the real GPUI preview:

```sh
python3 scripts/ui_catalog.py list
python3 scripts/ui_catalog.py open risk.partial
python3 scripts/ui_catalog.py capture reader.history-bottom image.zoom risk.partial
```

Captures go to a fresh temporary directory and are candidates for review, never silently
updated golden images. Keep new stories linked to relevant Gherkin IDs; validate behavior
with the GPUI test platform in addition to inspecting native captures.


## Keep product contracts and screen references current

Every user-facing change must update the affected `specs/features/*.feature` contracts and
relevant screen references in the same batch. Add a scenario before introducing a new UI
state. Capture synthetic data, inspect the result, and record the command, language, size,
source revision and known limitations in `docs/images/README.md`. Never publish private MR
content or credential-bearing captures. A successful build is not a visual verification.

The native catalog includes `languages.french` and `risk.french`. `MRDE_LANGUAGE=en|fr`
selects a preview language. Terminal references use the actual Ratatui buffer:

```sh
cargo run --locked -p mrde-tui -- --demo --screen radar --snapshot /tmp/tui-radar.svg
cargo run --locked -p mrde-tui -- --demo --screen settings --language fr --snapshot /tmp/tui-settings-fr.svg
```

`mrde-core/locales/fr.json` is shared by both interfaces. Application copy uses translation
keys and named placeholders. Preserve source titles, labels, descriptions, code and tool
output. New keys need French translations; the catalog test checks placeholder parity.
Test both languages and narrow layouts. Information language applies to new Claude
requests; it does not translate an existing conversation or an external tool's output.

`mrde-service` owns connection settings, project catalogs and the injected `Executor`.
Keep GPUI and Ratatui dependencies in their respective frontend crates. Terminal state
produces effects; its runtime executes them off the input thread. Never add Git, HTTP,
Keychain or process waits to a renderer.
