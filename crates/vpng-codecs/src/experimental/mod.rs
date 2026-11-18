//! `vpng-codecs::experimental` — **research** codecs that are not part of the
//! stable VPNG format.
//!
//! # Why this module is separate
//!
//! The stable format has exactly four implemented codec ids (0 Raw, 1 Palette,
//! 2 Run-length, 3 Predictor+entropy) and three reserved ids (4, 5, 6) that
//! the specification has already claimed for future production bitstreams. A
//! research codec must therefore never take one of those ids, and must never
//! appear in [`crate::IMPLEMENTED_CODEC_IDS`] or be reachable through
//! [`crate::get_codec`].
//!
//! Every codec in this module:
//!
//! * uses a **research-only id** in the `0x80..=0xFF` range, so it can never
//!   collide with a stable or reserved id;
//! * is therefore **undecodable by the production decoder** — a file carrying
//!   one is rejected with `UnsupportedFeature`, which is the honest outcome;
//! * implements [`TileCodec`], so it obeys the same exactness, determinism,
//!   bounded-memory and independent-tile rules as a stable codec;
//! * is measured by the research harness, not by the production encoder.
//!
//! Nothing in this module changes the bytes of a stable VPNG file. The stable
//! codecs are untouched; this module only *reads* the shared sample arithmetic
//! and the shared arithmetic coder from [`crate::predictor`].
//!
//! # Experimental ids
//!
//! | id | codec | notes |
//! |---:|-------|-------|
//! | `0x80` | CARC model A | independent residual model (baseline) |
//! | `0x81` | CARC model B | neighbour-conditioned |
//! | `0x82` | CARC model C | gradient-conditioned |
//! | `0x83` | CARC model D | combined spatial + channel |
//! | `0x90` | LZ77 | tile-local bounded dictionary |
//! | `0x91` | Predictor + LZ77 | hybrid |
//! | `0x92` | LZ77 + entropy | hybrid |
//! | `0x93` | Predictor + LZ77 + entropy | hybrid |
//!
//! These ids are **research handles only**. They are not a specification.

pub mod carc;
pub mod entropy;
pub mod hybrid;
pub mod lz77;

use crate::TileCodec;

pub use carc::{CarcCodec, CarcModel};
pub use hybrid::{HybridCodec, HybridKind};
pub use lz77::{Lz77Codec, Lz77Params, Lz77Strategy};

/// Research-only codec id: CARC model A (independent residual model).
pub const EXP_CARC_A: u8 = 0x80;
/// Research-only codec id: CARC model B (neighbour-conditioned).
pub const EXP_CARC_B: u8 = 0x81;
/// Research-only codec id: CARC model C (gradient-conditioned).
pub const EXP_CARC_C: u8 = 0x82;
/// Research-only codec id: CARC model D (combined spatial + channel).
pub const EXP_CARC_D: u8 = 0x83;
/// Research-only codec id: standalone LZ77.
pub const EXP_LZ77: u8 = 0x90;
/// Research-only codec id: Predictor followed by LZ77.
pub const EXP_PRED_LZ77: u8 = 0x91;
/// Research-only codec id: LZ77 followed by entropy coding.
pub const EXP_LZ77_ENTROPY: u8 = 0x92;
/// Research-only codec id: Predictor, then LZ77, then entropy coding.
pub const EXP_PRED_LZ77_ENTROPY: u8 = 0x93;

/// The standalone LZ77 codec with the default (balanced) parameters.
pub static LZ77: Lz77Codec = Lz77Codec::new(Lz77Params::DEFAULT);

/// The four CARC model variants, as concrete codec instances.
pub static CARC_A: CarcCodec = CarcCodec::new(CarcModel::Independent);
pub static CARC_B: CarcCodec = CarcCodec::new(CarcModel::NeighbourConditioned);
pub static CARC_C: CarcCodec = CarcCodec::new(CarcModel::GradientConditioned);
pub static CARC_D: CarcCodec = CarcCodec::new(CarcModel::CombinedSpatialChannel);

/// Predictor followed by LZ77.
pub static PRED_LZ77: HybridCodec = HybridCodec::new(HybridKind::PredictorThenLz77);
/// LZ77 followed by entropy coding.
pub static LZ77_ENTROPY: HybridCodec = HybridCodec::new(HybridKind::Lz77ThenEntropy);
/// Predictor, then LZ77, then entropy coding.
pub static PRED_LZ77_ENTROPY: HybridCodec =
    HybridCodec::new(HybridKind::PredictorThenLz77ThenEntropy);

/// Every experimental codec, in a fixed order.
///
/// The order is stable and deterministic so a benchmark report lists the
/// candidates in the same sequence on every run.
pub fn all() -> Vec<(&'static str, &'static dyn TileCodec)> {
    vec![
        ("carc-a", &CARC_A as &dyn TileCodec),
        ("carc-b", &CARC_B as &dyn TileCodec),
        ("carc-c", &CARC_C as &dyn TileCodec),
        ("carc-d", &CARC_D as &dyn TileCodec),
        ("lz77", &LZ77 as &dyn TileCodec),
        ("pred-lz77", &PRED_LZ77 as &dyn TileCodec),
        ("lz77-entropy", &LZ77_ENTROPY as &dyn TileCodec),
        ("pred-lz77-entropy", &PRED_LZ77_ENTROPY as &dyn TileCodec),
    ]
}

/// Look up an experimental codec by its research name.
///
/// The name is the same string the codec's [`TileCodec::name`] returns.
pub fn by_name(name: &str) -> Option<&'static dyn TileCodec> {
    all().into_iter().find(|(n, _)| *n == name).map(|(_, c)| c)
}

/// Look up an experimental codec by its research id.
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
    fn experimental_ids_never_collide_with_stable_or_reserved_ids() {
        for &stable in IMPLEMENTED_CODEC_IDS {
            assert!(!is_experimental_id(stable), "stable id {stable} collided");
        }
        for reserved in [4u8, 5, 6] {
            assert!(
                !is_experimental_id(reserved),
                "reserved id {reserved} collided"
            );
        }
    }

    #[test]
    fn experimental_codecs_are_not_in_the_stable_registry() {
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
    fn every_experimental_codec_has_a_unique_id_and_a_lookup() {
        let mut ids = Vec::new();
        for (name, codec) in all() {
            ids.push(codec.codec_id());
            assert_eq!(by_name(name).unwrap().codec_id(), codec.codec_id());
            assert_eq!(by_id(codec.codec_id()).unwrap().name(), codec.name());
        }
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate experimental codec id");
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
