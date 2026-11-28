//! The v0.5 file format: the optional GPU payload table.
//!
//! These tests pin the rules that make a v0.5 file safe:
//!
//! * a file that carries a GPU table says so in its minor version, and a
//!   file that does not is still a v0.1 file with the exact bytes the golden
//!   fixtures pin;
//! * the **lossless** payload is untouched: the file still reconstructs the
//!   picture exactly, byte for byte;
//! * the GPU payload is **lossy**, and the file says so;
//! * a directory entry may **never** name a GPU block codec;
//! * the GPU table's records match the directory exactly, and every payload's
//!   length is what the tile's geometry implies;
//! * the GPU table and the GPU payloads are accounted for byte for byte.

use vpng_container::{
    crc32c, Directory, GpuTable, Header, HEADER_SIZE_V0_5, MINOR_V0_1, MINOR_V0_5,
};
use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};

/// A picture with enough structure that every codec has something to chew on.
fn picture(width: u32, height: u32) -> CanonicalImage {
    let bpp = 4usize;
    let mut data = vec![0u8; (width * height) as usize * bpp];
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            if x < width / 2 {
                data[base] = 0x20;
                data[base + 1] = 0x30;
                data[base + 2] = 0x40;
            } else {
                for c in 0..3 {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    data[base + c] = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
                }
            }
            data[base + 3] = 0xFF;
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// An RGB8 picture, for the "no alpha blocks" case.
fn rgb_picture(width: u32, height: u32) -> CanonicalImage {
    let mut data = vec![0u8; (width * height) as usize * 3];
    let mut state = 0x0BAD_C0DE_0BAD_C0DEu64;
    for byte in data.iter_mut() {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgb8(), data).unwrap()
}

/// Encode a picture with a GPU payload.
fn with_gpu(image: &CanonicalImage) -> vpng_encoder::EncodeReport {
    encode(
        image,
        &EncodeOptions::default().with_tile_size(64, 64).with_gpu(),
    )
    .unwrap()
}

#[test]
fn the_default_path_is_still_a_v0_1_file() {
    // The GPU milestone must not move a single published byte.
    let image = picture(300, 180);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_1);
    assert_eq!(header.feature_flags, 0);
    assert_eq!(header.gpu_table_length, 0);
    assert_eq!(header.gpu_table_offset, 0);
    assert_eq!(report.gpu_payload_bytes, 0);
    assert_eq!(report.gpu_table_bytes, 0);
}

#[test]
fn a_gpu_file_is_a_v0_5_file_with_a_72_byte_header() {
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_5);
    assert!(header.has_gpu());
    assert!(header.gpu_table_length > 0);
    assert_eq!(header.header_size(), HEADER_SIZE_V0_5);
    // The directory starts after the longer header.
    assert_eq!(header.directory_offset, HEADER_SIZE_V0_5 as u64);
    // The header's own size field says 72.
    assert_eq!(
        u32::from_le_bytes(report.bytes[12..16].try_into().unwrap()),
        HEADER_SIZE_V0_5 as u32
    );
}

#[test]
fn the_gpu_flag_is_orthogonal_to_the_others() {
    // A GPU payload is a sibling of the lossless payload, not a replacement
    // for it, so a file may have a GPU table and promoted codecs at once.
    let image = picture(300, 180);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_gpu()
        .with_extended_codecs();
    let report = encode(&image, &options).unwrap();

    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_5);
    assert!(header.has_gpu());
}

#[test]
fn the_lossless_payload_still_reconstructs_the_picture_exactly() {
    // This is the load-bearing rule: adding a lossy sibling must not touch
    // the lossless payload in any way.
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    let decoded = document.decode_all(&limits).unwrap();
    assert_eq!(decoded.data(), image.data());
    assert_eq!(decoded.width(), image.width());
    assert_eq!(decoded.height(), image.height());
}

