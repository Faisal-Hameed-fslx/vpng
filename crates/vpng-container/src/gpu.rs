//! The GPU payload table.
//!
//! A v0.5 container may carry a table that gives every tile an optional
//! **lossy** GPU sibling of its lossless payload: the block payload a GPU
//! samples as a texture without decompressing anything.
//!
//! The table holds one fixed-size record per tile, in the same canonical
//! order as the directory, so a reader finds a tile's GPU payload by
//! indexing it — without scanning, and without a second lookup structure.
//!
//! The table's layout:
//!
//! ```text
//! offset  size  field
//!      0     4  record count
//!      4     4  descriptor array length
//!      8     4  CRC32C over the descriptor array
//!     12     4  reserved (must be zero)
//!     16     8  total GPU payload bytes
//!     24   ...  descriptor array
//! ```
//!
//! and one record:
//!
//! ```text
//! offset  size  field
//!      0     8  GPU payload offset
//!      8     4  GPU payload length
//!     12     1  colour codec id (0: none, 7: BC1)
//!     13     1  alpha codec id (0: none, 8: BC4)
//!     14     2  reserved (must be zero)
//!     16     4  CRC32C of the GPU payload
//! ```

use vpng_core::{Error, Result};

use crate::checksum;
use crate::leb;

/// The size of the GPU table's own header.
pub const GPU_TABLE_HEADER_SIZE: usize = 24;

/// The size of one GPU record.
pub const GPU_RECORD_SIZE: usize = 20;

/// One tile's GPU payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuRecord {
    /// Where the tile's GPU payload starts, in bytes from the file's start.
    pub payload_offset: u64,
    /// The GPU payload's length in bytes.
    pub payload_length: u32,
    /// The codec of the payload's colour blocks: 0 for none, 7 for BC1.
    pub colour_codec: u8,
    /// The codec of the payload's alpha blocks: 0 for none, 8 for BC4.
    pub alpha_codec: u8,
    /// The CRC32C of the GPU payload's bytes.
    pub payload_checksum: u32,
}

impl GpuRecord {
    /// Whether the record points at a payload at all.
    pub fn is_present(&self) -> bool {
        self.colour_codec != 0 || self.alpha_codec != 0
    }

    /// Serialize the record.
    pub fn encode(&self) -> Result<[u8; GPU_RECORD_SIZE]> {
        let mut out = [0u8; GPU_RECORD_SIZE];
        out[0..8].copy_from_slice(&self.payload_offset.to_le_bytes());
        out[8..12].copy_from_slice(&self.payload_length.to_le_bytes());
        out[12] = self.colour_codec;
        out[13] = self.alpha_codec;
        out[16..20].copy_from_slice(&self.payload_checksum.to_le_bytes());
        Ok(out)
    }

    /// Parse a record.
    pub fn decode(bytes: &[u8]) -> Result<GpuRecord> {
        if bytes.len() < GPU_RECORD_SIZE {
            return Err(Error::corrupt("GPU record is truncated"));
        }
        if leb::read_u16(bytes, 14) != 0 {
            return Err(Error::corrupt("GPU record reserved bytes must be zero"));
        }
        Ok(GpuRecord {
            payload_offset: leb::read_u64(bytes, 0),
            payload_length: leb::read_u32(bytes, 8),
            colour_codec: bytes[12],
            alpha_codec: bytes[13],
            payload_checksum: leb::read_u32(bytes, 16),
        })
    }
}

/// The whole GPU payload table: one record per tile.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GpuTable {
    /// The records, in the directory's canonical order.
    pub records: Vec<GpuRecord>,
}

impl GpuTable {
    /// Build a table from its records.
    pub fn new(records: Vec<GpuRecord>) -> Self {
        GpuTable { records }
    }

    /// The number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the table holds no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// One record by position.
    pub fn get(&self, index: usize) -> Option<&GpuRecord> {
        self.records.get(index)
    }

    /// The number of bytes every GPU payload in the table occupies.
    pub fn total_payload_bytes(&self) -> u64 {
        self.records
            .iter()
            .map(|record| u64::from(record.payload_length))
            .sum()
    }

    /// Serialize the table.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut descriptors = Vec::with_capacity(self.records.len() * GPU_RECORD_SIZE);
        for record in &self.records {
            descriptors.extend_from_slice(&record.encode()?);
        }

