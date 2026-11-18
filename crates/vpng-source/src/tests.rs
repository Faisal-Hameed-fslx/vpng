//! The reader's tests.
//!
//! The reader's promises are small and exact: it reads the container once,
//! it reads a payload when one is asked for and nothing else, it decodes
//! the same bytes the in-memory decoder decodes, and it says how much it
//! read. Each promise has a test.

use std::fs;
use std::path::PathBuf;

use vpng_core::{ErrorKind, Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::gradient;

use crate::{ContainerMeta, TileReader};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-source-{name}-{}-{}",
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

/// The limits the tests run under. They are generous enough for a test
/// image and tight enough that a runaway allocation fails loudly.
fn limits() -> Limits {
    Limits::default()
}

/// Write a gradient to a temporary file and return its path and bytes.
fn write_file(
    directory: &TempDir,
    name: &str,
    width: u32,
    height: u32,
    tile: u32,
) -> (PathBuf, Vec<u8>) {
    let image = gradient(width, height, PixelFormat::rgba8(), 1);
    let bytes = encode(&image, &EncodeOptions::default().with_tile_size(tile, tile))
        .unwrap()
        .bytes;
    let path = directory.path(name);
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

#[test]
fn the_reader_reads_the_container_once_and_nothing_else() {
    let directory = TempDir::new("container-once");
    let (path, bytes) = write_file(&directory, "image.vpng", 128, 96, 32);

    let reader = TileReader::open(&path, &limits()).unwrap();

    // The header and the directory, and nothing more.
    let directory_size = 4 + 40 * reader.meta().tile_count() as usize;
    assert_eq!(reader.bytes_read(), 64 + directory_size as u64);
    assert_eq!(reader.payloads_read(), 0);

    // The file is far bigger than what has been read so far.
    assert!(reader.bytes_read() < bytes.len() as u64);
}

#[test]
fn a_payload_costs_exactly_its_own_bytes() {
    let directory = TempDir::new("payload-cost");
    let (path, _) = write_file(&directory, "image.vpng", 128, 96, 32);

    let mut reader = TileReader::open(&path, &limits()).unwrap();
    let before = reader.bytes_read();

    let payload = reader.tile_payload(0).unwrap();
    let entry = *reader.meta().tile_entry(0).unwrap();

    assert_eq!(reader.bytes_read(), before + entry.compressed_length);
    assert_eq!(reader.bytes_read(), before + payload.len() as u64);
    assert_eq!(reader.payloads_read(), 1);
}

#[test]
fn the_reader_never_reads_a_payload_it_was_not_asked_for() {
    let directory = TempDir::new("no-read-ahead");
    let (path, _) = write_file(&directory, "image.vpng", 256, 256, 32);

    let mut reader = TileReader::open(&path, &limits()).unwrap();
    let before = reader.bytes_read();

    // Four tiles exist; ask for one.
    let entry = *reader.meta().tile_entry(2).unwrap();
    reader.tile_payload(2).unwrap();

    // Exactly that one payload's bytes were read — not the three others,
    // and not the whole file.
    assert_eq!(reader.bytes_read(), before + entry.compressed_length);
    assert_eq!(reader.payloads_read(), 1);
}

#[test]
fn the_reader_decodes_the_same_bytes_as_the_in_memory_decoder() {
    let directory = TempDir::new("same-bytes");
    let (path, bytes) = write_file(&directory, "image.vpng", 200, 140, 32);

    let mut reader = TileReader::open(&path, &limits()).unwrap();
    let document = Document::parse(&bytes, &limits()).unwrap();

    for index in 0..reader.meta().tile_count() as usize {
        let from_disk = reader.decode_tile(index, &limits()).unwrap();
        let from_memory = document.decode_tile(index, &limits()).unwrap();
        assert_eq!(
            from_disk, from_memory,
            "tile {index} decoded differently from the in-memory decoder"
        );
    }
}

#[test]
fn a_forked_reader_shares_the_metadata_but_not_what_it_has_read() {
    let directory = TempDir::new("fork");
    let (path, _) = write_file(&directory, "image.vpng", 128, 96, 32);

    let mut reader = TileReader::open(&path, &limits()).unwrap();
    reader.tile_payload(0).unwrap();

    let mut forked = reader.fork().unwrap();

    // The metadata is the same object, not a copy.
    assert!(std::sync::Arc::ptr_eq(
        &reader.meta_handle(),
        &forked.meta_handle()
    ));

    // The forked handle has read nothing yet: it borrowed the header and
    // the directory rather than reading them again.
    assert_eq!(forked.bytes_read(), 0);
    assert_eq!(forked.payloads_read(), 0);

    // And it can read for itself.
    forked.tile_payload(1).unwrap();
    assert_eq!(forked.payloads_read(), 1);
    assert!(forked.bytes_read() > 0);
}

#[test]
fn a_reader_can_be_moved_to_another_thread() {
    // The parallel decoder gives each worker its own reader. The compiler
    // enforces that a reader may cross a thread boundary.
    fn assert_send<T: Send>() {}
    assert_send::<TileReader>();
    assert_send::<ContainerMeta>();
}

#[test]
fn reopening_picks_up_the_replaced_file() {
    let directory = TempDir::new("reopen");
    let (path, first) = write_file(&directory, "image.vpng", 64, 64, 32);

    let mut reader = TileReader::open(&path, &limits()).unwrap();
    assert_eq!(reader.meta().tile_count(), tile_count_of(&first));

    // Replace the file with a different one.
    let image = gradient(128, 128, PixelFormat::rgba8(), 2);
    let second = encode(&image, &EncodeOptions::default().with_tile_size(32, 32))
        .unwrap()
        .bytes;
    fs::write(&path, &second).unwrap();

    // Before the reopen the reader still describes the old file.
    assert_eq!(reader.meta().dimensions().width, 64);

    reader.reopen().unwrap();

    assert_eq!(reader.meta().dimensions().width, 128);
    assert_eq!(reader.meta().tile_count(), tile_count_of(&second));
}

/// The tile count a file's header declares.
fn tile_count_of(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]])
}

