//! `vpng-container` — how a VPNG file is laid out.
//!
//! This crate owns the byte-level truth: the magic bytes, the 64-byte header
//! (72 from format v0.5), the 40-byte tile directory entries, checksums, and
//! the canonical digest. It performs no codec work and holds no pixel buffers.

#![forbid(unsafe_code)]

mod leb;

pub mod checksum;
pub mod directory;
pub mod gpu;
pub mod header;
pub mod layers;

pub use checksum::{canonical_digest, crc32c, sha256, to_hex};
pub use directory::{
    Directory, DirectoryEntry, DIRECTORY_TRAILER_SIZE, ENTRY_SIZE as DIRECTORY_ENTRY_SIZE,
    MAX_LAYERS, TILE_FLAG_LAYER_MASK, TILE_FLAG_LEVEL_MASK, TILE_FLAG_RESERVED_MASK,
};
pub use gpu::{GpuRecord, GpuTable, GPU_RECORD_SIZE, GPU_TABLE_HEADER_SIZE};
pub use header::{
    Header, FEATURE_GPU, FEATURE_LAYERS, FEATURE_PYRAMID, FORMAT_MAJOR, FORMAT_MINOR,
    GPU_TABLE_LENGTH_OFFSET, GPU_TABLE_OFFSET_OFFSET, HEADER_SIZE, HEADER_SIZE_V0_5, MAGIC,
    MINOR_V0_1, MINOR_V0_2, MINOR_V0_3, MINOR_V0_4, MINOR_V0_5,
};
pub use layers::{
    LayerDescriptor, LayerKind, LayerTable, LAYER_DESCRIPTOR_SIZE, LAYER_TABLE_HEADER_SIZE, NO_MASK,
};
