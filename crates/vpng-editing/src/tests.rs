//! The editing engine's tests.
//!
//! The tests cover the promises the engine makes: no full-image buffer,
//! untouched tiles keep their bytes, a failed save leaves the original
//! intact, and undo and redo put the pixels back exactly.

use std::fs;
use std::path::{Path, PathBuf};

use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};

use crate::{CommitOptions, Editor, FailurePoint, FileEditor};

/// A deterministic gradient, so a test can predict what a region holds.
fn gradient(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            for c in 0..bpp {
                data[base + c] = ((x * 7 + y * 13 + c as u32 * 29) & 0xff) as u8;
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-editing-{name}-{}-{}",
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

/// Write a file to a temporary path and return the path and the bytes.
fn write_file(
    directory: &TempDir,
    name: &str,
    image: &CanonicalImage,
    tile: u32,
) -> (PathBuf, Vec<u8>) {
    let bytes = encode(image, &EncodeOptions::default().with_tile_size(tile, tile))
        .unwrap()
        .bytes;
    let path = directory.path(name);
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

#[test]
fn a_region_read_returns_exactly_the_region() {
    let limits = Limits::default();
    let image = gradient(300, 180, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    let rect = Rect::new(100, 40, 150, 90).unwrap();
    let region = editor.read_region(rect).unwrap();
    assert_eq!(region.width(), 150);
    assert_eq!(region.height(), 90);
    assert_eq!(region.len(), 150 * 90 * 3);

    // The region must equal the same crop of the whole image.
    let full = Document::parse(&bytes, &limits)
        .unwrap()
        .decode_all(&limits)
        .unwrap();
    let bpp = 3usize;
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
fn a_region_read_does_not_decode_unrelated_tiles() {
    let limits = Limits::default();
    let image = gradient(512, 512, PixelFormat::rgba8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(256, 256))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    let rect = Rect::new(0, 0, 8, 8).unwrap();
    let region = editor.read_region(rect).unwrap();
    assert_eq!(region.len(), 8 * 8 * 4);

    let stats = editor.cache_stats();
    assert_eq!(stats.decoded_tiles, 1, "only one tile may be decoded");
    assert_eq!(stats.current_bytes, 256 * 256 * 4);
}

#[test]
fn a_region_write_changes_only_the_tiles_it_intersects() {
    let limits = Limits::default();
    let image = gradient(512, 512, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(256, 256))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    // A 4x4 patch at the very corner touches exactly one tile.
    let rect = Rect::new(2, 2, 4, 4).unwrap();
    let patch = vec![0xAB; 4 * 4 * 3];
    editor.write_region(rect, &patch).unwrap();

    assert_eq!(editor.dirty_tiles(), vec![0]);
    assert!(editor.is_dirty());

    // The patch reads back exactly.
    let region = editor.read_region(rect).unwrap();
    assert_eq!(region.data(), &patch[..]);
}

#[test]
fn a_region_write_rejects_a_wrong_sized_buffer() {
    let limits = Limits::default();
    let image = gradient(64, 64, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    let rect = Rect::new(0, 0, 4, 4).unwrap();
    let err = editor.write_region(rect, &[0u8; 10]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_region_write_outside_the_image_is_rejected() {
    let limits = Limits::default();
    let image = gradient(64, 64, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    let rect = Rect::new(60, 60, 10, 10).unwrap();
    let err = editor.write_region(rect, &[0u8; 300]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn undo_and_redo_restore_the_pixels_exactly() {
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    let rect = Rect::new(10, 10, 16, 16).unwrap();
    let before = editor.read_region(rect).unwrap();

    let patch = vec![0x5A; 16 * 16 * 4];
    editor.write_region(rect, &patch).unwrap();
    assert_eq!(editor.read_region(rect).unwrap().data(), &patch[..]);

    assert!(editor.undo().unwrap());
    let after = editor.read_region(rect).unwrap();
    assert_eq!(after.data(), before.data(), "undo must restore the pixels");

    assert!(editor.redo().unwrap());
    assert_eq!(editor.read_region(rect).unwrap().data(), &patch[..]);
}

#[test]
fn undo_makes_a_tile_clean_again() {
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    let rect = Rect::new(10, 10, 16, 16).unwrap();
    editor.write_region(rect, &vec![0x5A; 16 * 16 * 4]).unwrap();
    assert_eq!(editor.dirty_tiles().len(), 1);

    editor.undo().unwrap();
    assert!(
        editor.dirty_tiles().is_empty(),
        "a tile that matches the file again does not need a new payload"
    );

    editor.redo().unwrap();
    assert_eq!(editor.dirty_tiles().len(), 1);
}

#[test]
fn undo_with_no_history_reports_that_there_is_nothing_to_do() {
    let limits = Limits::default();
    let image = gradient(64, 64, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    assert!(!editor.undo().unwrap());
    assert!(!editor.redo().unwrap());
}

#[test]
fn a_new_edit_invalidates_the_redo_chain() {
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &[1; 64 * 3])
        .unwrap();
    editor.undo().unwrap();
    assert!(editor.history_stats().redo_depth > 0);

    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &[2; 64 * 3])
        .unwrap();
    assert_eq!(editor.history_stats().redo_depth, 0);
}

#[test]
fn an_abort_restores_the_document() {
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
        .unwrap()
        .bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();

    let rect = Rect::new(10, 10, 16, 16).unwrap();
    let before = editor.read_region(rect).unwrap().data().to_vec();

    editor.begin().unwrap();
    editor.write_region(rect, &vec![0x5A; 16 * 16 * 4]).unwrap();
    assert!(editor.is_dirty());
    editor.abort().unwrap();

    assert!(!editor.is_dirty());
    assert_eq!(editor.read_region(rect).unwrap().data(), &before[..]);
}

#[test]
fn a_second_begin_is_rejected() {
    let limits = Limits::default();
    let image = gradient(64, 64, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor.begin().unwrap();
    assert!(editor.begin().is_err());
    editor.abort().unwrap();
}

#[test]
fn an_abort_without_a_transaction_is_rejected() {
    let limits = Limits::default();
    let image = gradient(64, 64, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    assert_eq!(
        editor.abort().unwrap_err().kind(),
        ErrorKind::InvalidArgument
    );
}

#[test]
fn a_commit_writes_a_file_that_decodes_to_the_edited_pixels() {
    let directory = TempDir::new("commit");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    let rect = Rect::new(10, 10, 16, 16).unwrap();
    let patch = vec![0x77; 16 * 16 * 4];
    editor.write_region(rect, &patch).unwrap();

    let report = editor.commit(&path, &CommitOptions::default()).unwrap();
    assert_eq!(report.tiles_rewritten, 1);
    assert_eq!(report.tiles_copied, 15);

    // The new file is a valid VPNG file with the edited pixels.
    let saved = fs::read(&path).unwrap();
    let document = Document::parse(&saved, &limits).unwrap();
    let decoded = document.decode_all(&limits).unwrap();
    assert_eq!(
        &decoded.data()[10 * 256 * 4 + 10 * 4..10 * 256 * 4 + 10 * 4 + 16 * 4],
        &patch[..16 * 4]
    );
    // Everything outside the patch is untouched.
    assert_eq!(decoded.data()[0], image.data()[0]);
    assert_eq!(
        decoded.data()[decoded.len() - 1],
        image.data()[image.len() - 1]
    );
}

#[test]
fn a_commit_copies_the_untouched_payloads_byte_for_byte() {
    let directory = TempDir::new("copy");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let (path, original) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &vec![0x11; 8 * 8 * 4])
        .unwrap();
    editor.commit(&path, &CommitOptions::default()).unwrap();

    let saved = fs::read(&path).unwrap();
    let before = Document::parse(&original, &limits).unwrap();
    let after = Document::parse(&saved, &limits).unwrap();

    // Tile 0 was rewritten; every other tile keeps exactly the same bytes.
    assert_ne!(
        before.tile_payload(0).unwrap(),
        after.tile_payload(0).unwrap()
    );
    for index in 1..before.tile_count() as usize {
        assert_eq!(
            before.tile_payload(index).unwrap(),
            after.tile_payload(index).unwrap(),
            "tile {index} must keep its bytes"
        );
    }
}

#[test]
fn a_failed_write_leaves_the_original_file_untouched() {
    let directory = TempDir::new("failure");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let (path, original) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &vec![0x11; 8 * 8 * 4])
        .unwrap();

    let options = CommitOptions {
        failure: Some(FailurePoint::AfterBytes(100)),
        ..CommitOptions::default()
    };
    assert!(editor.commit(&path, &options).is_err());

    // The original file is byte for byte what it was.
    assert_eq!(fs::read(&path).unwrap(), original);

    // And no partial file was left behind.
    let leftovers: Vec<_> = fs::read_dir(&directory.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("tmp"))
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[test]
fn a_failure_before_the_rename_leaves_the_original_untouched() {
    let directory = TempDir::new("rename-failure");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let (path, original) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &vec![0x11; 8 * 8 * 4])
        .unwrap();

    let options = CommitOptions {
        failure: Some(FailurePoint::BeforeRename),
        ..CommitOptions::default()
    };
    assert!(editor.commit(&path, &options).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn the_file_backed_editor_reads_and_writes_the_same_way() {
    let directory = TempDir::new("file-backed");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    let rect = Rect::new(20, 20, 8, 8).unwrap();
    let before = editor.editor().read_region(rect).unwrap();

    let patch = vec![0x3C; 8 * 8 * 4];
    editor.editor().write_region(rect, &patch).unwrap();
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &patch[..]
    );

    editor
        .editor()
        .commit(&path, &CommitOptions::default())
        .unwrap();

    let saved = fs::read(&path).unwrap();
    let document = Document::parse(&saved, &limits).unwrap();
    let decoded = document.decode_all(&limits).unwrap();
    assert_eq!(
        &decoded.data()[20 * 256 * 4 + 20 * 4..20 * 256 * 4 + 20 * 4 + 8 * 4],
        &patch[..8 * 4]
    );

    // The region outside the patch is unchanged.
    let mut check = FileEditor::open(&path, &limits).unwrap();
    let elsewhere = Rect::new(100, 100, 8, 8).unwrap();
    let after = check.editor().read_region(elsewhere).unwrap();
    let full = gradient(256, 256, PixelFormat::rgba8());
    let bpp = 4usize;
    let src_row = full.row_bytes().unwrap() as usize;
    for y in 0..8usize {
        let src = (100 + y) * src_row + 100 * bpp;
        let dst = y * 8 * bpp;
        assert_eq!(
            &after.data()[dst..dst + 8 * bpp],
            &full.data()[src..src + 8 * bpp]
        );
    }
    assert_eq!(before.len(), 8 * 8 * 4);
}

#[test]
fn the_file_backed_editor_commits_to_a_different_path() {
    let directory = TempDir::new("save-as");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);
    let target = directory.path("copy.vpng");

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    editor
        .editor()
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &vec![0x22; 8 * 8 * 4])
        .unwrap();
    editor
        .editor()
        .commit(&target, &CommitOptions::default())
        .unwrap();

    assert!(target.exists());
    let saved = fs::read(&target).unwrap();
    let document = Document::parse(&saved, &limits).unwrap();
    assert_eq!(document.width(), 128);
}

#[test]
fn a_commit_with_nothing_to_change_still_writes_a_valid_file() {
    let directory = TempDir::new("no-change");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgb8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    let report = editor.commit(&path, &CommitOptions::default()).unwrap();
    assert_eq!(report.tiles_rewritten, 0);
    assert_eq!(report.tiles_copied, 4);

    let saved = fs::read(&path).unwrap();
    let decoded = Document::parse(&saved, &limits)
        .unwrap()
        .decode_all(&limits)
        .unwrap();
    assert_eq!(decoded, image);
}

#[test]
fn a_pyramid_file_loses_its_previews_and_keeps_its_original_tiles() {
    // An edit invalidates the previews: they are derived from pixels the
    // edit changes. The editor cannot rebuild them without re-reading the
    // whole original, so the commit writes the original level only and says
    // how many previews it left out.
    let directory = TempDir::new("pyramid");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let bytes = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_pyramid(2),
    )
    .unwrap()
    .bytes;
    let path = directory.path("pyramid.vpng");
    fs::write(&path, &bytes).unwrap();

    let before = Document::parse(&bytes, &limits).unwrap();
    assert_eq!(before.level_count(), 3);
    let base = before.base_level();
    let untouched = base.entry_index(3).unwrap();
    let untouched_payload = before.tile_payload(untouched).unwrap().to_vec();

    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &vec![0x11; 8 * 8 * 4])
        .unwrap();
    let report = editor.commit(&path, &CommitOptions::default()).unwrap();
    assert_eq!(report.preview_levels_dropped, 2);
    assert_eq!(report.tiles_rewritten, 1);
    assert_eq!(report.tiles_copied, 15);

    let saved = fs::read(&path).unwrap();
    let after = Document::parse(&saved, &limits).unwrap();
    assert_eq!(after.level_count(), 1, "the previews are gone");
    assert!(!after.header().has_pyramid());

    // The untouched tile keeps its payload byte for byte.
    assert_eq!(
        after.tile_payload(untouched).unwrap(),
        &untouched_payload[..]
    );

    // The original is still exact.
    let decoded = after.decode_all(&limits).unwrap();
    assert_eq!(crop(&decoded, 0, 0, 8, 8), vec![0x11; 8 * 8 * 4]);
    assert_eq!(crop(&decoded, 200, 200, 8, 8), crop(&image, 200, 200, 8, 8));
}

#[test]
fn the_cache_evicts_when_the_budget_is_exceeded() {
    let limits = Limits::default();
    let image = gradient(512, 512, PixelFormat::rgba8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(128, 128))
        .unwrap()
        .bytes;
    // One 128x128 RGBA tile is 64 KiB; a 100 KiB budget holds one.
    let mut editor = Editor::parse(&bytes, &limits)
        .unwrap()
        .with_cache_bytes(100 * 1024)
        .unwrap();

    editor
        .read_region(Rect::new(0, 0, 128, 128).unwrap())
        .unwrap();
    editor
        .read_region(Rect::new(128, 0, 128, 128).unwrap())
        .unwrap();

    let stats = editor.cache_stats();
    assert!(stats.evictions > 0, "the cache must evict");
    assert!(stats.current_bytes <= 100 * 1024);
}

#[test]
fn a_forced_codec_is_used_for_the_rewritten_tiles() {
    let directory = TempDir::new("forced");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor
        .write_region(Rect::new(0, 0, 8, 8).unwrap(), &vec![0x11; 8 * 8 * 4])
        .unwrap();

    let options = CommitOptions {
        codec_id: Some(vpng_codecs::CODEC_RAW),
        ..CommitOptions::default()
    };
    editor.commit(&path, &options).unwrap();

    let saved = fs::read(&path).unwrap();
    let document = Document::parse(&saved, &limits).unwrap();
    assert_eq!(
        document.tile_entry(0).unwrap().codec_id,
        vpng_codecs::CODEC_RAW
    );
}

#[test]
fn the_engine_never_holds_a_full_image_buffer() {
    // A 2048x2048 RGBA image is 16 MiB of pixels. The engine must be able
    // to edit a region of it while holding only the tiles the region
    // touches.
    let limits = Limits::default();
    let image = gradient(2048, 2048, PixelFormat::rgba8());
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(256, 256))
        .unwrap()
        .bytes;

    let mut editor = Editor::parse(&bytes, &limits)
        .unwrap()
        .with_cache_bytes(4 * 1024 * 1024)
        .unwrap();
    let rect = Rect::new(1000, 1000, 64, 64).unwrap();
    let region = editor.read_region(rect).unwrap();
    assert_eq!(region.len(), 64 * 64 * 4);
    editor.write_region(rect, &vec![0x42; 64 * 64 * 4]).unwrap();

    // The patch straddles a tile boundary, so it dirties four 256x256
    // tiles: a megabyte, against a sixteen-megabyte image.
    assert_eq!(editor.dirty_tiles().len(), 4);
    assert_eq!(editor.dirty_bytes(), 4 * 256 * 256 * 4);
    assert!(
        editor.dirty_bytes() < image.len() as u64,
        "the engine must not hold the whole image"
    );
}

#[test]
fn the_document_parse_api_is_unchanged() {
    // The editing engine must not have changed the decoder's contract.
    let limits = Limits::default();
    let image = gradient(64, 64, PixelFormat::rgb8());
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;
    let document = Document::parse(&bytes, &limits).unwrap();
    assert_eq!(document.width(), 64);
    assert_eq!(document.height(), 64);
    assert_eq!(document.tile_count(), 1);
    assert!(document.check_integrity().is_ok());
}

#[test]
fn a_committed_file_passes_the_integrity_check() {
    let directory = TempDir::new("integrity");
    let limits = Limits::default();
    let image = gradient(256, 256, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let bytes = fs::read(&path).unwrap();
    let mut editor = Editor::parse(&bytes, &limits).unwrap();
    editor
        .write_region(Rect::new(0, 0, 16, 16).unwrap(), &vec![0x9A; 16 * 16 * 4])
        .unwrap();
    editor.commit(&path, &CommitOptions::default()).unwrap();

    let saved = fs::read(&path).unwrap();
    let document = Document::parse(&saved, &limits).unwrap();
    assert!(document.check_integrity().is_ok());
    assert_eq!(document.directory().entries.len(), 16);
}

/// The path a temporary file is written to, for a test that wants to look.
#[allow(dead_code)]
fn temporary_path_for(target: &Path) -> PathBuf {
    target.to_path_buf()
}

/// The pixels of a rectangle of an image.
fn crop(image: &CanonicalImage, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
    let bpp = image.format().bytes_per_pixel() as usize;
    let row = image.row_bytes().unwrap() as usize;
    let mut out = Vec::with_capacity(width as usize * height as usize * bpp);
    for row_index in 0..height as usize {
        let start = (y as usize + row_index) * row + x as usize * bpp;
        out.extend_from_slice(&image.data()[start..start + width as usize * bpp]);
    }
    out
}

/// Give one directory entry a distinctive predictor id, and repair the
/// directory checksum.
///
/// No codec reads the predictor id today, so a file that carries one still
/// parses and still decodes. It is the only way to tell a tile that *kept*
/// its metadata from a tile whose metadata was *reset*.
///
/// The entry's `flags` field is **not** used for this: from v0.2 onwards the
/// field carries the pyramid level and its upper byte is reserved, so a
/// v0.1 file with a non-zero `flags` is a corrupt file and is rejected.
fn patch_tile_predictor(bytes: &mut [u8], index: usize, predictor_id: u8) {
    const ENTRY_SIZE: usize = 40;
    const TRAILER_SIZE: usize = 4;
    // The main header: tile count at byte 28, directory offset at byte 32.
    let tile_count = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
    let directory_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;

    let start = directory_offset + index * ENTRY_SIZE;
    bytes[start + 13] = predictor_id;

    let body_len = tile_count * ENTRY_SIZE;
    let crc = vpng_container::crc32c(&bytes[directory_offset..directory_offset + body_len]);
    let trailer = directory_offset + body_len;
    bytes[trailer..trailer + TRAILER_SIZE].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn a_rewritten_tile_gets_fresh_metadata_and_an_untouched_tile_keeps_its_own() {
    let directory = TempDir::new("metadata");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, original) = write_file(&directory, "image.vpng", &image, 64);

    // Give the *first* tile a distinctive predictor id.
    let mut patched = original.clone();
    patch_tile_predictor(&mut patched, 0, 7);
    fs::write(&path, &patched).unwrap();

    let before = Document::parse(&patched, &limits).unwrap();
    assert_eq!(before.tile_entry(0).unwrap().predictor_id, 7);
    assert_eq!(before.tile_entry(0).unwrap().flags, 0);
    assert!(
        before.check_integrity().is_ok(),
        "the patched file is still intact"
    );

    // Edit the *second* tile only.
    let mut editor = Editor::parse(&patched, &limits).unwrap();
    editor
        .write_region(Rect::new(64, 0, 8, 8).unwrap(), &vec![0x11; 8 * 8 * 4])
        .unwrap();
    let report = editor.commit(&path, &CommitOptions::default()).unwrap();
    assert_eq!(report.tiles_rewritten, 1);
    assert_eq!(report.tiles_copied, 3);

    let saved = fs::read(&path).unwrap();
    let after = Document::parse(&saved, &limits).unwrap();

    // The rewritten tile describes the payload that was just written: it
    // must not inherit the metadata of the tile it replaced.
    assert_eq!(after.tile_entry(1).unwrap().predictor_id, 0);
    assert_eq!(after.tile_entry(1).unwrap().flags, 0);

    // The untouched tile keeps its own metadata.
    assert_eq!(after.tile_entry(0).unwrap().predictor_id, 7);
    assert_eq!(after.tile_entry(0).unwrap().flags, 0);

    // The pixels are still exact: the edited tile holds the patch, and the
    // untouched tile holds the original pixels.
    let decoded = after.decode_all(&limits).unwrap();
    assert_eq!(
        crop(&decoded, 64, 0, 8, 8),
        vec![0x11; 8 * 8 * 4],
        "the edited tile holds the patch"
    );
    assert_eq!(
        crop(&decoded, 0, 0, 8, 8),
        crop(&image, 0, 0, 8, 8),
        "the untouched tile holds the original pixels"
    );
}

#[test]
fn an_undo_cannot_cross_a_commit() {
    let directory = TempDir::new("commit-boundary");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    let rect = Rect::new(0, 0, 8, 8).unwrap();
    let patch = vec![0x11; 8 * 8 * 4];
    editor.editor().write_region(rect, &patch).unwrap();
    assert_eq!(editor.editor().edit_count(), 1);

    let report = editor
        .editor()
        .commit(&path, &CommitOptions::default())
        .unwrap();
    assert!(report.baseline_advanced);

    // The commit is the new baseline: there is nothing left to undo, and
    // nothing left to redo.
    assert!(
        !editor.editor().undo().unwrap(),
        "the undo history does not cross the commit"
    );
    assert!(!editor.editor().redo().unwrap());
    assert_eq!(editor.editor().edit_count(), 0);
    assert!(!editor.editor().is_dirty());

    // The committed pixels are still there.
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &patch[..]
    );
}

#[test]
fn an_edit_after_a_commit_starts_a_new_history_sequence() {
    let directory = TempDir::new("new-sequence");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    let rect = Rect::new(0, 0, 8, 8).unwrap();
    let first = vec![0x11; 8 * 8 * 4];
    let second = vec![0x22; 8 * 8 * 4];

    editor.editor().write_region(rect, &first).unwrap();
    editor
        .editor()
        .commit(&path, &CommitOptions::default())
        .unwrap();

    // The new edit is the first step of a new sequence.
    editor.editor().write_region(rect, &second).unwrap();
    assert_eq!(editor.editor().history_stats().undo_depth, 1);
    assert_eq!(editor.editor().edit_count(), 1);

    // The undo undoes the *new* edit and lands on the committed pixels.
    assert!(editor.editor().undo().unwrap());
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &first[..]
    );
    assert!(
        !editor.editor().is_dirty(),
        "the session matches the committed file again"
    );
}

#[test]
fn a_redo_after_a_commit_undo_restores_the_new_edit() {
    let directory = TempDir::new("redo-after-commit");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    let rect = Rect::new(0, 0, 8, 8).unwrap();
    let first = vec![0x11; 8 * 8 * 4];
    let second = vec![0x22; 8 * 8 * 4];

    editor.editor().write_region(rect, &first).unwrap();
    editor
        .editor()
        .commit(&path, &CommitOptions::default())
        .unwrap();
    editor.editor().write_region(rect, &second).unwrap();

    assert!(editor.editor().undo().unwrap());
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &first[..]
    );

    assert!(editor.editor().redo().unwrap());
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &second[..]
    );
    assert_eq!(editor.editor().dirty_tiles(), vec![0]);
}

#[test]
fn a_failed_commit_keeps_the_undo_history() {
    let directory = TempDir::new("failed-commit");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, original) = write_file(&directory, "image.vpng", &image, 64);

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    let rect = Rect::new(0, 0, 8, 8).unwrap();
    let patch = vec![0x11; 8 * 8 * 4];
    editor.editor().write_region(rect, &patch).unwrap();

    let options = CommitOptions {
        failure: Some(FailurePoint::BeforeRename),
        ..CommitOptions::default()
    };
    assert!(editor.editor().commit(&path, &options).is_err());

    // The file is untouched.
    assert_eq!(fs::read(&path).unwrap(), original);

    // The editing state survived the failure: the history, the dirty set,
    // and the edit counter are all as they were.
    assert_eq!(editor.editor().history_stats().undo_depth, 1);
    assert_eq!(editor.editor().edit_count(), 1);
    assert_eq!(editor.editor().dirty_tiles(), vec![0]);

    // And the undo is still available.
    assert!(editor.editor().undo().unwrap());
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &crop(&image, 0, 0, 8, 8)[..]
    );
    assert!(!editor.editor().is_dirty());
}

#[test]
fn an_abort_after_a_commit_reports_that_no_transaction_is_open() {
    let directory = TempDir::new("abort-after-commit");
    let limits = Limits::default();
    let image = gradient(128, 128, PixelFormat::rgba8());
    let (path, _) = write_file(&directory, "image.vpng", &image, 64);

    let mut editor = FileEditor::open(&path, &limits).unwrap();
    let rect = Rect::new(0, 0, 8, 8).unwrap();
    let patch = vec![0x11; 8 * 8 * 4];

    editor.editor().begin().unwrap();
    editor.editor().write_region(rect, &patch).unwrap();
    editor
        .editor()
        .commit(&path, &CommitOptions::default())
        .unwrap();

    // The commit closed the transaction.
    assert!(!editor.editor().in_transaction());
    let err = editor.editor().abort().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);

    // The committed pixels survived the failed abort.
    assert_eq!(
        editor.editor().read_region(rect).unwrap().data(),
        &patch[..]
    );
}
