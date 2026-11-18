//! Fuzz the tile directory parser.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run directory
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::directory_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = directory_target(data) {
        panic!("{finding}");
    }
});
