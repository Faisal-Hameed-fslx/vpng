//! Regressions found by the coverage-guided fuzzer.
//!
//! Every input in this file was found by libFuzzer in CI and is replayed on
//! every `cargo test` run. A fix that is reverted, or that is made in one
//! place but not the other, fails here instead of waiting for the next
//! coverage-guided run to find it again.
//!
//! The inputs are stored as the literal bytes libFuzzer wrote to its crash
//! artifact, so the test pins the *exact* input rather than a reconstruction
//! of it.

use vpng_container::Header;
use vpng_core::ErrorKind;
use vpng_fuzzing::targets::header_target;

/// `fuzz/artifacts/header/crash-32233a96f431ba8ee472ae5d927905192ef267aa`
///
/// Found by the `header` target on 2026-11-18, in GitHub Actions run
/// 44266490454. The header is well formed in every other respect — correct
/// magic, version, header size, channel count, valid CRC32C, zeroed reserved
/// bytes — but its **width is zero**.
///
/// `Header::parse` used to accept it, while `Header::to_bytes` refused to
/// serialize it ("image dimensions must be non-zero"). The parser and the
/// serializer therefore disagreed about the same 74 bytes: one implementation
/// would accept a file the other refused to write.
const HEADER_WITH_ZERO_WIDTH: [u8; 74] = [
    0x56, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x27, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa3, 0x00, 0xad, 0x04, 0xff, 0xff, 0xff, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xa3, 0xbc, 0x15, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2f,
];

#[test]
fn a_header_with_a_zero_width_is_rejected() {
    let err = Header::parse(&HEADER_WITH_ZERO_WIDTH).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::InvalidArgument,
        "a zero width must be rejected by the parser, not only by the serializer"
    );
}

#[test]
fn the_fuzz_target_no_longer_reports_the_zero_width_header() {
    // The target's invariant is "an accepted header re-encodes to its own
    // bytes". With the parser rejecting the input, the target has nothing to
    // report — which is the fix, not a weakened check.
    assert!(
        header_target(&HEADER_WITH_ZERO_WIDTH).is_ok(),
        "the header target still reports the zero-width header: {:?}",
        header_target(&HEADER_WITH_ZERO_WIDTH)
    );
}

#[test]
fn the_zero_width_header_is_still_rejected_after_ten_bytes_are_dropped() {
    // The crash input carried ten trailing bytes after the 64-byte header.
    // The parser reads the header and ignores the rest, so the trailing
    // bytes must not change the verdict.
    let truncated = &HEADER_WITH_ZERO_WIDTH[..64];
    assert!(Header::parse(truncated).is_err());
    assert!(header_target(truncated).is_ok());
}
