//! The parallel encoder's contract: **one worker and N workers produce the
//! same file, byte for byte**, and the streaming writer produces that same
//! file too.
//!
//! This is not a performance test. It is the correctness gate that makes
//! parallel encoding safe to switch on: the file's bytes must depend on the
//! pixels and the options, never on the thread count, the chunk size or the
//! order the workers happened to finish in.

use std::fs;
use std::path::{Path, PathBuf};

use vpng_core::{CanonicalImage, ErrorKind, PixelFormat};
use vpng_encoder::parallel::{encode_parallel, ParallelOptions};
use vpng_encoder::source::PixelSource;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::{encode, EncodeOptions};
use vpng_tiling::TileGeometry;

/// Every format v0.1 enables.
const FORMATS: [PixelFormat; 8] = [
    PixelFormat::gray8(),
    PixelFormat::gray16(),
    PixelFormat::gray_alpha8(),
    PixelFormat::gray_alpha16(),
    PixelFormat::rgb8(),
    PixelFormat::rgb16(),
    PixelFormat::rgba8(),
    PixelFormat::rgba16(),
];

/// Exact fit, single edge tiles, double edge tiles and the degenerate cases.
const SIZES: [(u32, u32); 6] = [(1, 1), (7, 3), (64, 64), (256, 256), (257, 130), (300, 180)];

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-encoder-identity-{}-{}",
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
/// smooth region and an incompressible region. Per-tile codec selection is
/// the thing most likely to break under parallelism, because it is the part
/// whose result must not depend on which thread ran it.
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
                    // Flat.
                    0x40
                } else if x < (2 * width) / 3 {
                    // Smooth.
                    ((x * 5 + y * 11 + c as u32 * 17) & 0xff) as u8
                } else {
                    // Incompressible.
                    (noise >> 33) as u8
                };
                data[base + c] = value;
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

