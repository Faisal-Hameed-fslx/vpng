//! The tile-level undo and redo history.
//!
//! An editing session records what each edit changed, at **tile**
//! granularity: an edit that touches four tiles records the four tiles' pixel
//! buffers as they were *before* the edit. Undo puts those buffers back;
//! redo re-applies the edited buffers.
//!
//! # The memory budget
//!
//! History is the part of an editing session that grows without bound, so it
//! has an explicit budget in bytes. When the recorded history would exceed
//! the budget, the oldest entries are dropped. Dropping an undo entry
//! invalidates the redo chain, because a redo needs the matching undo to
//! have been recorded — so the redo stack is cleared when the undo stack is
//! trimmed.
//!
//! The budget covers the pixel buffers only. The small per-entry bookkeeping
//! (the tile indices and the entry headers) is counted too, so the reported
//! figure is an honest upper bound on the history's own memory rather than
//! just the bulk of it.
//!
//! # Granularity
//!
//! The history is tile-level by design. A finer granularity would record
//! smaller pieces of an edit and would let undo restore a single pixel; a
//! coarser one would restore whole images. Tiles are the unit the format
//! already stores, so tiles are the unit the history stores, and a saved file
//! can never disagree with the undo stack about what a tile contains.

use std::collections::VecDeque;

use vpng_core::{Error, Result};

/// The bytes one history entry spends on its own bookkeeping, beyond the
/// pixel buffers it holds.
///
/// It covers the tile index list and the entry header. The constant is a
/// deliberate over-estimate, so the reported history size is never an
/// under-estimate.
const ENTRY_OVERHEAD_BYTES: u64 = 64;

/// One tile's pixels, as they were before or after an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSnapshot {
    /// The tile's index in the document's directory.
    pub index: usize,
    /// The tile's decoded pixels.
    pub pixels: Vec<u8>,
    /// Whether these pixels are identical to the pixels the file stores for
    /// this tile.
    ///
    /// The flag lets undo and redo decide whether a tile still has to be
    /// rewritten. A tile whose pixels match the file's does not need a new
    /// payload, and an edit engine that rewrote it anyway would break the
    /// promise that untouched tiles are copied verbatim.
    pub file_clean: bool,
}

/// One undoable step.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// The tiles the step touched, with their **previous** pixels.
    pub previous: Vec<TileSnapshot>,
    /// The tiles the step touched, with their **new** pixels.
    ///
    /// The new pixels are filled in when the step is undone, so only one copy
    /// of a tile's pixels is retained at a time.
    pub next: Vec<TileSnapshot>,
}

impl HistoryEntry {
    /// The bytes this entry occupies.
    pub fn bytes(&self) -> u64 {
        let pixels: u64 = self
            .previous
            .iter()
            .chain(self.next.iter())
            .map(|snapshot| snapshot.pixels.len() as u64)
            .sum();
        pixels + ENTRY_OVERHEAD_BYTES
    }
}

/// The statistics a history reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryStats {
    /// The number of undoable steps.
    pub undo_depth: usize,
    /// The number of redoable steps.
    pub redo_depth: usize,
    /// The bytes the history currently holds.
    pub current_bytes: u64,
    /// The highest byte total the history ever held.
    pub peak_bytes: u64,
    /// The number of steps dropped to stay inside the budget.
    pub dropped_entries: u64,
}

/// The undo and redo stacks, bounded by a byte budget.
#[derive(Debug)]
pub struct History {
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    max_bytes: u64,
    current_bytes: u64,
    peak_bytes: u64,
    dropped_entries: u64,
}

impl History {
    /// A history with a byte budget.
    ///
    /// A budget of zero is legal: nothing is recorded, so nothing can be
    /// undone. A negative budget is not.
    pub fn new(max_bytes: u64) -> Result<Self> {
        if max_bytes > (1u64 << 40) {
            return Err(Error::invalid(format!(
                "history budget {max_bytes} bytes is unreasonably large"
            )));
        }
        Ok(History {
            undo: VecDeque::new(),
            redo: Vec::new(),
            max_bytes,
            current_bytes: 0,
            peak_bytes: 0,
            dropped_entries: 0,
        })
    }

    /// The history's byte budget.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Whether an undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The number of undoable steps.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// The number of redoable steps.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Record a step.
    ///
    /// Recording a step invalidates the redo chain: the new branch of the
    /// history replaces the old one.
    ///
    /// The step is trimmed against the budget *before* the peak is
    /// recorded, so the reported peak is the high-water mark of the bytes
    /// the history **retains**. A step too large for the budget is dropped
    /// at once and counted in `dropped_entries`; the caller's buffers are
    /// freed with it.
    pub fn push(&mut self, entry: HistoryEntry) {
        self.clear_redo();
        let size = entry.bytes();
        self.undo.push_back(entry);
        self.current_bytes += size;
        self.trim();
        self.peak_bytes = self.peak_bytes.max(self.current_bytes);
    }

    /// Take the newest undoable step.
    ///
    /// The caller is responsible for restoring the step's previous pixels and
    /// for filling in the step's next pixels before calling [`History::push_redo`].
    pub fn pop_undo(&mut self) -> Option<HistoryEntry> {
        let entry = self.undo.pop_back()?;
        self.current_bytes -= entry.bytes();
        Some(entry)
    }

    /// Put a step on the redo stack.
    ///
    /// As with [`History::push`], the step is trimmed before the peak is
    /// recorded.
    pub fn push_redo(&mut self, entry: HistoryEntry) {
        let size = entry.bytes();
        self.redo.push(entry);
        self.current_bytes += size;
        self.trim();
        self.peak_bytes = self.peak_bytes.max(self.current_bytes);
    }

