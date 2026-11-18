//! Fuzz the stable tile payload decoders.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run tile_payload
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::tile_payload_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = tile_payload_target(data) {
        panic!("{finding}");
    }
});