fn gradient(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for y in 0..height {
        for x in 0..width {
            let base = (y as usize * width as usize + x as usize) * bpp;
            for c in 0..bpp {
                data[base + c] = ((x * 7 + y * 13 + c as u32 * 29) & 0xff) as u8;
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

    let parallel =
        encode_parallel(image, options, &ParallelOptions::default().with_workers(4)).unwrap();
    assert_eq!(
        parallel.bytes, batch.bytes,
        "{label}: four workers produced a different file from the batch encoder"
    );
    assert_eq!(
        parallel.codec_ids, batch.codec_ids,
        "{label}: four workers chose different codecs"
    );

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
        streamed.uncompressed_lengths, batch.uncompressed_lengths,
        "{label}: the streaming writer reported different canonical sizes"
    );
}

#[test]
fn parallel_and_streaming_match_the_batch_encoder_everywhere() {
    let directory = TempDir::new();
    let mut cases = 0usize;

    for format in FORMATS {
        for (width, height) in SIZES {
            for tile_size in [16u32, 64, 256] {
                let image = mixed(width, height, format);
                let options = EncodeOptions::default().with_tile_size(tile_size, tile_size);
                let label = format!("{format}-{width}x{height}-tile{tile_size}");
                let path = directory.0.join(format!("case-{cases}.vpng"));
                assert_all_paths_agree(&image, &options, &path, &label);
                cases += 1;
            }
        }
    }

    // 8 formats x 6 sizes x 3 tile geometries.
    assert_eq!(cases, 144);
}

#[test]
fn neither_the_worker_count_nor_the_chunk_size_changes_the_bytes() {
    let image = mixed(300, 180, PixelFormat::rgba8());
    let options = EncodeOptions::default().with_tile_size(32, 32);
    let expected = encode(&image, &options).unwrap().bytes;

    for workers in [1usize, 2, 3, 4, 8, 16] {
        for chunk_tiles in [1usize, 2, 7, 64] {
            let parallel = ParallelOptions::default()
                .with_workers(workers)
                .with_chunk_tiles(chunk_tiles);
            let report = encode_parallel(&image, &options, &parallel).unwrap();
            assert_eq!(
                report.bytes, expected,
                "{workers} workers with {chunk_tiles} tiles per chunk changed the file"
            );
        }
    }
}

#[test]
fn a_forced_codec_is_also_identical_under_parallelism() {
    // Auto selection is the interesting case, but a forced codec must not
    // change either: it takes a different branch in `select_payload`.
    for codec in [
        vpng_codecs::CODEC_RAW,
        vpng_codecs::CODEC_RLE,
        vpng_codecs::CODEC_PREDICTOR,
    ] {
        let image = gradient(257, 130, PixelFormat::rgb8());
        let options = EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(codec);
        let expected = encode(&image, &options).unwrap().bytes;
        let report = encode_parallel(
            &image,
            &options,
            &ParallelOptions::default().with_workers(8),
        )
        .unwrap();
        assert_eq!(
            report.bytes, expected,
            "codec {codec} changed under parallelism"
        );
    }
}

/// A source that generates its tiles on demand, so there is never an image
/// buffer anywhere. This is the case the streaming writer exists for.
struct GeneratedSource {
    width: u32,
    height: u32,
    format: PixelFormat,
}

impl PixelSource for GeneratedSource {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn format(&self) -> PixelFormat {
        self.format
    }

    fn tile_pixels(
        &self,
        _index: usize,
        geom: &TileGeometry,
        out: &mut Vec<u8>,
    ) -> vpng_core::Result<()> {
        let bpp = self.format.bytes_per_pixel() as usize;
        out.clear();
        out.resize(geom.width as usize * geom.height as usize * bpp, 0);
        for row in 0..geom.height {
            for column in 0..geom.width {
                let base = (row as usize * geom.width as usize + column as usize) * bpp;
                for c in 0..bpp {
                    out[base + c] = (((geom.x + column) * 7 + (geom.y + row) * 13 + c as u32 * 29)
                        & 0xff) as u8;
                }
            }
        }
        Ok(())
    }
}

#[test]
fn a_generated_source_streams_the_same_file_as_the_image_it_describes() {
    let directory = TempDir::new();
    let format = PixelFormat::rgba8();
    let source = GeneratedSource {
        width: 300,
        height: 180,
        format,
    };
    let options = EncodeOptions::default().with_tile_size(64, 64);

    // The same pixels as a real image, so the two can be compared.
    let image = gradient(300, 180, format);
    let expected = encode(&image, &options).unwrap().bytes;

    let path = directory.0.join("generated.vpng");
    for workers in [1usize, 4] {
        encode_to_file(
            &source,
            &options,
            &ParallelOptions::default().with_workers(workers),
            &path,
        )
        .unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            expected,
            "{workers} workers: the generated source produced a different file"
        );
    }
}

#[test]
fn a_size_limit_failure_leaves_no_file_behind() {
    let directory = TempDir::new();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let options = EncodeOptions::default().with_max_size(1024);
    let path = directory.0.join("too-big.vpng");

    let error = encode_to_file(
        &image,
        &options,
        &ParallelOptions::default().with_workers(4),
        &path,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::SizeLimitExceeded);

    // Neither the target nor the temporary file may survive a failed encode.
    assert!(
        !path.exists(),
        "the target file was created despite the failure"
    );
    let leftovers: Vec<_> = fs::read_dir(&directory.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .collect();
    assert!(
        leftovers.is_empty(),
        "a failed encode left {} file(s) behind",
        leftovers.len()
    );
}

#[test]
fn a_failing_source_propagates_the_error_and_leaves_no_file() {
    struct Broken;
    impl PixelSource for Broken {
        fn width(&self) -> u32 {
            128
        }
        fn height(&self) -> u32 {
            128
        }
        fn format(&self) -> PixelFormat {
            PixelFormat::rgba8()
        }
        fn tile_pixels(
            &self,
            index: usize,
            _: &TileGeometry,
            _: &mut Vec<u8>,
        ) -> vpng_core::Result<()> {
            if index == 3 {
                Err(vpng_core::Error::corrupt(
                    "the source cannot produce tile 3",
                ))
            } else {
                Ok(())
            }
        }
    }

    let directory = TempDir::new();
    let path = directory.0.join("broken.vpng");
    let error = encode_to_file(
        &Broken,
        &EncodeOptions::default().with_tile_size(64, 64),
        &ParallelOptions::default().with_workers(4),
        &path,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::CorruptFile);
    assert!(
        !path.exists(),
        "the target file was created despite the failure"
    );
    let leftovers: Vec<_> = fs::read_dir(&directory.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .collect();
    assert!(leftovers.is_empty(), "a failed encode left files behind");
}
