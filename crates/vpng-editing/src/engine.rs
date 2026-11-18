//! The editing engine.
//!
//! An editing session holds a source of tiles, a bounded cache of decoded
//! tiles, the tiles the user has changed, and an undo history. It never holds
//! a full-image pixel buffer: a region read allocates exactly the region it
//! returns, and a region write touches exactly the tiles the region
//! intersects.
//!
//! # The dirty set
//!
//! A tile is *dirty* when its pixels differ from the pixels the file stores
//! for it. The engine tracks the dirty tiles explicitly, so a save knows
//! exactly which tiles have to be re-encoded and can copy every other
//! payload verbatim.
//!
//! # Transactions
//!
//! [`Editor::begin`] opens a transaction. Inside it, every edit is recorded.
//! [`Editor::commit`] writes the result to a file and replaces the original
//! atomically; [`Editor::abort`] throws the edits away and restores the
//! document to the state it had when the transaction began. A transaction
//! that fails at any point leaves the original file untouched.
//!
//! # The commit boundary
//!
//! A successful commit establishes a **new baseline**: the committed file is
//! what the session now edits. Undo and redo never cross that boundary, so
//! both history stacks are cleared, the edit counter restarts, the open
//! transaction ends, and the dirty set is reconciled with the committed
//! file.
//!
//! A commit that *fails* changes nothing: the dirty tiles, the undo and redo
//! history, the open transaction, and the edit counter all survive.
//!
//! # Undo and redo
//!
//! The history is tile-level and has a byte budget. See
//! [`crate::history`] for the details.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use vpng_core::{CanonicalImage, Error, Limits, PixelFormat, Rect, Result};

use crate::cache::{CacheStats, TileCache};
use crate::commit::{write_document, CommitOptions, CommitReport};
use crate::history::{History, HistoryEntry, HistoryStats, TileSnapshot};
use crate::source::{DocumentSource, FileSource, TileSource};

/// The default byte budget of the decoded-tile cache: 32 MiB.
pub const DEFAULT_CACHE_BYTES: u64 = 32 * 1024 * 1024;

/// The default byte budget of the undo history: 64 MiB.
pub const DEFAULT_HISTORY_BYTES: u64 = 64 * 1024 * 1024;

/// The state a transaction remembers so that it can be rolled back.
#[derive(Debug)]
struct TransactionState {
    /// The dirty tiles, and their pixels, when the transaction began.
    base: BTreeMap<usize, Vec<u8>>,
    /// The number of undoable steps when the transaction began.
    undo_depth: usize,
}

/// The editing engine.
///
/// The engine is generic over its [`TileSource`], so the same code edits a
/// file that is already in memory and a file that is far too large to hold
/// in memory.
#[derive(Debug)]
pub struct Editor<S: TileSource> {
    source: S,
    limits: Limits,
    format: PixelFormat,
    width: u32,
    height: u32,
    bpp: usize,
    cache: TileCache,
    /// The dirty tiles and their new pixels.
    dirty: BTreeMap<usize, Vec<u8>>,
    history: History,
    transaction: Option<TransactionState>,
    /// The number of edits the session has made since the last commit.
    edits: u64,
}

impl<S: TileSource> Editor<S> {
    /// Open an editing session over a source.
    pub fn new(source: S, limits: &Limits) -> Result<Self> {
        let format = source.format();
        let width = source.width();
        let height = source.height();
        let bpp = format.bytes_per_pixel() as usize;
        Ok(Editor {
            source,
            limits: *limits,
            format,
            width,
            height,
            bpp,
            cache: TileCache::new(DEFAULT_CACHE_BYTES)?,
            dirty: BTreeMap::new(),
            history: History::new(DEFAULT_HISTORY_BYTES)?,
            transaction: None,
            edits: 0,
        })
    }

    /// Set the decoded-tile cache's byte budget.
    pub fn with_cache_bytes(mut self, bytes: u64) -> Result<Self> {
        self.cache = TileCache::new(bytes)?;
        Ok(self)
    }

    /// Set the undo history's byte budget.
    pub fn with_history_bytes(mut self, bytes: u64) -> Result<Self> {
        self.history = History::new(bytes)?;
        Ok(self)
    }

    /// The source the session edits.
    pub fn source(&self) -> &S {
        &self.source
    }

    /// The source the session edits.
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// The file's pixel format.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The image's width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The image's height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The number of tiles.
    pub fn tile_count(&self) -> u32 {
        self.source.tile_count()
    }

    /// The tile grid.
    pub fn grid(&self) -> &vpng_tiling::TileGrid {
        self.source.grid()
    }

    /// The cache's statistics.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    /// The history's statistics.
    pub fn history_stats(&self) -> HistoryStats {
        self.history.stats()
    }

