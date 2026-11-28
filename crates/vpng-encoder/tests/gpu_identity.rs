//! The GPU payload's contract: **one worker and N workers produce the same
//! file, byte for byte**, and the streaming writer produces that same file
//! too.
//!
//! The GPU payload is built per tile, from that tile's original pixels, so
//! it is as independent of the thread count as the lossless payload is. This
//! test is the gate that proves it: the file's bytes must depend on the
//! pixels and the options, never on the thread count, the chunk size or the
//! order the workers happened to finish in.
//!
//! It also pins the rule that matters most: the **lossless** payload is
//! exactly the bytes the same encode without `--gpu` produces. Adding a
//! lossy sibling must not move a single lossless byte.

use std::fs;
use std::path::{Path, PathBuf};

use vpng_core::{CanonicalImage, ErrorKind, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::parallel::{encode_parallel, ParallelOptions};
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::{encode, EncodeOptions};

/// Every eight-bit format: the ones a GPU payload can describe.
const FORMATS: [PixelFormat; 4] = [
    PixelFormat::gray8(),
    PixelFormat::gray_alpha8(),
    PixelFormat::rgb8(),
    PixelFormat::rgba8(),
];

/// Exact fit, single edge tiles, double edge tiles and the degenerate cases.
const SIZES: [(u32, u32); 6] = [(1, 1), (7, 3), (64, 64), (256, 256), (257, 130), (300, 180)];

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-encoder-gpu-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        TempDir(base)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Mixed content, so different tiles want different codecs: a flat region, a
/// smooth region and an incompressible region.
fn mixed(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for y in 0..height {
        for x in 0..width {
            let base = (y as usize * width as usize + x as usize) * bpp;
            for c in 0..bpp {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                let noise = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
                let value = if x < width / 3 {
                    0x40
                } else if x < (2 * width) / 3 {
                    ((x * 5 + y * 11 + c as u32 * 17) & 0xff) as u8
                } else {
                    (noise >> 33) as u8
                };
                data[base + c] = value;
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// Encode with the batch encoder, with the parallel encoder, and with the
/// streaming writer, and assert that all three agree byte for byte.
fn assert_all_paths_agree(
    image: &CanonicalImage,
    options: &EncodeOptions,
    path: &Path,
    label: &str,
) {
    let batch = encode(image, options).unwrap();

    for workers in [1usize, 2, 4, 8] {
        let parallel = encode_parallel(
            image,
            options,
            &ParallelOptions::default().with_workers(workers),
        )
        .unwrap();
        assert_eq!(
            parallel.bytes, batch.bytes,
            "{label}: {workers} workers produced a different file from the batch encoder"
        );
        assert_eq!(
            parallel.codec_ids, batch.codec_ids,
            "{label}: {workers} workers chose different codecs"
        );
    }

    let streamed = encode_to_file(
        image,
        options,
        &ParallelOptions::default().with_workers(4),
        path,
    )
    .unwrap();
    let on_disk = fs::read(path).unwrap();
    assert_eq!(
        on_disk, batch.bytes,
        "{label}: the streamed file differs from the batch encoder's bytes"
    );
    assert_eq!(
        streamed.file_size, batch.file_size,
        "{label}: the streamed file size disagrees"
    );
    assert_eq!(
        streamed.codec_ids, batch.codec_ids,
        "{label}: the streaming writer chose different codecs"
    );
    assert_eq!(
        streamed.gpu_payload_bytes, batch.gpu_payload_bytes,
        "{label}: the streaming writer reported different GPU payload bytes"
    );
    assert_eq!(
        streamed.gpu_table_bytes, batch.gpu_table_bytes,
        "{label}: the streaming writer reported a different GPU table size"
    );
}

#[test]
fn the_gpu_payload_agrees_across_every_path() {
    let directory = TempDir::new();
    let mut cases = 0usize;

    for format in FORMATS {
        for (width, height) in SIZES {
            for tile_size in [16u32, 64, 256] {
                let image = mixed(width, height, format);
                let options = EncodeOptions::default()
                    .with_tile_size(tile_size, tile_size)
                    .with_gpu();
                let label = format!("gpu-{format}-{width}x{height}-tile{tile_size}");
                let path = directory.0.join(format!("case-{cases}.vpng"));
                assert_all_paths_agree(&image, &options, &path, &label);
                cases += 1;
            }
        }
    }

    // 4 formats x 6 sizes x 3 tile geometries.
    assert_eq!(cases, 72);
}

#[test]
fn the_gpu_payload_makes_the_file_v0_5() {
    let image = mixed(300, 180, PixelFormat::rgba8());
    let options = EncodeOptions::default().with_tile_size(64, 64).with_gpu();
    let report = encode(&image, &options).unwrap();

    let header = vpng_container::Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, vpng_container::MINOR_V0_5);
    assert!(header.has_gpu());
    assert!(report.gpu_payload_bytes > 0);
    assert!(report.gpu_table_bytes > 0);
}

#[test]
fn the_lossless_payload_is_untouched() {
    // The load-bearing rule: adding a lossy sibling must not change a single
    // lossless byte. The file's directory entries keep their geometry, their
    // codec ids, their lengths and their checksums; only the payload
    // *offsets* move, because the GPU table and the GPU payloads take up
    // room in the file.
    let image = mixed(300, 180, PixelFormat::rgba8());
    let plain = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let gpu = encode(
        &image,
        &EncodeOptions::default().with_tile_size(64, 64).with_gpu(),
    )
    .unwrap();

    assert_eq!(plain.codec_ids, gpu.codec_ids);
    assert_eq!(plain.compressed_lengths, gpu.compressed_lengths);
    assert_eq!(plain.uncompressed_lengths, gpu.uncompressed_lengths);

    let plain_directory = vpng_container::Directory::decode(
        &plain.bytes[64..64 + plain.tile_count as usize * 40 + 4],
        plain.tile_count,
    )
    .unwrap();
    let gpu_directory = vpng_container::Directory::decode(
        &gpu.bytes[72..72 + gpu.tile_count as usize * 40 + 4],
        gpu.tile_count,
    )
    .unwrap();

    for (index, (left, right)) in plain_directory
        .entries
        .iter()
        .zip(gpu_directory.entries.iter())
        .enumerate()
    {
        assert_eq!(left.x, right.x);
        assert_eq!(left.y, right.y);
        assert_eq!(left.width, right.width);
        assert_eq!(left.height, right.height);
        assert_eq!(left.codec_id, right.codec_id);
        assert_eq!(left.compressed_length, right.compressed_length);
        assert_eq!(left.uncompressed_length, right.uncompressed_length);
        assert_eq!(
            left.payload_checksum, right.payload_checksum,
            "tile {index}'s lossless payload checksum moved"
        );

        // The payload's own bytes are the same.
        let from = left.payload_offset as usize;
        let to = from + left.compressed_length as usize;
        let plain_payload = &plain.bytes[from..to];
        let from = right.payload_offset as usize;
        let to = from + right.compressed_length as usize;
        let gpu_payload = &gpu.bytes[from..to];
        assert_eq!(
            plain_payload, gpu_payload,
            "tile {index}'s lossless payload bytes moved"
        );
    }

    // And the pixels are exactly the original's, in both files.
    let limits = vpng_core::Limits::default();
    let decoded = Document::parse(&gpu.bytes, &limits)
        .unwrap()
        .decode_all(&limits)
        .unwrap();
    assert_eq!(decoded.data(), image.data());
}

#[test]
fn a_gpu_file_is_larger_than_the_same_file_without_one() {
    // The GPU payload costs bytes. The report must say how many, so a caller
    // can see the trade rather than infer it.
    let image = mixed(300, 180, PixelFormat::rgba8());
    let plain = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let gpu = encode(
        &image,
        &EncodeOptions::default().with_tile_size(64, 64).with_gpu(),
    )
    .unwrap();

    assert_eq!(plain.gpu_payload_bytes, 0);
    assert!(gpu.gpu_payload_bytes > 0);
    assert_eq!(
        gpu.file_size,
        plain.file_size + gpu.gpu_payload_bytes + gpu.gpu_table_bytes + 8,
        "the file grew by something other than the GPU table and its payloads"
    );
}

#[test]
fn a_sixteen_bit_picture_is_refused() {
    let image = mixed(64, 64, PixelFormat::rgba16());
    let err = encode(
        &image,
        &EncodeOptions::default().with_tile_size(64, 64).with_gpu(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_mask_layers_gpu_payload_carries_only_its_coverage() {
    // A mask layer's colour channels carry no meaning: its coverage is its
    // alpha channel. Its GPU payload is therefore BC4 alone, with **no**
    // colour blocks at all.
    let base = mixed(64, 64, PixelFormat::rgba8());
    let mut mask_pixels = vec![0u8; 64 * 64 * 4];
    for (index, pixel) in mask_pixels.chunks_exact_mut(4).enumerate() {
        pixel[0] = 200;
        pixel[1] = 100;
        pixel[2] = 50;
        pixel[3] = (index % 256) as u8;
    }
    let mask = CanonicalImage::from_vec(64, 64, PixelFormat::rgba8(), mask_pixels).unwrap();

    let options = EncodeOptions::default().with_tile_size(64, 64).with_gpu();
    let report = vpng_encoder::encode_layers_parallel(
        &[
            vpng_encoder::LayerInput::pixels(
                "base",
                vpng_core::Rect::new(0, 0, 64, 64).unwrap(),
                &base,
            ),
            vpng_encoder::LayerInput::mask(
                "coverage",
                vpng_core::Rect::new(0, 0, 64, 64).unwrap(),
                &mask,
            ),
        ],
        &options,
        &ParallelOptions::default().with_workers(1),
    )
    .unwrap();

    let limits = vpng_core::Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    let table = document.gpu_table().unwrap();

    // The records are in the canonical `(layer, level, y, x)` order, so the
    // base layer's single tile comes first and the mask layer's after it.
    assert_eq!(table.len(), 2);

    let base_record = table.records[0];
    assert_eq!(
        base_record.colour_codec, 7,
        "the base layer's tile has BC1 blocks"
    );
    assert_eq!(
        base_record.alpha_codec, 8,
        "the base layer's tile has BC4 blocks"
    );

    let mask_record = table.records[1];
    assert_eq!(
        mask_record.colour_codec, 0,
        "a mask's tile must carry no colour blocks"
    );
    assert_eq!(
        mask_record.alpha_codec, 8,
        "a mask's tile carries its coverage in BC4 blocks"
    );

    // The payload's length is the BC4 length and nothing else.
    let blocks = (64u32).div_ceil(4) as u64 * (64u32).div_ceil(4) as u64;
    assert_eq!(u64::from(mask_record.payload_length), blocks * 8);

    // The decoded mask is black with the coverage in its alpha.
    let payload = document.gpu_payload(1).unwrap();
    let decoded = vpng_gpu::decode_tile(64, 64, None, Some(payload)).unwrap();
    for (index, pixel) in decoded.chunks_exact(4).enumerate() {
        assert_eq!(pixel[0], 0, "a mask's colour is black");
        assert_eq!(pixel[1], 0, "a mask's colour is black");
        assert_eq!(pixel[2], 0, "a mask's colour is black");

        // The coverage is **quantised**, not exact: BC4 holds eight palette
        // entries per block, so a block whose coverage varies is reproduced
        // within a fraction of the block's own range.
        let x = index % 64;
        let y = index / 64;
        let mut lowest = 255u8;
        let mut highest = 0u8;
        for row in 0..4 {
            for column in 0..4 {
                let bx = x / 4 * 4 + column;
                let by = y / 4 * 4 + row;
                let value = ((by * 64 + bx) % 256) as u8;
                lowest = lowest.min(value);
                highest = highest.max(value);
            }
        }
        let original = (index % 256) as i32;
        let error = original - i32::from(pixel[3]);
        let bound = i32::from(highest - lowest) / 10 + 1;
        assert!(
            error.abs() <= bound,
            "the coverage moved by {error} levels at pixel {index}, and the block's \
             range is {lowest}..{highest}"
        );
    }
}
