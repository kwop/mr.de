#!/bin/sh
# Isolated worktrees for parallel coding agents (OpenCode, Codex, ...). Each task gets its
# own branch and checkout; the main checkout only receives merges.
#
#   agent-worktree.sh start <task>     new worktree on branch agent/<task>, from the trunk
#   agent-worktree.sh start <task> --from-checkout
#                                      same, from a snapshot of the main checkout including
#                                      its uncommitted work (the checkout is not modified)
#   agent-worktree.sh sync <task>      merge the trunk into agent/<task>, inside its worktree
#   agent-worktree.sh land <task>      merge agent/<task> into the trunk in the main checkout
#   agent-worktree.sh status           every agent worktree, ahead/behind, local changes
#   agent-worktree.sh remove <task>    delete a landed worktree and its branch
#   agent-worktree.sh baseline         commit the main checkout's pending work on the trunk
#                                      (once, with the user's approval)
#
# Environment: AGENT_WORKTREES (default: <main checkout>-worktrees next to it),
# AGENT_TRUNK (default: main). Agent branches are local: never push them.
set -eu

die() {
    printf 'agent-worktree: %s\n' "$*" >&2
    exit 1
}
MAIN=$(git worktree list --porcelain 2>/dev/null | sed -n '1s/^worktree //p')
[ -n "$MAIN" ] || die "run inside the Git repository or one of its worktrees"
ROOT=${AGENT_WORKTREES:-"$(dirname "$MAIN")/$(basename "$MAIN")-worktrees"}
TRUNK=${AGENT_TRUNK:-main}

check_name() {
    printf '%s' "$1" | grep -Eq '^[a-z0-9][a-z0-9-]{0,47}$' ||
        die "invalid task name '$1': use 1-48 lowercase letters, digits, and hyphens"
}
changes() {
    git -C "$1" status --porcelain --untracked-files=normal
}
require_clean() {
    [ -z "$(changes "$1")" ] || die "$2"
}
on_trunk() {
    [ "$(git -C "$MAIN" symbolic-ref --quiet --short HEAD || true)" = "$TRUNK" ] ||
        die "the main checkout must be on '$TRUNK'"
}

# A commit of the main checkout as it is now (tracked and untracked files, minus ignored
# ones), built with a temporary index: the checkout's own index and files are untouched.
snapshot() {
    index=$(mktemp "${TMPDIR:-/tmp}/agent-index.XXXXXX")
    rm -f "$index"
    GIT_INDEX_FILE="$index" git -C "$MAIN" read-tree "$1"
    GIT_INDEX_FILE="$index" git -C "$MAIN" add -A
    tree=$(GIT_INDEX_FILE="$index" git -C "$MAIN" write-tree)
    rm -f "$index"
    git -C "$MAIN" commit-tree "$tree" -p "$1" -m "$2"
}
# Snapshots chain on each other so a later baseline can extend the same history, which
# keeps the merges of snapshot-based branches clean.
SNAPSHOT_REF=refs/agent-worktrees/snapshot
latest_snapshot() {
    latest=$(git -C "$MAIN" rev-parse --verify --quiet "$SNAPSHOT_REF" || true)
    if [ -n "$latest" ] && git -C "$MAIN" merge-base --is-ancestor "$TRUNK" "$latest"; then
        echo "$latest"
    else
        git -C "$MAIN" rev-parse "$TRUNK"
    fi
}

# Copy-on-write clones (APFS clonefile) of build outputs and tools: the first build in the
# worktree recompiles workspace crates only, not every dependency. Missing or failing
# copies are skipped; Cargo then builds from scratch.
warm_cache() {
    src="$MAIN/target"
    dst="$1/target"
    [ -d "$src" ] || return 0
    mkdir -p "$dst/debug"
    for item in tools zig; do
        [ -e "$src/$item" ] && cp -cR "$src/$item" "$dst/" 2>/dev/null || true
    done
    for item in deps build .fingerprint; do
        [ -e "$src/debug/$item" ] && cp -cR "$src/debug/$item" "$dst/debug/" 2>/dev/null || true
    done
}

cmd_start() {
    check_name "$1"
    dir="$ROOT/$1"
    branch="agent/$1"
    [ ! -e "$dir" ] || die "$dir already exists; continue there or pick another name"
    if git -C "$MAIN" show-ref --verify --quiet "refs/heads/$branch"; then
        die "branch $branch already exists"
    fi
    start=$TRUNK
    if [ "${2:-}" = --from-checkout ]; then
        start=$(snapshot "$(latest_snapshot)" "Snapshot of the main checkout for agent/$1")
        git -C "$MAIN" update-ref "$SNAPSHOT_REF" "$start"
    elif [ -n "$(changes "$MAIN")" ]; then
        die "the main checkout has uncommitted work that a worktree from $TRUNK would miss. Use --from-checkout to start from it, or ask the user to baseline it."
    fi
    mkdir -p "$ROOT"
    git -C "$MAIN" worktree add --quiet -b "$branch" "$dir" "$start"
    warm_cache "$dir"
    printf 'Worktree ready: %s (branch %s, from %s)\n' "$dir" "$branch" \
        "$(git -C "$dir" log -1 --format='%h %s')"
}

