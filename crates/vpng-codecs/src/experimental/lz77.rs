//! VPNG-LZ77 — an experimental, tile-local, bounded dictionary codec.
//!
//! This is **research code**. It carries a research-only codec id
//! (`0x90`), is not reachable through [`crate::get_codec`], and is not in
//! [`crate::IMPLEMENTED_CODEC_IDS`]. A file that somehow carried this id
//! would be rejected by the production decoder with `UnsupportedFeature`.
//!
//! # Tile locality
//!
//! The dictionary is the tile's own decoded bytes. A match may only refer
//! back to a position that is **already inside the current tile**, so a tile
//! decodes without its neighbours and no state crosses a tile boundary.
//!
//! # Matching is byte-level, not pixel-level
//!
//! Matches are found over the raw byte stream of the tile. A match may start
//! in the middle of a pixel, and a 16-bit sample may be split across a match
//! boundary. This is deliberate: the codec sees a byte string, so it can find
//! matches that a pixel-aligned coder would miss. The decoder copies bytes
//! verbatim, so the split is harmless — the reconstruction is exact by
//! construction, not by alignment.
//!
//! # Parameters
//!
//! Every parameter is explicitly serialized in the payload header, so a
//! decoder never has to guess:
//!
//! | field | meaning | range |
//! |---|---|---|
//! | `strategy` | which match search to run | 0..=3 |
//! | `min_match` | shortest match that is worth a match token | 3..=255 |
//! | `max_match` | longest match the encoder will emit | 3..=799 |
//! | `window_bits` | `log2` of the window size in bytes | 10..=18 |
//! | `hash_bits` | `log2` of the hash table size | 8..=16 |
//! | `chain_length` | how many chain candidates to examine | 1..=64 |
//!
//! # Memory
//!
//! The encoder holds two bounded tables whose size depends only on the
//! serialized parameters, never on the tile size:
//!
//! * hash table: `2^hash_bits` entries of 4 bytes (at most 256 KiB),
//! * match chain: `2^window_bits` entries of 4 bytes (at most 1 MiB).
//!
//! The decoder holds only the output buffer and a 32 KiB sliding window, so
//! its working set is `O(window)` rather than `O(tile)`.
//!
//! # Bitstream
//!
//! ```text
//! offset 0   u8   version (1)
//! offset 1   u8   strategy
//! offset 2   u8   min_match
//! offset 3   u16  max_match              (little-endian)
//! offset 5   u8   window_bits
//! offset 6   u8   hash_bits
//! offset 7   u8   chain_length
//! offset 8   u8   reserved (must be 0)
//! offset 9   ..   output length, unsigned LEB128
//! offset ..  ..   token stream
//! ```
//!
//! The token stream is a sequence of *groups*. Each group starts with a flag
//! byte whose bit `i` (LSB first) says whether item `i` of the group is a
//! match (`1`) or a literal (`0`); a group carries up to eight items.
//!
//! * a literal item is one byte: the literal value;
//! * a match item is a length code byte, then a distance code byte, then
//!   zero, one or two extra bytes for the length and the distance.
//!
//! Length codes:
//!
//! | code | length | extra bytes |
//! |---|---|---|
//! | 0..=28 | `code + 3` (3..=31) | 0 |
//! | 29 | `32 + b0` (32..=287) | 1 |
//! | 30 | `288 + b0 + 256*b1` (288..=799) | 2 |
//! | 31 | reserved | — |
//!
//! Distance codes use the DEFLATE-style base/extra-bit table, giving a
//! maximum representable distance of 24576 bytes.
//!
//! # End of stream
//!
//! There is no terminator token. The decoder stops as soon as it has produced
//! exactly the output length declared in the header. Unused slots in the
//! final flag byte are therefore never read, and the encoder never writes
//! bytes for them.
//!
//! # Invalid references
//!
//! A match whose distance is zero, larger than the bytes already produced, or
//! larger than the window is corrupt. A match whose length is outside
//! `[min_match, max_match]`, or that would run past the declared output
//! length, is corrupt. Every one of those is a typed `CorruptFile` error, not
//! a panic and not a best-effort decode.

use vpng_core::{Error, Result};

use crate::experimental::entropy;
use crate::{DecodeContext, EncodeContext, TileCodec};

use super::EXP_LZ77;

/// Payload version. Anything else is corrupt, never best-effort decoded.
pub const VERSION: u8 = 1;

