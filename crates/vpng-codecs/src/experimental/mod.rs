//! `vpng-codecs::experimental` — **research** codecs that are not part of the
//! stable VPNG format.
//!
//! # Why this module is separate
//!
//! Three of the eight research codecs were promoted into the stable format
//! in **v0.3**: the standalone LZ77 dictionary (id 4), prediction layered over
//! that dictionary (id 5), and the gradient-conditioned context-adaptive model
//! (id 6). They live in [`crate::lz77`], [`crate::hybrid`] and [`crate::carc`]
//! now, and they are reachable through [`crate::get_codec`].
//!
//! The five codecs that remain in this module:
//!
//! * use a **research-only id** in the `0x80..=0xFF` range, so they can never
//!   collide with a stable id;
//! * are therefore **undecodable by the production decoder** — a file carrying
//!   one is rejected with `UnsupportedFeature`, which is the honest outcome;
//! * implement [`TileCodec`], so they obey the same exactness, determinism,
//!   bounded-memory and independent-tile rules as a stable codec;
//! * are measured by the research harness, not by the production encoder.
//!
//! Nothing in this module changes the bytes of a stable VPNG file. The stable
//! codecs are untouched; this module only *reads* the shared sample arithmetic
//! and the shared arithmetic coder from [`crate::predictor`].
//!
//! # Research ids
//!
//! | id | codec | notes |
//! |---:|-------|-------|
//! | `0x80` | CARC model A | independent residual model (baseline) |
//! | `0x81` | CARC model B | neighbour-conditioned |
//! | `0x83` | CARC model D | combined spatial + channel |
//! | `0x92` | LZ77 + entropy | hybrid |
//! | `0x93` | Predictor + LZ77 + entropy | hybrid |
//!
//! These ids are **research handles only**. They are not a specification.
//!
//! # Retired research ids
//!
//! Three research ids were retired when their codecs were promoted. They
//! never named a stable codec, and they are kept here only so a file that
//! carries one is still recognisably a research file:
//!
//! | id | codec | became |
//! |---:|-------|--------|
//! | `0x82` | CARC model C | stable id 6 |
//! | `0x90` | LZ77 | stable id 4 |
//! | `0x91` | Predictor + LZ77 | stable id 5 |

use crate::TileCodec;

pub use crate::carc::{CarcCodec, CarcModel};
pub use crate::hybrid::{HybridCodec, HybridKind};
pub use crate::lz77::{Lz77Codec, Lz77Params, Lz77Strategy};

/// Research-only codec id: CARC model A (independent residual model).
pub const EXP_CARC_A: u8 = 0x80;
/// Research-only codec id: CARC model B (neighbour-conditioned).
pub const EXP_CARC_B: u8 = 0x81;
/// Research-only codec id: CARC model D (combined spatial + channel).
pub const EXP_CARC_D: u8 = 0x83;
/// Research-only codec id: LZ77 followed by entropy coding.
pub const EXP_LZ77_ENTROPY: u8 = 0x92;
/// Research-only codec id: Predictor, then LZ77, then entropy coding.
pub const EXP_PRED_LZ77_ENTROPY: u8 = 0x93;

/// Retired research id: CARC model C. Promoted to stable id 6.
pub const EXP_CARC_C: u8 = 0x82;
/// Retired research id: standalone LZ77. Promoted to stable id 4.
pub const EXP_LZ77: u8 = 0x90;
/// Retired research id: Predictor followed by LZ77. Promoted to stable id 5.
pub const EXP_PRED_LZ77: u8 = 0x91;

/// Every **research-only** codec, in a fixed order.
///
/// The order is ascending research id, so it is also the tie-break order of
/// [`select_best`] once the stable codecs have had their turn.
pub fn all() -> Vec<(&'static str, &'static dyn TileCodec)> {
    vec![
        ("carc-a", &crate::carc::CARC_A as &dyn TileCodec),
        ("carc-b", &crate::carc::CARC_B as &dyn TileCodec),
        ("carc-d", &crate::carc::CARC_D as &dyn TileCodec),
        (
            "lz77-entropy",
            &crate::hybrid::LZ77_ENTROPY_CODEC as &dyn TileCodec,
        ),
        (
            "pred-lz77-entropy",
            &crate::hybrid::PREDICTOR_LZ77_ENTROPY_CODEC as &dyn TileCodec,
        ),
    ]
}

/// Look up a research-only codec by its research name.
///
/// The name is the same string the codec's [`TileCodec::name`] returns.
pub fn by_name(name: &str) -> Option<&'static dyn TileCodec> {
    all().into_iter().find(|(n, _)| *n == name).map(|(_, c)| c)
}

/// Look up a research-only codec by its research id.
pub fn by_id(id: u8) -> Option<&'static dyn TileCodec> {
    all()
        .into_iter()
        .find(|(_, c)| c.codec_id() == id)
        .map(|(_, c)| c)
}

/// True when `id` belongs to the experimental research range.
///
/// A production decoder must reject such an id with `UnsupportedFeature`; it
/// must never guess at the bitstream.
pub fn is_experimental_id(id: u8) -> bool {
    id >= 0x80
}

// ---------------------------------------------------------------------------
// Candidate selection
// ---------------------------------------------------------------------------

/// One encoded candidate for a tile.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The candidate's name.
    pub name: &'static str,
    /// The codec id the candidate's codec uses.
    pub codec_id: u8,
    /// The complete payload, framing bytes included.
    pub payload: Vec<u8>,
}

