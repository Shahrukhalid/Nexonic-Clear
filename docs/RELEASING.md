# Releasing

Releases are built by `.github/workflows/packages.yml` when a `v*` tag is pushed.
Every package is tested on its own platform before it is published. Signing is
optional: each signing step runs only when its secrets exist, so releases keep
working unsigned until certificates are added.

## Secrets

Add these under **Settings → Secrets and variables → Actions**.

### Windows (code signing)

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE` | The code-signing certificate as a base64 `.pfx` (`base64 -w0 cert.pfx`) |
| `WINDOWS_CERTIFICATE_PASSWORD` | The `.pfx` password |

Both `nexonic-clear.exe` and `nexonic-clear-cli.exe` are signed with SHA-256 and
a timestamp, then checked with `signtool verify`. A certificate kept in a
hardware token or a cloud signing service (for example SignPath) needs that
service's own signing step in place of the `.pfx` step.

### macOS (signing and notarization)

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | "Developer ID Application" certificate as a base64 `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` password |
| `APPLE_SIGNING_IDENTITY` | For example `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | The Apple ID used for notarization |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | The 10-character team ID |

The Tauri bundler signs the app with the hardened runtime and submits it for
notarization; the command-line tool is signed with the same identity.

### Google Play

| Secret | Value |
| --- | --- |
| `ANDROID_KEYSTORE` | The upload keystore as base64 (`base64 -w0 upload.jks`) |
| `ANDROID_KEYSTORE_PASSWORD` | Keystore password |
| `ANDROID_KEY_ALIAS` | Key alias |
| `ANDROID_KEY_PASSWORD` | Key password |

The workflow builds `NexonicClear-<version>-Android-GooglePlay.aab`, signed
with the upload key. Enrol the app in Play App Signing; Google then signs what
users install. Upload the AAB in the Play Console (internal testing first).
The APKs on GitHub keep their own signing, so users moving between the two
must uninstall first.

## Linux

The Linux job builds an AppImage and a `.deb` on Ubuntu 22.04 (for older
glibc compatibility), runs the native self-test, and checks that the `.deb`
contains the app and its notices. No secrets are needed.

## Checklist

1. Merge to `main` with every check green.
2. Bump the version in `Cargo.toml`, `package.json`, `src-tauri/tauri.conf.json` and `src/main.ts`.
3. Push the tag `vX.Y.Z`. The publish job refuses to release unless every
   platform's validation file matches the uploaded files.
