//! The bounded tile cache.
//!
//! An editing session decodes tiles on demand. Without a bound, a long
//! session would accumulate every tile it ever touched, and the working set
//! would grow with the session rather than with the edit. The cache therefore
//! has a **byte budget**, not a tile count: the caller says how much decoded
//! pixel memory the session may hold, and the cache evicts the least recently
//! used tiles until the next insert fits.
//!
//! # Why bytes and not tiles
//!
//! Tiles are not the same size. A 256x256 RGBA tile is 256 KiB; a 16x16
//! grayscale tile is 256 bytes. A tile-count budget would let a session hold
//! either a handful of large tiles or thousands of tiny ones, and the memory
//! would not be predictable. A byte budget makes the memory bound explicit,
//! which is what the 16K validation needs.
//!
//! # Eviction
//!
//! The cache is a strict LRU. A hit moves the tile to the most-recently-used
//! end; an insert evicts from the least-recently-used end until the new tile
//! fits. A tile that is larger than the whole budget is **not** cached: it is
//! counted as rejected and returned to the caller, which keeps the budget
//! honest instead of silently exceeding it.
//!
//! # Accounting
//!
//! The cache reports hits, misses, evictions, the number of tiles it decoded,
//! the tiles it refused to cache, the bytes it currently holds, the highest
//! byte total it ever reached, and the time spent decoding. The byte totals
//! count the decoded pixel buffers only; the decoded tile's own payload is
//! not part of the cache.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use vpng_core::{Error, Result};

/// The statistics a cache reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    /// Lookups that found the tile in the cache.
    pub hits: u64,
    /// Lookups that did not.
    pub misses: u64,
    /// Tiles dropped to make room for another.
    pub evictions: u64,
    /// Tiles decoded by the caller through this cache.
    pub decoded_tiles: u64,
    /// Tiles the cache refused to hold because they exceed the whole budget.
    pub rejected_tiles: u64,
    /// Tiles currently resident.
    pub resident_tiles: u64,
    /// Decoded bytes currently resident.
    pub current_bytes: u64,
    /// The highest number of decoded bytes the cache ever held.
    pub peak_bytes: u64,
    /// The time spent decoding tiles.
    pub decode_time: Duration,
}

impl CacheStats {
    /// The hit rate, or `None` when nothing was looked up.
    pub fn hit_rate(&self) -> Option<f64> {
        let total = self.hits + self.misses;
        if total == 0 {
            None
        } else {
            Some(self.hits as f64 / total as f64)
        }
    }

    /// The decode time in milliseconds.
    pub fn decode_time_ms(&self) -> f64 {
        self.decode_time.as_secs_f64() * 1000.0
    }
}

/// A bounded, least-recently-used cache of decoded tiles.
#[derive(Debug)]
pub struct TileCache {
    capacity_bytes: u64,
    entries: HashMap<usize, Vec<u8>>,
    /// Tile indices, least recently used first.
    order: VecDeque<usize>,
    current_bytes: u64,
    peak_bytes: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    decoded_tiles: u64,
    rejected_tiles: u64,
    decode_time: Duration,
}

impl TileCache {
    /// A cache with a byte budget.
    ///
    /// A budget of zero is legal: the cache then holds nothing and only
    /// counts. A negative budget is not.
    pub fn new(capacity_bytes: u64) -> Result<Self> {
        if capacity_bytes > (1u64 << 40) {
            return Err(Error::invalid(format!(
                "tile cache budget {capacity_bytes} bytes is unreasonably large"
            )));
        }
        Ok(TileCache {
            capacity_bytes,
            entries: HashMap::new(),
            order: VecDeque::new(),
            current_bytes: 0,
            peak_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            decoded_tiles: 0,
            rejected_tiles: 0,
            decode_time: Duration::ZERO,
        })
    }

    /// The cache's byte budget.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    /// The number of tiles currently resident.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no tiles.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look a tile up.
    ///
    /// A hit moves the tile to the most-recently-used end. A miss counts the
    /// miss and leaves the cache untouched.
    pub fn get(&mut self, index: usize) -> Option<&[u8]> {
        if !self.entries.contains_key(&index) {
            self.misses += 1;
            return None;
        }
        self.hits += 1;
        self.touch(index);
        self.entries.get(&index).map(|bytes| bytes.as_slice())
    }

    /// Look a tile up without changing the recency order.
    pub fn peek(&self, index: usize) -> Option<&[u8]> {
        self.entries.get(&index).map(|bytes| bytes.as_slice())
    }

    /// Whether the cache holds a tile, without changing the recency order.
    pub fn contains(&self, index: usize) -> bool {
        self.entries.contains_key(&index)
    }

    /// Record an access to a tile and report whether it was resident.
    ///
    /// The method counts the hit or the miss and moves a resident tile to the
    /// most-recently-used end, but it does not hand the tile back. A caller
    /// that needs the bytes can fetch them with [`TileCache::peek`], which
    /// keeps the borrow of the cache short.
    pub fn note_access(&mut self, index: usize) -> bool {
        if self.entries.contains_key(&index) {
            self.hits += 1;
            self.touch(index);
            true
        } else {
            self.misses += 1;
            false
        }
    }