    /// The number of edits the session has made.
    pub fn edit_count(&self) -> u64 {
        self.edits
    }

    /// The indices of the dirty tiles, in ascending order.
    pub fn dirty_tiles(&self) -> Vec<usize> {
        self.dirty.keys().copied().collect()
    }

    /// Whether any tile differs from the file.
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Whether a transaction is open.
    pub fn in_transaction(&self) -> bool {
        self.transaction.is_some()
    }

    /// The bytes the session holds for the dirty tiles.
    pub fn dirty_bytes(&self) -> u64 {
        self.dirty.values().map(|pixels| pixels.len() as u64).sum()
    }

    /// Read a region of the image.
    ///
    /// The returned buffer is exactly the requested rectangle. No tile
    /// outside the rectangle is decoded, and no full-image buffer is ever
    /// allocated.
    pub fn read_region(&mut self, rect: Rect) -> Result<CanonicalImage> {
        let dims = self.source.dimensions();
        if !rect.is_within(dims) {
            return Err(Error::invalid(format!(
                "region {}x{}+{}+{} is outside the {}x{} image",
                rect.width, rect.height, rect.x, rect.y, dims.width, dims.height
            )));
        }
        let region_dims = vpng_core::Dimensions::new(rect.width, rect.height)?;
        self.limits.validate_dimensions(region_dims, "region")?;

        let mut out =
            CanonicalImage::new_zeroed(rect.width, rect.height, self.format, &self.limits)?;
        let out_row = out.row_bytes()? as usize;

        for geometry in self.source.grid().geometries_for_rect(rect)? {
            let index = self.source.grid().index(geometry.col, geometry.row)?;
            let pixels = self.tile_pixels(index)?;
            self.blit(&pixels, &geometry, rect, &mut out, out_row)?;
        }
        Ok(out)
    }

    /// Write a region of the image.
    ///
    /// `pixels` holds the rectangle's rows, tightly packed. Only the tiles
    /// the rectangle intersects are decoded and changed; every other tile
    /// keeps the payload the file stores for it.
    pub fn write_region(&mut self, rect: Rect, pixels: &[u8]) -> Result<()> {
        let dims = self.source.dimensions();
        if !rect.is_within(dims) {
            return Err(Error::invalid(format!(
                "region {}x{}+{}+{} is outside the {}x{} image",
                rect.width, rect.height, rect.x, rect.y, dims.width, dims.height
            )));
        }
        let expected = rect.width as usize * rect.height as usize * self.bpp;
        if pixels.len() != expected {
            return Err(Error::invalid(format!(
                "region holds {} bytes, expected {expected}",
                pixels.len()
            )));
        }

        let geometries = self.source.grid().geometries_for_rect(rect)?;
        let mut snapshots: Vec<TileSnapshot> = Vec::with_capacity(geometries.len());

        for geometry in geometries {
            let index = self.source.grid().index(geometry.col, geometry.row)?;
            let was_clean = !self.dirty.contains_key(&index);

            // The tile's pixels before this edit.
            let previous = self.tile_pixels(index)?;
            snapshots.push(TileSnapshot {
                index,
                pixels: previous.clone(),
                file_clean: was_clean,
            });

            // Patch the tile.
            let mut tile = previous;
            self.patch_tile(&mut tile, &geometry, rect, pixels)?;

            self.dirty.insert(index, tile);
        }

        if !snapshots.is_empty() {
            self.history.push(HistoryEntry {
                previous: snapshots,
                next: Vec::new(),
            });
            self.edits += 1;
        }
        Ok(())
    }

    /// Undo the newest edit.
    ///
    /// Returns `false` when there is nothing to undo.
    pub fn undo(&mut self) -> Result<bool> {
        let Some(entry) = self.history.pop_undo() else {
            return Ok(false);
        };
        let mut next = Vec::with_capacity(entry.previous.len());
        for snapshot in &entry.previous {
            let current = self.tile_pixels(snapshot.index)?;
            next.push(TileSnapshot {
                index: snapshot.index,
                pixels: current,
                file_clean: false,
            });
            if snapshot.file_clean {
                // The tile is back to the pixels the file stores, so it no
                // longer needs a new payload.
                self.dirty.remove(&snapshot.index);
            } else {
                self.dirty.insert(snapshot.index, snapshot.pixels.clone());
            }
        }
        self.history.push_redo(HistoryEntry {
            previous: entry.previous,
            next,
        });
        self.edits += 1;
        Ok(true)
    }

