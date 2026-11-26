//! The container metadata and the reader over a byte source.

use std::path::Path;
use std::sync::Arc;

use vpng_container::{Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{CanonicalImage, Dimensions, Error, Limits, PixelFormat, Rect, Result};
use vpng_decoder::Level;
use vpng_tiling::Partition;

use crate::source::{ByteSource, FileSource};

/// What a container says about itself: the header, the directory, the tile
/// levels and the pixel format.
///
/// This is the part of a file that is read once and then shared. It never
/// changes while a reader holds it, so it can be handed to as many workers
/// as there are without any synchronisation.
#[derive(Debug, Clone)]
pub struct ContainerMeta {
    header: Header,
    directory: Directory,
    levels: Vec<Level>,
    format: PixelFormat,
}

impl ContainerMeta {
    /// The file's main header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The file's tile directory.
    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    /// The file's levels: the original image first, then any derived previews.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The number of levels in the file.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The original image's level.
    pub fn base_level(&self) -> &Level {
        &self.levels[0]
    }

    /// One level of the file, by its index.
    ///
    /// Level 0 is the original image; every higher level holds a derived
    /// preview. `None` means the file has no such level.
    pub fn level(&self, index: usize) -> Option<&Level> {
        self.levels.get(index)
    }

    /// The original image's exact partition.
    pub fn partition(&self) -> &Partition {
        self.base_level().partition()
    }

    /// The file's pixel format.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The image's dimensions.
    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.header.width,
            height: self.header.height,
        }
    }

    /// The image's width in pixels.
    pub fn width(&self) -> u32 {
        self.header.width
    }

    /// The image's height in pixels.
    pub fn height(&self) -> u32 {
        self.header.height
    }

    /// The number of tiles in the file.
    pub fn tile_count(&self) -> u32 {
        self.header.tile_count
    }

    /// One tile's directory entry.
    pub fn tile_entry(&self, index: usize) -> Option<&DirectoryEntry> {
        self.directory.entries.get(index)
    }

    /// Parse and validate a container's metadata from its header and its
    /// directory.
    ///
    /// The rules are [`vpng_decoder::parse_container_parts`]'s, so this is
    /// the same implementation the in-memory decoder uses and the same one
    /// the WebAssembly package will use. A caller that holds the whole
    /// container uses the same function through [`RangeReader::open_source`].
    ///
    /// `file_size` is the number of bytes the object has. A source that
    /// knows the object's length passes that length; a caller that holds
    /// only the header and the directory — a browser that fetched two byte
    /// ranges — passes the header's declared size, so the payload *ranges*
    /// are checked against the size the container claims for itself.
    pub fn parse(
        header_bytes: &[u8],
        directory_bytes: &[u8],
        file_size: u64,
        limits: &Limits,
    ) -> Result<Self> {
        let header = Header::parse(header_bytes)?;
        Self::from_parts(header, directory_bytes, file_size, limits)
    }

    /// Parse and validate a container's metadata from an already parsed
    /// header and its directory bytes.
    ///
    /// The rules are [`vpng_decoder::parse_container_parts`]'s, so this is
    /// the same implementation the in-memory decoder uses and the same one
    /// the WebAssembly package will use.
    pub fn from_parts(
        header: Header,
        directory_bytes: &[u8],
        file_size: u64,
        limits: &Limits,
    ) -> Result<Self> {
        let parts =
            vpng_decoder::parse_container_parts(header, directory_bytes, file_size, limits)?;

        Ok(ContainerMeta {
            header: *parts.header(),
            directory: parts.directory().clone(),
            levels: parts.levels().to_vec(),
            format: parts.format(),
        })
    }
}

/// A reader over a VPNG container, reading it from a [`ByteSource`].
///
/// The reader owns the source and the parsed container. It reads the header
/// and the directory when it is opened; after that it reads a tile's payload
/// only when one is asked for, and it counts the bytes it has read so a
/// report can state the I/O volume rather than guess at it.
///
/// The source is a parameter, not an assumption: a local file and an HTTP
/// object store are the same reader over a different source. The container
/// is validated **exactly the same way** either way.
#[derive(Debug)]
pub struct RangeReader<S> {
    source: S,
    meta: Arc<ContainerMeta>,
    /// The limits the container was opened with, kept so that the reader can
    /// re-open itself after the object has been replaced.
    limits: Limits,
    /// The bytes read from the source so far, including the header, the
    /// directory and every payload.
    bytes_read: u64,
    /// The number of payloads fetched so far.
    payloads_read: u64,
}

/// The reader the command line and the editing engine use: a container on
/// disk.
pub type TileReader = RangeReader<FileSource>;

