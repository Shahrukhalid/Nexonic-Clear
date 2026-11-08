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
