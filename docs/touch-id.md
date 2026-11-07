# Touch ID and macOS Keychain

The provisioned `mr.de.app` uses the macOS Data Protection Keychain with a `userPresence`
access policy. macOS offers Touch ID when available, with the Mac password as its fallback.
The application cannot force biometrics when macOS requires a password.

## Build and launch

Sign into your Apple Developer account in Xcode, then run:

```sh
python3 scripts/bootstrap-ghostty.py
python3 scripts/package-macos.py --team YOURTEAMID
open target/macos/mr.de.app
```

The packaging command uses Xcode automatic signing. It can register this Mac and the
`io.github.kwop.mrde` App ID, and creates/downloads the required development provisioning
profile. Team IDs, certificates, profiles, and signed products stay outside tracked source.
Use `--release` for an optimized binary; `--provision-only` checks signing before compiling Rust.
This is a development app, not a notarized distribution package. Re-run the command when
its profile or certificate expires. Use the same team and bundle identifier across updates.

Ordinary `cargo run` binaries are ad-hoc signed and keep the legacy `keyring` backend.
Their authorization can be requested again after rebuilding. Launch the packaged app to use
Touch ID. A stable Apple signature and provisioned Keychain entitlements are both required.

## Existing credentials

In the signed app, open GitLab settings and click **Save** once. Existing empty secret fields
retain their saved values. macOS may request the login Keychain password one final time to
read the legacy entries. Save copies the GitLab token and both Cloudflare values into one
protected, device-local Keychain item. Test connection never performs this migration.

On later launches, the first credential read authenticates once for the whole bundle; values
stay in memory for that application session. Save updates all three values atomically.
Cancellation, unavailable authentication, missing entitlements, and malformed protected
entries are errors, never a reason to bypass protection through the legacy backend.

Legacy entries are retained for recovery and older development builds. They are not updated
by the new backend. An intentionally cleared protected value stays empty; it does not cause
an old legacy value to be restored. Credentials never go through a shell, files, logs, or SQLite.

## Validation

Unit tests use an in-memory fake backend and cover grouped reads, explicit migration,
cancellation, failed saves, instance isolation, and clearing credentials without resurrection.
Packaging validates the profile, final signature, and Keychain entitlement values. It does
not read real credentials or simulate a fingerprint; the native authentication prompt needs
interactive verification on the user's Mac.

Apple references: [Keychain APIs and implementations](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains),
[Touch ID access to Keychain items](https://developer.apple.com/documentation/localauthentication/accessing-keychain-items-with-face-id-or-touch-id),
and [restricted entitlements for command-line tools](https://developer.apple.com/forums/thread/724013).
