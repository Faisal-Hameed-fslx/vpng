//! The source-backed decoder.
//!
//! A worker decodes tiles from a byte source: a file on disk today, an HTTP
//! object store tomorrow. Each worker gets its own [`RangeReader`], forked
//! from one that has already parsed the header and the directory: the
//! metadata is parsed once and shared, while the source's position — the
//! part that cannot be shared — is private to the worker.
//!
//! This is the decoder the parallel engine uses in practice. It is the same
//! code path as the ordinary decoder: the payload's CRC32C is verified,
//! the codec is the one the directory names, and the decoded length is
//! checked against the directory's declaration. The pool changes *when* a
//! tile is decoded, never *what* it decodes.

use std::path::Path;

use vpng_core::{Limits, Result};
use vpng_source::{ByteSource, FileSource, RangeReader};

use crate::driver::TileDecoder;
use crate::pool::DecoderFactory;

/// A decoder that reads tiles from a byte source.
#[derive(Debug)]
pub struct SourceDecoder<S> {
    reader: RangeReader<S>,
    limits: Limits,
}

impl<S: ByteSource> SourceDecoder<S> {
    /// The reader behind this decoder.
    pub fn reader(&self) -> &RangeReader<S> {
        &self.reader
    }

    /// The number of bytes this decoder has read from the source.
    pub fn bytes_read(&self) -> u64 {
        self.reader.bytes_read()
    }
}

impl<S: ByteSource> TileDecoder for SourceDecoder<S> {
    fn decode(&mut self, tile: usize) -> Result<Vec<u8>> {
        self.reader.decode_tile(tile, &self.limits)
    }

    fn bytes_read(&self) -> u64 {
        self.reader.bytes_read()
    }
}

/// Makes one [`SourceDecoder`] per worker.
///
/// The factory opens the source once, so the header and the directory are
/// parsed once for the whole session; every worker then forks its own
/// reader from it.
#[derive(Debug)]
pub struct SourceDecoderFactory<S> {
    reader: RangeReader<S>,
    limits: Limits,
}

impl<S: ByteSource> SourceDecoderFactory<S> {
    /// Open a source and keep it open as the source for every worker.
    pub fn open_source(source: S, limits: &Limits) -> Result<Self> {
        let reader = RangeReader::open_source(source, limits)?;
        Ok(SourceDecoderFactory {
            reader,
            limits: *limits,
        })
    }

    /// Build a factory from a reader that is already open.
    pub fn from_reader(reader: RangeReader<S>) -> Self {
        let limits = *reader.limits();
        SourceDecoderFactory { reader, limits }
    }

    /// The container's metadata.
    pub fn meta(&self) -> &vpng_source::ContainerMeta {
        self.reader.meta()
    }

    /// The reader the workers fork from.
    pub fn reader(&self) -> &RangeReader<S> {
        &self.reader
    }
}

impl<S: ByteSource + 'static> DecoderFactory for SourceDecoderFactory<S> {
    type Decoder = SourceDecoder<S>;

    fn make_decoder(&self, _generation: crate::scheduler::Generation) -> Result<Self::Decoder> {
        Ok(SourceDecoder {
            reader: self.reader.fork()?,
            limits: self.limits,
        })
    }
}

/// A decoder that reads tiles from a file on disk.
pub type FileDecoder = SourceDecoder<FileSource>;

/// Makes one [`FileDecoder`] per worker.
///
/// The factory opens the file once, so the header and the directory are
/// parsed once for the whole session; every worker then forks its own
/// reader from it.
pub type FileDecoderFactory = SourceDecoderFactory<FileSource>;

impl FileDecoderFactory {
    /// Open a file and keep it open as the source for every worker.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let reader = RangeReader::open(path, limits)?;
        Ok(FileDecoderFactory::from_reader(reader))
    }

    /// The file the workers read from.
    pub fn path(&self) -> &Path {
        self.reader.path()
    }
}
