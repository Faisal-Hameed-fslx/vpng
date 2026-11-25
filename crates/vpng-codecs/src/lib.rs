//! `vpng-codecs` — tile codecs behind a single trait.
//!
//! Implemented: Raw (id 0), Palette (id 1), Run-length (id 2), the spatial
//! predictor with an arithmetic entropy coder (id 3), the tile-local LZ77
//! dictionary (id 4), prediction layered over that dictionary (id 5), and
//! context-adaptive entropy coding (id 6). Every other id is reserved and
//! rejected with [`vpng_core::ErrorKind::UnsupportedFeature`] rather than
//! silently mis-decoded. See ADR-0004 for the id table.
//!
//! # The two codec generations
//!
//! Ids 0 to 3 are the **v0.1 generation**: they may appear in any file. Ids 4
//! to 6 are the **v0.3 generation**: they may appear only in a file whose
//! minor version is 3 or later. A v0.1 or v0.2 file that names one of them is
//! malformed, and a build that predates the promotion rejects them with
//! [`vpng_core::ErrorKind::UnsupportedFeature`] as well.

#![forbid(unsafe_code)]

pub mod carc;
pub mod entropy;
pub mod experimental;
pub mod hybrid;
pub mod lz77;
pub mod palette;
pub mod predictor;
pub mod raw;
pub mod rle;

use vpng_core::{Error, Limits, PixelFormat, Result};

/// Codec id 0: uncompressed tile bytes. Always available.
pub const CODEC_RAW: u8 = 0;
/// Codec id 1: per-tile palette.
pub const CODEC_PALETTE: u8 = 1;
/// Codec id 2: run-length.
pub const CODEC_RLE: u8 = 2;
/// Codec id 3: spatial prediction plus an adaptive arithmetic entropy coder.
pub const CODEC_PREDICTOR: u8 = 3;
/// Codec id 4: LZ77-style dictionary. Stable from format v0.3.
pub const CODEC_LZ77: u8 = 4;
/// Codec id 5: prediction layered over a dictionary. Stable from format v0.3.
pub const CODEC_PREDICTOR_DICTIONARY: u8 = 5;
/// Codec id 6: context-adaptive entropy coding. Stable from format v0.3.
pub const CODEC_CONTEXT_ENTROPY: u8 = 6;

/// Sentinel for "let the encoder pick the smallest codec for each tile".
///
/// It is never written to a file: the directory entry records the concrete
/// codec that was actually used.
pub const CODEC_AUTO: u8 = u8::MAX;

/// Every codec this build implements, in ascending id order.
///
/// A cost-based selector iterates this in order, so when two codecs produce
/// payloads of equal size the lowest id wins (a defined, deterministic
/// tie-break).
pub const IMPLEMENTED_CODEC_IDS: &[u8] = &[
    CODEC_RAW,
    CODEC_PALETTE,
    CODEC_RLE,
    CODEC_PREDICTOR,
    CODEC_LZ77,
    CODEC_PREDICTOR_DICTIONARY,
    CODEC_CONTEXT_ENTROPY,
];

/// The codec ids a **v0.1 or v0.2** file may name.
///
/// The three dictionary and context-adaptive codecs were promoted in format
/// v0.3, so an older file that names one of them is malformed.
pub const CODEC_IDS_V0_2: &[u8] = &[CODEC_RAW, CODEC_PALETTE, CODEC_RLE, CODEC_PREDICTOR];

/// The codec ids a **v0.3** file may name.
///
/// This is every implemented codec.
pub const CODEC_IDS_V0_3: &[u8] = IMPLEMENTED_CODEC_IDS;

/// Whether a codec id was promoted in format v0.3.
pub fn is_v0_3_codec(id: u8) -> bool {
    !CODEC_IDS_V0_2.contains(&id) && IMPLEMENTED_CODEC_IDS.contains(&id)
}

/// The set of codecs a selector may choose from.
///
/// The two sets exist because the two codec generations are not equally
/// cheap. The v0.1 generation is fast; the v0.3 generation costs several
/// times more to run, because a dictionary match search and a
/// context-adaptive model are both far more work than a predictor pass. A
/// caller that wants the smallest file pays for that; a caller that wants the
/// old speed does not have to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecSet {
    /// The four original stable codecs: raw, palette, rle and predictor.
    ///
    /// A file that uses only these codecs is a v0.1 or v0.2 file.
    V0_1,
    /// Every implemented codec, including the three promoted in v0.3.
    ///
    /// A file that uses one of the promoted codecs is a v0.3 file.
    V0_3,
}

