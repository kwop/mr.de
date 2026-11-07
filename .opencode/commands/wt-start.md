---
description: Start a task in its own Git worktree, then move this session into it
---

Load the `isolated-worktree` skill and follow it to start the task named `$1`.

Current state:

!`.agents/skills/isolated-worktree/scripts/agent-worktree.sh status`

Run `.agents/skills/isolated-worktree/scripts/agent-worktree.sh start $1` with the shell tool. If it succeeds, move this session into the new worktree with `tools.opencode.session_move` before editing anything, then continue with: $2