        let mut out = Vec::with_capacity(GPU_TABLE_HEADER_SIZE + descriptors.len());
        out.extend_from_slice(&(self.records.len() as u32).to_le_bytes());
        out.extend_from_slice(&(descriptors.len() as u32).to_le_bytes());
        out.extend_from_slice(&checksum::crc32c(&descriptors).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved
        out.extend_from_slice(&self.total_payload_bytes().to_le_bytes());
        out.extend_from_slice(&descriptors);
        Ok(out)
    }

    /// Parse and checksum-verify a GPU payload table.
    pub fn decode(bytes: &[u8]) -> Result<GpuTable> {
        if bytes.len() < GPU_TABLE_HEADER_SIZE {
            return Err(Error::corrupt("GPU table is truncated"));
        }
        let count = leb::read_u32(bytes, 0);
        let descriptor_bytes = leb::read_u32(bytes, 4);
        let stored = leb::read_u32(bytes, 8);
        if leb::read_u32(bytes, 12) != 0 {
            return Err(Error::corrupt("GPU table reserved bytes must be zero"));
        }
        let total = leb::read_u64(bytes, 16);

        let body = &bytes[GPU_TABLE_HEADER_SIZE..];
        if body.len() != descriptor_bytes as usize {
            return Err(Error::corrupt(format!(
                "GPU table holds {} descriptor bytes, the header says {descriptor_bytes}",
                body.len()
            )));
        }
        let computed = checksum::crc32c(body);
        if stored != computed {
            return Err(Error::corrupt(format!(
                "GPU table checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            )));
        }
        if count == 0 {
            return Err(Error::corrupt("a GPU table must hold at least one record"));
        }
        if descriptor_bytes as usize != count as usize * GPU_RECORD_SIZE {
            return Err(Error::corrupt(format!(
                "a GPU table of {count} records holds {descriptor_bytes} descriptor bytes, not {}",
                count as usize * GPU_RECORD_SIZE
            )));
        }

        let mut records = Vec::with_capacity(count as usize);
        let mut cursor = 0usize;
        for _ in 0..count {
            let record = GpuRecord::decode(&body[cursor..])?;
            cursor += GPU_RECORD_SIZE;
            records.push(record);
        }
        if cursor != body.len() {
            return Err(Error::corrupt(
                "the GPU table has bytes left over after its last record",
            ));
        }

        let table = GpuTable { records };
        if table.total_payload_bytes() != total {
            return Err(Error::corrupt(format!(
                "the GPU table claims {total} payload bytes, its records hold {}",
                table.total_payload_bytes()
            )));
        }

        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> GpuRecord {
        GpuRecord {
            payload_offset: 1024,
            payload_length: 64,
            colour_codec: 7,
            alpha_codec: 8,
            payload_checksum: 0xDEAD_BEEF,
        }
    }

    #[test]
    fn a_table_round_trips() {
        let table = GpuTable::new(vec![record(), record()]);
        let bytes = table.encode().unwrap();
        assert_eq!(bytes.len(), GPU_TABLE_HEADER_SIZE + 2 * GPU_RECORD_SIZE);
        assert_eq!(GpuTable::decode(&bytes).unwrap(), table);
    }

    #[test]
    fn a_corrupt_table_is_refused() {
        let table = GpuTable::new(vec![record()]);
        let mut bytes = table.encode().unwrap();
        bytes[GPU_TABLE_HEADER_SIZE] ^= 0xff;
        assert!(GpuTable::decode(&bytes).is_err());
    }

    #[test]
    fn a_truncated_table_is_refused() {
        let table = GpuTable::new(vec![record()]);
        let bytes = table.encode().unwrap();
        assert!(GpuTable::decode(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn a_table_with_the_wrong_record_count_is_refused() {
        let table = GpuTable::new(vec![record()]);
        let mut bytes = table.encode().unwrap();
        bytes[0..4].copy_from_slice(&2u32.to_le_bytes());
        assert!(GpuTable::decode(&bytes).is_err());
    }

    #[test]
    fn a_table_whose_total_is_wrong_is_refused() {
        let table = GpuTable::new(vec![record()]);
        let mut bytes = table.encode().unwrap();
        bytes[16..24].copy_from_slice(&999u64.to_le_bytes());
        assert!(GpuTable::decode(&bytes).is_err());
    }

    #[test]
    fn an_empty_table_is_refused() {
        let table = GpuTable::new(Vec::new());
        let bytes = table.encode().unwrap();
        assert!(GpuTable::decode(&bytes).is_err());
    }
}
