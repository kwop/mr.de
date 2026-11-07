#!/bin/sh
# Captures the native window for every preview scenario, with synthetic data only.
# Usage: scripts/ui-snapshots.sh [output-dir] [scenario ...]
# Windows briefly appear on screen; no network, Keychain, Git, or Claude access occurs.
set -eu
out="${1:-target/ui-snapshots}"
[ "$#" -gt 0 ] && shift
scenarios="${*:-queue detail:description detail:files detail:review detail:statistics focus:structure focus:history focus:risks focus:checks focus:visual confirm worktrees settings logs}"
mkdir -p "$out"
cargo build --locked -p mrde --quiet
for scenario in $scenarios; do
    file="$out/$(echo "$scenario" | tr ':' '-').png"
    MRDE_SCENARIO="$scenario" MRDE_SNAPSHOT="$file" ./target/debug/mrde --ui-preview
done
echo "Snapshots in $out"
