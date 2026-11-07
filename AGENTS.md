# Repository Guidelines

## Project Structure & Module Organization

MR.DE is a macOS desktop application built with Rust 2024 and GPUI. Its package, library, and binary are named `mrde`.

- `crates/mrde-app`: GPUI views, theme, settings, and application composition.
- `crates/mrde-core`: domain models, scoring, and review orchestration.
- `crates/mrde-forge`: `Forge` trait and GitLab REST adapter.
- `crates/mrde-git` and `crates/mrde-claude`: worktrees, Claude CLI, and Ghostty integration.
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