cmd_sync() {
    check_name "$1"
    dir="$ROOT/$1"
    [ -d "$dir" ] || die "no worktree at $dir"
    require_clean "$dir" "commit or discard the changes in $dir before syncing"
    if ! git -C "$dir" merge --no-autostash --no-edit "$TRUNK"; then
        printf 'agent-worktree: resolve the conflicts in %s, then: git add <files> && git commit --no-edit\n' "$dir" >&2
        exit 1
    fi
}

cmd_land() {
    check_name "$1"
    dir="$ROOT/$1"
    branch="agent/$1"
    [ -d "$dir" ] || die "no worktree at $dir"
    require_clean "$dir" "commit the work in $dir before landing"
    on_trunk
    require_clean "$MAIN" "the main checkout has uncommitted changes; nobody should edit it directly. Ask the user before touching them."
    if git -C "$MAIN" merge-base --is-ancestor "$branch" "$TRUNK"; then
        echo "Nothing to land: $branch is already in $TRUNK."
        return 0
    fi
    if ! git -C "$MAIN" merge --no-autostash --no-ff --no-edit -m "Merge $branch" "$branch"; then
        printf 'agent-worktree: conflicts in the main checkout. Resolve them in %s, then: git add <files> && git commit --no-edit\n' "$MAIN" >&2
        printf 'Prefer: git -C %s merge --abort, then "sync %s", resolve in the worktree, and land again.\n' "$MAIN" "$1" >&2
        exit 1
    fi
    echo "Landed $branch into $TRUNK. Run the checks in $MAIN, then remove the worktree when done."
}

cmd_status() {
    printf 'Main checkout: %s (%s)%s\n' "$MAIN" \
        "$(git -C "$MAIN" symbolic-ref --quiet --short HEAD || echo detached)" \
        "$([ -n "$(changes "$MAIN")" ] && echo ', UNCOMMITTED CHANGES' || true)"
    git -C "$MAIN" worktree list --porcelain | sed -n 's/^worktree //p' | while read -r dir; do
        [ "$dir" = "$MAIN" ] && continue
        branch=$(git -C "$dir" symbolic-ref --quiet --short HEAD || echo detached)
        counts=$(git -C "$dir" rev-list --left-right --count "$TRUNK...HEAD" 2>/dev/null || echo "? ?")
        behind=${counts%%[[:space:]]*}
        ahead=${counts##*[[:space:]]}
        local_changes=$(changes "$dir" | wc -l | tr -d ' ')
        printf '%s\n  branch %s · %s ahead, %s behind %s · %s local changes\n' \
            "$dir" "$branch" "$ahead" "$behind" "$TRUNK" "$local_changes"
    done
}

cmd_remove() {
    check_name "$1"
    dir="$ROOT/$1"
    branch="agent/$1"
    if [ ! -d "$dir" ] && ! git -C "$MAIN" show-ref --verify --quiet "refs/heads/$branch"; then
        die "no worktree or branch named $1"
    fi
    if [ -d "$dir" ]; then
        require_clean "$dir" "$dir has uncommitted changes; land or discard them first"
    fi
    if git -C "$MAIN" show-ref --verify --quiet "refs/heads/$branch" &&
        ! git -C "$MAIN" merge-base --is-ancestor "$branch" "$TRUNK"; then
        [ "${2:-}" = --force ] || die "$branch is not landed in $TRUNK; use --force to delete it anyway"
    fi
    if [ -d "$dir" ]; then
        # Build outputs are ignored files; remove them first so Git can drop the worktree.
        rm -rf "$dir/target"
        git -C "$MAIN" worktree remove "$dir"
    fi
    git -C "$MAIN" branch -D "$branch" >/dev/null 2>&1 || true
    echo "Removed $dir and $branch."
}

cmd_baseline() {
    on_trunk
    [ -n "$(changes "$MAIN")" ] || {
        echo "The main checkout is already clean."
        return 0
    }
    old=$(git -C "$MAIN" rev-parse HEAD)
    parent=$(latest_snapshot)
    git -C "$MAIN" add -A
    tree=$(git -C "$MAIN" write-tree)
    # Extends the latest snapshot, so branches started --from-checkout merge cleanly.
    commit=$(git -C "$MAIN" commit-tree "$tree" -p "$parent" \
        -m "Baseline: work in progress before isolated agent worktrees")
    git -C "$MAIN" update-ref "refs/heads/$TRUNK" "$commit" "$old"
    echo "Committed the pending work on $TRUNK as $(git -C "$MAIN" rev-parse --short HEAD). Nothing was pushed."
}

command=${1:-}
[ $# -gt 0 ] && shift
case "$command" in
start)
    [ $# -ge 1 ] && [ $# -le 2 ] || die "usage: start <task> [--from-checkout]"
    cmd_start "$@"
    ;;
sync | land)
    [ $# -eq 1 ] || die "usage: $command <task>"
    "cmd_$command" "$1"
    ;;
remove)
    [ $# -ge 1 ] || die "usage: remove <task> [--force]"
    cmd_remove "$@"
    ;;
status) cmd_status ;;
baseline) cmd_baseline ;;
*) sed -n '2,/^set -eu/p' "$0" | grep '^#' | sed 's/^# \{0,1\}//' ;;
esac
