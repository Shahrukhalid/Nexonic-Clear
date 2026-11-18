# Contributing to Scruvix

Bug reports, ideas, documentation fixes and code are all welcome.

## Reporting a bug or asking for a feature

Open an issue and choose a form. For bugs, include the app version, your operating system, the file type, the exact steps and any error message. Please do not attach files that contain private metadata; describe them instead or share a file you are happy to make public.

Security problems: report them privately as described in [SECURITY.md](SECURITY.md), not in a public issue.

## Changing code

1. Fork the repository and create a branch from `main`.
2. Keep the change focused on one thing, and add or update tests when behaviour changes. New features need tests on real files.
3. Run the checks:
   - `SCRUVIX_REQUIRE_FFMPEG=1 cargo test --workspace` (needs ffmpeg)
   - `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
   - `npx tsc --noEmit` and `node scripts/check_i18n.mjs`
4. Open a pull request that explains what changed and how you tested it.

Interface text lives in `src/i18n.ts`; a new string must be added for every language.

## Code of conduct

Be kind and patient. Disagree with ideas, not with people.
