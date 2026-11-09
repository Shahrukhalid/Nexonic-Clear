# Third-party notices

Nexonic Clear is MIT licensed. It is built with open-source components whose
licenses are preserved in every package:

- Desktop builds: the `notices` folder next to the application (inside the app bundle on macOS).
- Android builds: the `assets/notices` folder inside the APK.

Each folder contains `INDEX.json` (package, version, license), the original
license and notice files of every Rust, JavaScript and Android dependency, the
standard license texts, and the exact source archives of MPL-2.0 components.
The release workflow refuses to build if a dependency has a license outside the
reviewed allow-list.

Main components: Tauri (MIT or Apache-2.0), the C2PA Rust SDK `c2pa`
(MIT or Apache-2.0) for reading and validating Content Credentials, and Serde
(MIT or Apache-2.0).

## Test fixtures

The images and test signing keys in `crates/clear-core/tests/fixtures` come
from the C2PA Rust SDK (Copyright 2022 Adobe, MIT license; see
`LICENSE-c2pa-rs-MIT` in that folder). They are used only by the test suite and
are not included in any build.

The `media.*` fixtures (WAV, MP3, MP4, MOV, M4A, HEIC, AVIF and PDF) were made
for this project from synthetic test tones and patterns with FFmpeg, libheif
(through pillow-heif), libavif (through Pillow) and pikepdf. They contain no
third-party content and are used only by the test suite.

## Trust lists

The certificate lists in `crates/clear-core/trust` are built into the app and
used only for offline validation:

- `C2PA-TRUST-LIST.pem` and `C2PA-TSA-TRUST-LIST.pem`: the C2PA Conformance
  Program trust lists, from https://github.com/c2pa-org/conformance-public
  (`trust-list/`). Copyright Coalition for Content Provenance and Authenticity,
  licensed under Creative Commons Attribution 4.0 International
  (https://creativecommons.org/licenses/by/4.0/). Included unmodified.
- `interim-anchors.pem`, `interim-allowed.pem` and `store.cfg`: the interim
  (frozen) Content Credentials trust list, from
  https://github.com/contentauth/verify-site (`static/trust/`), licensed under
  the Apache License 2.0. Included unmodified.
