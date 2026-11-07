# mr.de

A native macOS review cockpit for GitLab merge requests, built in Rust with
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui). It gathers your review queue, merge request descriptions,
linked tickets, changed files, and approval in one window, and helps you understand large
merge requests before approving them: structure, history, contracts, risks, traceability to
a specification, and before/after screenshots.

AI assistance uses your local [Claude Code](https://github.com/anthropics/claude-code)
CLI and runs only when you click a button. There is no Anthropic API key and no background
analysis.

> **Status: experimental.** mr.de is an early personal project, published mainly for
> archiving and reference. It works against real GitLab instances, but expect rough edges,
> breaking changes, and missing packaging. macOS only.

![Review queue and reader](docs/images/reader.png)

## Features

- **Review queue.** Open, non-draft merge requests where you are reviewer, assignee, or
  author, across projects. Sorted by priority labels (`P0`, `P::1`, …), pipeline state, age,
  and size, or by date or diff size. Labels come with their GitLab colors, including those of
  the linked ticket when teams label tickets rather than merge requests.
- **Reader.** Markdown descriptions with tables, images from GitLab uploads, and Mermaid
  diagrams rendered in pure Rust. Linked issues, changed files with line counts, and a
  *Disktree* treemap of the most changed areas.
- **Pre-review with Claude** at three depths (Quick, Standard, Deep): summary, intention,
  severity-ranked risks, priority files, and confidence. Results are cached per commit.
- **Understand complex merge requests** (local Git analysis, no AI):
  - *Structure*: changed functions and types with signature changes, callers the MR did not
    update, and a graph of how the changed files depend on each other. Claude can add a
    guided reading tour with a flow diagram.
  - *History*: why removed code existed (`git blame` back to the commit and ticket), risky
    files with frequent fixes, who knows each area (including `CODEOWNERS`), and other merge
    requests touching the same files.
  - *Risks*: contract changes (database migrations, HTTP endpoints, API schemas, environment
    variables, permissions), dependency upgrades, and risky patterns in added lines.
- **Specification compliance.** Claude reads a folder of specifications and builds a
  traceability matrix: requirement, code, tests, status.
- **Visual comparison.** Your own screenshot command (for example Playwright) runs on the
  base and merge request commits; mr.de shows before, after, and a pixel difference.
- **Safe approval.** Approval requires confirmation and is bound to the reviewed commit SHA.
  mr.de never merges.
- **Discuss** opens [Ghostty](https://ghostty.org/) with Claude Code in a detached worktree
  of the merge request, with the context prepared for you.

| Structure | Risks | Traceability |
|---|---|---|
| ![Structure tab](docs/images/structure.png) | ![Risks tab](docs/images/risks.png) | ![Traceability matrix](docs/images/traceability.png) |

All screenshots use the built-in demo data (`--ui-preview`).

## Requirements

- macOS with Xcode and its command-line tools (the macOS SDK is required).
- Rust 1.97.1; the toolchain is pinned in `rust-toolchain.toml`.
- Git with SSH access to your GitLab repositories. Your `~/.ssh/config`, including any
  `ProxyCommand`, is used as is.
- A GitLab personal access token with the `api` scope. Self-hosted instances behind
  Cloudflare Access are supported with a service token.
- Optional: Claude Code installed and logged in, for pre-reviews, tours, and compliance.
- Optional: Ghostty in `/Applications`, for **Discuss**.

## Quick start

```sh
git clone git@github.com:kwop/mr.de.git
cd mr.de
cargo run --locked -p mrde -- --ui-preview   # explore with demo data, no network
cargo run --locked -p mrde                   # connect to your GitLab instance
```

On first launch, open **Settings**, enter your GitLab URL and token, click **Test
connection**, then **Save**. Secrets are stored in the macOS Keychain; other settings live in
`~/.config/mrde/config.toml` (see [`config/mrde.example.toml`](config/mrde.example.toml)).

To build an optimized binary:

```sh
cargo build --locked --release -p mrde
cargo install --locked --path crates/mrde-app   # installs `mrde` in ~/.cargo/bin
```

mr.de produces a native executable, not a signed `.app` bundle.

## Security and privacy

- Tokens live only in the macOS Keychain, never in files, the cache, URLs, or logs.
- GitLab is accessed over HTTPS; redirects are refused so credentials cannot leak to another
  host.
- Claude, analyses, screenshots, and approvals run only on explicit clicks. Merge request
  text reaches Claude on standard input, never in command-line arguments, and Claude gets
  read-only tools.
- The local cache (`~/Library/Caches/mrde/`) contains merge request descriptions and AI
  summaries from your instance, but no secrets.

## Documentation

- [User guide](docs/user-guide.md): configuration, review workflow, labels, analysis tabs,
  compliance, visual comparison, and worktrees.
- [Architecture](docs/architecture.md): crates, data flow, caching, and how Claude and Git
  are invoked.
- [Development](docs/development.md): building, testing, the UI preview and screenshot
  harness, and GPUI layout pitfalls.
- [Known limitations](docs/limitations.md).

## License

MIT, see [LICENSE](LICENSE). The vendored copy of `gpui-component` in
[`vendor/gpui-component`](vendor/gpui-component) keeps its Apache-2.0 license; its small
local patches are listed in [`MRDE-PATCH.txt`](vendor/gpui-component/MRDE-PATCH.txt).