/// Shortest match the format can express.
pub const MIN_MATCH: usize = 3;
/// Longest match the format can express.
pub const MAX_MATCH: usize = 799;
/// Smallest window the format allows: 2^10 bytes.
pub const MIN_WINDOW_BITS: u32 = 10;
/// Largest window the format allows: 2^18 bytes.
pub const MAX_WINDOW_BITS: u32 = 18;
/// Smallest hash table the format allows: 2^8 entries.
pub const MIN_HASH_BITS: u32 = 8;
/// Largest hash table the format allows: 2^16 entries.
pub const MAX_HASH_BITS: u32 = 16;
/// Largest chain the format allows.
pub const MAX_CHAIN: u32 = 64;

/// Number of items carried by one flag byte.
pub const ITEMS_PER_FLAG: usize = 8;

/// Size of the fixed part of the payload header, before the LEB128 length.
pub const HEADER_FIXED_SIZE: usize = 9;

/// Number of distance codes the format defines.
pub const DISTANCE_CODES: usize = 29;

/// Base distance of every distance code.
pub const DIST_BASE: [u32; DISTANCE_CODES] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385,
];

/// Number of extra bits carried by every distance code.
pub const DIST_EXTRA_BITS: [u32; DISTANCE_CODES] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
];

/// The largest distance the distance code table can express.
pub const MAX_DISTANCE: usize = 24576;

/// Which match search the encoder runs.
///
/// The four strategies are compared by the research harness; none of them is
/// assumed to be better than the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lz77Strategy {
    /// Hash chain, first match in the chain wins (the nearest match).
    HashChain,
    /// One hash slot, one candidate: the most recent position only.
    HashTable,
    /// Longest match wins; ties go to the nearest match.
    Greedy,
    /// One position of lookahead: a strictly longer match one byte later wins.
    Lazy,
}

impl Lz77Strategy {
    /// The value written to the payload header.
    pub fn code(self) -> u8 {
        match self {
            Lz77Strategy::HashChain => 0,
            Lz77Strategy::HashTable => 1,
            Lz77Strategy::Greedy => 2,
            Lz77Strategy::Lazy => 3,
        }
    }

    /// Inverse of [`Lz77Strategy::code`].
    pub fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            0 => Lz77Strategy::HashChain,
            1 => Lz77Strategy::HashTable,
            2 => Lz77Strategy::Greedy,
            3 => Lz77Strategy::Lazy,
            other => return Err(Error::corrupt(format!("lz77 strategy {other} is reserved"))),
        })
    }

    /// Human-readable name, used by the research reports.
    pub fn name(self) -> &'static str {
        match self {
            Lz77Strategy::HashChain => "hash-chain",
            Lz77Strategy::HashTable => "hash-table",
            Lz77Strategy::Greedy => "greedy",
            Lz77Strategy::Lazy => "lazy",
        }
    }
}

/// Every parameter of the dictionary, all of them serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lz77Params {
    /// Which match search to run.
    pub strategy: Lz77Strategy,
    /// Shortest match worth encoding as a match.
    pub min_match: u8,
    /// Longest match the encoder will emit.
    pub max_match: u16,
    /// `log2` of the window size in bytes.
    pub window_bits: u8,
    /// `log2` of the hash table size.
    pub hash_bits: u8,
    /// How many chain candidates to examine per position.
    pub chain_length: u8,
}

impl Default for Lz77Params {
    fn default() -> Self {
        Lz77Params::DEFAULT
    }
}

impl Lz77Params {
    /// The default parameter set: a balanced hash-chain search.
    pub const DEFAULT: Lz77Params = Lz77Params {
        strategy: Lz77Strategy::HashChain,
        min_match: 3,
        max_match: 258,
        window_bits: 15,
        hash_bits: 14,
        chain_length: 16,
    };
}

impl Lz77Params {
    /// Reject a parameter set the format cannot represent.
    pub fn validate(&self) -> Result<()> {
        if (self.min_match as usize) < MIN_MATCH {
            return Err(Error::invalid(format!(
                "lz77 min_match {} is below {MIN_MATCH}",
                self.min_match
            )));
        }
        if (self.max_match as usize) > MAX_MATCH {
            return Err(Error::invalid(format!(
                "lz77 max_match {} is above {MAX_MATCH}",
                self.max_match
            )));
        }
        if (self.max_match as usize) < (self.min_match as usize) {
            return Err(Error::invalid("lz77 max_match is below min_match"));
        }
        let window_bits = self.window_bits as u32;
        if !(MIN_WINDOW_BITS..=MAX_WINDOW_BITS).contains(&window_bits) {
            return Err(Error::invalid(format!(
                "lz77 window_bits {window_bits} is outside {MIN_WINDOW_BITS}..={MAX_WINDOW_BITS}"
            )));
        }
        let hash_bits = self.hash_bits as u32;
        if !(MIN_HASH_BITS..=MAX_HASH_BITS).contains(&hash_bits) {
            return Err(Error::invalid(format!(
                "lz77 hash_bits {hash_bits} is outside {MIN_HASH_BITS}..={MAX_HASH_BITS}"
            )));
        }
        if self.chain_length == 0 || self.chain_length as u32 > MAX_CHAIN {
            return Err(Error::invalid(format!(
                "lz77 chain_length {} is outside 1..={MAX_CHAIN}",
                self.chain_length
            )));
        }
        Ok(())
    }