#[test]
fn the_reader_rejects_a_file_that_is_not_a_container() {
    let directory = TempDir::new("not-a-container");
    let path = directory.path("junk.vpng");
    fs::write(&path, b"this is not a VPNG file at all, not even close").unwrap();

    let err = TileReader::open(&path, &limits()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}

#[test]
fn the_reader_and_the_in_memory_decoder_accept_the_same_files() {
    // The reader's claim is that it is the file-backed half of the decoder:
    // a file it accepts is a file the decoder accepts, and a file it rejects
    // is a file the decoder rejects. That claim is checked here over
    // truncations and single-byte corruptions, which reach the header, the
    // directory and the payloads.
    let directory = TempDir::new("same-verdict");
    let (path, bytes) = write_file(&directory, "image.vpng", 128, 96, 32);

    // The unmodified file is accepted by both.
    assert!(TileReader::open(&path, &limits()).is_ok());
    assert!(Document::parse(&bytes, &limits()).is_ok());

    let mut cases: Vec<Vec<u8>> = Vec::new();

    // Truncations: every length that matters, from nothing to nearly all.
    for length in [0usize, 1, 32, 63, 64, 65, 100, 200, 400, 800] {
        if length < bytes.len() {
            cases.push(bytes[..length].to_vec());
        }
    }

    // Single-byte corruptions across the header and the directory, and a
    // few in the payload area.
    for offset in 0..bytes.len().min(700) {
        let mut corrupted = bytes.clone();
        corrupted[offset] ^= 0x01;
        cases.push(corrupted);
    }

    let mut checked = 0;
    for (index, case) in cases.iter().enumerate() {
        let case_path = directory.path(&format!("case{index}.vpng"));
        fs::write(&case_path, case).unwrap();

        let reader = TileReader::open(&case_path, &limits());
        let decoder = Document::parse(case, &limits());

        assert_eq!(
            reader.is_ok(),
            decoder.is_ok(),
            "the reader and the decoder disagree about case {index} (length {})",
            case.len()
        );

        // A file the reader rejects is rejected because of its *content*,
        // not because the disk read ran out — an I/O error would hide a
        // malformed file behind an operating-system complaint.
        if let Err(err) = reader {
            assert!(
                err.is_input_error(),
                "case {index} was rejected with {:?}, which is not an input error",
                err.kind()
            );
        }
        checked += 1;
    }

    assert!(checked > 100, "only {checked} cases were checked");
}

#[test]
fn the_reader_rejects_a_truncated_file() {
    let directory = TempDir::new("truncated");
    let (_path, bytes) = write_file(&directory, "image.vpng", 128, 96, 32);

    // Cut the file in half: the declared size no longer matches.
    let truncated = directory.path("truncated.vpng");
    fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();

    let err = TileReader::open(&truncated, &limits()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}

#[test]
fn the_reader_rejects_a_file_whose_payload_is_corrupt() {
    let directory = TempDir::new("bad-payload");
    let (path, mut bytes) = write_file(&directory, "image.vpng", 128, 96, 32);

    // Flip a byte inside the first tile's payload, leaving the header and
    // the directory intact so that the container still opens.
    let offset = {
        let reader = TileReader::open(&path, &limits()).unwrap();
        let entry = *reader.meta().tile_entry(0).unwrap();
        entry.payload_offset as usize
    };
    bytes[offset] ^= 0xff;
    fs::write(&path, &bytes).unwrap();

    let mut reader = TileReader::open(&path, &limits()).unwrap();
    let err = reader.decode_tile(0, &limits()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}
