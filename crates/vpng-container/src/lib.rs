//! `vpng-container` — how a VPNG file is laid out.
//!
//! This crate owns the byte-level truth: the magic bytes, the 64-byte header,
//! the 40-byte tile directory entries, checksums, and the canonical digest.
//! It performs no codec work and holds no pixel buffers.

#![forbid(unsafe_code)]

mod leb;

pub mod checksum;
pub mod directory;
pub mod header;

pub use checksum::{canonical_digest, crc32c, sha256, to_hex};
pub use directory::{
    Directory, DirectoryEntry, DIRECTORY_TRAILER_SIZE, ENTRY_SIZE as DIRECTORY_ENTRY_SIZE,
    TILE_FLAG_LEVEL_MASK, TILE_FLAG_RESERVED_MASK,
};
pub use header::{
    Header, FEATURE_PYRAMID, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, MAGIC, MINOR_V0_1,
    MINOR_V0_2, MINOR_V0_3,
};