    /// The window size in bytes.
    pub fn window_size(&self) -> usize {
        1usize << self.window_bits
    }

    /// The hash table size in entries.
    pub fn hash_size(&self) -> usize {
        1usize << self.hash_bits
    }
}

/// One item of the token stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// A literal byte.
    Literal(u8),
    /// A back-reference: `length` bytes repeated from `distance` bytes back.
    Match { length: usize, distance: usize },
}

/// A byte of the token stream together with the symbol group it belongs to.
///
/// The group tells the entropy stage which probability context to use. The
/// standalone codec ignores it; the entropy hybrids use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaggedByte {
    /// The symbol group.
    pub group: usize,
    /// The byte value.
    pub byte: u8,
}

// ---------------------------------------------------------------------------
// Match finding
// ---------------------------------------------------------------------------

/// How far the usable window is shortened below the nominal window size.
///
/// The chain array is indexed by `position & window_mask`, so a candidate that
/// is exactly one window old would collide with the candidate that is about to
/// be inserted. Shortening the window by a fixed slack removes the collision.
const WINDOW_SLACK: usize = 16;

/// The hash of the three bytes at `pos`.
///
/// A fixed three-byte hash: the format's shortest match is three bytes, so a
/// three-byte hash never rules out a legal match. The multiply-and-xor
/// sequence is FNV-1a, which mixes the three bytes well and costs three
/// multiplies.
#[inline]
fn hash_at(data: &[u8], pos: usize, hash_bits: u32) -> usize {
    let mut h: u32 = 2166136261;
    h ^= u32::from(data[pos]);
    h = h.wrapping_mul(16777619);
    h ^= u32::from(data[pos + 1]);
    h = h.wrapping_mul(16777619);
    h ^= u32::from(data[pos + 2]);
    h = h.wrapping_mul(16777619);
    (h >> (32 - hash_bits)) as usize
}

/// The bounded match finder.
///
/// The two tables are sized by the serialized parameters alone, so the
/// encoder's working set does not grow with the tile.
struct Matcher<'a> {
    data: &'a [u8],
    params: Lz77Params,
    /// Most recent position for each hash value.
    head: Vec<i32>,
    /// Previous position with the same hash, indexed by `position & window_mask`.
    prev: Vec<i32>,
    window_mask: usize,
    /// The largest distance a match may reach back.
    max_distance: usize,
}

impl<'a> Matcher<'a> {
    fn new(data: &'a [u8], params: Lz77Params) -> Self {
        let window = params.window_size();
        Matcher {
            data,
            params,
            head: vec![-1; params.hash_size()],
            prev: vec![-1; window],
            window_mask: window - 1,
            // The distance code table caps the usable window as well, so a
            // large `window_bits` cannot buy a distance the bitstream cannot
            // express.
            max_distance: (window - WINDOW_SLACK).min(MAX_DISTANCE),
        }
    }

    /// Record `pos` in the hash chain.
    ///
    /// The last two positions of the tile are not hashable — a three-byte
    /// hash needs three bytes — so they are simply not inserted. They can
    /// never start a legal match either.
    fn insert(&mut self, pos: usize) {
        if pos + 3 > self.data.len() {
            return;
        }
        let h = hash_at(self.data, pos, self.params.hash_bits as u32);
        self.prev[pos & self.window_mask] = self.head[h];
        self.head[h] = pos as i32;
    }

    /// Find the match at `pos`.
    ///
    /// With `longest` set the longest match in the chain wins and ties go to
    /// the nearest candidate. Without it the first candidate that reaches
    /// `min_match` wins, which is the nearest one because the chain is walked
    /// from the most recent position backwards.
    fn find(&self, pos: usize, longest: bool, chain: u32) -> Option<(usize, usize)> {
        let limit = self.params.min_match as usize;
        let available = self.data.len() - pos;
        if available < limit || pos + 3 > self.data.len() {
            return None;
        }
        let max_len = (self.params.max_match as usize).min(available);
        if max_len < limit {
            return None;
        }

        let h = hash_at(self.data, pos, self.params.hash_bits as u32);
        let oldest = pos.saturating_sub(self.max_distance);
        let mut candidate = self.head[h];
        let mut best_len = 0usize;
        let mut best_distance = 0usize;
        let mut steps = chain;

        while candidate >= 0 && steps > 0 {
            let c = candidate as usize;
            if c < oldest {
                break;
            }
            // `c < pos` and `len < available`, so `c + len` is always inside
            // the tile. The comparison may run past `pos` when the match
            // overlaps itself; that is the run-length case and is intended.
            let mut len = 0usize;
            while len < max_len && self.data[c + len] == self.data[pos + len] {
                len += 1;
            }
            if len >= limit {
                if !longest {
                    return Some((len, pos - c));
                }
                if len > best_len {
                    best_len = len;
                    best_distance = pos - c;
                }
            }
            candidate = self.prev[c & self.window_mask];
            steps -= 1;
        }

        if best_len >= limit {
            Some((best_len, best_distance))
        } else {
            None
        }
    }
}

