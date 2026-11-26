//! `vpng-source` — random access to a VPNG container.
//!
//! A viewer of a very large image must not read the whole file. It reads
//! the header once, the directory once, and then the payload of the tiles it
//! actually needs — nothing else. This crate is that file half: it owns a
//! handle on the container, the parsed metadata, and the accounting that says
//! how much was read.
//!
//! # The three types
//!
//! * [`ContainerMeta`] is what the file *says*: the header, the directory,
//!   the tile grid and the pixel format. It is immutable and cheap to share.
//! * [`ByteSource`] is where the bytes come from: a file on disk today, an
//!   HTTP object store tomorrow, a buffer in memory. A source says how long
//!   the object is and how to fetch a range of it.
//! * [`RangeReader`] is the reader over a source: it can fetch a tile's
//!   payload, decode it, and count the bytes it has read. [`TileReader`] is
//!   the reader over a local file, which is what the command line and the
//!   editing engine use.
//!
//! # Why the metadata is separate
//!
//! A parallel decoder gives each worker its own handle on the source,
//! because a position is mutable state and a shared position would serialise
//! the workers. The *metadata* does not have to be shared that way: it is
//! read once, parsed once, and handed to every worker as an immutable
//! [`ContainerMeta`]. [`RangeReader::fork`] opens exactly that arrangement —
//! a second handle on the same object, with the metadata already parsed.
//!
//! # What the reader guarantees
//!
//! * It reads the header and the directory once, when it is opened, and
//!   nothing else until a payload is asked for.
//! * It reads exactly the bytes of the payload requested — no read-ahead, no
//!   whole-file slurp — and reports the total in
//!   [`RangeReader::bytes_read`].
//! * It validates the container through
//!   [`vpng_decoder::parse_container_parts`], the **one** implementation of
//!   the rules that decide which containers are acceptable. The in-memory
//!   decoder, this reader and the WebAssembly package all call it, so a
//!   file one of them accepts is a file all of them accept.
//!
//! The binary format is untouched by this crate: it reads what is there.

#![forbid(unsafe_code)]

mod reader;
mod source;
#[cfg(test)]
mod tests;

pub use reader::{ContainerMeta, RangeReader, TileReader};
pub use source::{ByteSource, FileSource, MemorySource};
