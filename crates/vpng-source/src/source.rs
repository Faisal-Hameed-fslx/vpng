//! The byte sources a reader can pull from.
//!
//! The reader's job is to fetch a header, a directory and a tile payload at
//! a time. *Where* those bytes come from is a separate question: a file on
//! disk, an HTTP object store, an in-memory buffer. This module names that
//! question and answers it for the local file and for the buffer.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use vpng_core::{Error, Result};

/// A source of byte ranges.
///
/// The reader never needs the whole object: it needs the header, the
/// directory, and one tile payload at a time. A source says how long the
/// object is and how to fetch a range of it.
///
/// # The contract
///
/// * [`length`](ByteSource::length) is the object's size in bytes, and it
///   must not change while the source is in use.
/// * [`read_at`](ByteSource::read_at) returns **exactly** the requested
///   number of bytes or fails. A short read is an error, never a short
///   buffer: a caller that asked for a payload must never be handed half a
///   payload and have to guess.
/// * [`fork`](ByteSource::fork) opens a second, independent handle on the
///   same object. The forked source has its own read state, so two workers
///   never share a position.
///
/// The source is `Send + Sync`: a pool of workers shares one source and
/// forks from it.
pub trait ByteSource: Send + Sync {
    /// The object's length in bytes.
    fn length(&mut self) -> Result<u64>;

    /// Read exactly `len` bytes starting at `offset`.
    ///
    /// The call must return exactly `len` bytes or fail.
    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>>;

    /// Open a second handle on the same object, with its own read state.
    fn fork(&self) -> Result<Self>
    where
        Self: Sized;

    /// A description of where the bytes come from.
    ///
    /// It appears in error messages, so it should name the object rather
    /// than describe the mechanism.
    fn describe(&self) -> String;
}

/// A local file, read by seeking.
///
/// This is the source the command line and the editing engine use.
#[derive(Debug)]
pub struct FileSource {
    path: PathBuf,
    file: File,
}

impl FileSource {
    /// Open a file for reading.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        Ok(FileSource {
            path: path.to_path_buf(),
            file,
        })
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ByteSource for FileSource {
    fn length(&mut self) -> Result<u64> {
        let metadata = self.file.metadata()?;
        Ok(metadata.len())
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut buffer = vec![0u8; len];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    fn fork(&self) -> Result<Self> {
        // A second handle on the same file, with its own position. The
        // metadata is not re-read: the forked reader borrows it.
        let file = File::open(&self.path)?;
        Ok(FileSource {
            path: self.path.clone(),
            file,
        })
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }
}

/// A source backed by a buffer that is already in memory.
///
/// This is the source the WebAssembly package uses: a browser fetches a
/// container's bytes into the module's linear memory, and the reader pulls
/// its ranges out of them. It is also the source a caller uses when the whole
/// container is in hand — a server that already has the bytes, a test, or a
/// pipeline that decoded a file once and wants to read it many times.
///
/// The buffer is copied into the source, so the source owns its bytes and
/// the reader may borrow from it for as long as it likes.
#[derive(Debug, Clone)]
pub struct MemorySource {
    bytes: Vec<u8>,
}

impl MemorySource {
    /// Take a buffer as the source.
    pub fn new(bytes: Vec<u8>) -> Self {
        MemorySource { bytes }
    }

    /// The source's bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// The number of bytes in the source.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the source is empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl ByteSource for MemorySource {
    fn length(&mut self) -> Result<u64> {
        Ok(self.bytes.len() as u64)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| Error::invalid(format!("the byte range {offset}..{len} overflows")))?;
        let total = self.bytes.len() as u64;
        if end > total {
            return Err(Error::invalid(format!(
                "the range {offset}..{end} is outside the {total} bytes the buffer has"
            )));
        }
        Ok(self.bytes[offset as usize..end as usize].to_vec())
    }

    fn fork(&self) -> Result<Self> {
        Ok(MemorySource {
            bytes: self.bytes.clone(),
        })
    }

    fn describe(&self) -> String {
        format!("{} bytes in memory", self.bytes.len())
    }
}
