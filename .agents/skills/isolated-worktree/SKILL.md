---
name: isolated-worktree
description: Start, sync, land, and clean up an isolated Git worktree so several coding agents (OpenCode, Codex) never edit the same checkout. Use before changing any file in this repository, and when merging a finished task back into main or resolving its conflicts.
---

# Isolated worktrees for parallel agents

Several agents work on this repository at the same time. Each task runs in its own
worktree and branch; the main checkout only receives merges. Nobody edits files in the main
checkout directly, except to resolve conflicts while landing.

All commands use `scripts/agent-worktree.sh` from this skill's directory. From the
repository root: `.agents/skills/isolated-worktree/scripts/agent-worktree.sh <command>`.

## Start a task

1. Pick a short kebab-case task name, such as `github-provider`.
2. Check what already exists: `agent-worktree.sh status`. Reuse your own worktree if the
   task is already started; never take over another agent's worktree.
3. Run `agent-worktree.sh start <task>`. It creates `../mr_de-worktrees/<task>` on branch
   `agent/<task>` from `main`, and clones the build cache (APFS copy-on-write), so the first
   build recompiles only workspace crates.
4. Move into the worktree before touching files:
   - OpenCode: call `tools.opencode.session_move({ directory: "<worktree>" })` in `execute`.
   - Codex CLI: work with `cd <worktree>` or start Codex with `-C <worktree>`.
   - Codex desktop app: a chat in **Worktree** mode based on `main` is equivalent; finish
     with **Create branch here** and land that branch like below.
5. If `start` refuses because the main checkout has uncommitted changes, someone may still
   be working there. Use `start <task> --from-checkout`: the worktree starts from a snapshot
   of the main checkout, including that uncommitted work, and the checkout itself is not
   modified. Tell the user. Landing needs a clean main checkout, so that work must be
   committed first: only the user may approve `baseline`, which commits it on `main`
   locally on top of the latest snapshot (merges stay clean) and never pushes.

## While working

- Commit often on your branch with clear messages. Never push `agent/*` branches and never
  commit to `main` from a worktree.
- Run builds, tests, and preview screenshots inside your worktree. Its `target/` is its own.
- Pull in other agents' landed work with `agent-worktree.sh sync <task>` (merges `main` into
  your branch, inside your worktree). Resolve conflicts there, then
  `git add <files> && git commit --no-edit`.
- While the main checkout still holds uncommitted work, `sync <task> --from-checkout` merges
  a fresh snapshot of it instead. A snapshot can catch another agent mid-edit: if your
  worktree stops compiling after one, wait and take another rather than fixing their code.

## Land

1. Commit everything in the worktree.
2. `agent-worktree.sh sync <task>`, resolve conflicts in the worktree, then run the checks
   from `AGENTS.md` (fmt, clippy, tests, screenshots for UI changes).
3. `agent-worktree.sh land <task>`: a `--no-ff` merge into `main` in the main checkout. It
   refuses while the main checkout has uncommitted changes. If `main` moved and the merge
   conflicts, prefer `git -C <main> merge --abort`, sync again, and land again; otherwise
   resolve in the main checkout, `git add`, and `git commit --no-edit`.
4. Run the checks once more in the main checkout. Do not push `main` without the user's
   approval.
5. `agent-worktree.sh remove <task>` deletes the worktree, its build outputs, and the branch.
   It refuses unlanded work unless given `--force`, which needs the user's approval.

## Lessons

Update this list when the workflow surprises you, and improve the script with it.

- A worktree branched from `main` does not contain uncommitted work from the main
  checkout. That is why the main checkout must stay clean.
- Codex app worktrees live under `$CODEX_HOME/worktrees` in detached HEAD. Its **Hand off**
  moves uncommitted changes between checkouts: do not hand a chat off to Local while the
  main checkout holds another agent's work.
- A snapshot taken while another agent was mid-edit did not compile (missing functions
  in `demo.rs`). Build right after `start --from-checkout` and resnapshot if needed.
- `target/` is about 30 GB here. `start` clones only `debug/deps`, `debug/build`,
  `debug/.fingerprint`, `tools`, and `zig`, not the 10 GB `debug/incremental`.
