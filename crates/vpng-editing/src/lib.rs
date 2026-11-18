//! `vpng-editing` — the VPNG editing engine.
//!
//! The engine edits a `.vpng` file without ever holding the whole image in
//! memory and without rewriting the tiles it did not touch.
//!
//! # The pieces
//!
//! * [`TileCache`] — a bounded, least-recently-used cache of decoded tiles.
//!   The budget is in bytes, so the memory a session holds is predictable
//!   whatever the tile size is.
//! * [`Editor`] — the editing session: region reads and writes, dirty-tile
//!   tracking, transactions, and undo and redo.
//! * [`CommitOptions`] and [`CommitReport`] — the copy-on-write save. The
//!   new file is written to a temporary file and then replaces the original
//!   atomically, so a failure leaves the original untouched.
//!
//! # The guarantees
//!
//! * **No full-image buffer.** A region read allocates exactly the region it
//!   returns. A region write decodes exactly the tiles the region
//!   intersects.
//! * **Untouched tiles keep their bytes.** A save copies every payload the
//!   edit did not change byte for byte, and re-encodes only the dirty ones.
//! * **The original survives a failure.** The save never writes to the
//!   original file. It writes a temporary file and renames it over the
//!   original in one step.
//! * **The format does not change.** An edited file is an ordinary VPNG
//!   file: the same header, the same directory, the same payloads. The
//!   editing engine adds no chunk, no field and no version.
//!
//! # An example
//!
//! ```no_run
//! use std::path::Path;
//! use vpng_core::{Limits, Rect};
//! use vpng_editing::{CommitOptions, FileEditor};
//!
//! # fn main() -> vpng_core::Result<()> {
//! let limits = Limits::default();
//! let mut editor = FileEditor::open(Path::new("image.vpng"), &limits)?;
//!
//! editor.editor().begin()?;
//! let region = editor.editor().read_region(Rect::new(10, 10, 32, 32)?)?;
//! // ... change `region` ...
//! editor.editor().write_region(Rect::new(10, 10, 32, 32)?, region.data())?;
//! editor
//!     .editor()
//!     .commit(Path::new("image.vpng"), &CommitOptions::default())?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod cache;
pub mod commit;
pub mod engine;
pub mod history;
pub mod source;

#[cfg(test)]
mod tests;

pub use cache::{CacheStats, TileCache};
pub use commit::{CommitOptions, CommitReport, FailurePoint};
pub use engine::{
    EditableDocument, Editor, FileEditor, DEFAULT_CACHE_BYTES, DEFAULT_HISTORY_BYTES,
};
pub use history::{History, HistoryEntry, HistoryStats, TileSnapshot};
pub use source::{DocumentSource, FileSource, TileSource};