    /// Redo the newest undone edit.
    ///
    /// Returns `false` when there is nothing to redo.
    pub fn redo(&mut self) -> Result<bool> {
        let Some(entry) = self.history.pop_redo() else {
            return Ok(false);
        };
        let mut previous = Vec::with_capacity(entry.next.len());
        for snapshot in &entry.next {
            let current = self.tile_pixels(snapshot.index)?;
            previous.push(TileSnapshot {
                index: snapshot.index,
                pixels: current,
                file_clean: false,
            });
            if snapshot.file_clean {
                self.dirty.remove(&snapshot.index);
            } else {
                self.dirty.insert(snapshot.index, snapshot.pixels.clone());
            }
        }
        self.history.push(HistoryEntry {
            previous,
            next: entry.next,
        });
        self.edits += 1;
        Ok(true)
    }

    /// Open a transaction.
    ///
    /// The document's current state is remembered so that
    /// [`Editor::abort`] can restore it.
    pub fn begin(&mut self) -> Result<()> {
        if self.transaction.is_some() {
            return Err(Error::invalid(
                "a transaction is already open; commit or abort it first",
            ));
        }
        self.transaction = Some(TransactionState {
            base: self.dirty.clone(),
            undo_depth: self.history.undo_depth(),
        });
        Ok(())
    }

    /// Throw the transaction's edits away.
    ///
    /// The dirty tiles return to the state they had when the transaction
    /// began, and the undo history is cut back to the same point.
    pub fn abort(&mut self) -> Result<()> {
        let Some(state) = self.transaction.take() else {
            return Err(Error::invalid("no transaction is open"));
        };
        self.dirty = state.base;
        while self.history.undo_depth() > state.undo_depth {
            self.history.pop_undo();
        }
        self.history.clear_redo();
        Ok(())
    }

    /// Write the document to `target` and replace it atomically.
    ///
    /// A successful commit establishes a **new baseline**: the committed file
    /// is what the session now edits, so the undo and redo history cannot
    /// cross the commit. Both history stacks are cleared, the edit counter
    /// restarts, an open transaction ends, and the dirty set is reconciled
    /// with the new baseline.
    ///
    /// A commit that **fails** changes nothing at all: the dirty tiles, the
    /// undo and redo history, and the open transaction all survive, so the
    /// user can try again or undo the edits.
    pub fn commit(&mut self, target: &Path, options: &CommitOptions) -> Result<CommitReport> {
        let header = *self.source.header();
        let grid = *self.source.grid();
        let mut report = write_document(
            &mut self.source,
            &self.dirty,
            &self.limits,
            self.format,
            self.width,
            self.height,
            &grid,
            &header,
            target,
            options,
        )?;

        // The file has been written and renamed into place, so the commit
        // succeeded. The baseline moves only when the source can re-read the
        // file that was just written.
        //
        // A source that cannot re-read its baseline keeps the old one. That
        // is *not* a failed commit — the file on disk is correct — so the
        // error is not propagated. The dirty set is kept in that case, so
        // the edit is not lost by a second save.
        let baseline_advanced = self.source.advance_baseline(target).unwrap_or(false);
        report.baseline_advanced = baseline_advanced;

        // The commit ends the history sequence: the steps that reached the
        // baseline are not undoable steps of the sequence that starts after
        // it.
        self.history.start_new_sequence();
        self.edits = 0;
        self.transaction = None;
        self.reconcile_after_commit(baseline_advanced);
        Ok(report)
    }

    /// Reconcile the dirty set with the committed baseline.
    ///
    /// When the baseline advanced, the committed file stores exactly the
    /// pixels the session holds, so the dirty set is empty again. The cache
    /// may still hold the *pre-edit* decode of a rewritten tile; those
    /// entries are replaced with the pixels the commit wrote, so a later
    /// read cannot see the old pixels.
    ///
    /// When the baseline did not advance, the dirty set is kept: the source
    /// still stores the old pixels, and dropping the set would silently
    /// revert the edit on the next save.
    fn reconcile_after_commit(&mut self, baseline_advanced: bool) {
        if !baseline_advanced {
            return;
        }
        let dirty = std::mem::take(&mut self.dirty);
        for (index, pixels) in dirty {
            // Only a tile the cache already holds needs replacing: a tile
            // that was never decoded is read from the new baseline when it
            // is needed, and caching it now would grow the working set for
            // nothing.
            if self.cache.contains(index) {
                self.cache.insert(index, pixels);
            }
        }
    }

    /// The pixels of one tile, as the session currently sees them.
    ///
    /// A dirty tile comes from the dirty set; a clean tile comes from the
    /// cache, or is decoded and cached.
    ///
    /// The method hands back an owned copy. A region read and a region write
    /// both copy the tile's bytes anyway, and returning an owned value keeps
    /// the engine from holding a borrow of the cache while it works on
    /// another part of the session.
    fn tile_pixels(&mut self, index: usize) -> Result<Vec<u8>> {
        if let Some(pixels) = self.dirty.get(&index) {
            return Ok(pixels.clone());
        }
        self.ensure_cached(index)?;
        Ok(self
            .cache
            .peek(index)
            .expect("the tile was just cached")
            .to_vec())
    }

