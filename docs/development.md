# Development

## Building and checking

```sh
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
| `MRDE_SCENARIO` | `queue`, `detail:<tab>`, `focus:<tab>` (queue hidden), `confirm`, `worktrees`, `settings`, `logs`. Tabs: `description`, `structure`, `files`, `review`, `statistics`, `history`, `risks`, `checks`, `visual`. |
| `MRDE_SCENARIO_MR` | Demo merge request to open, for example `244` (180 files). Default `231`. |
| `MRDE_SCENARIO_FRESH` | Show the tabs before any analysis, tour, or screenshots. |
| `MRDE_SCENARIO_RISK` | Color the Disktree by risk. |
| `MRDE_SCENARIO_SCROLL` | Scroll the reader down by this many points. |
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
then captures once that screen is ready. It never starts reviews, analyses, Discuss,
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
  overlapped the next line. The reading column is a child of a centered row with
  `flex_1().max_w(..)`, and long titles use `paragraph()` (`src/paragraph.rs`), whose
  measured and painted line breaks come from the same width.
- `img` replaces an `auto` height with the image's natural height when its width shrinks.
  Mermaid diagrams use a measured element that fits the width and keeps the aspect ratio.
- In tests, `debug_bounds` keeps selectors from earlier frames: assert absence only for
  elements never rendered, otherwise check the state behind them.
- To debug a layout, add a temporary GPUI test that calls `cx.debug_bounds(selector)`, or
  print `on_children_prepainted` bounds in the real application.

## Vendored gpui-component

[`vendor/gpui-component`](../vendor/gpui-component) is the crates.io source of
`gpui-component` 0.5.0 (Apache-2.0), used through `[patch.crates-io]`. Local changes, listed
in [`MRDE-PATCH.txt`](../vendor/gpui-component/MRDE-PATCH.txt):

- an opt-in image-click callback for Markdown images;
- list items keep their width (upstream rendered empty bullets);
- table cells share the width and wrap instead of being clipped;
- Markdown blocks keep their spacing, so headings no longer touch the paragraph above.

Remove these patches once a compatible upstream release fixes the same problems.
