Scruvix, formerly Nexonic Clear. Inspect. Sanitize. Verify.

Use Scruvix to inspect and remove metadata and Content Credentials from photos, audio, video and PDFs, fully offline.

**Downloads**

| Platform | File |
| --- | --- |
| Windows 10/11 (x64), portable | `Scruvix-Windows-x64-Portable.zip` |
| macOS Apple Silicon | `Scruvix-macOS-AppleSilicon.zip` |
| macOS Intel | `Scruvix-macOS-Intel.zip` |
| Android ARM64 (most phones) | `Scruvix-Android-ARM64.apk` |
| Android ARM32 (older phones) | `Scruvix-Android-ARM32.apk` |
| Linux x64 | `Scruvix-Linux-x64.AppImage` or `Scruvix-Linux-x64.deb` |
| Command-line tool | `Scruvix-CLI-<platform>.zip` (Windows, macOS, Linux) |

Verify downloads with `SHA256SUMS.txt`.

Desktop package validation exercises the app with a signed C2PA image and verifies that cleaning preserves image data. Android exercises the app on an x86_64 emulator and checks the signed ARM packages for ABI, native alignment, and bundled notices.

Windows and macOS signing depends on the configured certificates and notarization credentials. GitHub APKs are development-signed per build. If Android rejects an update due to a certificate mismatch, export files you need before uninstalling the previous app. The Android package ID is retained for the existing Play channel, which relies on its established app signing identity.

See [upgrade instructions](https://github.com/Shahrukhalid/Scruvix#upgrading-from-nexonic-clear) for desktop, CLI and Linux package changes.
