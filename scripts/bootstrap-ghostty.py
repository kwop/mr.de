#!/usr/bin/env python3
"""Install checksum-verified Zig in the ignored local build cache (macOS only)."""
import hashlib
import json
from pathlib import Path
import platform
import shutil
import tarfile
import urllib.request

if platform.system() != "Darwin":
    raise SystemExit("MR.DE currently supports macOS only.")
root = Path(__file__).resolve().parent.parent / "target" / "tools"
if (root / "zig" / "zig").exists():
    print("Local Zig is already installed.")
    raise SystemExit(0)
root.mkdir(parents=True, exist_ok=True)
with urllib.request.urlopen("https://ziglang.org/download/index.json", timeout=60) as response:
    index = json.load(response)
arch = "aarch64" if platform.machine() == "arm64" else "x86_64"
release = index["0.16.0"][f"{arch}-macos"]
archive = root / "zig-0.16.0.tar.xz"
with urllib.request.urlopen(release["tarball"], timeout=60) as response, archive.open("wb") as output:
    shutil.copyfileobj(response, output)
if hashlib.sha256(archive.read_bytes()).hexdigest() != release["shasum"]:
    archive.unlink()
    raise SystemExit("Zig checksum mismatch.")
with tarfile.open(archive) as source:
    top = source.getmembers()[0].name.split("/")[0]
    source.extractall(root, filter="data")
(root / top).rename(root / "zig")
archive.unlink()
print("Verified Zig 0.16.0 installed under target/tools/zig.")