impl CodecSet {
    /// The codec ids in this set, in ascending order.
    ///
    /// The order is the tie-break order of a cost-based selector: when two
    /// codecs produce payloads of equal size, the lower id wins.
    pub const fn ids(self) -> &'static [u8] {
        match self {
            CodecSet::V0_1 => CODEC_IDS_V0_2,
            CodecSet::V0_3 => CODEC_IDS_V0_3,
        }
    }

    /// Whether this set contains the given codec id.
    pub fn contains(self, id: u8) -> bool {
        self.ids().contains(&id)
    }
}

/// Everything a codec needs to encode one tile.
#[derive(Debug, Clone, Copy)]
pub struct EncodeContext {
    pub limits: Limits,
    pub format: PixelFormat,
    /// The tile's pixel width. A tile in the last column of an image is
    /// narrower than the nominal grid, so this is the *actual* width of the
    /// tile being encoded, not the grid's nominal size.
    pub tile_width: u16,
    /// The tile's pixel height. A tile in the last row is shorter than the
    /// nominal grid, so this is the actual height.
    pub tile_height: u16,
}

/// Everything a codec needs to decode one tile.
#[derive(Debug, Clone, Copy)]
pub struct DecodeContext {
    pub limits: Limits,
    pub format: PixelFormat,
}

/// One reversible tile transform.
///
/// Implementations must be exactly reversible: `decode(encode(x)) == x` for
/// every byte string `x` that is a valid tile buffer for the context's format
/// and geometry. They must never allocate from `input.len()` without first
/// checking a limit.
///
/// `encode` returns [`vpng_core::ErrorKind::UnsupportedFeature`] when the codec
/// cannot represent this particular tile (for example a palette meeting a 257th
/// distinct color). A cost-based selector treats that as "skip this candidate"
/// and falls back to another codec; it is not a failure of the input.
pub trait TileCodec: Send + Sync {
    /// Stable on-disk identifier.
    fn codec_id(&self) -> u8;

    /// Short human-readable name.
    fn name(&self) -> &'static str;

    /// Encode `input` into `out`, replacing `out`'s contents.
    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()>;

    /// Decode `input`, which must yield exactly `expected_len` bytes, into `out`.
    fn decode(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()>;
}

/// Human-readable name for any id, including reserved ones.
pub fn codec_name(id: u8) -> Option<&'static str> {
    Some(match id {
        CODEC_RAW => "raw",
        CODEC_PALETTE => "palette",
        CODEC_RLE => "rle",
        CODEC_PREDICTOR => "predictor",
        CODEC_LZ77 => "lz77",
        CODEC_PREDICTOR_DICTIONARY => "predictor+dictionary",
        CODEC_CONTEXT_ENTROPY => "context-entropy",
        _ => return None,
    })
}

/// The codec id a name refers to.
///
/// The names are the ones [`codec_name`] returns, plus the decimal id and
/// a few aliases that spell the same codec. `"auto"` is [`CODEC_AUTO`], which
/// asks the encoder to choose the smallest payload per tile.
///
/// This is the **only** name table in the workspace: the command line and
/// the Python extension both use it, so they cannot disagree about which
/// name means which codec.
pub fn codec_id_by_name(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "auto" => CODEC_AUTO,
        "raw" | "0" => CODEC_RAW,
        "palette" | "1" => CODEC_PALETTE,
        "rle" | "2" => CODEC_RLE,
        "predictor" | "3" => CODEC_PREDICTOR,
        "lz77" | "4" => CODEC_LZ77,
        "predictor+dictionary" | "predictor-dictionary" | "5" => CODEC_PREDICTOR_DICTIONARY,
        "context-entropy" | "context_adaptive" | "6" => CODEC_CONTEXT_ENTROPY,
        _ => return None,
    })
}

/// The codec set a name refers to.
///
/// `"stable"` is the four original codecs, the set the published
/// measurements and the golden fixtures pin. `"extended"` adds the three
/// codecs promoted in format v0.3.
pub fn codec_set_by_name(name: &str) -> Option<CodecSet> {
    match name.to_ascii_lowercase().as_str() {
        "stable" => Some(CodecSet::V0_1),
        "extended" => Some(CodecSet::V0_3),
        _ => None,
    }
}

