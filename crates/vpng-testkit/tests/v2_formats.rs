//! The v0.2 file format: the preview pyramid and the adaptive partition.
//!
//! These tests pin the rules that make a v0.2 file safe:
//!
//! * the original image is still **exact**, byte for byte, whatever the file
//!   holds around it;
//! * a derived level is marked as derived and is never used to rebuild the
//!   original;
//! * a derived level is strictly smaller than the level below it;
//! * the adaptive partition still covers every pixel exactly once;
//! * the v0.1 bytes do not move: the default path writes what it always
//!   wrote.

use std::fs;

use vpng_container::{DirectoryEntry, Header, FEATURE_PYRAMID, MINOR_V0_1, MINOR_V0_2};
use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::parallel::{encode_parallel, ParallelOptions};
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::{encode, EncodeOptions};

/// A temporary directory that removes itself.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let base = std::env::temp_dir().join(format!(
            "vpng-v2-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        TempDir(base)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A picture with a flat half and a busy half, so the adaptive planner has
/// something to decide.
fn mixed(width: u32, height: u32) -> CanonicalImage {
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

fn gradient(width: u32, height: u32) -> CanonicalImage {
    let bpp = 4usize;
    let mut data = vec![0u8; (width * height) as usize * bpp];
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            for c in 0..bpp {
                data[base + c] = ((x * 7 + y * 13 + c as u32 * 29) & 0xff) as u8;
            }
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

#[test]
fn the_default_path_is_still_a_v0_1_file() {
    let image = gradient(300, 180);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_1);
    assert_eq!(header.feature_flags, 0);
    assert_eq!(report.level_count, 1);

    // Every entry's flags field is zero, as v0.1 requires.
    let document = Document::parse(&report.bytes, &Limits::default()).unwrap();
    for entry in document.directory().entries.iter() {
        assert_eq!(entry.flags, 0);
    }
}

#[test]
fn a_pyramid_file_holds_the_original_and_its_previews() {
    let limits = Limits::default();
    let image = gradient(512, 256);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(3);
    let report = encode(&image, &options).unwrap();

    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_2);
    assert_eq!(header.feature_flags, FEATURE_PYRAMID);
    assert!(header.has_pyramid());
    assert_eq!(report.level_count, 4);

    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.level_count(), 4);

    // The original is level 0 and it is exact.
    let decoded = document.decode_all(&limits).unwrap();
    assert_eq!(decoded, image, "the original must survive the pyramid");

    // The derived levels are smaller, and smaller again.
    let mut previous = (512u32, 256u32);
    for position in 1..document.level_count() {
        let level = document.level(position).unwrap();
        assert!(level.is_derived());
        let dims = level.dimensions();
        assert!(
            dims.width < previous.0 && dims.height < previous.1,
            "level {position} is {dims:?}, which is not smaller than {previous:?}"
        );
        previous = (dims.width, dims.height);
    }

    // The entries carry their level in the flags field.
    for (index, entry) in document.directory().entries.iter().enumerate() {
        let level = document
            .levels()
            .iter()
            .find(|level| {
                (level.first_entry()..level.first_entry() + level.tile_count()).contains(&index)
            })
            .unwrap();
        assert_eq!(entry.level(), level.index());
        assert_eq!(entry.flags & 0xff00, 0, "the reserved bits must be zero");
    }
}

#[test]
fn a_derived_level_never_replaces_the_original() {
    let limits = Limits::default();
    let image = gradient(256, 256);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(2);
    let report = encode(&image, &options).unwrap();
    let document = Document::parse(&report.bytes, &limits).unwrap();

    // The preview is a different picture: it is a downsample, not the
    // original. Decoding it must not change what `decode_all` returns.
    let preview = document.decode_level(1, &limits).unwrap();
    assert_eq!(preview.width(), 128);
    assert_eq!(preview.height(), 128);
    assert_ne!(preview.data(), image.data());

    let original = document.decode_all(&limits).unwrap();
    assert_eq!(original, image);
}

#[test]
fn an_adaptive_file_covers_every_pixel_exactly_once() {
    let limits = Limits::default();
    let image = mixed(512, 512);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_adaptive();
    let report = encode(&image, &options).unwrap();

    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_2);
    assert_eq!(header.feature_flags, 0, "an adaptive file has no pyramid");

    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.level_count(), 1);

    // The original is still exact.
    let decoded = document.decode_all(&limits).unwrap();
    assert_eq!(decoded, image);

    // The partition holds tiles of different sizes.
    let partition = document.partition();
    assert!(
        !partition.is_uniform_size(),
        "the partition should be adaptive"
    );
    let widths: Vec<u32> = partition.tiles().iter().map(|tile| tile.width).collect();
    assert!(
        widths.windows(2).any(|pair| pair[0] != pair[1]),
        "the tiles should differ in size: {widths:?}"
    );
}