impl<S: ByteSource> RangeReader<S> {
    /// Open a source and validate its container.
    ///
    /// The validation is the same set of checks
    /// [`vpng_decoder::Document::parse`] performs, so a container this
    /// reader accepts is one the in-memory decoder accepts.
    pub fn open_source(mut source: S, limits: &Limits) -> Result<Self> {
        let file_size = source.length()?;
        limits.validate_file_size(file_size)?;

        // A container too short to hold a header is corrupt, not a short
        // read. The in-memory decoder says so, and the two must agree on
        // which containers they accept.
        if file_size < HEADER_SIZE as u64 {
            return Err(Error::corrupt(format!(
                "file is {file_size} bytes, shorter than the {HEADER_SIZE}-byte header"
            )));
        }

        // The header.
        let header_bytes = source.read_at(0, HEADER_SIZE)?;
        let header = Header::parse(&header_bytes)?;

        // The directory.
        let directory_size = Directory::total_size(header.tile_count)?;
        vpng_core::checked_range(header.directory_offset, directory_size as u64, file_size)?;
        let directory_bytes = source.read_at(header.directory_offset, directory_size)?;

        // The validation itself is shared with every other reader, so a
        // container this reader accepts is one every other reader accepts.
        let meta = ContainerMeta::from_parts(header, &directory_bytes, file_size, limits)?;

        Ok(RangeReader {
            source,
            meta: Arc::new(meta),
            limits: *limits,
            bytes_read: (HEADER_SIZE as u64) + directory_size as u64,
            payloads_read: 0,
        })
    }

    /// The container's metadata.
    pub fn meta(&self) -> &ContainerMeta {
        &self.meta
    }

    /// The container's metadata as a shared handle.
    ///
    /// A parallel decoder hands the same metadata to every worker, so that
    /// the header and the directory are parsed once for the whole session.
    pub fn meta_handle(&self) -> Arc<ContainerMeta> {
        Arc::clone(&self.meta)
    }

    /// The limits the container was opened with.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The source the reader reads from.
    pub fn source(&self) -> &S {
        &self.source
    }

    /// The source the reader reads from, mutably.
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// A description of where the bytes come from.
    pub fn describe(&self) -> String {
        self.source.describe()
    }

    /// The number of bytes read from the source so far.
    ///
    /// The count starts at the header and the directory — the bytes the
    /// reader had to read to know what the file contains — and grows by
    /// exactly the length of every payload fetched.
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// The number of payloads fetched so far.
    pub fn payloads_read(&self) -> u64 {
        self.payloads_read
    }

    /// Fetch one tile's compressed payload.
    ///
    /// The bytes are read from the file's own range: no read-ahead, and no
    /// part of any other tile.
    pub fn tile_payload(&mut self, index: usize) -> Result<Vec<u8>> {
        let entry = *self
            .meta
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let payload = self.read_at(entry.payload_offset, entry.compressed_length)?;
        self.payloads_read += 1;
        Ok(payload)
    }

    /// Decode one tile into a fresh buffer.
    ///
    /// The payload's CRC32C is verified before the codec sees it, and the
    /// decoded length is checked against the directory's declaration.
    pub fn decode_tile(&mut self, index: usize, limits: &Limits) -> Result<Vec<u8>> {
        let entry = *self
            .meta
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let payload = self.read_at(entry.payload_offset, entry.compressed_length)?;
        self.payloads_read += 1;
        vpng_decoder::decode_tile_payload(index, &payload, &entry, self.meta.format, limits)
    }

    /// Decode a region of the image into a fresh buffer.
    ///
    /// Only the tiles the region intersects are fetched, and only those
    /// payloads are read. The region is taken from the **original** level.
    ///
    /// The assembly is the same code the in-memory decoder uses, so the two
    /// cannot disagree about how a region is put together.
    pub fn decode_region(&mut self, rect: Rect, limits: &Limits) -> Result<CanonicalImage> {
        let level = self.meta.levels.first().cloned().ok_or_else(|| {
            Error::corrupt("the container has no levels, so it has no pixels to decode")
        })?;
        let format = self.meta.format;
        let reader = self;
        vpng_decoder::decode_region_from(&level, format, rect, limits, |index| {
            reader.decode_tile(index, limits)
        })
    }

    /// Open a second handle on the same container, sharing the parsed
    /// metadata.
    ///
    /// This is what a parallel decoder gives each worker: the metadata is
    /// shared, the source's position is not. The forked reader's byte count
    /// starts at zero, because *it* has read nothing yet — it did not read
    /// the header or the directory, it borrowed them.
    pub fn fork(&self) -> Result<Self> {
        Ok(RangeReader {
            source: self.source.fork()?,
            meta: Arc::clone(&self.meta),
            limits: self.limits,
            bytes_read: 0,
            payloads_read: 0,
        })
    }

    /// Read `len` bytes at `offset` from the source.
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let len = vpng_core::checked_usize(len)?;
        let buffer = self.source.read_at(offset, len)?;
        self.bytes_read += buffer.len() as u64;
        Ok(buffer)
    }
}

impl RangeReader<FileSource> {
    /// Open a file and validate its container.
    ///
    /// The validation is the same set of checks
    /// [`vpng_decoder::Document::parse`] performs, so a file this reader
    /// accepts is a file the in-memory decoder accepts.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let source = FileSource::open(path)?;
        RangeReader::open_source(source, limits)
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        self.source.path()
    }

    /// Re-read the container from the file on disk.
    ///
    /// A commit replaces the file, so the header, the directory and every
    /// payload offset change. The reader re-opens the path and re-reads
    /// them. The byte count is cumulative: the bytes the re-read costs are
    /// added to the bytes already read.
    pub fn reopen(&mut self) -> Result<()> {
        let refreshed = RangeReader::open(self.path(), &self.limits)?;
        self.source = refreshed.source;
        self.meta = refreshed.meta;
        self.bytes_read += refreshed.bytes_read;
        Ok(())
    }
}
