//! Fuzz the editing engine's region writes.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run editing
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::editing_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = editing_target(data) {
        panic!("{finding}");
    }
});