    /// Make sure a clean tile is decoded and held in the cache.
    fn ensure_cached(&mut self, index: usize) -> Result<()> {
        if self.cache.note_access(index) {
            return Ok(());
        }
        let started = Instant::now();
        let pixels = self.source.decode_tile(index, &self.limits)?;
        self.cache.record_decode(started.elapsed());
        self.cache.insert(index, pixels);
        Ok(())
    }

    /// Copy the part of a tile that a rectangle covers into the tile.
    fn patch_tile(
        &self,
        tile: &mut [u8],
        geometry: &vpng_tiling::TileGeometry,
        rect: Rect,
        source: &[u8],
    ) -> Result<()> {
        let ix0 = geometry.x.max(rect.x);
        let iy0 = geometry.y.max(rect.y);
        let ix1 = (geometry.x + geometry.width).min(rect.right()?);
        let iy1 = (geometry.y + geometry.height).min(rect.bottom()?);

        let tile_row = geometry.width as usize * self.bpp;
        let source_row = rect.width as usize * self.bpp;

        for y in iy0..iy1 {
            let tile_offset =
                (y - geometry.y) as usize * tile_row + (ix0 - geometry.x) as usize * self.bpp;
            let source_offset =
                (y - rect.y) as usize * source_row + (ix0 - rect.x) as usize * self.bpp;
            let length = (ix1 - ix0) as usize * self.bpp;
            tile[tile_offset..tile_offset + length]
                .copy_from_slice(&source[source_offset..source_offset + length]);
        }
        Ok(())
    }

    /// Copy the part of a tile that a rectangle covers into the output.
    fn blit(
        &self,
        tile: &[u8],
        geometry: &vpng_tiling::TileGeometry,
        rect: Rect,
        out: &mut CanonicalImage,
        out_row: usize,
    ) -> Result<()> {
        let ix0 = geometry.x.max(rect.x);
        let iy0 = geometry.y.max(rect.y);
        let ix1 = (geometry.x + geometry.width).min(rect.right()?);
        let iy1 = (geometry.y + geometry.height).min(rect.bottom()?);

        let tile_row = geometry.width as usize * self.bpp;
        for y in iy0..iy1 {
            let source_offset =
                (y - geometry.y) as usize * tile_row + (ix0 - geometry.x) as usize * self.bpp;
            let destination = (y - rect.y) as usize * out_row + (ix0 - rect.x) as usize * self.bpp;
            let length = (ix1 - ix0) as usize * self.bpp;
            out.data_mut()[destination..destination + length]
                .copy_from_slice(&tile[source_offset..source_offset + length]);
        }
        Ok(())
    }
}

/// An editable document backed by a file that is already in memory.
pub type EditableDocument<'a> = Editor<DocumentSource<'a>>;

impl<'a> Editor<DocumentSource<'a>> {
    /// Parse a complete file and open an editing session over it.
    ///
    /// The session borrows the bytes; the caller keeps them alive for as
    /// long as the session lasts. The decoder's own
    /// [`vpng_decoder::Document::parse`] API is unchanged.
    pub fn parse(bytes: &'a [u8], limits: &Limits) -> Result<Self> {
        let source = DocumentSource::parse(bytes, limits)?;
        Editor::new(source, limits)
    }
}

/// An editing session backed by a file on disk.
///
/// The session holds only the header, the directory, the dirty tiles and the
/// cache, so its memory does not grow with the file's size.
#[derive(Debug)]
pub struct FileEditor {
    inner: Editor<FileSource>,
}

impl FileEditor {
    /// Open a file for editing.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let source = FileSource::open(path, limits)?;
        Ok(FileEditor {
            inner: Editor::new(source, limits)?,
        })
    }

    /// Set the decoded-tile cache's byte budget.
    pub fn with_cache_bytes(mut self, bytes: u64) -> Result<Self> {
        self.inner = self.inner.with_cache_bytes(bytes)?;
        Ok(self)
    }

    /// Set the undo history's byte budget.
    pub fn with_history_bytes(mut self, bytes: u64) -> Result<Self> {
        self.inner = self.inner.with_history_bytes(bytes)?;
        Ok(self)
    }

    /// The editing session.
    pub fn editor(&mut self) -> &mut Editor<FileSource> {
        &mut self.inner
    }

    /// The editing session.
    pub fn editor_ref(&self) -> &Editor<FileSource> {
        &self.inner
    }
}