#[test]
fn the_gpu_payload_is_lossy() {
    // The file says the GPU payload is a *different*, lossy picture. A test
    // that found it identical would mean the block codecs are not doing
    // anything, which is exactly the bug this guards against.
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    let region = document
        .decode_gpu_region(Rect::new(0, 0, 300, 180).unwrap(), &limits)
        .unwrap();

    // The GPU payload is always RGBA8.
    assert_eq!(region.format(), PixelFormat::rgba8());
    assert_eq!(region.data().len(), 300 * 180 * 4);

    // It is close to the original, but it is not the original.
    let mut max_difference = 0i32;
    for (index, value) in region.data().iter().enumerate() {
        let channel = index % 4;
        if channel == 3 {
            continue;
        }
        let original = image.data()[index];
        let difference = (i32::from(*value) - i32::from(original)).abs();
        max_difference = max_difference.max(difference);
    }
    assert!(
        max_difference > 0,
        "the GPU payload is identical to the original, so it is not lossy at all"
    );
}

#[test]
fn the_gpu_table_holds_one_record_per_tile() {
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let start = header.gpu_table_offset as usize;
    let end = start + header.gpu_table_length as usize;
    let table = GpuTable::decode(&report.bytes[start..end]).unwrap();

    assert_eq!(table.len(), report.tile_count as usize);
    assert_eq!(
        header.gpu_table_length as usize,
        vpng_container::GPU_TABLE_HEADER_SIZE + 20 * report.tile_count as usize
    );

    // Every tile of an RGBA8 picture has BC1 colour blocks and BC4 alpha
    // blocks, and the records are in the directory's order.
    for record in table.records.iter() {
        assert_eq!(record.colour_codec, 7, "the colour codec is BC1");
        assert_eq!(record.alpha_codec, 8, "the alpha codec is BC4");
        assert!(record.is_present());
    }
}

#[test]
fn a_record_without_alpha_names_no_alpha_codec() {
    let image = rgb_picture(300, 180);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let start = header.gpu_table_offset as usize;
    let end = start + header.gpu_table_length as usize;
    let table = GpuTable::decode(&report.bytes[start..end]).unwrap();

    for record in table.records.iter() {
        assert_eq!(record.colour_codec, 7);
        assert_eq!(record.alpha_codec, 0, "an RGB8 tile has no alpha blocks");
    }
}

#[test]
fn every_payload_length_matches_the_tile_geometry() {
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let directory = Directory::decode(
        &report.bytes[HEADER_SIZE_V0_5..HEADER_SIZE_V0_5 + report.tile_count as usize * 40 + 4],
        report.tile_count,
    )
    .unwrap();

    let start = header.gpu_table_offset as usize;
    let end = start + header.gpu_table_length as usize;
    let table = GpuTable::decode(&report.bytes[start..end]).unwrap();

    for (index, record) in table.records.iter().enumerate() {
        let entry = &directory.entries[index];
        let blocks = u64::from(u32::from(entry.width).div_ceil(4))
            * u64::from(u32::from(entry.height).div_ceil(4));
        // BC1 is eight bytes per block, and so is BC4.
        assert_eq!(
            u64::from(record.payload_length),
            blocks * 8 * 2,
            "tile {index} has the wrong GPU payload length"
        );
    }
}

#[test]
fn every_payload_checksum_matches() {
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let start = header.gpu_table_offset as usize;
    let end = start + header.gpu_table_length as usize;
    let table = GpuTable::decode(&report.bytes[start..end]).unwrap();

    for (index, record) in table.records.iter().enumerate() {
        let from = record.payload_offset as usize;
        let to = from + record.payload_length as usize;
        let payload = &report.bytes[from..to];
        assert_eq!(
            crc32c(payload),
            record.payload_checksum,
            "tile {index} has the wrong GPU payload checksum"
        );
    }
}

