//! The tile sources an editing session can work on.
//!
//! A source answers three questions about a file: what its tiles are, how to
//! decode one tile, and how to fetch one tile's compressed payload. The
//! editing engine is written against that interface, so the same editing
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
//! source accepts is accepted by the other.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use vpng_container::{Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{Dimensions, Error, Limits, PixelFormat, Result};
use vpng_decoder::{derive_grid, Document};
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
#[derive(Debug)]
pub struct FileSource {
    path: PathBuf,
    file: File,
    header: Header,
    directory: Directory,
    grid: TileGrid,
    format: PixelFormat,
    /// The limits the file was opened with, kept so that the source can
    /// re-open itself after a commit.
    limits: Limits,
}

impl FileSource {
    /// Open a file and validate its container.
    ///
    /// The validation is the same set of checks
    /// [`vpng_decoder::Document::parse`] performs, so a file this source
    /// accepts is a file the decoder accepts.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let mut file = File::open(path)?;
        let file_size = file.metadata()?.len();
        limits.validate_file_size(file_size)?;

        // The header.
        let mut header_bytes = vec![0u8; HEADER_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header_bytes)?;
        let header = Header::parse(&header_bytes)?;

        let dims = header.dimensions()?;
        limits.validate_dimensions(dims, "file")?;
        let format = header.format()?;

        if header.declared_file_size != file_size {
            return Err(Error::corrupt(format!(
                "declared file size {} does not match the {file_size} bytes actually present",
                header.declared_file_size
            )));
        }

        limits.validate_tile_count(header.tile_count)?;

        if header.directory_offset < HEADER_SIZE as u64 {
            return Err(Error::corrupt(
                "directory offset points inside the fixed header",
            ));
        }

        // The directory.
        let directory_size = Directory::total_size(header.tile_count)?;
        let mut directory_bytes = vec![0u8; directory_size];
        file.seek(SeekFrom::Start(header.directory_offset))?;
        file.read_exact(&mut directory_bytes)?;
        let directory = Directory::decode(&directory_bytes, header.tile_count)?;

        if directory.len() != header.tile_count as usize {
            return Err(Error::corrupt(
                "directory entry count disagrees with the header",
            ));
        }

        for (index, entry) in directory.entries.iter().enumerate() {
            limits
                .validate_tile_payload(entry.compressed_length, entry.uncompressed_length as u64)?;
            if !vpng_codecs::is_implemented(entry.codec_id) {
                return Err(Error::unsupported(format!(
                    "tile {index} uses codec id {} ({}), which this build does not implement",
                    entry.codec_id,
                    vpng_codecs::codec_name(entry.codec_id).unwrap_or("unknown")
                )));
            }
        }

        directory.validate_payload_ranges(file_size)?;
        directory.validate_tile_sizes(format.bytes_per_pixel())?;

        let grid = derive_grid(&header, &directory, dims)?;

        Ok(FileSource {
            path: path.to_path_buf(),
            file,
            header,
            directory,
            grid,
            format,
            limits: *limits,
        })
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's directory.
    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    /// Read `len` bytes at `offset` from the file.
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let len = vpng_core::checked_usize(len)?;
        let mut buffer = vec![0u8; len];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buffer)?;
        Ok(buffer)
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
        self.format
    }

    fn width(&self) -> u32 {
        self.header.width
    }

    fn height(&self) -> u32 {
        self.header.height
    }

    fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.header.width,
            height: self.header.height,
        }
    }

    fn tile_count(&self) -> u32 {
        self.header.tile_count
    }

    fn grid(&self) -> &TileGrid {
        &self.grid
    }

    fn tile_entry(&self, index: usize) -> Option<&DirectoryEntry> {
        self.directory.entries.get(index)
    }

    fn header(&self) -> &Header {
        &self.header
    }

    fn decode_tile(&mut self, index: usize, limits: &Limits) -> Result<Vec<u8>> {
        let entry = *self
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let payload = self.read_at(entry.payload_offset, entry.compressed_length)?;

        let computed = vpng_container::crc32c(&payload);
        if computed != entry.payload_checksum {
            return Err(Error::corrupt(format!(
                "tile {index} payload checksum mismatch: stored {:#010x}, computed {computed:#010x}",
                entry.payload_checksum
            )));
        }

        let codec = vpng_codecs::get_codec(entry.codec_id)?;
        let ctx = vpng_codecs::DecodeContext {
            limits: *limits,
            format: self.format,
        };
        let mut out = Vec::new();
        codec.decode(&payload, entry.uncompressed_length as usize, &ctx, &mut out)?;

        if out.len() != entry.uncompressed_length as usize {
            return Err(Error::corrupt(format!(
                "tile {index} decoded to {} bytes, directory declares {}",
                out.len(),
                entry.uncompressed_length
            )));
        }
        Ok(out)
    }

    fn tile_payload(&mut self, index: usize) -> Result<Vec<u8>> {
        let entry = *self
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        self.read_at(entry.payload_offset, entry.compressed_length)
    }

    fn advance_baseline(&mut self, target: &Path) -> Result<bool> {
        // The baseline only moves when the commit replaced *this* source's
        // own file. A commit to some other path leaves this file alone, so
        // the source keeps its baseline.
        if !Self::same_file(&self.path, target) {
            return Ok(false);
        }

        // The open handle still refers to the file the commit replaced, so
        // the source re-opens the path and re-reads the header, the
        // directory and the grid.
        let refreshed = FileSource::open(&self.path, &self.limits)?;
        self.file = refreshed.file;
        self.header = refreshed.header;
        self.directory = refreshed.directory;
        self.grid = refreshed.grid;
        self.format = refreshed.format;
        Ok(true)
    }
}
