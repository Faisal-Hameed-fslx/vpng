//! The v0.3 version rule in the editing engine.
//!
//! An edit that rewrites a tile with a codec promoted in v0.3 turns the
//! committed file into a **v0.3** file, and the committed file says so. The
//! tiles the edit did not touch keep their payload bytes exactly, whatever
//! the version became.

use std::fs;
use std::path::PathBuf;

use vpng_container::{Directory, Header, HEADER_SIZE, MINOR_V0_1, MINOR_V0_3};
use vpng_core::{Limits, PixelFormat, Rect};
use vpng_editing::{CommitOptions, FileEditor};
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::gradient;

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let base = std::env::temp_dir().join(format!(
            "vpng-v3-edit-{name}-{}-{}",
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

/// The payload bytes of every tile, in directory order.
fn payload_bytes(bytes: &[u8]) -> Vec<Vec<u8>> {
    let tile_count = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    let directory_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let mut out = Vec::with_capacity(tile_count as usize);
    for index in 0..tile_count as usize {
        let entry = directory_offset as usize + index * 40;
        let offset = u64::from_le_bytes(bytes[entry + 16..entry + 24].try_into().unwrap());
        let length = u64::from_le_bytes(bytes[entry + 24..entry + 32].try_into().unwrap());
        out.push(bytes[offset as usize..(offset + length) as usize].to_vec());
    }
    out
}

/// Write a v0.1 file and return its path.
fn write_v0_1_file(path: &PathBuf) -> Vec<u8> {
    let image = gradient(300, 180, PixelFormat::rgba8(), 1);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    assert_eq!(
        Header::parse(&report.bytes).unwrap().minor,
        MINOR_V0_1,
        "the default path must still write a v0.1 file"
    );
    fs::write(path, &report.bytes).unwrap();
    report.bytes
}

#[test]
fn a_commit_that_uses_the_dictionary_codec_bumps_the_version_to_v0_3() {
    let directory = TempDir::new("bump");
    let path = directory.path("source.vpng");
    let original = write_v0_1_file(&path);

    let limits = Limits::default();
    let mut file = FileEditor::open(&path, &limits).unwrap();
    let editor = file.editor();

    // Change one tile's worth of pixels.
    let rect = Rect::new(0, 0, 32, 32).unwrap();
    let pixels = vec![0x7Fu8; 32 * 32 * 4];
    editor.write_region(rect, &pixels).unwrap();

    let target = directory.path("committed.vpng");
    let options = CommitOptions {
        codec_id: Some(vpng_codecs::CODEC_LZ77),
        ..CommitOptions::default()
    };
    let report = editor.commit(&target, &options).unwrap();
    assert_eq!(report.tiles_rewritten, 1);
    assert!(report.tiles_copied > 0);

    let committed = fs::read(&target).unwrap();
    let header = Header::parse(&committed).unwrap();
    assert_eq!(
        header.minor, MINOR_V0_3,
        "a file that names codec id 4 must say it is a v0.3 file"
    );

    // The untouched tiles keep their payload bytes exactly.
    let before = payload_bytes(&original);
    let after = payload_bytes(&committed);
    assert_eq!(before.len(), after.len());
    let mut copied = 0usize;
    for (index, (old, new)) in before.iter().zip(after.iter()).enumerate() {
        if old == new {
            copied += 1;
        } else {
            assert_eq!(index, 0, "only the edited tile may change");
        }
    }
    assert_eq!(copied, before.len() - 1);
}

#[test]
fn a_commit_that_keeps_the_stable_set_keeps_the_version() {
    let directory = TempDir::new("stable");
    let path = directory.path("source.vpng");
    write_v0_1_file(&path);

    let limits = Limits::default();
    let mut file = FileEditor::open(&path, &limits).unwrap();
    let editor = file.editor();

    let rect = Rect::new(0, 0, 32, 32).unwrap();
    let pixels = vec![0x7Fu8; 32 * 32 * 4];
    editor.write_region(rect, &pixels).unwrap();

    let target = directory.path("committed.vpng");
    editor.commit(&target, &CommitOptions::default()).unwrap();

    let committed = fs::read(&target).unwrap();
    let header = Header::parse(&committed).unwrap();
    assert_eq!(
        header.minor, MINOR_V0_1,
        "a commit that names no promoted codec must not move the version"
    );

    // The committed file names no promoted codec.
    let tile_count =
        u32::from_le_bytes([committed[28], committed[29], committed[30], committed[31]]);
    let directory_offset = u64::from_le_bytes(committed[32..40].try_into().unwrap());
    for index in 0..tile_count as usize {
        let entry = directory_offset as usize + index * 40;
        assert!(!vpng_codecs::is_v0_3_codec(committed[entry + 12]));
    }
}

#[test]
fn a_commit_with_the_extended_set_says_what_it_used() {
    let directory = TempDir::new("extended");
    let path = directory.path("source.vpng");
    write_v0_1_file(&path);

    let limits = Limits::default();
    let mut file = FileEditor::open(&path, &limits).unwrap();
    let editor = file.editor();

    let rect = Rect::new(0, 0, 32, 32).unwrap();
    let pixels = vec![0x7Fu8; 32 * 32 * 4];
    editor.write_region(rect, &pixels).unwrap();

    let target = directory.path("committed.vpng");
    let options = CommitOptions {
        codec_set: vpng_codecs::CodecSet::V0_3,
        ..CommitOptions::default()
    };
    editor.commit(&target, &options).unwrap();

    let committed = fs::read(&target).unwrap();
    let header = Header::parse(&committed).unwrap();

    let tile_count =
        u32::from_le_bytes([committed[28], committed[29], committed[30], committed[31]]);
    let directory_offset = u64::from_le_bytes(committed[32..40].try_into().unwrap());
    let mut uses_promoted = false;
    for index in 0..tile_count as usize {
        let entry = directory_offset as usize + index * 40;
        uses_promoted |= vpng_codecs::is_v0_3_codec(committed[entry + 12]);
    }

    if uses_promoted {
        assert_eq!(header.minor, MINOR_V0_3);
    } else {
        assert_eq!(header.minor, MINOR_V0_1);
    }

    // The committed file is a valid file whichever version it carries.
    let limits = Limits::default();
    let document = vpng_decoder::Document::parse(&committed, &limits).unwrap();
    assert!(document.decode_all(&limits).is_ok());
}

#[test]
fn the_directory_of_a_committed_file_is_still_well_formed() {
    let directory = TempDir::new("directory");
    let path = directory.path("source.vpng");
    write_v0_1_file(&path);

    let limits = Limits::default();
    let mut file = FileEditor::open(&path, &limits).unwrap();
    let editor = file.editor();
    let rect = Rect::new(0, 0, 32, 32).unwrap();
    let pixels = vec![0x7Fu8; 32 * 32 * 4];
    editor.write_region(rect, &pixels).unwrap();

    let target = directory.path("committed.vpng");
    let options = CommitOptions {
        codec_id: Some(vpng_codecs::CODEC_LZ77),
        ..CommitOptions::default()
    };
    editor.commit(&target, &options).unwrap();

    let committed = fs::read(&target).unwrap();
    let tile_count =
        u32::from_le_bytes([committed[28], committed[29], committed[30], committed[31]]);
    let size = Directory::total_size(tile_count).unwrap();
    let body_len = size - 4;
    let crc = vpng_container::crc32c(&committed[HEADER_SIZE..HEADER_SIZE + body_len]);
    assert_eq!(
        committed[HEADER_SIZE + body_len..HEADER_SIZE + body_len + 4],
        crc.to_le_bytes(),
        "the committed file's directory checksum must be correct"
    );
}
