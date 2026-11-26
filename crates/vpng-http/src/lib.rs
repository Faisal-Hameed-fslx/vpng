//! `vpng-http` — read a VPNG container over HTTP Range requests.
//!
//! The format already supports this. Every tile payload has its own offset
//! and length in the directory, so a reader over the network can fetch the
//! 64-byte header, the directory, and then exactly the payloads it needs.
//! **No format change is needed, and none is made.**
//!
//! # How it fits together
//!
//! [`vpng_source::ByteSource`] is where the bytes come from: a file on disk
//! today, an HTTP object store here. [`vpng_source::RangeReader`] is the
//! reader over a source: it validates the container, fetches a tile's
//! payload when one is asked for, and counts the bytes it has read.
//!
//! The two halves are deliberately separate. The container validation, the
//! codec dispatch and the region assembly are **the same code** for a local
//! file and for a remote object; only the byte-fetching differs. A file
//! fetched over HTTP is validated by exactly the rules a local file is.
//!
//! # The rules the source enforces
//!
//! * A range request asks for exactly the bytes the reader wants. A server
//!   that ignores the `Range` header and replies `200 OK` with the whole
//!   object is refused, not silently accepted.
//! * The `Content-Range` header of every `206 Partial Content` reply is
//!   checked against the range that was asked for.
//! * A short body is an error, never a short buffer.
//!
//! # Using it
//!
//! ```no_run
//! use vpng_core::Limits;
//! use vpng_http::{HttpSource, HttpSourceOptions};
//! use vpng_source::{ByteSource, RangeReader};
//!
//! let source = HttpSource::open(
//!     "https://example.invalid/picture.vpng",
//!     HttpSourceOptions::default(),
//! )?;
//! let mut reader = RangeReader::open_source(source, &Limits::default())?;
//! let pixels = reader.decode_tile(0, &Limits::default())?;
//! # Ok::<(), vpng_core::Error>(())
//! ```

#![forbid(unsafe_code)]

mod source;
#[cfg(any(test, feature = "test-support"))]
pub mod test_server;
#[cfg(test)]
mod tests;

pub use source::{HttpSource, HttpSourceInfo, HttpSourceOptions};

use vpng_source::RangeReader;

/// A reader over a VPNG container fetched over HTTP.
pub type HttpReader = RangeReader<HttpSource>;
