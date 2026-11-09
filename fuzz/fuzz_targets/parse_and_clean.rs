#![no_main]
//! Any input: inspection and cleaning must not panic; successful cleaning must
//! produce a file that parses and is unchanged by a second clean.
use clear_core::{clean, inspect, CleanOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = inspect(data);
    for opts in [CleanOptions::default(), CleanOptions { icc: true, c2pa: true, embedded: true, keep_orientation: false, ..CleanOptions::default() }] {
        if let Ok(out) = clean(data, &opts) {
            assert!(inspect(&out).is_ok());
            assert_eq!(clean(&out, &opts).ok().as_deref(), Some(&out[..]));
        }
    }
});
