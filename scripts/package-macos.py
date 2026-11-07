#!/usr/bin/env python3
"""Build a provisioned development app with a stable identity and Touch ID Keychain access.

Requires Xcode signed into the selected Apple Developer team. No credentials are read.
Xcode may register the bundle ID/device and create a development provisioning profile.
"""
import argparse
import hashlib
from pathlib import Path
import plistlib
import platform
import re
import shutil
import subprocess
import sys
import tempfile

BUNDLE_ID = "io.github.kwop.mrde"


def run(args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def validate_entitlements(entitlements, team):
    app_id = entitlements.get("com.apple.application-identifier", "")
    groups = entitlements.get("keychain-access-groups", [])
    if (not app_id.endswith("." + BUNDLE_ID)
            or entitlements.get("com.apple.developer.team-identifier") != team
            or app_id not in groups):
        raise ValueError("The signed wrapper is missing MR.DE's Keychain entitlements")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--team", required=True, help="Your Apple Developer Team ID")
    parser.add_argument("--release", action="store_true", help="Use an optimized Rust build")
    parser.add_argument("--provision-only", action="store_true", help="Prepare the Xcode profile before compiling Rust")
    args = parser.parse_args()
    if sys.platform != "darwin" or not re.fullmatch(r"[A-Z0-9]{10}", args.team):
        parser.error("A macOS host and a ten-character Apple Team ID are required")
    root = Path(__file__).resolve().parent.parent
    derived = root / "target" / "macos-signing"
    run(["xcodebuild", "-quiet", "-project", str(root / "packaging/macos/MRDE.xcodeproj"),
         "-scheme", "MRDE", "-configuration", "Debug", "-destination", f"platform=macOS,arch={platform.machine()}",
         f"SYMROOT={derived / 'build'}", f"OBJROOT={derived / 'objects'}",
         f"DEVELOPMENT_TEAM={args.team}", "-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration", "build"], cwd=root)
    wrapper = derived / "build/Debug/mr.de.app"
    profile = wrapper / "Contents/embedded.provisionprofile"
    if not profile.is_file():
        raise RuntimeError("Xcode did not embed a provisioning profile; Touch ID cannot be enabled")
    result = run(["codesign", "-d", "--entitlements", ":-", str(wrapper)], capture_output=True)
    entitlements = plistlib.loads(result.stdout)
    validate_entitlements(entitlements, args.team)
    if args.provision_only:
        print("Provisioned wrapper ready; run again without --provision-only to package Rust.")
        return
    cargo = ["cargo", "build", "--locked", "-p", "mrde", "-p", "mrde-tui"]
    if args.release:
        cargo.append("--release")
    run(cargo, cwd=root)
    output = root / "target/macos"
    output.mkdir(exist_ok=True)
    app = output / "mr.de.app"
    if app.exists():
        shutil.rmtree(app)
    shutil.copytree(wrapper, app)
    # Extract the public leaf certificate to identify the exact key Xcode selected.
    # No private key material is exported or stored by this script.
    with tempfile.TemporaryDirectory(prefix="mrde-signing-") as temporary:
        directory = Path(temporary)
        prefix = directory / "certificate"
        run(["codesign", "-d", f"--extract-certificates={prefix}", str(wrapper)], capture_output=True)
        identity = hashlib.sha1(Path(str(prefix) + "0").read_bytes()).hexdigest()
        rights = directory / "entitlements.plist"
        rights.write_bytes(plistlib.dumps(entitlements))
        binary = root / "target" / ("release" if args.release else "debug") / "mrde"
        shutil.copy2(binary, app / "Contents/MacOS/mrde")
        terminal = app / "Contents/MacOS/mrde-tui"
        shutil.copy2(binary.with_name("mrde-tui"), terminal)
        run(["codesign", "--force", "--sign", identity, "--identifier", BUNDLE_ID,
             "--entitlements", str(rights), str(terminal)])
        run(["codesign", "--verify", "--strict", str(terminal)])
        run(["codesign", "--force", "--sign", identity, "--identifier", BUNDLE_ID,
             "--entitlements", str(rights), str(app)])
    run(["codesign", "--verify", "--strict", "--verbose=2", str(app)])
    final = run(["codesign", "-d", "--entitlements", ":-", str(app)], capture_output=True)
    validate_entitlements(plistlib.loads(final.stdout), args.team)
    print(f"Signed app ready: {app}\nLaunch this app, then Save your GitLab settings once to enable Touch ID.")


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, ValueError, RuntimeError) as error:
        raise SystemExit(str(error)) from error
