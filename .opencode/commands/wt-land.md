---
description: Land a finished task: sync, check, merge into main, verify, clean up
---

Load the `isolated-worktree` skill and land task `$1` following its Land section: commit, sync, resolve conflicts in the worktree, run the checks, `agent-worktree.sh land $1`, run the checks again in the main checkout, then ask me before removing the worktree. Never push.

Current state:

!`.agents/skills/isolated-worktree/scripts/agent-worktree.sh status`
