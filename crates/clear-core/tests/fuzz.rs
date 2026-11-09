//! Deterministic mutation fuzzing on stable Rust. Every mutated input must be
//! handled without panicking; whenever cleaning succeeds, the output must parse
//! and cleaning it again must change nothing. Deeper coverage-guided runs use
//! the cargo-fuzz targets in /fuzz.

use clear_core::{clean, inspect, CleanOptions};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut d = src.to_vec();
    for _ in 0..1 + rng.below(4) {
        if d.is_empty() {
            break;
        }
        let at = rng.below(d.len());
        match rng.below(7) {
            0 => d[at] ^= 1 << rng.below(8),
            1 => d[at] = rng.next() as u8,
            2 => d[at] = [0x00, 0xFF, 0x7F, 0x80][rng.below(4)],
            3 => {
                let n = rng.below(16).min(d.len() - at);
                d.drain(at..at + n);
            }
            4 => {
                let b = rng.next() as u8;
                for _ in 0..rng.below(16) {
                    d.insert(at, b);
                }
            }
            5 => d.truncate(at),
            _ => {
                // Corrupt what may be a length field.
                let v = (rng.next() as u32).to_be_bytes();
                for (k, b) in v.iter().enumerate() {
                    if let Some(x) = d.get_mut(at + k) {
                        *x = *b;
                    }
                }
            }
        }
    }
    d
}

#[test]
fn mutated_real_files_never_panic_and_clean_output_is_stable() {
    let opts = [
        CleanOptions::default(),
        CleanOptions {
            exif: true,
            xmp: true,
            iptc: true,
            icc: true,
            c2pa: true,
            comments: true,
            other: true,
            embedded: true,
            keep_orientation: false,
        },
    ];
    let mut checked = 0;
    for (n, name) in [
        "IMG_0003.jpg",
        "no_manifest.jpg",
        "libpng-test.png",
        "test_xmp.webp",
        "mars.webp",
        "test.webp",
        "CA.jpg",
    ]
    .iter()
    .enumerate()
    {
        let src = std::fs::read(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let rounds = if *name == "CA.jpg" { 150 } else { 1500 };
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (n as u64 + 1));
        for _ in 0..rounds {
            let d = mutate(&mut rng, &src);
            let _ = inspect(&d);
            for o in &opts {
                if let Ok(out) = clean(&d, o) {
                    inspect(&out)
                        .unwrap_or_else(|e| panic!("{name}: cleaned output does not parse: {e}"));
                    assert_eq!(
                        clean(&out, o).unwrap(),
                        out,
                        "{name}: cleaning is not idempotent"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(
        checked > 100,
        "too few mutations produced parseable files ({checked})"
    );
}
