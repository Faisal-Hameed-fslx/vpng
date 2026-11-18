//! Fuzz the region and tile-grid arithmetic.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run region
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::region_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = region_target(data) {
        panic!("{finding}");
    }
});