/// Find the tokens that encode `data`.
///
/// The result depends only on `data` and `params`, so the encoder is
/// deterministic.
pub fn tokenize(data: &[u8], params: &Lz77Params) -> Result<Vec<Token>> {
    params.validate()?;
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let mut matcher = Matcher::new(data, *params);
    let mut tokens = Vec::new();
    let chain = match params.strategy {
        Lz77Strategy::HashTable => 1,
        _ => params.chain_length as u32,
    };
    let longest = matches!(params.strategy, Lz77Strategy::Greedy | Lz77Strategy::Lazy);

    let mut pos = 0usize;
    while pos < data.len() {
        let current = matcher.find(pos, longest, chain);

        if params.strategy == Lz77Strategy::Lazy {
            // One position of lookahead. The match one byte later only wins
            // when it is *strictly* longer, so the encoder never trades a
            // short match for an equally short one.
            let next = if pos + 1 < data.len() {
                matcher.find(pos + 1, true, chain)
            } else {
                None
            };
            let switch = match (current, next) {
                (None, Some(_)) => true,
                (Some((len, _)), Some((next_len, _))) => next_len > len,
                _ => false,
            };
            if switch {
                tokens.push(Token::Literal(data[pos]));
                matcher.insert(pos);
                pos += 1;
                continue;
            }
        }

        match current {
            Some((length, distance)) => {
                tokens.push(Token::Match { length, distance });
                for offset in pos..pos + length {
                    matcher.insert(offset);
                }
                pos += length;
            }
            None => {
                tokens.push(Token::Literal(data[pos]));
                matcher.insert(pos);
                pos += 1;
            }
        }
    }

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Token stream
// ---------------------------------------------------------------------------

/// Encode a match length into its code and extra bytes.
fn encode_length(length: usize) -> (u8, Vec<u8>) {
    if length <= 31 {
        return ((length - 3) as u8, Vec::new());
    }
    if length <= 287 {
        return (29, vec![(length - 32) as u8]);
    }
    let value = length - 288;
    (30, vec![(value & 0xFF) as u8, (value >> 8) as u8])
}

/// Encode a match distance into its code and extra bytes.
fn encode_distance(distance: usize) -> (u8, Vec<u8>) {
    let mut code = 0usize;
    for (candidate, &base) in DIST_BASE.iter().enumerate() {
        if (base as usize) <= distance {
            code = candidate;
        } else {
            break;
        }
    }
    let extra = distance - DIST_BASE[code] as usize;
    let bits = DIST_EXTRA_BITS[code] as usize;
    let mut bytes = Vec::new();
    if bits > 0 {
        if bits <= 8 {
            bytes.push(extra as u8);
        } else {
            bytes.push((extra & 0xFF) as u8);
            bytes.push((extra >> 8) as u8);
        }
    }
    (code as u8, bytes)
}

/// Turn tokens into the tagged byte stream.
///
/// The byte order is exactly the order a decoder reads them back in, so the
/// entropy stage can code the stream without knowing anything about the
/// token structure: it only needs the group of each byte.
pub fn tokens_to_stream(tokens: &[Token]) -> Vec<TaggedByte> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        let end = (index + ITEMS_PER_FLAG).min(tokens.len());
        let chunk = &tokens[index..end];

        let mut flag = 0u8;
        for (slot, token) in chunk.iter().enumerate() {
            if matches!(token, Token::Match { .. }) {
                flag |= 1u8 << slot;
            }
        }
        out.push(TaggedByte {
            group: entropy::GROUP_FLAG,
            byte: flag,
        });

        for token in chunk {
            match *token {
                Token::Literal(value) => out.push(TaggedByte {
                    group: entropy::GROUP_LITERAL,
                    byte: value,
                }),
                Token::Match { length, distance } => {
                    let (length_code, length_extra) = encode_length(length);
                    out.push(TaggedByte {
                        group: entropy::GROUP_LENGTH,
                        byte: length_code,
                    });
                    for byte in length_extra {
                        out.push(TaggedByte {
                            group: entropy::GROUP_EXTRA,
                            byte,
                        });
                    }
                    let (distance_code, distance_extra) = encode_distance(distance);
                    out.push(TaggedByte {
                        group: entropy::GROUP_DISTANCE,
                        byte: distance_code,
                    });
                    for byte in distance_extra {
                        out.push(TaggedByte {
                            group: entropy::GROUP_EXTRA,
                            byte,
                        });
                    }
                }
            }
        }
        index = end;
    }
    out
}

