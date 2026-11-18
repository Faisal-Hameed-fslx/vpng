//! `vpng-source` — random access to a VPNG file on disk.
//!
//! A viewer of a very large image must not read the whole file. It reads
//! the header once, the directory once, and then the payload of the tiles it
//! actually needs — nothing else. This crate is that file half: it owns a
//! handle on the file, the parsed container, and the accounting that says
//! how much was read.
//!
//! # The two types
//!
//! * [`ContainerMeta`] is what the file *says*: the header, the directory,
//!   the tile grid and the pixel format. It is immutable and cheap to share.
//! * [`TileReader`] is a handle on the file: it can fetch a tile's payload,
//!   decode it, and count the bytes it has read.
//!
//! # Why the metadata is separate
//!
//! A parallel decoder gives each worker its own file handle, because a file
//! position is mutable state and a shared position would serialise the
//! workers. The *metadata* does not have to be shared that way: it is read
//! once, parsed once, and handed to every worker as an immutable
//! [`ContainerMeta`]. [`TileReader::fork`] opens exactly that arrangement —
//! a second handle on the same file, with the metadata already parsed.
//!
//! # What the reader guarantees
//!
//! * It reads the header and the directory once, when it is opened, and
//!   nothing else until a payload is asked for.
//! * It reads exactly the bytes of the payload requested — no read-ahead, no
//!   whole-file slurp — and reports the total in [`TileReader::bytes_read`].
//! * It validates the container exactly the way
//!   [`vpng_decoder::Document::parse`] does, so a file this reader accepts
//!   is a file the in-memory decoder accepts.
//!
//! The binary format is untouched by this crate: it reads what is there.

#![forbid(unsafe_code)]

mod reader;
#[cfg(test)]
mod tests;

pub use reader::{ContainerMeta, TileReader};
