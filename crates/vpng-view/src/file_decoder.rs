//! The file-backed decoder.
//!
//! A worker decodes tiles from a file on disk. Each worker gets its own
//! [`TileReader`], forked from one that has already parsed the header and
//! the directory: the metadata is parsed once and shared, while the file
//! position — the part that cannot be shared — is private to the worker.
//!
//! This is the decoder the parallel engine uses in practice. It is the same
//! code path as the ordinary decoder: the payload's CRC32C is verified,
//! the codec is the one the directory names, and the decoded length is
//! checked against the directory's declaration. The pool changes *when* a
//! tile is decoded, never *what* it decodes.

use std::path::{Path, PathBuf};

use vpng_core::{Limits, Result};
use vpng_source::TileReader;

use crate::driver::TileDecoder;
use crate::pool::DecoderFactory;

/// A decoder that reads tiles from a file on disk.
#[derive(Debug)]
pub struct FileDecoder {
    reader: TileReader,
    limits: Limits,
}

impl FileDecoder {
    /// The reader behind this decoder.
    pub fn reader(&self) -> &TileReader {
        &self.reader
    }

    /// The number of bytes this decoder has read from the file.
    pub fn bytes_read(&self) -> u64 {
        self.reader.bytes_read()
    }
}

impl TileDecoder for FileDecoder {
    fn decode(&mut self, tile: usize) -> Result<Vec<u8>> {
        self.reader.decode_tile(tile, &self.limits)
    }
}

/// Makes one [`FileDecoder`] per worker.
///
/// The factory opens the file once, so the header and the directory are
/// parsed once for the whole session; every worker then forks its own
/// reader from it.
#[derive(Debug)]
pub struct FileDecoderFactory {
    reader: TileReader,
    limits: Limits,
    path: PathBuf,
}

impl FileDecoderFactory {
    /// Open a file and keep it open as the source for every worker.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let reader = TileReader::open(path, limits)?;
        Ok(FileDecoderFactory {
            reader,
            limits: *limits,
            path: path.to_path_buf(),
        })
    }

    /// Build a factory from a reader that is already open.
    pub fn from_reader(reader: TileReader) -> Self {
        let path = reader.path().to_path_buf();
        let limits = *reader.limits();
        FileDecoderFactory {
            reader,
            limits,
            path,
        }
    }

    /// The file the workers read from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The container's metadata.
    pub fn meta(&self) -> &vpng_source::ContainerMeta {
        self.reader.meta()
    }
}

impl DecoderFactory for FileDecoderFactory {
    type Decoder = FileDecoder;

    fn make_decoder(&self, _generation: crate::scheduler::Generation) -> Result<Self::Decoder> {
        Ok(FileDecoder {
            reader: self.reader.fork()?,
            limits: self.limits,
        })
    }
}
