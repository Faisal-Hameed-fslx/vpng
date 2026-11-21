//! Exact rectangular partitions of a level.
//!
//! A [`Partition`] is a rectangle together with a list of axis-aligned tile
//! rectangles that **exactly** cover it: no pixel is covered twice, and no
//! pixel is left uncovered. That is the whole contract, and it is what makes
//! a decode deterministic — the pixels of a region do not depend on the order
//! the tiles happen to be visited in.
//!
//! A uniform grid is a partition, and [`TileGrid::partition`] builds one. An
//! *adaptive* partition is one whose tiles have different sizes; it is built
//! from explicit rectangles by [`Partition::from_tiles`], which verifies the
//! exact-cover property before returning.
//!
//! # How exact cover is checked
//!
//! The check is a sweep line. Every tile contributes a start event at its top
//! edge and an end event at its bottom edge. Between two consecutive event
//! heights the set of *active* tiles is constant, and the level is covered
//! exactly on that band if and only if the active tiles' horizontal intervals
//! are pairwise disjoint and their widths add up to the level's width. The
//! sweep maintains the active intervals in a `BTreeMap` keyed by their left
//! edge, so the disjointness test only has to look at the immediate
//! predecessor and successor of a new interval, and the whole check costs
//! `O(n log n)`.
//!
//! The check is deliberately *not* an area comparison. Tiles that overlap can
//! have the same total area as tiles that leave a gap, so the area alone
//! cannot tell a valid partition from an invalid one.

use std::collections::BTreeMap;
use std::ops::Bound;

use vpng_core::{Dimensions, Error, Rect, Result};

use crate::TileGrid;

/// The pixel rectangle of one tile within a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRect {
    /// The tile's left edge in pixels.
    pub x: u32,
    /// The tile's top edge in pixels.
    pub y: u32,
    /// The tile's width in pixels; never zero.
    pub width: u32,
    /// The tile's height in pixels; never zero.
    pub height: u32,
}

impl TileRect {
    /// Build a tile rectangle, rejecting zero sizes and overflow.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::invalid("tile dimensions must be non-zero"));
        }
        let tile = TileRect {
            x,
            y,
            width,
            height,
        };
        tile.right()?;
        tile.bottom()?;
        Ok(tile)
    }

    /// The exclusive right edge of the tile.
    pub fn right(&self) -> Result<u32> {
        self.x
            .checked_add(self.width)
            .ok_or_else(|| Error::invalid("tile x + width overflows"))
    }

    /// The exclusive bottom edge of the tile.
    pub fn bottom(&self) -> Result<u32> {
        self.y
            .checked_add(self.height)
            .ok_or_else(|| Error::invalid("tile y + height overflows"))
    }

    /// The tile's area in pixels.
    pub fn area(&self) -> Result<u64> {
        vpng_core::checked_mul(self.width as u64, self.height as u64)
    }

    /// The tile as the shared geometry rectangle.
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    /// Whether the tile shares at least one pixel with `other`.
    pub fn intersects(&self, other: &TileRect) -> bool {
        let right = self.right().unwrap_or(u32::MAX);
        let bottom = self.bottom().unwrap_or(u32::MAX);
        let other_right = other.right().unwrap_or(u32::MAX);
        let other_bottom = other.bottom().unwrap_or(u32::MAX);
        self.x < other_right && other.x < right && self.y < other_bottom && other.y < bottom
    }

    /// Whether the tile shares at least one pixel with `region`.
    pub fn intersects_rect(&self, region: &Rect) -> bool {
        let right = self.right().unwrap_or(u32::MAX);
        let bottom = self.bottom().unwrap_or(u32::MAX);
        let region_right = region.right().unwrap_or(u32::MAX);
        let region_bottom = region.bottom().unwrap_or(u32::MAX);
        self.x < region_right && region.x < right && self.y < region_bottom && region.y < bottom
    }

    /// Whether the tile contains the pixel `(x, y)`.
    pub fn contains_pixel(&self, x: u32, y: u32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.right().unwrap_or(0)
            && y < self.bottom().unwrap_or(0)
    }
}

/// An exact partition of a rectangle into axis-aligned tiles.
///
/// The tiles keep the order they were given in. A decoder builds a partition
/// from the directory entries of one level, so a tile's position in the
/// partition is also its directory-entry index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    region: Rect,
    tiles: Vec<TileRect>,
}

impl Partition {
    /// Build a partition from explicit tiles and verify that they cover
    /// `region` exactly.
    ///
    /// The tiles are kept in the order given; this function does not reorder
    /// them. A caller that needs the canonical order sorts before calling.
    pub fn from_tiles(region: Rect, tiles: Vec<TileRect>) -> Result<Self> {
        validate_exact_cover(&region, &tiles)?;
        Ok(Partition { region, tiles })
    }

