<p align="center"><img src="src-tauri/icons/128x128.png" width="96" alt="Nexonic Clear" /></p>
<h1 align="center">Nexonic Clear</h1>
<p align="center">Inspect and remove image metadata and Content Credentials. Offline, lossless, open source.</p>

<p align="center"><img src="docs/screenshot.png" width="860" alt="Nexonic Clear showing a cleaned photo and its verification report" /></p>

## What it does

- **Inspects real files.** EXIF (camera, dates, GPS location, serial numbers), XMP, IPTC, ICC colour profiles, comments, unknown blocks and data hidden after the image.
- **Reads Content Credentials (C2PA).** Signer, signing time, the app that made the claim, recorded actions, and whether the credentials declare AI-generated content. Validation runs offline and reports exactly what the validator found.
- **Removes what you choose.** Each kind of metadata can be kept or removed. Content Credentials and colour profiles are kept unless you turn them off. Orientation can be kept so photos stay upright.
- **Never re-encodes.** Metadata is removed at the container level, so pixels, resolution, transparency, animation and colour data stay byte-for-byte identical.
- **Verifies every result.** Each output is parsed again and checked: image data identical to the original, nothing selected left behind, and the state of any kept credentials.
- **Batch processing.** Clean many images at once and save them to a folder. Originals are never overwritten.
- **Private by design.** No network access, accounts, telemetry or ads. Files never leave your device.

Supported formats: JPEG, PNG and WebP (including animated PNG and WebP).

## Download

Portable builds for Windows (x64), macOS (Apple Silicon and Intel) and Android (ARM64 and ARM32) are on the [Releases](https://github.com/Shahrukhalid/Nexonic-Clear/releases) page. Nothing to install on desktop: unzip and run.

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
