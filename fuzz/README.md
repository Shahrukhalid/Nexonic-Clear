# Fuzzing

Coverage-guided fuzzing of the parsers and the cleaner (needs nightly Rust):

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run parse_and_clean fuzz/corpus -- -max_total_time=300
```

Seed the corpus with the test fixtures: `mkdir -p fuzz/corpus && cp crates/clear-core/tests/fixtures/*.{jpg,png,webp} fuzz/corpus/`.
CI runs this target for a short time on every change. A stable-Rust mutation fuzzer also runs in the normal test suite (`crates/clear-core/tests/fuzz.rs`).
