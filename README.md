<p align="center"><img src="src-tauri/icons/128x128.png" width="96" alt="Nexonic Clear" /></p>
<h1 align="center">Nexonic Clear</h1>
<p align="center">Inspect and remove metadata and Content Credentials from photos, audio, video and PDFs. Offline, lossless, open source.</p>

<p align="center"><img src="docs/screenshot.png" width="860" alt="Nexonic Clear showing a cleaned photo and its verification report" /></p>

## What it does

- **Inspects real files.** EXIF (camera, dates, GPS location, serial numbers), XMP, IPTC, ICC colour profiles, comments, unknown blocks and data hidden after the image.
- **Reads Content Credentials (C2PA).** Signer, signing time, the app that made the claim, recorded actions, and whether the credentials declare AI-generated content. Validation runs offline and reports exactly what the validator found.
- **Checks trust offline.** Each file is validated three ways: signature and integrity only, against the bundled C2PA trust list, and against the bundled interim trust list. Every result lists its failure codes.
- **Inspects the whole manifest store.** Every manifest with its claim version, signer, assertions, ingredients and the passed, informational and failed checks.
- **Reads `.c2pa` sidecar files.** A `photo.c2pa` or `photo.jpg.c2pa` next to an image is validated against that image (desktop).
- **Shows every field.** A searchable list of all readable metadata (EXIF, GPS, XMP, IPTC, PNG text, comments, colour profile), with identifying fields highlighted, and a field-by-field before-and-after comparison after cleaning.
- **Keeps what you need.** Optionally keep orientation, capture dates and time zone, print resolution, the colour profile and, on desktop, the original file date.
- **JSON reports.** Export a detailed report for one image or a whole batch: what was found, what was removed, verification results and fingerprints.
- **Eight languages.** English, Español, Français, Deutsch, Português, العربية, اردو and 中文, every screen and message fully translated, with right-to-left layout for Arabic and Urdu.
- **Removes what you choose.** Each kind of metadata can be kept or removed. Content Credentials and colour profiles are kept unless you turn them off. Orientation can be kept so photos stay upright.
- **Strict, crash-safe parsing.** Damaged or malformed files are rejected with a clear message. PNG checksums and WebP structure are validated, and the parsers are fuzz-tested.
- **Never re-encodes.** Metadata is removed at the container level, so pixels, resolution, transparency, animation and colour data stay byte-for-byte identical.
- **Verifies every result.** Each output is parsed again and checked with SHA-256: image data identical to the original, kept embedded content identical, nothing selected left behind, and the state of any kept credentials.
- **Keeps motion photos and HDR intact.** Motion-photo videos, depth maps and Ultra HDR gain maps stored after the main image are detected and kept unless you choose to remove them, and their internal index is updated so they still work.
- **Folders and batches.** Drop files or whole folders (searched recursively) and clean them in one go. Results are stored on disk, not in memory, so large batches stay light on phones.
- **Safe saving.** Only results that pass verification can be saved. Desktop saves are atomic, so a file is never left half-written, and originals are never overwritten.
- **Private by design.** No network access, accounts, telemetry or ads. Files never leave your device.

Supported formats:

- **Images:** JPEG, PNG, WebP (including animated PNG and WebP), HEIC and AVIF.
- **Audio:** WAV (including RF64 and Broadcast WAV), MP3 and M4A.
- **Video:** MP4 and MOV.
- **Documents:** PDF.

Audio and video are cleaned without re-encoding: tags, QuickTime location, make and model, XMP, cover art and Content Credentials are removed while every audio and video sample is copied byte for byte, and chunk offsets are rewritten and verified. On desktop, audio and video up to 64 GB are processed from disk. PDFs are rewritten as a complete new file, so removed document information, XMP and attachments do not survive in older revisions; encrypted PDFs are not supported.

## Command-line tool

`nexonic-clear-cli` uses the same engine and the same verification as the app. It runs on Windows, macOS and Linux.

```sh
nexonic-clear-cli inspect photo.heic                   # what is in a file
nexonic-clear-cli clean -r ~/Pictures/trip -o ~/clean  # clean a folder into another folder
nexonic-clear-cli clean --in-place --keep c2pa doc.pdf # replace the original, keep credentials
nexonic-clear-cli clean --json clip.mov > report.json  # JSON report
nexonic-clear-cli watch ~/Inbox --out ~/Clean          # clean new files as they arrive
nexonic-clear-cli integrate install                    # add "Clean with Nexonic Clear" to the file manager
```

