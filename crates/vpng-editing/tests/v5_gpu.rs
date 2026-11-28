//! The editing engine refuses a file that carries a GPU payload table.
//!
//! The engine rewrites the tiles it changes and copies the payloads of the
//! tiles it does not. A GPU payload is a **lossy sibling** of a tile's
//! lossless payload, and the engine has no way to rebuild it from the
//! rewritten pixels: BC1 and BC4 throw information away, so the result
//! would not be the same bytes.
//!
//! Copying the old GPU payload would leave a stale picture in the file.
//! Dropping it would quietly remove a feature. The engine refuses the file
//! instead, and the message says why.

use std::fs;
use std::path::PathBuf;

use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat};
use vpng_editing::{EditableDocument, FileEditor};
use vpng_encoder::{encode, EncodeOptions};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let base = std::env::temp_dir().join(format!(
            "vpng-v5-edit-{}-{}",
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

/// A file that carries a GPU payload table.
fn gpu_file() -> Vec<u8> {
    let image = picture(128, 96);
    encode(
        &image,
        &EncodeOptions::default().with_tile_size(64, 64).with_gpu(),
    )
    .unwrap()
    .bytes
}

#[test]
fn the_in_memory_editor_refuses_a_gpu_file() {
    let bytes = gpu_file();
    let limits = Limits::default();

    let err = EditableDocument::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
    assert!(err.to_string().contains("GPU table"));
}

#[test]
fn the_file_backed_editor_refuses_a_gpu_file() {
    let bytes = gpu_file();
    let dir = TempDir::new();
    let path = dir.path("gpu.vpng");
    fs::write(&path, &bytes).unwrap();

    let limits = Limits::default();
    let err = FileEditor::open(&path, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
    assert!(err.to_string().contains("GPU table"));
}

#[test]
fn a_flat_file_is_still_accepted() {
    // The refusal must be narrow: a file without a GPU table is still
    // edited exactly as before.
    let image = picture(128, 96);
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let limits = Limits::default();

    assert!(EditableDocument::parse(&bytes, &limits).is_ok());
}