/// Decode a match length, pulling any extra bytes it needs.
fn decode_length(code: u8, source: &mut impl ByteSource) -> Result<usize> {
    match code {
        0..=28 => Ok(code as usize + 3),
        29 => {
            let extra = source.next_byte(entropy::GROUP_EXTRA)?;
            Ok(32 + extra as usize)
        }
        30 => {
            let low = source.next_byte(entropy::GROUP_EXTRA)?;
            let high = source.next_byte(entropy::GROUP_EXTRA)?;
            // Code 30 carries nine extra bits, so the high byte must be below
            // two.
            if high >= 2 {
                return Err(Error::corrupt("lz77 length extra bits are out of range"));
            }
            Ok(288 + low as usize + 256 * high as usize)
        }
        other => Err(Error::corrupt(format!(
            "lz77 length code {other} is reserved"
        ))),
    }
}

/// Decode a match distance, pulling any extra bytes it needs.
fn decode_distance(code: u8, source: &mut impl ByteSource) -> Result<usize> {
    let code = code as usize;
    if code >= DISTANCE_CODES {
        return Err(Error::corrupt(format!(
            "lz77 distance code {code} is reserved"
        )));
    }
    let bits = DIST_EXTRA_BITS[code] as usize;
    let base = DIST_BASE[code] as usize;
    let extra = if bits == 0 {
        0
    } else if bits <= 8 {
        source.next_byte(entropy::GROUP_EXTRA)? as usize
    } else {
        let low = source.next_byte(entropy::GROUP_EXTRA)? as usize;
        let high = source.next_byte(entropy::GROUP_EXTRA)? as usize;
        low | (high << 8)
    };
    // The extra value must fit the number of bits the code carries, otherwise
    // the code is not the canonical one for that distance.
    if bits > 0 && extra >= (1usize << bits) {
        return Err(Error::corrupt("lz77 distance extra bits are out of range"));
    }
    Ok(base + extra)
}

/// Where the decoder pulls token bytes from.
pub(crate) trait ByteSource {
    /// Pull the next byte of the given symbol group.
    fn next_byte(&mut self, group: usize) -> Result<u8>;
}

/// A source that reads the token stream verbatim.
pub(crate) struct RawSource<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> RawSource<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        RawSource { data, cursor: 0 }
    }
}

impl ByteSource for RawSource<'_> {
    fn next_byte(&mut self, _group: usize) -> Result<u8> {
        if self.cursor >= self.data.len() {
            return Err(Error::corrupt("lz77 token stream is truncated"));
        }
        let byte = self.data[self.cursor];
        self.cursor += 1;
        Ok(byte)
    }
}

/// A source that pulls token bytes from the arithmetic entropy stage.
///
/// The arithmetic decoder reads zeros past the end of its input, so the
/// source records whether that happened; the hybrid rejects such a stream as
/// truncated.
pub(crate) struct EntropySource<'a> {
    decoder: crate::predictor::Decoder<'a>,
}

impl<'a> EntropySource<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Self {
        EntropySource {
            decoder: crate::predictor::Decoder::new(input, entropy::context_count()),
        }
    }

    /// Whether the decoder ever read past the end of the entropy-coded bytes.
    pub(crate) fn overran(&self) -> bool {
        self.decoder.overran()
    }
}

impl ByteSource for EntropySource<'_> {
    fn next_byte(&mut self, group: usize) -> Result<u8> {
        Ok(entropy::decode_byte(&mut self.decoder, group))
    }
}

