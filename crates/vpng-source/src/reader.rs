//! The container metadata and the file reader.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use vpng_container::{Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{Dimensions, Error, Limits, PixelFormat, Result};
use vpng_decoder::derive_grid;
use vpng_tiling::TileGrid;

/// What a container says about itself: the header, the directory, the tile
/// grid and the pixel format.
///
/// This is the part of a file that is read once and then shared. It never
/// changes while a reader holds it, so it can be handed to as many workers
/// as there are without any synchronisation.
#[derive(Debug, Clone)]
pub struct ContainerMeta {
    header: Header,
    directory: Directory,
    grid: TileGrid,
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

    /// The file's tile grid.
    pub fn grid(&self) -> &TileGrid {
        &self.grid
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
}

/// A reader over a VPNG file on disk.
///
/// The reader owns a file handle and the parsed container. It reads the
/// header and the directory when it is opened; after that it reads a tile's
/// payload only when one is asked for, and it counts the bytes it has read
/// so a report can state the I/O volume rather than guess at it.
#[derive(Debug)]
pub struct TileReader {
    path: PathBuf,
    file: File,
    meta: Arc<ContainerMeta>,
    /// The limits the file was opened with, kept so that the reader can
    /// re-open itself after the file has been replaced.
    limits: Limits,
    /// The bytes read from the file so far, including the header, the
    /// directory and every payload.
    bytes_read: u64,
    /// The number of payloads fetched so far.
    payloads_read: u64,
}

impl TileReader {
    /// Open a file and validate its container.
    ///
    /// The validation is the same set of checks
    /// [`vpng_decoder::Document::parse`] performs, so a file this reader
    /// accepts is a file the in-memory decoder accepts.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let mut file = File::open(path)?;
        let file_size = file.metadata()?.len();
        limits.validate_file_size(file_size)?;

        // A file too short to hold a header is corrupt, not a short read.
        // The in-memory decoder says so, and the two must agree on which
        // files they accept.
        if file_size < HEADER_SIZE as u64 {
            return Err(Error::corrupt(format!(
                "file is {file_size} bytes, shorter than the {HEADER_SIZE}-byte header"
            )));
        }

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
        vpng_core::checked_range(header.directory_offset, directory_size as u64, file_size)?;
        let mut directory_bytes = vec![0u8; directory_size];
        file.seek(SeekFrom::Start(header.directory_offset))?;
        file.read_exact(&mut directory_bytes)?;
        let directory = Directory::decode(&directory_bytes, header.tile_count)?;

        if directory.len() != header.tile_count as usize {
            return Err(Error::corrupt(
                "directory entry count disagrees with the header",
            ));
        }

        let directory_end = vpng_core::checked_add(header.directory_offset, directory_size as u64)?;

        for (index, entry) in directory.entries.iter().enumerate() {
            if entry.payload_offset < directory_end {
                return Err(Error::corrupt(format!(
                    "tile {index} payload begins before the end of the tile directory"
                )));
            }
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

        let meta = ContainerMeta {
            header,
            directory,
            grid,
            format,
        };

        Ok(TileReader {
            path: path.to_path_buf(),
            file,
            meta: Arc::new(meta),
            limits: *limits,
            bytes_read: (HEADER_SIZE as u64) + directory_size as u64,
            payloads_read: 0,
        })
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
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

    /// The limits the file was opened with.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The number of bytes read from the file so far.
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
            format: self.meta.format,
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

    /// Open a second handle on the same file, sharing the parsed metadata.
    ///
    /// This is what a parallel decoder gives each worker: the metadata is
    /// shared, the file position is not. The forked reader's byte count
    /// starts at zero, because *it* has read nothing yet — it did not read
    /// the header or the directory, it borrowed them.
    pub fn fork(&self) -> Result<Self> {
        let file = File::open(&self.path)?;
        Ok(TileReader {
            path: self.path.clone(),
            file,
            meta: Arc::clone(&self.meta),
            limits: self.limits,
            bytes_read: 0,
            payloads_read: 0,
        })
    }

    /// Re-read the container from the file on disk.
    ///
    /// A commit replaces the file, so the header, the directory and every
    /// payload offset change. The reader re-opens the path and re-reads
    /// them. The byte count is cumulative: the bytes the re-read costs are
    /// added to the bytes already read.
    pub fn reopen(&mut self) -> Result<()> {
        let refreshed = TileReader::open(&self.path, &self.limits)?;
        self.file = refreshed.file;
        self.meta = refreshed.meta;
        self.bytes_read += refreshed.bytes_read;
        Ok(())
    }

    /// Read `len` bytes at `offset` from the file.
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let len = vpng_core::checked_usize(len)?;
        let mut buffer = vec![0u8; len];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buffer)?;
        self.bytes_read += buffer.len() as u64;
        Ok(buffer)
    }
}