/// Whether this build can decode and encode the given codec id.
pub fn is_implemented(id: u8) -> bool {
    IMPLEMENTED_CODEC_IDS.contains(&id)
}

/// Look up a codec, failing with [`vpng_core::ErrorKind::UnsupportedFeature`] for
/// reserved-but-unimplemented ids.
pub fn get_codec(id: u8) -> Result<&'static dyn TileCodec> {
    match id {
        CODEC_RAW => Ok(&raw::RAW_CODEC),
        CODEC_PALETTE => Ok(&palette::PALETTE_CODEC),
        CODEC_RLE => Ok(&rle::RLE_CODEC),
        CODEC_PREDICTOR => Ok(&predictor::PREDICTOR_CODEC),
        CODEC_LZ77 => Ok(&lz77::LZ77_CODEC),
        CODEC_PREDICTOR_DICTIONARY => Ok(&hybrid::PREDICTOR_DICTIONARY_CODEC),
        CODEC_CONTEXT_ENTROPY => Ok(&carc::CONTEXT_ENTROPY_CODEC),
        other => {
            let name = codec_name(other).unwrap_or("unknown");
            if codec_name(other).is_some() {
                Err(Error::unsupported(format!(
                    "codec id {other} ({name}) is reserved but not implemented"
                )))
            } else {
                Err(Error::unsupported(format!("unknown codec id {other}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_is_available() {
        assert!(is_implemented(CODEC_RAW));
        assert_eq!(get_codec(CODEC_RAW).unwrap().codec_id(), CODEC_RAW);
    }

    #[test]
    fn palette_is_available() {
        assert!(is_implemented(CODEC_PALETTE));
        assert_eq!(get_codec(CODEC_PALETTE).unwrap().codec_id(), CODEC_PALETTE);
        assert_eq!(get_codec(CODEC_PALETTE).unwrap().name(), "palette");
    }

    #[test]
    fn rle_is_available() {
        assert!(is_implemented(CODEC_RLE));
        assert_eq!(get_codec(CODEC_RLE).unwrap().codec_id(), CODEC_RLE);
        assert_eq!(get_codec(CODEC_RLE).unwrap().name(), "rle");
    }

    #[test]
    fn implemented_ids_are_ascending_and_consistent() {
        let ids = IMPLEMENTED_CODEC_IDS;
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids must ascend");
        for &id in ids {
            assert!(is_implemented(id));
            assert_eq!(get_codec(id).unwrap().codec_id(), id);
            assert!(codec_name(id).is_some());
        }
        // The Raw fallback must always be present.
        assert!(ids.contains(&CODEC_RAW));
    }

    #[test]
    fn predictor_is_available() {
        assert!(is_implemented(CODEC_PREDICTOR));
        assert_eq!(
            get_codec(CODEC_PREDICTOR).unwrap().codec_id(),
            CODEC_PREDICTOR
        );
        assert_eq!(get_codec(CODEC_PREDICTOR).unwrap().name(), "predictor");
    }

    #[test]
    fn the_promoted_codecs_are_available() {
        for (id, name) in [
            (CODEC_LZ77, "lz77"),
            (CODEC_PREDICTOR_DICTIONARY, "predictor+dictionary"),
            (CODEC_CONTEXT_ENTROPY, "context-entropy"),
        ] {
            assert!(is_implemented(id), "codec id {id} must be implemented");
            let codec = get_codec(id).unwrap();
            assert_eq!(codec.codec_id(), id);
            assert_eq!(codec.name(), name);
            assert_eq!(codec_name(id), Some(name));
            assert!(is_v0_3_codec(id));
            assert!(!CODEC_IDS_V0_2.contains(&id));
        }
    }

    #[test]
    fn the_v0_2_codec_set_is_the_four_original_codecs() {
        assert_eq!(CODEC_IDS_V0_2, &[0, 1, 2, 3]);
        assert_eq!(CODEC_IDS_V0_3, IMPLEMENTED_CODEC_IDS);
        for &id in CODEC_IDS_V0_2 {
            assert!(!is_v0_3_codec(id));
        }
    }

    #[test]
    fn unknown_codec_is_unsupported() {
        assert!(codec_name(200).is_none());
        assert_eq!(
            get_codec(200).err().unwrap().kind(),
            vpng_core::ErrorKind::UnsupportedFeature
        );
    }
}
