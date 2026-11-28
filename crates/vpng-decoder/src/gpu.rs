//! The lossy GPU payload: what a file's GPU payload decodes to.
//!
//! A file's **lossless** payload reconstructs the image exactly. The **GPU
//! payload** is the other thing: the same picture in the block format a GPU
//! samples as a texture. BC1 and BC4 throw information away, so what comes
//! out of here is close to the original and **not equal** to it.
//!
//! Nothing in this module is used by an exact operation. The functions here
//! are reached only through the explicitly named GPU API.

use vpng_container::GpuRecord;
use vpng_core::{Error, Result};

/// The number of bytes a record's payload must hold for a tile of this size.
///
/// A record with no codec ids at all holds nothing.
pub fn payload_length(record: &GpuRecord, width: u32, height: u32) -> u64 {
    let blocks = vpng_gpu::block_count(width, height);
    let mut length = 0u64;
    if record.colour_codec != 0 {
        length += blocks * vpng_gpu::GpuCodec::Bc1.bytes_per_block();
    }
    if record.alpha_codec != 0 {
        length += blocks * vpng_gpu::GpuCodec::Bc4.bytes_per_block();
    }
    length
}

/// Check a record against the tile it describes.
///
/// The rules are the ones the specification states: the codec ids must be
/// ones this build knows, and the payload's length must be exactly what the
/// tile's geometry and the codec ids imply.
pub fn validate_record(record: &GpuRecord, width: u32, height: u32) -> Result<()> {
    if record.colour_codec != 0 && vpng_gpu::GpuCodec::from_id(record.colour_codec).is_none() {
        return Err(Error::unsupported(format!(
            "a GPU payload names colour codec id {}, which this build does not implement",
            record.colour_codec
        )));
    }
    if record.alpha_codec != 0 && vpng_gpu::GpuCodec::from_id(record.alpha_codec).is_none() {
        return Err(Error::unsupported(format!(
            "a GPU payload names alpha codec id {}, which this build does not implement",
            record.alpha_codec
        )));
    }

    let expected = payload_length(record, width, height);
    if u64::from(record.payload_length) != expected {
        return Err(Error::corrupt(format!(
            "a GPU payload for a {width}x{height} tile is {expected} bytes, the table says {}",
            record.payload_length
        )));
    }

    if !record.is_present() && (record.payload_offset != 0 || record.payload_checksum != 0) {
        return Err(Error::corrupt(
            "a GPU record with no payload must have a zero offset and checksum",
        ));
    }

    Ok(())
}

/// Decode a tile's GPU payload into **RGBA8** pixels.
///
/// The result is always four channels, because a GPU payload is always
/// eight-bit and a texture is always RGBA. A payload with no alpha blocks is
/// fully opaque.
///
/// The result is **not** the file's own format, and it is **not** the
/// original pixels: BC1 and BC4 are lossy.
pub fn decode_gpu_tile(
    record: &GpuRecord,
    width: u32,
    height: u32,
    payload: &[u8],
) -> Result<Vec<u8>> {
    validate_record(record, width, height)?;

    if !record.is_present() {
        return Ok(vec![0u8; width as usize * height as usize * 4]);
    }

    let blocks = vpng_gpu::block_count(width, height);
    let colour_len = if record.colour_codec != 0 {
        blocks * vpng_gpu::GpuCodec::Bc1.bytes_per_block()
    } else {
        0
    };
    let alpha_len = if record.alpha_codec != 0 {
        blocks * vpng_gpu::GpuCodec::Bc4.bytes_per_block()
    } else {
        0
    };

    if payload.len() as u64 != colour_len + alpha_len {
        return Err(Error::corrupt(format!(
            "a GPU payload is {} bytes, the table says {}",
            payload.len(),
            record.payload_length
        )));
    }

    let colour = if colour_len == 0 {
        None
    } else {
        Some(&payload[..colour_len as usize])
    };
    let alpha = if alpha_len == 0 {
        None
    } else {
        Some(&payload[colour_len as usize..])
    };

    vpng_gpu::decode_tile(width, height, colour, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(colour: u8, alpha: u8, length: u32) -> GpuRecord {
        GpuRecord {
            payload_offset: if colour == 0 && alpha == 0 { 0 } else { 1024 },
            payload_length: length,
            colour_codec: colour,
            alpha_codec: alpha,
            payload_checksum: if colour == 0 && alpha == 0 { 0 } else { 0xABCD },
        }
    }

    #[test]
    fn a_record_with_no_payload_holds_nothing() {
        let empty = record(0, 0, 0);
        assert_eq!(payload_length(&empty, 8, 8), 0);
        validate_record(&empty, 8, 8).unwrap();
    }

    #[test]
    fn a_record_with_a_codec_but_no_bytes_is_refused() {
        let broken = record(7, 0, 0);
        assert!(validate_record(&broken, 8, 8).is_err());
    }

    #[test]
    fn the_length_follows_the_geometry() {
        // Four blocks across and two down, eight bytes each.
        let bc1 = record(7, 0, 4 * 2 * 8);
        assert_eq!(payload_length(&bc1, 16, 8), 64);
        validate_record(&bc1, 16, 8).unwrap();
    }

    #[test]
    fn an_unknown_codec_is_refused() {
        let broken = record(99, 0, 8);
        let error = validate_record(&broken, 4, 4).unwrap_err();
        assert_eq!(error.kind(), vpng_core::ErrorKind::UnsupportedFeature);
    }
}
