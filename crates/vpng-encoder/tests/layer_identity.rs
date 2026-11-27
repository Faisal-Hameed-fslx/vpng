//! The layered encoder's contract: the bytes of a v0.4 file depend on the
//! layers and the options, and on nothing else.
//!
//! Three rules are pinned here:
//!
//! * the batch encoder and the streaming writer produce the **same file**;
//! * one worker and N workers produce the **same file**;
//! * a layer is encoded **on its own**: changing one layer does not change
//!   another layer's payload bytes.
//!
//! This is not a performance test. It is the correctness gate that makes the
//! parallel and streaming paths safe to use for layered files.

use std::fs;
use std::path::PathBuf;

use vpng_core::{CanonicalImage, ErrorKind, PixelFormat, Rect};
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::stream::encode_layers_to_file;
use vpng_encoder::{encode_layers, encode_layers_parallel, EncodeOptions, LayerInput};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-encoder-layers-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        TempDir(base)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Mixed content, so different tiles want different codecs.
fn mixed(width: u32, height: u32) -> CanonicalImage {
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for y in 0..height {
        for x in 0..width {
            let base = (y as usize * width as usize + x as usize) * 4;
            for c in 0..4 {
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
            data[base + 3] = 0xFF;
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// A flat picture, which every codec compresses to almost nothing.
fn flat(width: u32, height: u32) -> CanonicalImage {
    let mut data = vec![0u8; width as usize * height as usize * 4];
    for pixel in data.chunks_exact_mut(4) {
        pixel[0] = 0x40;
        pixel[1] = 0x40;
        pixel[2] = 0x40;
        pixel[3] = 0xFF;
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// A mask whose alpha ramps from 0 to 255 across its width.
fn coverage(width: u32, height: u32) -> CanonicalImage {
    let mut data = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let base = (y as usize * width as usize + x as usize) * 4;
            data[base] = 0x10;
            data[base + 1] = 0x20;
            data[base + 2] = 0x30;
            data[base + 3] = if width == 1 {
                255
            } else {
                (x * 255 / (width - 1)) as u8
            };
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// The whole canvas.
fn canvas(width: u32, height: u32) -> Rect {
    Rect::new(0, 0, width, height).unwrap()
}

/// The three layers every test here uses: a base, an overlay and a mask.
fn three_layers() -> (CanonicalImage, CanonicalImage, CanonicalImage) {
    (mixed(300, 180), mixed(100, 60), coverage(120, 80))
}

/// The layers of a file: a base covering the canvas, an overlay in one
/// corner, and a mask over the overlay.
fn layers<'a>(
    base: &'a CanonicalImage,
    overlay: &'a CanonicalImage,
    mask: &'a CanonicalImage,
) -> [LayerInput<'a>; 3] {
    [
        LayerInput::pixels("base", canvas(base.width(), base.height()), base),
        LayerInput::pixels("overlay", Rect::new(20, 20, 100, 60).unwrap(), overlay).with_mask(2),
        LayerInput::mask("m", Rect::new(20, 20, 120, 80).unwrap(), mask),
    ]
}

#[test]
fn the_batch_encoder_and_the_streaming_writer_agree() {
    let (base, overlay, mask) = three_layers();
    let inputs = layers(&base, &overlay, &mask);
    let options = EncodeOptions::default().with_tile_size(64, 64);

    let batch = encode_layers(&inputs, &options).unwrap();

    let dir = TempDir::new();
    let path = dir.path("layered.vpng");
    encode_layers_to_file(&inputs, &options, &ParallelOptions::default(), &path).unwrap();
    let streamed = fs::read(&path).unwrap();

    assert_eq!(batch.bytes, streamed);
    assert_eq!(batch.file_size as usize, streamed.len());
}

#[test]
fn the_worker_count_does_not_change_the_bytes() {
    let (base, overlay, mask) = three_layers();
    let inputs = layers(&base, &overlay, &mask);
    let options = EncodeOptions::default().with_tile_size(64, 64);

    let serial = encode_layers_parallel(
        &inputs,
        &options,
        &ParallelOptions::default().with_workers(1),
    )
    .unwrap();

    for workers in [2usize, 4, 8] {
        let parallel = encode_layers_parallel(
            &inputs,
            &options,
            &ParallelOptions::default().with_workers(workers),
        )
        .unwrap();
        assert_eq!(
            serial.bytes, parallel.bytes,
            "{workers} workers changed the bytes"
        );
    }
}

#[test]
fn the_streaming_writer_does_not_depend_on_the_worker_count_either() {
    let (base, overlay, mask) = three_layers();
    let inputs = layers(&base, &overlay, &mask);
    let options = EncodeOptions::default().with_tile_size(64, 64);

    let dir = TempDir::new();
    let serial_path = dir.path("serial.vpng");
    let parallel_path = dir.path("parallel.vpng");

    encode_layers_to_file(
        &inputs,
        &options,
        &ParallelOptions::default().with_workers(1),
        &serial_path,
    )
    .unwrap();
    encode_layers_to_file(
        &inputs,
        &options,
        &ParallelOptions::default().with_workers(4),
        &parallel_path,
    )
    .unwrap();

    assert_eq!(
        fs::read(&serial_path).unwrap(),
        fs::read(&parallel_path).unwrap()
    );
}

#[test]
fn a_layer_is_encoded_on_its_own() {
    // Two files that differ only in the overlay must hold the same bytes for
    // the base layer: cross-layer prediction is out of scope, so one layer's
    // pixels can never influence another's payload.
    let (base, overlay, mask) = three_layers();
    let flat = flat(100, 60);

    let options = EncodeOptions::default().with_tile_size(64, 64);
    let first = encode_layers(&layers(&base, &overlay, &mask), &options).unwrap();
    let second = encode_layers(&layers(&base, &flat, &mask), &options).unwrap();

    assert_eq!(first.layer_count, 3);
    assert_eq!(second.layer_count, 3);

    // The base layer's directory entries — its geometry, its codecs, its
    // lengths and its payload checksums — are identical in both files.
    let first_entries = layer_entries(&first.bytes, 0);
    let second_entries = layer_entries(&second.bytes, 0);
    assert_eq!(first_entries, second_entries);

    // The overlay's entries are not: its pixels changed.
    assert_ne!(
        layer_entries(&first.bytes, 1),
        layer_entries(&second.bytes, 1)
    );
}

/// One directory entry, as a comparable tuple.
type Entry = (u32, u32, u16, u16, u8, u64, u64, u32, u32);

/// The directory entries of one layer, as comparable tuples.
fn layer_entries(bytes: &[u8], layer: u16) -> Vec<Entry> {
    let header = vpng_container::Header::parse(bytes).unwrap();
    let directory = vpng_container::Directory::decode(
        &bytes[vpng_container::HEADER_SIZE
            ..vpng_container::HEADER_SIZE + header.tile_count as usize * 40 + 4],
        header.tile_count,
    )
    .unwrap();

    directory
        .entries
        .iter()
        .filter(|entry| entry.layer() == layer)
        .map(|entry| {
            (
                entry.x,
                entry.y,
                entry.width,
                entry.height,
                entry.codec_id,
                entry.payload_offset,
                entry.compressed_length,
                entry.uncompressed_length,
                entry.payload_checksum,
            )
        })
        .collect()
}

#[test]
fn a_layered_file_may_not_ask_for_a_pyramid() {
    let (base, overlay, mask) = three_layers();
    let inputs = layers(&base, &overlay, &mask);
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(3);

    let err = encode_layers(&inputs, &options).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_file_may_not_hold_more_than_sixteen_layers() {
    let base = mixed(64, 64);
    let inputs: Vec<LayerInput<'_>> = (0..17)
        .map(|index| {
            LayerInput::pixels(
                format!("layer {index}"),
                canvas(base.width(), base.height()),
                &base,
            )
        })
        .collect();

    let err = encode_layers(&inputs, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}