    /// Build the partition of a uniform grid.
    pub fn uniform(grid: &TileGrid) -> Result<Self> {
        let region = grid.region();
        let tiles: Vec<TileRect> = grid
            .iter()
            .map(|geom| TileRect {
                x: geom.x,
                y: geom.y,
                width: geom.width,
                height: geom.height,
            })
            .collect();
        Partition::from_tiles(region, tiles)
    }

    /// The rectangle the partition covers.
    pub fn region(&self) -> Rect {
        self.region
    }

    /// The covered rectangle's size.
    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.region.width,
            height: self.region.height,
        }
    }

    /// The partition's tiles, in order.
    pub fn tiles(&self) -> &[TileRect] {
        &self.tiles
    }

    /// The number of tiles.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Whether the partition has no tiles. It never does: an empty partition
    /// cannot cover a rectangle, and [`Partition::from_tiles`] rejects one.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// One tile by position.
    pub fn tile(&self, index: usize) -> Option<&TileRect> {
        self.tiles.get(index)
    }

    /// The widest tile in the partition.
    pub fn max_tile_width(&self) -> u32 {
        self.tiles.iter().map(|t| t.width).max().unwrap_or(0)
    }

    /// The tallest tile in the partition.
    pub fn max_tile_height(&self) -> u32 {
        self.tiles.iter().map(|t| t.height).max().unwrap_or(0)
    }

    /// The positions of the tiles that intersect `region`, in ascending order.
    ///
    /// The scan is linear in the number of tiles. A level's tile count is
    /// bounded by the caller's limits, and the result is usually a small
    /// fraction of the level, so the scan is not the bottleneck of a region
    /// decode.
    pub fn tiles_for(&self, region: Rect) -> Result<Vec<usize>> {
        if !region.is_within(self.dimensions()) {
            return Err(Error::invalid(
                "region is outside the level the partition covers",
            ));
        }
        let mut out = Vec::new();
        for (index, tile) in self.tiles.iter().enumerate() {
            if tile.intersects_rect(&region) {
                out.push(index);
            }
        }
        Ok(out)
    }

    /// The position of the tile that contains the pixel `(x, y)`.
    pub fn tile_at(&self, x: u32, y: u32) -> Result<usize> {
        self.tiles
            .iter()
            .position(|tile| tile.contains_pixel(x, y))
            .ok_or_else(|| {
                Error::invalid(format!("pixel ({x}, {y}) is not covered by the partition"))
            })
    }

    /// The distinct left edges of the tiles, in ascending order.
    ///
    /// For a uniform grid these are the columns' left edges, so
    /// `column_origins()[column]` is the column's left edge.
    pub fn column_origins(&self) -> Vec<u32> {
        let mut origins: Vec<u32> = self.tiles.iter().map(|tile| tile.x).collect();
        origins.sort_unstable();
        origins.dedup();
        origins
    }

    /// The distinct top edges of the tiles, in ascending order.
    pub fn row_origins(&self) -> Vec<u32> {
        let mut origins: Vec<u32> = self.tiles.iter().map(|tile| tile.y).collect();
        origins.sort_unstable();
        origins.dedup();
        origins
    }

    /// The position of the tile at grid position `(column, row)`.
    ///
    /// The columns and the rows are the distinct origins in ascending order,
    /// which is the same mapping a uniform grid's `index(column, row)`
    /// provides. Two tiles cannot share an origin, because they would
    /// overlap, so the position is unique.
    pub fn tile_at_grid_position(&self, column: u32, row: u32) -> Option<usize> {
        let columns = self.column_origins();
        let rows = self.row_origins();
        let x = *columns.get(column as usize)?;
        let y = *rows.get(row as usize)?;
        self.tiles
            .iter()
            .position(|tile| tile.x == x && tile.y == y)
    }

    /// The nominal tile size, derived from the distinct origins.
    ///
    /// This is the rule ADR-0005 uses to recover a v0.1 grid's tile size: the
    /// nominal width is the distance between the first two distinct left
    /// edges, or the covered width if there is only one; symmetrically for
    /// the height. For a uniform grid it is the grid's tile size.
    ///
    /// The rule is kept unchanged for v0.1 files, whose bytes are pinned by
    /// the golden fixtures. An adaptive partition has no single tile size, and
    /// this rule reports the first gap between two origins, which for one can
    /// be the *smallest* tile in the partition; [`Partition::typical_tile_size`]
    /// is the honest summary for an adaptive partition.
    pub fn nominal_tile_size(&self) -> (u32, u32) {
        let columns = self.column_origins();
        let rows = self.row_origins();
        let width = if columns.len() >= 2 {
            columns[1] - columns[0]
        } else {
            self.region.width
        };
        let height = if rows.len() >= 2 {
            rows[1] - rows[0]
        } else {
            self.region.height
        };
        (width, height)
    }

    /// The most common tile size in the partition.
    ///
    /// For a uniform grid this is the grid's tile size, so it agrees with
    /// [`Partition::nominal_tile_size`] except when the image has a single
    /// column or row of clipped edge tiles. For an adaptive partition it is
    /// the size the partition mostly uses, which is what a reader wants to
    /// know.
    ///
    /// A tie is broken in favour of the **larger** size, so a partition with
    /// one full tile and one clipped edge tile reports the full tile's size.
    pub fn typical_tile_size(&self) -> (u32, u32) {
        (mode_width(&self.tiles), mode_height(&self.tiles))
    }

    /// Whether every tile in the partition has the same size.
    ///
    /// A partition with one tile is uniform by this test: there is nothing to
    /// differ from.
    pub fn is_uniform_size(&self) -> bool {
        let first = match self.tiles.first() {
            Some(tile) => tile,
            None => return false,
        };
        self.tiles
            .iter()
            .all(|tile| tile.width == first.width && tile.height == first.height)
    }
}

