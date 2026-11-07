#!/bin/sh
set -eu
mrde_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -x "$mrde_root/target/tools/zig/zig" ]; then
    exec "$mrde_root/target/tools/zig/zig" "$@"
fi
if command -v zig >/dev/null 2>&1 && [ "$(zig version)" = "0.16.0" ]; then
    exec zig "$@"
fi
echo "Ghostty requires Zig 0.16.0. Run python3 scripts/bootstrap-ghostty.py or set ZIG to a compatible executable." >&2
exit 1