#[test]
fn the_bytes_add_up() {
    let image = picture(300, 180);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let directory_size = report.tile_count as u64 * 40 + 4;
    let payloads: u64 = report.compressed_lengths.iter().sum();

    assert_eq!(
        report.file_size,
        (HEADER_SIZE_V0_5 as u64)
            + directory_size
            + header.gpu_table_length as u64
            + payloads
            + report.gpu_payload_bytes
    );
    assert_eq!(report.gpu_table_bytes, header.gpu_table_length as u64);
}

#[test]
fn a_directory_entry_may_never_name_a_gpu_block_codec() {
    // The GPU payload is a sibling of the lossless payload, never a
    // replacement for it. A directory entry that names BC1 or BC4 would be
    // a file whose picture cannot be reconstructed exactly, and the reader
    // refuses it.
    let image = picture(128, 96);
    let report = with_gpu(&image);

    let directory = Directory::decode(
        &report.bytes[HEADER_SIZE_V0_5..HEADER_SIZE_V0_5 + report.tile_count as usize * 40 + 4],
        report.tile_count,
    )
    .unwrap();

    for entry in directory.entries.iter() {
        assert!(
            !vpng_codecs::is_gpu_block_codec(entry.codec_id),
            "a tile names the GPU block codec {}",
            entry.codec_id
        );
        assert!(vpng_codecs::is_implemented(entry.codec_id));
    }
}

#[test]
fn a_gpu_payload_with_a_flipped_byte_is_refused() {
    let image = picture(128, 96);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let start = header.gpu_table_offset as usize;
    let end = start + header.gpu_table_length as usize;
    let table = GpuTable::decode(&report.bytes[start..end]).unwrap();

    let record = table.records[0];
    let from = record.payload_offset as usize;
    let mut bytes = report.bytes.clone();
    bytes[from] ^= 0xFF;

    let limits = Limits::default();
    let document = Document::parse(&bytes, &limits).unwrap();
    let err = document.decode_gpu_tile(0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}

#[test]
fn a_gpu_table_with_the_wrong_record_count_is_refused() {
    let image = picture(128, 96);
    let report = with_gpu(&image);

    let header = Header::parse(&report.bytes).unwrap();
    let start = header.gpu_table_offset as usize;

    let mut bytes = report.bytes.clone();
    // The record count is the table's first field.
    bytes[start..start + 4].copy_from_slice(&99u32.to_le_bytes());
    // Repair the header's checksum, because the header still points at the
    // same table.
    let sum = vpng_container::checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..72]]);
    bytes[52..56].copy_from_slice(&sum.to_le_bytes());

    let limits = Limits::default();
    let err = Document::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}

#[test]
fn an_older_file_may_not_claim_the_gpu_feature() {
    let image = picture(128, 96);
    let report = with_gpu(&image);

    // Claim an older version and repair the header checksum: the reader must
    // then refuse the file.
    let mut bytes = report.bytes.clone();
    bytes[10..12].copy_from_slice(&MINOR_V0_1.to_le_bytes());
    let sum = vpng_container::checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..72]]);
    bytes[52..56].copy_from_slice(&sum.to_le_bytes());

    let limits = Limits::default();
    let err = Document::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}

#[test]
fn a_sixteen_bit_picture_has_no_gpu_payload() {
    // The block codecs hold eight bits per channel. A 16-bit picture has no
    // GPU payload, and the encoder refuses rather than quietly dropping the
    // precision.
    let image =
        CanonicalImage::from_vec(64, 64, PixelFormat::rgba16(), vec![0u8; 64 * 64 * 8]).unwrap();
    let err = encode(
        &image,
        &EncodeOptions::default().with_tile_size(64, 64).with_gpu(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_file_without_a_gpu_table_has_none() {
    // Every file written before v0.5 has no GPU table, so a caller never has
    // to ask "is this a GPU file?" before asking "what is its GPU payload?".
    let image = picture(300, 180);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert!(document.gpu_table().is_none());
    let err = document.decode_gpu_tile(0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
}
