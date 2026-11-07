---
description: Merge main into this task's worktree branch and resolve conflicts there
---

Load the `isolated-worktree` skill. Commit pending work in the worktree of task `$1`, then run `.agents/skills/isolated-worktree/scripts/agent-worktree.sh sync $1`. Resolve any conflicts inside that worktree, commit the merge, and run the checks from AGENTS.md.
