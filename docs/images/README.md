# Screen reference provenance

These references were captured on **macOS, 2026-11-07**, using synthetic projects only.
Codex visually inspected the PNGs (SVGs rendered with `rsvg-convert`) for visible clipping,
overlap and readability. They are documentation references, **not an approved pixel baseline**
or an automated visual-regression gate. [manifest.json](manifest.json) records each state,
source revision, timestamp, dimensions and file hash.

## Native application

Built source: `0c8aabc` (the capture batch started before that source commit, from the dirty
`77f0900` worktree). The same binary produced History and Statistics afterwards. Captures
use the application's own window, GPUI renderer and fake executor; no credentials or network.
Actual PNG size is 2350×2218 pixels. macOS overrode the requested point dimensions; these
images do not establish exact responsive breakpoints.

| File | Story or preview scenario |
| --- | --- |
| `queue.png`, `search.png` | `inbox.default`, `inbox.filtered` |
| `reader.png`, `context.png` | `reader.inbox`, `context.docked` |
| `structure.png`, `risks.png` | `analysis.structure`, `analysis.risks` |
| `traceability.png`, `changes.png` | `checks.compliance`, `risk.partial` |
| `languages-fr.png`, `radar-fr.png` | `languages.french`, `risk.french` |
| `history.png`, `disktree.png` | `focus:history`, `focus:statistics` |

```sh
python3 scripts/ui_catalog.py capture inbox.default inbox.filtered reader.inbox \
  context.docked analysis.structure analysis.risks checks.compliance risk.partial \
  languages.french risk.french --output /tmp/mrde-native-reference
MRDE_SCENARIO=focus:history MRDE_SNAPSHOT=/tmp/history.png ./target/debug/mrde --ui-preview
MRDE_SCENARIO=focus:statistics MRDE_SNAPSHOT=/tmp/disktree.png ./target/debug/mrde --ui-preview
```

Long content continues below the captured viewport. These images show selected states;
scrolling, focus, failures and responsive boundaries also require interaction tests.

## Terminal application

Built source: `7d9617a`. The SVGs export the **actual Ratatui cell buffer**, with terminal
colors, from the same renderer as interactive `mrde-tui`. They are not hand-drawn mockups
or a capture of another terminal application. Raster inspection used Menlo/monospace.
The inbox, radar and French languages page are 140×44 cells; the compact French radar is
80×24. Missing axes remain n/a; the compact mode scrolls all measurements and evidence.

```sh
cargo run --locked -p mrde-tui -- --demo --screen inbox --snapshot /tmp/tui-inbox.svg
cargo run --locked -p mrde-tui -- --demo --screen radar --snapshot /tmp/tui-radar.svg
cargo run --locked -p mrde-tui -- --demo --screen settings --language fr --snapshot /tmp/tui-languages-fr.svg
cargo run --locked -p mrde-tui -- --demo --screen radar --language fr --width 80 --height 24 --snapshot /tmp/tui-radar-compact.svg
```

An actual 80×24 PTY smoke test also exercised navigation, local demo radar/median,
approval cancellation, independent language saving and clean terminal restoration. Automated
rendering tests cover long confirmation context and pale/dark label contrast. Live forge
approval, protected Keychain access and interactive Claude handoff were not exercised.

## Claude Code and standalone radar

`claude-radar-session.svg` renders the final response of a **real plugin invocation** on the
synthetic fixture in `examples/radar-demo.py`. It is not Claude's interactive UI.
`claude-radar.svg` is the CLI's own export of the same two commits. The engine source is
included in `0c8aabc`; the fixture and transcript renderer are committed with this guide.
See the [complete recording and reproduction instructions](../examples/claude-radar.md).

No raw session log, credential, private path or private MR data is published. The fixture
has no linked ticket or team history, and external checks are off: six of nine axes are
measured and no median is invented. Model wording may differ on another run.

## Updating references

For every product change, update the relevant Gherkin contracts and affected images in
the same batch. Capture to a new temporary directory, inspect the actual output, then copy
only reviewed synthetic artifacts here. Update this manifest and the source revision.
Do not regenerate unrelated images merely to hide visual differences. Keep source/tool
versions and intentional limitations explicit; a build alone is not visual validation.