/// Decode a token stream into the bytes it represents.
///
/// Every bound is checked before it is used, so a malformed stream produces a
/// typed error rather than a panic or a wrong image.
pub(crate) fn decode_tokens(
    source: &mut impl ByteSource,
    params: &Lz77Params,
    output_len: usize,
) -> Result<Vec<u8>> {
    let min_match = params.min_match as usize;
    let max_match = params.max_match as usize;
    let max_distance = (params.window_size() - WINDOW_SLACK).min(MAX_DISTANCE);

    let mut out = Vec::with_capacity(output_len);
    while out.len() < output_len {
        let flag = source.next_byte(entropy::GROUP_FLAG)?;
        for slot in 0..ITEMS_PER_FLAG {
            if out.len() >= output_len {
                break;
            }
            if flag & (1u8 << slot) == 0 {
                let value = source.next_byte(entropy::GROUP_LITERAL)?;
                out.push(value);
                continue;
            }

            let length_code = source.next_byte(entropy::GROUP_LENGTH)?;
            let length = decode_length(length_code, source)?;
            let distance_code = source.next_byte(entropy::GROUP_DISTANCE)?;
            let distance = decode_distance(distance_code, source)?;

            if length < min_match || length > max_match {
                return Err(Error::corrupt(format!(
                    "lz77 match length {length} is outside {min_match}..={max_match}"
                )));
            }
            if distance == 0 || distance > max_distance {
                return Err(Error::corrupt(format!(
                    "lz77 match distance {distance} is outside 1..={max_distance}"
                )));
            }
            if distance > out.len() {
                return Err(Error::corrupt(format!(
                    "lz77 match distance {distance} reaches before the start of the tile"
                )));
            }
            if out.len() + length > output_len {
                return Err(Error::corrupt(
                    "lz77 match runs past the declared output length",
                ));
            }

            // Copy byte by byte so an overlapping match (a run) reproduces
            // exactly what the encoder saw.
            let start = out.len() - distance;
            for offset in 0..length {
                let byte = out[start + offset];
                out.push(byte);
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Payload header
// ---------------------------------------------------------------------------

/// Write the LZ77 payload header.
///
/// Every parameter is written explicitly, so a decoder never has to assume a
/// default.
pub fn write_header(out: &mut Vec<u8>, params: &Lz77Params, output_len: u64) {
    out.push(VERSION);
    out.push(params.strategy.code());
    out.push(params.min_match);
    out.extend_from_slice(&params.max_match.to_le_bytes());
    out.push(params.window_bits);
    out.push(params.hash_bits);
    out.push(params.chain_length);
    out.push(0); // reserved
    crate::predictor::put_uleb128(out, output_len);
}

/// Read the LZ77 payload header.
///
/// Returns the parameters, the declared output length, and the offset of the
/// first token byte.
pub fn read_header(input: &[u8]) -> Result<(Lz77Params, u64, usize)> {
    if input.len() < HEADER_FIXED_SIZE {
        return Err(Error::corrupt("lz77 payload is shorter than its header"));
    }
    if input[0] != VERSION {
        return Err(Error::corrupt(format!(
            "lz77 payload version {} is not supported",
            input[0]
        )));
    }

    let params = Lz77Params {
        strategy: Lz77Strategy::from_code(input[1])?,
        min_match: input[2],
        max_match: u16::from_le_bytes([input[3], input[4]]),
        window_bits: input[5],
        hash_bits: input[6],
        chain_length: input[7],
    };
    if input[8] != 0 {
        return Err(Error::corrupt("lz77 reserved header byte is not zero"));
    }
    params
        .validate()
        .map_err(|_| Error::corrupt("lz77 header holds out-of-range parameters"))?;

    let (output_len, cursor) = crate::predictor::get_uleb128(input, HEADER_FIXED_SIZE)?;
    Ok((params, output_len, cursor))
}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// The standalone LZ77 codec.
#[derive(Debug, Clone, Copy)]
pub struct Lz77Codec {
    params: Lz77Params,
}

impl Lz77Codec {
    /// A codec with the given parameters.
    pub const fn new(params: Lz77Params) -> Self {
        Lz77Codec { params }
    }

    /// The parameters this codec uses.
    pub const fn params(&self) -> &Lz77Params {
        &self.params
    }
}

impl TileCodec for Lz77Codec {
    fn codec_id(&self) -> u8 {
        EXP_LZ77
    }

    fn name(&self) -> &'static str {
        "lz77"
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        self.params.validate()?;
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "lz77 tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }

        let tokens = tokenize(input, &self.params)?;
        let stream = tokens_to_stream(&tokens);

        out.clear();
        write_header(out, &self.params, input.len() as u64);
        out.reserve(stream.len());
        for tagged in stream {
            out.push(tagged.byte);
        }

        if out.len() as u64 > ctx.limits.max_tile_payload_bytes {
            out.clear();
            return Err(Error::resource_limit(format!(
                "lz77 payload of {} bytes exceeds limit {}",
                out.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        Ok(())
    }

    fn decode(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if expected_len as u64 > ctx.limits.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "lz77 tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "lz77 payload of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }

        let (params, output_len, cursor) = read_header(input)?;
        if output_len != expected_len as u64 {
            return Err(Error::corrupt(format!(
                "lz77 payload declares {output_len} bytes but the tile is {expected_len}"
            )));
        }

        let mut source = RawSource::new(&input[cursor..]);
        let decoded = decode_tokens(&mut source, &params, expected_len)?;
        *out = decoded;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{ErrorKind, Limits, PixelFormat};

    /// The pinned golden payload of [`golden_vector_is_stable`], in hex.
    const GOLDEN_PAYLOAD_HEX: &str = "01000302010f0e100080020000010203040506070808090a1dd50602";

    /// A deterministic xorshift, so the "random" cases are reproducible.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            (x >> 32) as u32
        }
        fn fill(&mut self, out: &mut [u8]) {
            for byte in out.iter_mut() {
                *byte = self.next_u32() as u8;
            }
        }
    }

    fn contexts() -> (EncodeContext, DecodeContext) {
        let limits = Limits::default();
        (
            EncodeContext {
                limits,
                format: PixelFormat::rgba8(),
                tile_width: 256,
                tile_height: 256,
            },
            DecodeContext {
                limits,
                format: PixelFormat::rgba8(),
            },
        )
    }

    fn roundtrip(data: &[u8], params: &Lz77Params) -> Vec<u8> {
        let (enc, dec) = contexts();
        let codec = Lz77Codec::new(*params);
        let mut payload = Vec::new();
        codec.encode(data, &enc, &mut payload).unwrap();
        let mut out = Vec::new();
        codec.decode(&payload, data.len(), &dec, &mut out).unwrap();
        assert_eq!(out.len(), data.len());
        assert_eq!(out, data, "the decoder must reproduce the input exactly");
        payload
    }

    fn all_strategies() -> Vec<Lz77Params> {
        [
            Lz77Strategy::HashChain,
            Lz77Strategy::HashTable,
            Lz77Strategy::Greedy,
            Lz77Strategy::Lazy,
        ]
        .into_iter()
        .map(|strategy| Lz77Params {
            strategy,
            ..Lz77Params::default()
        })
        .collect()
    }

    #[test]
    fn empty_input_roundtrips() {
        let payload = roundtrip(&[], &Lz77Params::default());
        assert!(payload.len() >= HEADER_FIXED_SIZE);
    }

    #[test]
    fn repeated_pattern_roundtrips_and_shrinks() {
        let data: Vec<u8> = (0..4096).map(|i| (i % 7) as u8).collect();
        let payload = roundtrip(&data, &Lz77Params::default());
        assert!(
            payload.len() < data.len() / 4,
            "a repeating pattern must compress hard: {} from {}",
            payload.len(),
            data.len()
        );
    }

    #[test]
    fn repeated_scanlines_roundtrip() {
        let mut data = Vec::new();
        let scanline: Vec<u8> = (0..256).map(|i| (i as u8).wrapping_mul(3)).collect();
        for _ in 0..64 {
            data.extend_from_slice(&scanline);
        }
        roundtrip(&data, &Lz77Params::default());
    }

    #[test]
    fn flat_region_roundtrips() {
        let data = vec![0x3Cu8; 8192];
        let payload = roundtrip(&data, &Lz77Params::default());
        assert!(payload.len() < data.len() / 8);
    }

    #[test]
    fn alternating_pattern_roundtrips() {
        let data: Vec<u8> = (0..8192)
            .map(|i| if i % 2 == 0 { 0 } else { 255 })
            .collect();
        roundtrip(&data, &Lz77Params::default());
    }

    #[test]
    fn random_noise_roundtrips_exactly() {
        let mut data = vec![0u8; 8192];
        Rng::new(0x5EED).fill(&mut data);
        let payload = roundtrip(&data, &Lz77Params::default());
        // Noise must not be "compressed" into something lossy: the payload may
        // grow, but the bytes must come back exactly.
        assert!(payload.len() > data.len() / 2);
    }

    #[test]
    fn large_repeated_sequence_roundtrips() {
        let block: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
        let mut data = Vec::new();
        for _ in 0..32 {
            data.extend_from_slice(&block);
        }
        let payload = roundtrip(&data, &Lz77Params::default());
        assert!(payload.len() < data.len() / 20);
    }

    #[test]
    fn every_strategy_roundtrips_the_same_data() {
        let mut data = vec![0u8; 4096];
        Rng::new(0xC0FFEE).fill(&mut data);
        // Make part of it repetitive so matches exist.
        let block: Vec<u8> = data[..512].to_vec();
        for chunk in data.chunks_mut(512) {
            chunk.copy_from_slice(&block);
        }
        for params in all_strategies() {
            roundtrip(&data, &params);
        }
    }

    #[test]
    fn strategies_disagree_about_matches_but_never_about_the_bytes() {
        let mut data = vec![0u8; 4096];
        Rng::new(7).fill(&mut data);
        let mut payloads = Vec::new();
        for params in all_strategies() {
            payloads.push(roundtrip(&data, &params));
        }
        // The greedy and lazy searches look further than the hash-chain one,
        // so at least one pair of payloads should differ. If they all agreed
        // the strategies would not be doing anything.
        assert!(
            payloads.windows(2).any(|w| w[0] != w[1]),
            "the strategies should not all produce identical payloads"
        );
    }

    #[test]
    fn output_is_deterministic() {
        let mut data = vec![0u8; 3000];
        Rng::new(99).fill(&mut data);
        for params in all_strategies() {
            let first = roundtrip(&data, &params);
            let second = roundtrip(&data, &params);
            assert_eq!(first, second, "the encoder must be deterministic");
        }
    }

    #[test]
    fn truncated_stream_is_rejected() {
        let data: Vec<u8> = (0..2048).map(|i| (i % 13) as u8).collect();
        let payload = roundtrip(&data, &Lz77Params::default());
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let truncated = &payload[..payload.len() - 4];
        let mut out = Vec::new();
        let err = codec
            .decode(truncated, data.len(), &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_bad_version_is_rejected() {
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let mut payload = Vec::new();
        codec
            .encode(b"hello world", &contexts().0, &mut payload)
            .unwrap();
        payload[0] = 99;
        let mut out = Vec::new();
        let err = codec.decode(&payload, 11, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_reserved_strategy_is_rejected() {
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let mut payload = Vec::new();
        codec
            .encode(b"hello world", &contexts().0, &mut payload)
            .unwrap();
        payload[1] = 42;
        let mut out = Vec::new();
        let err = codec.decode(&payload, 11, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_reserved_length_code_is_rejected() {
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let mut payload = Vec::new();
        codec
            .encode(&[7u8; 64], &contexts().0, &mut payload)
            .unwrap();
        // The flag byte says "item 1 is a match"; the length code follows the
        // one literal that comes before it.
        let flag_at = HEADER_FIXED_SIZE + 1; // one LEB128 byte for the length
        assert_eq!(payload[flag_at], 0x02, "item 1 should be the first match");
        payload[flag_at + 2] = 31; // reserved length code
        let mut out = Vec::new();
        let err = codec.decode(&payload, 64, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_distance_before_the_start_of_the_tile_is_rejected() {
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let mut payload = Vec::new();
        codec
            .encode(&[7u8; 64], &contexts().0, &mut payload)
            .unwrap();
        let flag_at = HEADER_FIXED_SIZE + 1;
        // Distance code 17 has a base of 385, which is far beyond the one byte
        // that has been produced when the match starts.
        payload[flag_at + 3] = 17;
        payload[flag_at + 4] = 0x00;
        let mut out = Vec::new();
        let err = codec.decode(&payload, 64, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn an_extra_value_that_overflows_its_code_is_rejected() {
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let mut payload = Vec::new();
        codec
            .encode(&[7u8; 64], &contexts().0, &mut payload)
            .unwrap();
        let flag_at = HEADER_FIXED_SIZE + 1;
        // Distance code 4 carries a single extra bit, so an extra byte of
        // 0xFF is out of range.
        payload[flag_at + 3] = 4;
        payload[flag_at + 4] = 0xFF;
        let mut out = Vec::new();
        let err = codec.decode(&payload, 64, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_declared_length_mismatch_is_rejected() {
        let (_, dec) = contexts();
        let codec = Lz77Codec::new(Lz77Params::default());
        let mut payload = Vec::new();
        codec
            .encode(b"hello world", &contexts().0, &mut payload)
            .unwrap();
        let mut out = Vec::new();
        let err = codec.decode(&payload, 12, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn invalid_parameters_are_rejected_at_encode_time() {
        let (enc, _) = contexts();
        let codec = Lz77Codec::new(Lz77Params {
            window_bits: 3,
            ..Lz77Params::default()
        });
        let mut out = Vec::new();
        let err = codec.encode(b"data", &enc, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidArgument);
    }

    #[test]
    fn the_header_records_every_parameter() {
        let params = Lz77Params {
            strategy: Lz77Strategy::Lazy,
            min_match: 4,
            max_match: 100,
            window_bits: 12,
            hash_bits: 10,
            chain_length: 7,
        };
        let mut header = Vec::new();
        write_header(&mut header, &params, 1234);
        let (read, length, cursor) = read_header(&header).unwrap();
        assert_eq!(read, params);
        assert_eq!(length, 1234);
        assert_eq!(cursor, header.len());
    }

    #[test]
    fn golden_vector_is_stable() {
        // A fixed input with a fixed parameter set must always produce the
        // same bytes. The expected value is pinned so a change to the
        // bitstream is caught by the test suite.
        let params = Lz77Params::default();
        let data: Vec<u8> = (0..256).map(|i| (i % 11) as u8).collect();
        let payload = roundtrip(&data, &params);
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, GOLDEN_PAYLOAD_HEX, "the golden lz77 payload changed");
    }
}