    /// Put a decoded tile into the cache.
    ///
    /// Tiles larger than the whole budget are rejected: the cache hands the
    /// bytes back to the caller instead of exceeding its bound.
    pub fn insert(&mut self, index: usize, pixels: Vec<u8>) {
        let size = pixels.len() as u64;
        if size > self.capacity_bytes {
            self.rejected_tiles += 1;
            // A tile that was already resident must still leave the cache.
            self.remove(index);
            return;
        }
        self.remove(index);
        while self.current_bytes + size > self.capacity_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(bytes) = self.entries.remove(&oldest) {
                self.current_bytes -= bytes.len() as u64;
                self.evictions += 1;
            }
        }
        self.current_bytes += size;
        self.entries.insert(index, pixels);
        self.order.push_back(index);
        self.peak_bytes = self.peak_bytes.max(self.current_bytes);
    }

    /// Drop one tile from the cache.
    pub fn remove(&mut self, index: usize) -> bool {
        let Some(bytes) = self.entries.remove(&index) else {
            return false;
        };
        self.current_bytes -= bytes.len() as u64;
        if let Some(position) = self.order.iter().position(|&entry| entry == index) {
            self.order.remove(position);
        }
        true
    }

    /// Drop every tile, keeping the statistics.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.current_bytes = 0;
    }

    /// Record that a tile was decoded, and how long the decode took.
    pub fn record_decode(&mut self, elapsed: Duration) {
        self.decoded_tiles += 1;
        self.decode_time += elapsed;
    }

    /// Reset the counters, keeping the resident tiles.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.evictions = 0;
        self.decoded_tiles = 0;
        self.rejected_tiles = 0;
        self.peak_bytes = self.current_bytes;
        self.decode_time = Duration::ZERO;
    }

    /// The cache's statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            decoded_tiles: self.decoded_tiles,
            rejected_tiles: self.rejected_tiles,
            resident_tiles: self.entries.len() as u64,
            current_bytes: self.current_bytes,
            peak_bytes: self.peak_bytes,
            decode_time: self.decode_time,
        }
    }

    /// Move a tile to the most-recently-used end.
    fn touch(&mut self, index: usize) {
        if let Some(position) = self.order.iter().position(|&entry| entry == index) {
            self.order.remove(position);
        }
        self.order.push_back(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(byte: u8, size: usize) -> Vec<u8> {
        vec![byte; size]
    }

    #[test]
    fn a_fresh_cache_is_empty() {
        let cache = TileCache::new(1024).unwrap();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().hits, 0);
    }

    #[test]
    fn a_miss_is_counted_and_a_hit_is_not() {
        let mut cache = TileCache::new(1024).unwrap();
        cache.insert(0, tile(1, 16));
        assert!(cache.get(0).is_some());
        assert!(cache.get(1).is_none());
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn the_cache_never_exceeds_its_budget() {
        let mut cache = TileCache::new(100).unwrap();
        for index in 0..10 {
            cache.insert(index, tile(index as u8, 30));
        }
        assert!(cache.stats().current_bytes <= 100);
        assert_eq!(cache.stats().current_bytes, 90);
    }

    #[test]
    fn the_least_recently_used_tile_is_evicted_first() {
        let mut cache = TileCache::new(100).unwrap();
        cache.insert(0, tile(0, 30));
        cache.insert(1, tile(1, 30));
        cache.insert(2, tile(2, 30));
        // Touch 0 so that 1 becomes the least recently used.
        assert!(cache.get(0).is_some());
        cache.insert(3, tile(3, 30));
        assert!(cache.peek(0).is_some());
        assert!(cache.peek(1).is_none());
        assert!(cache.peek(2).is_some());
        assert!(cache.peek(3).is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn a_tile_larger_than_the_budget_is_rejected_not_stored() {
        let mut cache = TileCache::new(64).unwrap();
        cache.insert(0, tile(0, 100));
        assert!(cache.is_empty());
        assert_eq!(cache.stats().rejected_tiles, 1);
        assert_eq!(cache.stats().current_bytes, 0);
    }

    #[test]
    fn reinserting_a_tile_does_not_double_count_it() {
        let mut cache = TileCache::new(100).unwrap();
        cache.insert(0, tile(0, 40));
        cache.insert(0, tile(0, 40));
        assert_eq!(cache.stats().current_bytes, 40);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_peak_is_remembered_after_an_eviction() {
        let mut cache = TileCache::new(100).unwrap();
        cache.insert(0, tile(0, 60));
        cache.insert(1, tile(1, 60));
        assert_eq!(cache.stats().current_bytes, 60);
        assert_eq!(cache.stats().peak_bytes, 60);
    }

    #[test]
    fn removing_a_tile_frees_its_bytes() {
        let mut cache = TileCache::new(100).unwrap();
        cache.insert(0, tile(0, 40));
        assert!(cache.remove(0));
        assert_eq!(cache.stats().current_bytes, 0);
        assert!(!cache.remove(0));
    }

    #[test]
    fn the_decode_time_is_accumulated() {
        let mut cache = TileCache::new(100).unwrap();
        cache.record_decode(Duration::from_millis(3));
        cache.record_decode(Duration::from_millis(4));
        assert_eq!(cache.stats().decoded_tiles, 2);
        assert_eq!(cache.stats().decode_time_ms(), 7.0);
    }

    #[test]
    fn a_zero_budget_caches_nothing() {
        let mut cache = TileCache::new(0).unwrap();
        cache.insert(0, tile(0, 1));
        assert!(cache.is_empty());
        assert_eq!(cache.stats().rejected_tiles, 1);
    }

    #[test]
    fn a_huge_budget_is_rejected() {
        assert!(TileCache::new(1u64 << 50).is_err());
    }
}