#[test]
fn an_adaptive_region_decode_equals_the_crop() {
    let limits = Limits::default();
    let image = mixed(400, 300);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_adaptive();
    let report = encode(&image, &options).unwrap();
    let document = Document::parse(&report.bytes, &limits).unwrap();

    let full = document.decode_all(&limits).unwrap();
    let rect = Rect::new(37, 11, 200, 150).unwrap();
    let region = document.decode_region(rect, &limits).unwrap();

    let bpp = 4usize;
    let src_row = image.row_bytes().unwrap() as usize;
    for y in 0..rect.height as usize {
        let src = (rect.y as usize + y) * src_row + rect.x as usize * bpp;
        let dst = y * region.row_bytes().unwrap() as usize;
        assert_eq!(
            &region.data()[dst..dst + rect.width as usize * bpp],
            &full.data()[src..src + rect.width as usize * bpp]
        );
    }
}

#[test]
fn the_worker_count_does_not_change_a_v0_2_file() {
    let directory = TempDir::new("workers");
    let image = mixed(300, 200);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(2)
        .with_adaptive();

    let batch = encode(&image, &options).unwrap();

    for workers in [1usize, 4] {
        let parallel = encode_parallel(
            &image,
            &options,
            &ParallelOptions::default().with_workers(workers),
        )
        .unwrap();
        assert_eq!(
            parallel.bytes, batch.bytes,
            "{workers} workers produced a different file"
        );

        let path = directory.path(&format!("stream-{workers}.vpng"));
        encode_to_file(
            &image,
            &options,
            &ParallelOptions::default().with_workers(workers),
            &path,
        )
        .unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            batch.bytes,
            "{workers} workers streamed a different file"
        );
    }
}

#[test]
fn a_pyramid_file_survives_a_round_trip_through_the_decoder() {
    let limits = Limits::default();
    let image = mixed(256, 128);
    let options = EncodeOptions::default()
        .with_tile_size(32, 32)
        .with_pyramid(3);
    let report = encode(&image, &options).unwrap();
    let document = Document::parse(&report.bytes, &limits).unwrap();

    assert!(document.check_integrity().is_ok());
    assert_eq!(document.decode_all(&limits).unwrap(), image);

    // Every level decodes to a picture of the level's size.
    for position in 0..document.level_count() {
        let level = document.decode_level(position, &limits).unwrap();
        let dims = document.level(position).unwrap().dimensions();
        assert_eq!(level.width(), dims.width);
        assert_eq!(level.height(), dims.height);
    }
}

#[test]
fn a_v0_1_file_that_claims_a_pyramid_is_refused() {
    // The rule that keeps the two versions apart: a file that says it is
    // v0.1 cannot carry a v0.2 feature flag.
    let image = gradient(128, 128);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(1);
    let report = encode(&image, &options).unwrap();

    let mut bytes = report.bytes.clone();
    bytes[10..12].copy_from_slice(&MINOR_V0_1.to_le_bytes());
    // Fix the header checksum so only the version rule can fire.
    let sum = vpng_container::checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..64]]);
    bytes[52..56].copy_from_slice(&sum.to_le_bytes());

    let error = Document::parse(&bytes, &Limits::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFeature);
}

#[test]
fn a_reserved_flag_bit_is_rejected() {
    let limits = Limits::default();
    let image = gradient(128, 128);
    let options = EncodeOptions::default().with_adaptive();
    let report = encode(&image, &options).unwrap();

    // Set a reserved bit in the first entry's flags and repair the
    // directory checksum. Bit 8 is the lowest reserved bit: the low byte is
    // the level index, which is a defined field.
    let mut bytes = report.bytes.clone();
    let directory_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    bytes[directory_offset + 15] |= 0x01;
    let tile_count = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
    let body = &bytes[directory_offset..directory_offset + tile_count * 40];
    let crc = vpng_container::crc32c(body);
    let trailer = directory_offset + tile_count * 40;
    bytes[trailer..trailer + 4].copy_from_slice(&crc.to_le_bytes());

    let error = Document::parse(&bytes, &limits).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::CorruptFile);
}

#[test]
fn the_entry_flags_carry_the_level() {
    let image = gradient(128, 128);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(2);
    let report = encode(&image, &options).unwrap();
    let document = Document::parse(&report.bytes, &Limits::default()).unwrap();

    let base = document.base_level();
    for position in 0..base.tile_count() {
        let index = base.entry_index(position).unwrap();
        let entry: &DirectoryEntry = document.tile_entry(index).unwrap();
        assert_eq!(entry.level(), 0);
        assert!(!entry.is_derived());
    }
    let preview = document.level(1).unwrap();
    for position in 0..preview.tile_count() {
        let index = preview.entry_index(position).unwrap();
        let entry = document.tile_entry(index).unwrap();
        assert_eq!(entry.level(), 1);
        assert!(entry.is_derived());
    }
}
