//! The tile sources an editing session can work on.
//!
//! A source answers three questions about a file: what its tiles are, how to
//! decode one tile, and how to fetch one tile's compressed payload. The
//! editing engine is written against this interface, so the same editing
//! code serves both a file that is already in memory and a file that is far
//! too large to hold in memory.
//!
//! # The two sources
//!
//! * [`DocumentSource`] wraps [`vpng_decoder::Document`], which borrows a
//!   complete file. It is the fast path for small and medium files.
//! * [`FileSource`] opens a file on disk and reads only what it needs: the
//!   header, the directory, and one tile payload at a time. Its working set
//!   does not grow with the file's size, which is what makes a 16K image
//!   editable on a machine with a few gigabytes of memory.
//!
//! Both sources validate the file exactly the way [`vpng_decoder::Document::parse`]
//! does — same header rules, same directory checksum, same coverage and
//! payload-range checks, same codec-availability check — so a file that one
//! source accepts is accepted by the other. The file-backed source does not
//! implement those rules itself: it uses [`vpng_source::TileReader`], which
//! the interactive viewer shares, so there is one implementation of them
//! rather than two.

use std::path::Path;

use vpng_container::{Directory, DirectoryEntry, Header};
use vpng_core::{Dimensions, Limits, PixelFormat, Result};
use vpng_decoder::Document;
use vpng_source::TileReader;
use vpng_tiling::TileGrid;

/// Everything an editing session needs to know about a file's tiles.
pub trait TileSource {
    /// The file's pixel format.
    fn format(&self) -> PixelFormat;

    /// The image's width in pixels.
    fn width(&self) -> u32;

    /// The image's height in pixels.
    fn height(&self) -> u32;

    /// The image's dimensions.
    fn dimensions(&self) -> Dimensions;

    /// The number of tiles.
    fn tile_count(&self) -> u32;

    /// The tile grid.
    fn grid(&self) -> &TileGrid;

    /// One tile's directory entry.
    fn tile_entry(&self, index: usize) -> Option<&DirectoryEntry>;

    /// The file's main header.
    fn header(&self) -> &Header;

    /// Decode one tile into a fresh buffer.
    fn decode_tile(&mut self, index: usize, limits: &Limits) -> Result<Vec<u8>>;

    /// Fetch one tile's compressed payload.
    fn tile_payload(&mut self, index: usize) -> Result<Vec<u8>>;

    /// Advance the session's baseline after a successful commit.
    ///
    /// The method is called once the commit has written the new file and
    /// replaced the target. A source that can re-read its baseline — the
    /// file-backed source — does so here and returns `true`, so that the
    /// tiles the session calls "unchanged" are the ones the committed file
    /// actually stores.
    ///
    /// A source that cannot re-read its baseline — the borrowed in-memory
    /// source, whose bytes are immutable — returns `false` and keeps its
    /// pre-commit baseline. **That is not an error.** The engine then keeps
    /// the edited tiles in its dirty set, so their pixels are not lost.
    fn advance_baseline(&mut self, target: &Path) -> Result<bool> {
        let _ = target;
        Ok(false)
    }
}

/// A source that borrows a complete file from memory.
#[derive(Debug)]
pub struct DocumentSource<'a> {
    document: Document<'a>,
}

impl<'a> DocumentSource<'a> {
    /// Parse a complete file and wrap it.
    pub fn parse(bytes: &'a [u8], limits: &Limits) -> Result<Self> {
        Ok(DocumentSource {
            document: Document::parse(bytes, limits)?,
        })
    }

    /// The underlying decoder document.
    pub fn document(&self) -> &Document<'a> {
        &self.document
    }
}

impl TileSource for DocumentSource<'_> {
    fn format(&self) -> PixelFormat {
        self.document.format()
    }

    fn width(&self) -> u32 {
        self.document.width()
    }

    fn height(&self) -> u32 {
        self.document.height()
    }

    fn dimensions(&self) -> Dimensions {
        self.document.dimensions()
    }

    fn tile_count(&self) -> u32 {
        self.document.tile_count()
    }

    fn grid(&self) -> &TileGrid {
        self.document.grid()
    }

    fn tile_entry(&self, index: usize) -> Option<&DirectoryEntry> {
        self.document.tile_entry(index)
    }

    fn header(&self) -> &Header {
        self.document.header()
    }

    fn decode_tile(&mut self, index: usize, limits: &Limits) -> Result<Vec<u8>> {
        self.document.decode_tile(index, limits)
    }

    fn tile_payload(&mut self, index: usize) -> Result<Vec<u8>> {
        Ok(self.document.tile_payload(index)?.to_vec())
    }
}

/// A source that reads a file from disk, one tile at a time.
///
/// The file is opened once and kept open. The header and the directory are
/// read into memory when the source is opened; everything else — the tile
/// payloads — is read on demand, so the source's memory is the directory
/// plus one payload.
///
/// The file access itself lives in [`vpng_source::TileReader`], which the
/// interactive viewer shares: there is one implementation of the rules that
/// decide which files are acceptable, not two.
#[derive(Debug)]
pub struct FileSource {
    reader: TileReader,
}

impl FileSource {
    /// Open a file and validate its container.
    ///
    /// The validation is the same set of checks
    /// [`vpng_decoder::Document::parse`] performs, so a file this source
    /// accepts is a file the decoder accepts.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        Ok(FileSource {
            reader: TileReader::open(path, limits)?,
        })
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        self.reader.path()
    }

    /// The file's directory.
    pub fn directory(&self) -> &Directory {
        self.reader.meta().directory()
    }

    /// Whether two paths name the same file.
    ///
    /// The direct comparison covers the common case. The canonical
    /// comparison covers the rest — `./image.vpng` and `image.vpng` are the
    /// same file — and is skipped when either path cannot be canonicalized.
    fn same_file(left: &Path, right: &Path) -> bool {
        if left == right {
            return true;
        }
        match (left.canonicalize(), right.canonicalize()) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }
}

impl TileSource for FileSource {
    fn format(&self) -> PixelFormat {
        self.reader.meta().format()
    }

    fn width(&self) -> u32 {
        self.reader.meta().width()
    }

    fn height(&self) -> u32 {
        self.reader.meta().height()
    }

    fn dimensions(&self) -> Dimensions {
        self.reader.meta().dimensions()
    }

    fn tile_count(&self) -> u32 {
        self.reader.meta().tile_count()
    }

    fn grid(&self) -> &TileGrid {
        self.reader.meta().grid()
    }

    fn tile_entry(&self, index: usize) -> Option<&DirectoryEntry> {
        self.reader.meta().tile_entry(index)
    }

    fn header(&self) -> &Header {
        self.reader.meta().header()
    }

    fn decode_tile(&mut self, index: usize, limits: &Limits) -> Result<Vec<u8>> {
        self.reader.decode_tile(index, limits)
    }

    fn tile_payload(&mut self, index: usize) -> Result<Vec<u8>> {
        self.reader.tile_payload(index)
    }

    fn advance_baseline(&mut self, target: &Path) -> Result<bool> {
        // The baseline only moves when the commit replaced *this* source's
        // own file. A commit to some other path leaves this file alone, so
        // the source keeps its baseline.
        if !Self::same_file(self.reader.path(), target) {
            return Ok(false);
        }

        // The open handle still refers to the file the commit replaced, so
        // the reader re-opens the path and re-reads the header, the
        // directory and the grid.
        self.reader.reopen()?;
        Ok(true)
    }
}
