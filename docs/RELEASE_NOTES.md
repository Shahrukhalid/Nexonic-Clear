Nexonic Clear: inspect and remove metadata and Content Credentials from photos, audio, video and PDFs, fully offline.

**Downloads**

| Platform | File |
| --- | --- |
| Windows 10/11 (x64), portable | `NexonicClear-Windows-x64-Portable.zip` |
| macOS Apple Silicon | `NexonicClear-macOS-AppleSilicon.zip` |
| macOS Intel | `NexonicClear-macOS-Intel.zip` |
| Android ARM64 (most phones) | `NexonicClear-Android-ARM64.apk` |
| Android ARM32 (older phones) | `NexonicClear-Android-ARM32.apk` |
| Linux x64 | `NexonicClear-Linux-x64.AppImage` or `NexonicClear-Linux-x64.deb` |
| Command-line tool | `NexonicClear-CLI-<platform>.zip` (Windows, macOS, Linux) |

Verify downloads with `SHA256SUMS.txt`.

Every package in this release passed the engine test suite and a native self-test on its platform: the real app inspected a genuinely signed C2PA image, removed all metadata over its own interface, and verified the image data was unchanged.

Builds are code-signed and notarized only when signing certificates are configured for the release; otherwise Windows SmartScreen and macOS Gatekeeper will ask for confirmation. Android APKs use a development signing certificate, so uninstall before installing a future store-signed version.