/// Every candidate codec, in a fixed order.
///
/// The stable codecs come first, in ascending id order, then the research
/// codecs in [`all`]'s order. This order is the tie-break order of
/// [`select_best`], so a selection is reproducible.
pub fn candidate_codecs() -> Vec<(&'static str, &'static dyn TileCodec)> {
    let mut out: Vec<(&'static str, &'static dyn TileCodec)> = vec![
        ("raw", &crate::raw::RAW_CODEC as &dyn TileCodec),
        ("palette", &crate::palette::PALETTE_CODEC as &dyn TileCodec),
        ("rle", &crate::rle::RLE_CODEC as &dyn TileCodec),
        (
            "predictor",
            &crate::predictor::PREDICTOR_CODEC as &dyn TileCodec,
        ),
        ("lz77", &crate::lz77::LZ77_CODEC as &dyn TileCodec),
        (
            "predictor+dictionary",
            &crate::hybrid::PREDICTOR_DICTIONARY_CODEC as &dyn TileCodec,
        ),
        (
            "context-entropy",
            &crate::carc::CONTEXT_ENTROPY_CODEC as &dyn TileCodec,
        ),
    ];
    out.extend(all());
    out
}

/// Encode a tile with every candidate codec.
///
/// A codec that cannot represent the tile at all — a palette meeting a 257th
/// distinct colour, for instance — is skipped. Every other error is
/// propagated, because it means the candidate is broken rather than
/// inapplicable.
pub fn encode_candidates(
    input: &[u8],
    ctx: &crate::EncodeContext,
) -> vpng_core::Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for (name, codec) in candidate_codecs() {
        let mut payload = Vec::new();
        match codec.encode(input, ctx, &mut payload) {
            Ok(()) => out.push(Candidate {
                name,
                codec_id: codec.codec_id(),
                payload,
            }),
            Err(error) if error.kind() == vpng_core::ErrorKind::UnsupportedFeature => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(out)
}

/// Choose the smallest candidate.
///
/// The comparison uses the **complete payload size**, every framing byte
/// included — not just the compressed data. Ties go to the candidate that
/// comes first in [`candidate_codecs`], so the choice is deterministic and
/// reproducible.
///
/// Raw is always a candidate, so a selection always exists.
pub fn select_best(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .min_by_key(|candidate| candidate.payload.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IMPLEMENTED_CODEC_IDS;

    #[test]
    fn experimental_ids_never_collide_with_stable_ids() {
        for &stable in IMPLEMENTED_CODEC_IDS {
            assert!(!is_experimental_id(stable), "stable id {stable} collided");
        }
    }

    #[test]
    fn research_codecs_are_not_in_the_stable_registry() {
        for (name, codec) in all() {
            assert!(is_experimental_id(codec.codec_id()), "{name}");
            assert!(
                !IMPLEMENTED_CODEC_IDS.contains(&codec.codec_id()),
                "{name} must not be in IMPLEMENTED_CODEC_IDS"
            );
            assert!(
                crate::get_codec(codec.codec_id()).is_err(),
                "{name} must not be reachable from get_codec"
            );
        }
    }

    #[test]
    fn every_research_codec_has_a_unique_id_and_a_lookup() {
        let mut ids = Vec::new();
        for (name, codec) in all() {
            ids.push(codec.codec_id());
            assert_eq!(by_name(name).unwrap().codec_id(), codec.codec_id());
            assert_eq!(by_id(codec.codec_id()).unwrap().name(), codec.name());
        }
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate research codec id");
    }

    #[test]
    fn the_candidate_list_is_in_ascending_codec_id_order() {
        let ids: Vec<u8> = candidate_codecs()
            .iter()
            .map(|(_, codec)| codec.codec_id())
            .collect();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "candidate ids must ascend: {ids:?}"
        );
    }

    #[test]
    fn the_promoted_codecs_are_candidates() {
        let names: Vec<&str> = candidate_codecs().iter().map(|(name, _)| *name).collect();
        for name in [
            "raw",
            "palette",
            "rle",
            "predictor",
            "lz77",
            "predictor+dictionary",
            "context-entropy",
        ] {
            assert!(names.contains(&name), "missing candidate {name}");
        }
    }

    #[test]
    fn candidate_selection_prefers_the_smallest_complete_payload() {
        let format = vpng_core::PixelFormat::rgba8();
        let limits = vpng_core::Limits::default();
        let ctx = crate::EncodeContext {
            limits,
            format,
            tile_width: 16,
            tile_height: 16,
        };
        let mut tile = vec![0u8; 16 * 16 * 4];
        for (index, byte) in tile.iter_mut().enumerate() {
            *byte = (index % 16) as u8;
        }
        let candidates = encode_candidates(&tile, &ctx).unwrap();
        assert!(!candidates.is_empty());
        assert!(candidates.iter().any(|c| c.name == "raw"));
        let best = select_best(&candidates).unwrap();
        let smallest = candidates.iter().map(|c| c.payload.len()).min().unwrap();
        assert_eq!(best.payload.len(), smallest);
        // The selection must be stable across calls.
        let repeated = encode_candidates(&tile, &ctx).unwrap();
        let again = select_best(&repeated).unwrap();
        assert_eq!(best.name, again.name);
    }
}
