# MR.DE radar for Claude Code

Invoke `/mrde:radar` to measure local changes and discuss their risk in the current Claude
Code session. The plugin calls the installed `mrde-radar` binary; it does not need the MR.DE
GUI, a forge token, an Anthropic API key, or a second Claude session.

## Install

From a checkout of [MR.DE](https://github.com/kwop/mr.de), install the native binary:

```sh
cargo install --locked --path crates/mrde-radar
mrde-radar --version
```

Git and the repository's pinned Rust toolchain are required to build. The radar has no
GPUI or Ghostty dependency. The binary must be on the PATH used by Claude Code.

For a local plugin session, pass the absolute plugin folder when starting Claude from
the repository you want to inspect:

```sh
claude --plugin-dir /path/to/mr.de/plugins/mrde
```

Alternatively, add the repository's marketplace from inside Claude Code:

```text
/plugin marketplace add kwop/mr.de
/plugin install mrde@mrde-tools
```

Installing the plugin does not install or update the Rust binary. Update the binary and
plugin together when the report schema changes.

## Use

```text
/mrde:radar
/mrde:radar origin/main
/mrde:radar origin/main --head HEAD
/mrde:radar origin/main --run-checks
```

The default compares current files with HEAD, including staged, unstaged and non-ignored
new files. Supplying a base includes branch changes since its merge base; `--head` selects
commits only. The source index, branch and files remain unchanged. Analysis uses an owned
temporary Git snapshot, removed when the scan finishes.

External complexity/lint commands are disabled unless requested with `--run-checks`.
Locally configured commands are trusted in that mode; repository-proposed commands require
explicit approval of their fingerprint. Missing measurements stay n/a. Local checks are
ordinary processes, not an OS sandbox, so run commands you trust.

The skill is manual, with no hooks or automatic invocation. It reports eight axes, evidence,
a verdict and limitations; the model explains the measurements rather than inventing them.
Team medians are unavailable in standalone scans. The configured coverage report is not
verified for freshness. No files are fetched, no forge approval is sent, and no change is
pushed. A successful scan means a report was produced, not that the code is safe to merge.

To export a radar directly:

```sh
mrde-radar --base origin/main --worktree --format svg > /tmp/mrde-radar.svg
```

## Validate the plugin

```sh
claude plugin validate plugins/mrde --strict
claude plugin validate .claude-plugin/marketplace.json --strict
cargo test --locked -p mrde-radar
```

These commands validate packaging and the real Rust workflow with temporary repositories.
They do not exercise a live model conversation. See the repository's `docs/radar-cli.md`
for the JSON contract and API, and `specs/features/radar_cli.feature` for product scenarios.
