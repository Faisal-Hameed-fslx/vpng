//! Checksums and the canonical image digest.

use sha2::{Digest, Sha256};

use vpng_core::CanonicalImage;

/// Prefix that domain-separates the canonical pixel digest from any other use
/// of SHA-256 in this project. Bump the version suffix if the digest input
/// definition ever changes.
pub const CANONICAL_DIGEST_PREFIX: &[u8] = b"VPNG-CANON-v1\0";

/// CRC32C (Castagnoli) over a contiguous slice. Used for per-tile and
/// per-header corruption detection.
pub fn crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

/// Continue a CRC32C computation across non-contiguous regions.
pub fn crc32c_append(state: u32, data: &[u8]) -> u32 {
    crc32c::crc32c_append(state, data)
}

/// CRC32C over several regions treated as one logical buffer.
pub fn crc32c_parts(parts: &[&[u8]]) -> u32 {
    let mut state = 0u32;
    for part in parts {
        state = crc32c::crc32c_append(state, part);
    }
    state
}

/// SHA-256 over a contiguous slice.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    finish(hasher)
}

/// SHA-256 over several regions treated as one logical buffer.
pub fn sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    finish(hasher)
}

fn finish(hasher: Sha256) -> [u8; 32] {
    let out = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&out);
    digest
}

/// Digest of the canonical pixel data of `image`.
///
/// Defined in `docs/VPNG_SPEC_0_1.md`: the hash covers, in order, a fixed
/// domain-separation prefix, the little-endian width and height, the color
/// model id, the bits-per-channel value, and the packed pixel bytes with no row
/// padding. Bytes under fully transparent pixels are included verbatim.
pub fn canonical_digest(image: &CanonicalImage) -> [u8; 32] {
    let width = image.width().to_le_bytes();
    let height = image.height().to_le_bytes();
    let format = image.format();
    let model = [format.color_model as u8];
    let bits = [format.bits.bits()];
    sha256_parts(&[
        CANONICAL_DIGEST_PREFIX,
        &width,
        &height,
        &model,
        &bits,
        image.data(),
    ])
}

/// Lowercase hexadecimal rendering, used by the CLI and tests.
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{Limits, PixelFormat};

    #[test]
    fn crc32c_matches_known_vector() {
        // Standard CRC32C test vector for "123456789".
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn crc32c_parts_equals_concatenation() {
        let whole = crc32c(b"hello world");
        let split = crc32c_parts(&[b"hello", b" ", b"world"]);
        assert_eq!(whole, split);
    }

    #[test]
    fn sha256_matches_known_vector() {
        let digest = sha256(b"abc");
        assert_eq!(
            to_hex(&digest),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn digest_changes_with_pixels() {
        let a = CanonicalImage::from_vec(1, 1, PixelFormat::rgba8(), vec![0, 0, 0, 0]).unwrap();
        let b = CanonicalImage::from_vec(1, 1, PixelFormat::rgba8(), vec![0, 0, 0, 1]).unwrap();
        assert_ne!(canonical_digest(&a), canonical_digest(&b));
        let _ = Limits::default();
    }
}
