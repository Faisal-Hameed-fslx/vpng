//! A byte-level adaptive entropy stage for the hybrid research codecs.
//!
//! This is deliberately *not* a new entropy coder. It reuses the stable
//! arithmetic coder from [`crate::predictor`] and drives it with a small set of
//! **symbol-group contexts**: the bytes of the LZ77 token stream are not
//! uniformly distributed (flag bytes, literal bytes, length codes and distance
//! codes all have different statistics), so each group gets its own adaptive
//! model instead of sharing one.
//!
//! Each group is coded as eight binary decisions, one per bit position, and
//! every bit position of every group has its own probability. That is an
//! order-1-on-bits, order-0-on-group model: cheap, bounded, deterministic, and
//! enough to expose whether an entropy stage is worth anything on top of LZ77.
//!
//! No floating point is involved: the coder's own arithmetic is integer-only
//! and the shared probability model is the stable one.

use crate::predictor::{BitSink, Decoder, Encoder};

/// Number of symbol groups the token stream is split into.
pub const GROUPS: usize = 5;

/// Group 0: the per-group flag byte that says which of the next eight items
/// are literals and which are matches.
pub const GROUP_FLAG: usize = 0;
/// Group 1: literal bytes.
pub const GROUP_LITERAL: usize = 1;
/// Group 2: match length codes.
pub const GROUP_LENGTH: usize = 2;
/// Group 3: match distance codes.
pub const GROUP_DISTANCE: usize = 3;
/// Group 4: the extra bytes that follow a length or distance code.
pub const GROUP_EXTRA: usize = 4;

/// Total probability contexts: one per group per bit position.
pub fn context_count() -> usize {
    GROUPS * 8
}

/// Code one byte in `group` into `sink`.
#[inline]
pub(crate) fn code_byte(sink: &mut impl BitSink, group: usize, byte: u8) {
    let base = group * 8;
    for bit in (0..8).rev() {
        sink.encode_bit(base + bit, u32::from((byte >> bit) & 1));
    }
}

/// Decode one byte in `group` from `decoder`.
#[inline]
pub(crate) fn decode_byte(decoder: &mut Decoder<'_>, group: usize) -> u8 {
    let base = group * 8;
    let mut byte = 0u8;
    for bit in (0..8).rev() {
        byte = (byte << 1) | decoder.decode_bit(base + bit) as u8;
    }
    byte
}

/// Entropy-code a byte sequence, tagging each byte with its group.
///
/// The group tags are supplied alongside the bytes so the decoder can rebuild
/// the identical model. `groups[i]` is the group of `bytes[i]`.
pub fn encode_bytes(groups: &[usize], bytes: &[u8]) -> Vec<u8> {
    debug_assert_eq!(groups.len(), bytes.len());
    let mut encoder = Encoder::new(context_count());
    for (group, &byte) in groups.iter().zip(bytes.iter()) {
        code_byte(&mut encoder, *group, byte);
    }
    encoder.finish()
}

/// Inverse of [`encode_bytes`].
pub fn decode_bytes(groups: &[usize], encoded: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(encoded, context_count());
    let mut out = Vec::with_capacity(groups.len());
    for &group in groups {
        out.push(decode_byte(&mut decoder, group));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_roundtrip_through_the_entropy_stage() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let groups: Vec<usize> = (0..bytes.len())
            .map(|i| match i % 4 {
                0 => GROUP_FLAG,
                1 => GROUP_LITERAL,
                2 => GROUP_LENGTH,
                _ => GROUP_DISTANCE,
            })
            .collect();
        let encoded = encode_bytes(&groups, &bytes);
        let restored = decode_bytes(&groups, &encoded);
        assert_eq!(restored, bytes);
    }

    #[test]
    fn empty_input_roundtrips() {
        let encoded = encode_bytes(&[], &[]);
        assert!(decode_bytes(&[], &encoded).is_empty());
    }

    #[test]
    fn repeated_bytes_compress() {
        // A constant byte stream must cost far less than one byte per input
        // byte, otherwise the entropy stage is not doing its job.
        let bytes = vec![0x5au8; 4096];
        let groups = vec![GROUP_LITERAL; bytes.len()];
        let encoded = encode_bytes(&groups, &bytes);
        assert!(
            encoded.len() < bytes.len() / 4,
            "expected a large reduction, got {} from {}",
            encoded.len(),
            bytes.len()
        );
    }

    #[test]
    fn truncation_is_detected_as_wrong_bytes_not_a_panic() {
        let bytes = vec![0x11u8; 64];
        let groups = vec![GROUP_LITERAL; bytes.len()];
        let encoded = encode_bytes(&groups, &bytes);
        let truncated = &encoded[..encoded.len() / 2];
        // The decoder tolerates short input by reading zeros past the end; the
        // result must simply differ, never panic.
        let restored = decode_bytes(&groups, truncated);
        assert_ne!(restored, bytes);
    }
}
