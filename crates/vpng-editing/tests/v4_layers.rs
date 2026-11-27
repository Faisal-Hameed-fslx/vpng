//! The editing engine refuses a file that carries a layer table.
//!
//! The engine edits **one** grid of tiles — the base layer. A v0.4 file holds
//! several grids, and a commit would have to decide what to do with the
//! layers it does not know about. Rather than quietly rewriting the file as a
//! flat image, the engine refuses it: the data is never lost to a surprise.

use std::fs;
use std::path::PathBuf;

use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat, Rect};
use vpng_editing::{EditableDocument, FileEditor};
use vpng_encoder::{encode_layers, EncodeOptions, LayerInput};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let base = std::env::temp_dir().join(format!(
            "vpng-v4-edit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
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

/// A picture with some structure.
fn picture(width: u32, height: u32) -> CanonicalImage {
    let mut data = vec![0u8; width as usize * height as usize * 4];
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        pixel[0] = (index % 251) as u8;
        pixel[1] = (index % 97) as u8;
        pixel[2] = (index % 31) as u8;
        pixel[3] = 0xFF;
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// A layered file: a base covering the canvas and an overlay on top of it.
fn layered_file() -> Vec<u8> {
    let base = picture(128, 96);
    let overlay = picture(64, 32);
    let layers = [
        LayerInput::pixels("base", Rect::new(0, 0, 128, 96).unwrap(), &base),
        LayerInput::pixels("overlay", Rect::new(10, 10, 64, 32).unwrap(), &overlay),
    ];
    encode_layers(&layers, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes
}

#[test]
fn the_in_memory_editor_refuses_a_layered_file() {
    let bytes = layered_file();
    let limits = Limits::default();

    let err = EditableDocument::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
    assert!(err.to_string().contains("layer table"));
}

#[test]
fn the_file_backed_editor_refuses_a_layered_file() {
    let bytes = layered_file();
    let dir = TempDir::new();
    let path = dir.path("layered.vpng");
    fs::write(&path, &bytes).unwrap();

    let limits = Limits::default();
    let err = FileEditor::open(&path, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
    assert!(err.to_string().contains("layer table"));
}

#[test]
fn a_flat_file_is_still_accepted() {
    // The refusal must be narrow: a file without a layer table is still
    // edited exactly as before.
    let image = picture(128, 96);
    let bytes = vpng_encoder::encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let limits = Limits::default();

    assert!(EditableDocument::parse(&bytes, &limits).is_ok());
}