/// The most common width among the tiles, ties going to the larger width.
fn mode_width(tiles: &[TileRect]) -> u32 {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for tile in tiles {
        *counts.entry(tile.width).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(width, count)| (count, width))
        .map(|(width, _)| width)
        .unwrap_or(0)
}

/// The most common height among the tiles, ties going to the larger height.
fn mode_height(tiles: &[TileRect]) -> u32 {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for tile in tiles {
        *counts.entry(tile.height).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(height, count)| (count, height))
        .map(|(height, _)| height)
        .unwrap_or(0)
}

/// Verify that `tiles` cover `region` exactly: no gap, no overlap.
///
/// The sweep line is described in the module documentation.
fn validate_exact_cover(region: &Rect, tiles: &[TileRect]) -> Result<()> {
    if tiles.is_empty() {
        return Err(Error::corrupt("a level must contain at least one tile"));
    }

    let region_right = region.right()?;
    let region_bottom = region.bottom()?;

    for (index, tile) in tiles.iter().enumerate() {
        if tile.width == 0 || tile.height == 0 {
            return Err(Error::corrupt(format!(
                "tile {index} has zero width or height"
            )));
        }
        if tile.x < region.x || tile.y < region.y {
            return Err(Error::corrupt(format!(
                "tile {index} starts outside the level it belongs to"
            )));
        }
        if tile.right()? > region_right || tile.bottom()? > region_bottom {
            return Err(Error::corrupt(format!(
                "tile {index} extends outside the level it belongs to"
            )));
        }
    }

    // (y, kind, tile index). The kind orders an end event before a start
    // event at the same height, so a tile that ends exactly where another
    // begins is not mistaken for an overlap.
    let mut events: Vec<(u32, u8, usize)> = Vec::with_capacity(tiles.len() * 2);
    for (index, tile) in tiles.iter().enumerate() {
        events.push((tile.y, 1, index));
        events.push((tile.bottom()?, 0, index));
    }
    events.sort_unstable();

    // The active intervals, keyed by their left edge. Because the intervals
    // are kept disjoint, at most one active interval can start at a given x.
    let mut active: BTreeMap<u32, u32> = BTreeMap::new();
    let mut covered: u64 = 0;
    let mut previous_y = region.y;

    let mut cursor = 0;
    while cursor < events.len() {
        let y = events[cursor].0;

        // The band between the previous event and this one is governed by the
        // active set as it stands now.
        if y > previous_y && covered != region.width as u64 {
            return Err(Error::corrupt(format!(
                "the tiles do not cover the level exactly: the band {previous_y}..{y} is \
                 covered by {covered} of the level's {} pixels",
                region.width
            )));
        }

        while cursor < events.len() && events[cursor].0 == y {
            let (_, kind, index) = events[cursor];
            let tile = tiles[index];
            let right = tile.right()?;

            if kind == 0 {
                match active.remove(&tile.x) {
                    Some(end) if end == right => {}
                    _ => {
                        return Err(Error::corrupt(format!(
                            "tile {index} ends where no tile of the same width began"
                        )))
                    }
                }
                covered -= tile.width as u64;
            } else {
                // The active intervals are disjoint, so only the immediate
                // predecessor and successor of the new interval can overlap it.
                if let Some((_, end)) = active.range(..=tile.x).next_back() {
                    if *end > tile.x {
                        return Err(Error::corrupt(format!(
                            "tile {index} overlaps another tile"
                        )));
                    }
                }
                if let Some((start, _)) = active
                    .range((Bound::Excluded(tile.x), Bound::Unbounded))
                    .next()
                {
                    if *start < right {
                        return Err(Error::corrupt(format!(
                            "tile {index} overlaps another tile"
                        )));
                    }
                }
                active.insert(tile.x, right);
                covered += tile.width as u64;
            }

            cursor += 1;
        }

        previous_y = y;
    }

    if previous_y != region_bottom {
        return Err(Error::corrupt(
            "the tiles do not reach the bottom of the level they belong to",
        ));
    }
    if covered != 0 {
        return Err(Error::corrupt(
            "the tiles extend past the bottom of the level they belong to",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: u32, y: u32, width: u32, height: u32) -> TileRect {
        TileRect::new(x, y, width, height).unwrap()
    }

    #[test]
    fn a_uniform_grid_is_an_exact_partition() {
        let grid = TileGrid::new(64, 64, 300, 180).unwrap();
        let partition = Partition::uniform(&grid).unwrap();
        assert_eq!(partition.len(), grid.tile_count().unwrap() as usize);
        assert_eq!(partition.dimensions().width, 300);
        assert_eq!(partition.dimensions().height, 180);
    }

    #[test]
    fn a_gap_is_rejected() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![tile(0, 0, 50, 100), tile(50, 0, 40, 100)];
        assert!(Partition::from_tiles(region, tiles).is_err());
    }

    #[test]
    fn an_overlap_is_rejected() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![tile(0, 0, 60, 100), tile(50, 0, 50, 100)];
        assert!(Partition::from_tiles(region, tiles).is_err());
    }

    #[test]
    fn a_tile_outside_the_region_is_rejected() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![tile(0, 0, 100, 100), tile(90, 0, 20, 100)];
        assert!(Partition::from_tiles(region, tiles).is_err());
    }

    #[test]
    fn an_empty_partition_is_rejected() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        assert!(Partition::from_tiles(region, Vec::new()).is_err());
    }

    #[test]
    fn a_variable_size_partition_is_accepted() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![
            tile(0, 0, 100, 30),
            tile(0, 30, 40, 70),
            tile(40, 30, 60, 40),
            tile(40, 70, 60, 30),
        ];
        let partition = Partition::from_tiles(region, tiles).unwrap();
        assert_eq!(partition.len(), 4);
        assert_eq!(partition.max_tile_width(), 100);
        assert_eq!(partition.max_tile_height(), 70);
    }

    #[test]
    fn a_vertical_split_is_accepted() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![tile(0, 0, 30, 100), tile(30, 0, 70, 100)];
        assert!(Partition::from_tiles(region, tiles).is_ok());
    }

    #[test]
    fn tiles_for_finds_exactly_the_intersecting_tiles() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![
            tile(0, 0, 100, 30),
            tile(0, 30, 40, 70),
            tile(40, 30, 60, 40),
            tile(40, 70, 60, 30),
        ];
        let partition = Partition::from_tiles(region, tiles).unwrap();

        let query = Rect::new(10, 10, 20, 20).unwrap();
        assert_eq!(partition.tiles_for(query).unwrap(), vec![0]);

        let query = Rect::new(10, 40, 80, 50).unwrap();
        assert_eq!(partition.tiles_for(query).unwrap(), vec![1, 2, 3]);

        let query = Rect::new(0, 0, 100, 100).unwrap();
        assert_eq!(partition.tiles_for(query).unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn tile_at_finds_the_owning_tile() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![
            tile(0, 0, 100, 30),
            tile(0, 30, 40, 70),
            tile(40, 30, 60, 70),
        ];
        let partition = Partition::from_tiles(region, tiles).unwrap();
        assert_eq!(partition.tile_at(0, 0).unwrap(), 0);
        assert_eq!(partition.tile_at(99, 29).unwrap(), 0);
        assert_eq!(partition.tile_at(0, 30).unwrap(), 1);
        assert_eq!(partition.tile_at(99, 99).unwrap(), 2);
    }

    #[test]
    fn a_region_outside_the_partition_is_rejected() {
        let region = Rect::new(0, 0, 100, 100).unwrap();
        let tiles = vec![tile(0, 0, 100, 100)];
        let partition = Partition::from_tiles(region, tiles).unwrap();
        assert!(partition
            .tiles_for(Rect::new(90, 90, 20, 20).unwrap())
            .is_err());
    }

    #[test]
    fn a_partition_that_leaves_a_band_uncovered_is_rejected() {
        // Two tiles that leave the strip between them uncovered.
        let region = Rect::new(0, 0, 10, 100).unwrap();
        let tiles = vec![tile(0, 0, 10, 40), tile(0, 60, 10, 40)];
        assert!(Partition::from_tiles(region, tiles).is_err());
    }

    #[test]
    fn a_partition_that_stops_short_is_rejected() {
        let region = Rect::new(0, 0, 10, 100).unwrap();
        let tiles = vec![tile(0, 0, 10, 90)];
        assert!(Partition::from_tiles(region, tiles).is_err());
    }
}