    /// Take the newest redoable step.
    pub fn pop_redo(&mut self) -> Option<HistoryEntry> {
        let entry = self.redo.pop()?;
        self.current_bytes -= entry.bytes();
        Some(entry)
    }

    /// Forget the redo chain.
    pub fn clear_redo(&mut self) {
        for entry in self.redo.drain(..) {
            self.current_bytes -= entry.bytes();
        }
    }

    /// Forget every step.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.current_bytes = 0;
    }

    /// Forget every step and start a new sequence.
    ///
    /// The method clears both stacks **and** resets the statistics, so the
    /// peak and the dropped-entry count describe the new sequence rather
    /// than the one that just ended.
    ///
    /// A commit uses it. The committed file is the new baseline, and the
    /// history that led to it cannot be undone across that boundary — so
    /// the steps that reached the baseline are not undoable steps of the
    /// sequence that starts after it.
    pub fn start_new_sequence(&mut self) {
        self.clear();
        self.peak_bytes = 0;
        self.dropped_entries = 0;
    }

    /// The history's statistics.
    pub fn stats(&self) -> HistoryStats {
        HistoryStats {
            undo_depth: self.undo.len(),
            redo_depth: self.redo.len(),
            current_bytes: self.current_bytes,
            peak_bytes: self.peak_bytes,
            dropped_entries: self.dropped_entries,
        }
    }

    /// Drop the oldest steps until the history fits its budget.
    fn trim(&mut self) {
        while self.current_bytes > self.max_bytes {
            // The redo stack is trimmed first: it is the cheaper history to
            // lose, and dropping it keeps the undo chain intact.
            if let Some(entry) = self.redo.pop() {
                self.current_bytes -= entry.bytes();
                self.dropped_entries += 1;
                continue;
            }
            match self.undo.pop_front() {
                Some(entry) => {
                    self.current_bytes -= entry.bytes();
                    self.dropped_entries += 1;
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(indices: &[usize], size: usize) -> HistoryEntry {
        HistoryEntry {
            previous: indices
                .iter()
                .map(|&index| TileSnapshot {
                    index,
                    pixels: vec![0u8; size],
                    file_clean: false,
                })
                .collect(),
            next: Vec::new(),
        }
    }

    #[test]
    fn a_fresh_history_is_empty() {
        let history = History::new(1024).unwrap();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.stats().undo_depth, 0);
    }

    #[test]
    fn a_step_can_be_pushed_and_taken() {
        let mut history = History::new(1024).unwrap();
        history.push(entry(&[0, 1], 10));
        assert!(history.can_undo());
        assert_eq!(history.undo_depth(), 1);
        let taken = history.pop_undo().unwrap();
        assert_eq!(taken.previous.len(), 2);
        assert_eq!(taken.previous[1].index, 1);
        assert!(!history.can_undo());
    }

    #[test]
    fn pushing_a_step_clears_the_redo_chain() {
        let mut history = History::new(4096).unwrap();
        history.push(entry(&[0], 10));
        let taken = history.pop_undo().unwrap();
        history.push_redo(taken);
        assert!(history.can_redo());
        history.push(entry(&[1], 10));
        assert!(!history.can_redo());
    }

    #[test]
    fn the_budget_drops_the_oldest_steps() {
        // Each entry costs 20 bytes of pixels plus 64 bytes of overhead.
        let mut history = History::new(200).unwrap();
        for index in 0..10 {
            history.push(entry(&[index], 20));
        }
        assert!(history.stats().current_bytes <= 200);
        assert!(history.stats().dropped_entries > 0);
        assert!(history.undo_depth() < 10);
    }

    #[test]
    fn the_peak_is_remembered() {
        let mut history = History::new(1000).unwrap();
        history.push(entry(&[0], 100));
        assert_eq!(history.stats().current_bytes, 164);
        assert_eq!(history.stats().peak_bytes, 164);
    }

    #[test]
    fn the_peak_never_exceeds_the_budget() {
        // One entry costs 164 bytes. A budget of 200 keeps one entry.
        let mut history = History::new(200).unwrap();
        for index in 0..10 {
            history.push(entry(&[index], 100));
        }
        assert!(history.stats().peak_bytes <= 200);
        assert!(history.stats().current_bytes <= 200);
        assert!(history.stats().dropped_entries > 0);
    }

    #[test]
    fn a_step_too_large_for_the_budget_is_dropped_at_once() {
        let mut history = History::new(100).unwrap();
        history.push(entry(&[0], 100));
        assert_eq!(history.undo_depth(), 0);
        assert_eq!(history.stats().current_bytes, 0);
        assert_eq!(history.stats().peak_bytes, 0);
        assert_eq!(history.stats().dropped_entries, 1);
    }

    #[test]
    fn a_zero_budget_records_nothing() {
        let mut history = History::new(0).unwrap();
        history.push(entry(&[0], 100));
        assert_eq!(history.undo_depth(), 0);
        assert!(history.stats().dropped_entries > 0);
    }

    #[test]
    fn clearing_resets_the_bytes() {
        let mut history = History::new(4096).unwrap();
        history.push(entry(&[0], 100));
        history.clear();
        assert_eq!(history.stats().current_bytes, 0);
        assert_eq!(history.undo_depth(), 0);
    }

    #[test]
    fn a_huge_budget_is_rejected() {
        assert!(History::new(1u64 << 50).is_err());
    }
}