- Without `--out` or `--in-place`, cleaned copies are written next to the originals as `name.clean.ext`. Existing files are never replaced unless you pass `--overwrite`.
- Files run in parallel (`-j` sets how many), progress goes to standard error, and Ctrl+C stops after the files in progress.
- Exit codes: 0 success, 1 a result failed verification (nothing is written for it), 2 usage error, 3 a file could not be read or is unsupported, 130 cancelled.
- `watch` waits until a new file stops changing, then cleans it into `--out`. Unverified results are never written.
- `integrate install` adds the menu for the current user: Windows Explorer (files and folders), macOS Finder (Quick Action) and Linux (GNOME Files script and KDE Dolphin service menu). `integrate uninstall` removes it.

On Android, share photos, audio, video or PDFs to Nexonic Clear from any app.

## Download

<p align="center">
  <a href="https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Windows-x64-Portable.zip"><img alt="Download for Windows" src="https://img.shields.io/badge/Windows-Download-0078D4?style=for-the-badge&logo=windows&logoColor=white"></a>
  <a href="https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-macOS-AppleSilicon.zip"><img alt="Download for Mac with Apple Silicon" src="https://img.shields.io/badge/Mac_Apple_Silicon-Download-000000?style=for-the-badge&logo=apple&logoColor=white"></a>
  <a href="https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-macOS-Intel.zip"><img alt="Download for Intel Mac" src="https://img.shields.io/badge/Mac_Intel-Download-555555?style=for-the-badge&logo=apple&logoColor=white"></a>
  <a href="https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Android-ARM64.apk"><img alt="Download for Android" src="https://img.shields.io/badge/Android-Download-3DDC84?style=for-the-badge&logo=android&logoColor=white"></a>
</p>

| Platform | Direct download |
| --- | --- |
| Windows 10/11 (x64), portable | [NexonicClear-Windows-x64-Portable.zip](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Windows-x64-Portable.zip) |
| Mac with Apple Silicon (M1 and later) | [NexonicClear-macOS-AppleSilicon.zip](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-macOS-AppleSilicon.zip) |
| Mac with Intel | [NexonicClear-macOS-Intel.zip](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-macOS-Intel.zip) |
| Android (most phones, ARM64) | [NexonicClear-Android-ARM64.apk](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Android-ARM64.apk) |
| Android (older phones, ARM32) | [NexonicClear-Android-ARM32.apk](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Android-ARM32.apk) |
| Linux x64, AppImage | [NexonicClear-Linux-x64.AppImage](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Linux-x64.AppImage) |
| Linux x64, Debian and Ubuntu | [NexonicClear-Linux-x64.deb](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-Linux-x64.deb) |
| Command-line tool | [Windows](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-CLI-Windows-x64.zip), [Mac Apple Silicon](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-CLI-macOS-AppleSilicon.zip), [Mac Intel](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-CLI-macOS-Intel.zip), [Linux](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/NexonicClear-CLI-Linux-x64.zip) |

Nothing to install on desktop: unzip and run. Verify downloads with [SHA256SUMS.txt](https://github.com/Shahrukhalid/Nexonic-Clear/releases/latest/download/SHA256SUMS.txt). All versions are on the [Releases](https://github.com/Shahrukhalid/Nexonic-Clear/releases) page.

## Honest limits

- **Credentials are never forged or repaired.** A signature is shown as intact only when the C2PA validator says so. Trust lists are not bundled, so a valid signature means the file matches its credentials, not that the signer's identity is confirmed.
- **Content Credentials cover the whole file.** Removing any other metadata while keeping them makes them fail validation. The app warns before and reports after.
- **Removing metadata does not remove every trace.** Copies elsewhere, cloud provenance records, invisible watermarks and labels added by platforms are not affected. No tool can promise a platform will stop labelling an image.
- Builds are not code-signed yet. Windows SmartScreen and macOS Gatekeeper will ask for confirmation (on macOS: right-click the app, then Open).

## Build from source

Requirements: Rust (stable), Node.js 20+, and the [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```bash
npm ci
cargo test -p clear-core      # engine tests on real image files
npm run tauri -- build --no-bundle
```

The engine (`crates/clear-core`) has no GUI dependencies and can be used on its own.

## License

MIT. Third-party licenses are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and shipped with every build.
