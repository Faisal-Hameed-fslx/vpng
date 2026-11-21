//! The levels of a container: the original image and its derived previews.
//!
//! A **level** is one grid of tiles. Level 0 is the original image and holds
//! original pixels. Every other level holds *derived preview* pixels: a
//! downsampled copy that is useful to show quickly and is never used to
//! reconstruct the original.
//!
//! The levels of a file are derived from its directory, the same way a v0.1
//! reader derives the tile grid from it (ADR-0005). The rules are:
//!
//! * The entries are in canonical `(level, y, x)` order.
//! * Each level's entries are an **exact partition** of that level's
//!   rectangle: every pixel is covered once, and no pixel twice.
//! * Level 0's rectangle is the image itself, so its dimensions are the
//!   header's width and height.
//! * A derived level's rectangle starts at the origin and is strictly smaller
//!   than the level above it in both dimensions.
//!
//! A v0.1 file has exactly one level and it is a uniform grid, which is what
//! ADR-0005 requires. A v0.2 file may have any number of levels, and the
//! tiles of a level may have different sizes.

use vpng_container::{Directory, Header, MINOR_V0_2};
use vpng_core::{Dimensions, Error, Rect, Result};
use vpng_tiling::{Partition, TileRect};

use crate::derive_grid;

/// One level of the preview pyramid.
#[derive(Debug, Clone)]
pub struct Level {
    index: u16,
    derived: bool,
    dimensions: Dimensions,
    start: usize,
    count: usize,
    partition: Partition,
}

impl Level {
    /// The level's index. Level 0 is the original image.
    pub fn index(&self) -> u16 {
        self.index
    }

    /// Whether the level holds derived preview pixels rather than original ones.
    pub fn is_derived(&self) -> bool {
        self.derived
    }

    /// The level's dimensions.
    pub fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    /// The directory-entry index of the level's first tile.
    pub fn first_entry(&self) -> usize {
        self.start
    }

    /// The number of tiles in the level.
    pub fn tile_count(&self) -> usize {
        self.count
    }

    /// The level's exact partition.
    pub fn partition(&self) -> &Partition {
        &self.partition
    }

    /// The directory-entry index of one of the level's tiles.
    ///
    /// The position is the tile's index within the level; the result is the
    /// tile's index in the file's directory.
    pub fn entry_index(&self, position: usize) -> Option<usize> {
        if position < self.count {
            Some(self.start + position)
        } else {
            None
        }
    }
}

/// Derive a file's levels from its header and directory.
///
/// This is the one implementation of the rules, shared by the in-memory
/// decoder and the file-backed reader, so the two cannot disagree about what
/// a level is.
pub fn derive_levels(
    header: &Header,
    directory: &Directory,
    dims: Dimensions,
) -> Result<Vec<Level>> {
    if directory.entries.is_empty() {
        return Err(Error::corrupt("the tile directory is empty"));
    }

    if header.minor < MINOR_V0_2 {
        return derive_v0_1_levels(header, directory, dims);
    }

    let entries = &directory.entries;

    // The entries are in canonical `(level, y, x)` order. A file whose
    // entries are out of order is corrupt: a reader must be able to find a
    // level's tiles without scanning the whole directory, and the order is
    // what makes that possible.
    for index in 1..entries.len() {
        let previous = (
            entries[index - 1].level(),
            entries[index - 1].y,
            entries[index - 1].x,
        );
        let current = (entries[index].level(), entries[index].y, entries[index].x);
        if current <= previous {
            return Err(Error::corrupt(format!(
                "tile entries are not in canonical (level, y, x) order: \
                 entry {index} ({}, {}, {}) does not follow ({}, {}, {})",
                current.0, current.1, current.2, previous.0, previous.1, previous.2
            )));
        }
    }

    let mut levels: Vec<Level> = Vec::new();
    let mut start = 0usize;
    while start < entries.len() {
        let index = entries[start].level();
        if index == 0 && !levels.is_empty() {
            // The levels come in order, so level 0 can only be the first one.
            return Err(Error::corrupt(
                "the directory holds a second level 0; the levels are not in order",
            ));
        }
        let mut end = start;
        while end < entries.len() && entries[end].level() == index {
            end += 1;
        }

        let tiles: Vec<TileRect> = entries[start..end]
            .iter()
            .map(|entry| TileRect {
                x: entry.x,
                y: entry.y,
                width: entry.width as u32,
                height: entry.height as u32,
            })
            .collect();

        let region = bounding_region(&tiles)?;
        let partition = Partition::from_tiles(region, tiles)?;

        if index == 0 {
            if region.width != dims.width || region.height != dims.height {
                return Err(Error::corrupt(format!(
                    "the original level covers {}x{} but the header declares {}x{}",
                    region.width, region.height, dims.width, dims.height
                )));
            }
        } else {
            let previous = levels
                .last()
                .ok_or_else(|| Error::corrupt("a derived level has no level below it"))?;
            if region.width >= previous.dimensions.width
                || region.height >= previous.dimensions.height
            {
                return Err(Error::corrupt(format!(
                    "derived level {index} is {}x{}, which is not smaller than the {}x{} level below it",
                    region.width,
                    region.height,
                    previous.dimensions.width,
                    previous.dimensions.height
                )));
            }
        }

        levels.push(Level {
            index,
            derived: index != 0,
            dimensions: Dimensions {
                width: region.width,
                height: region.height,
            },
            start,
            count: end - start,
            partition,
        });

        start = end;
    }

    // The feature flag and the levels must agree: a file that says it has a
    // pyramid has one, and a file that has one says so.
    let has_pyramid = levels.len() > 1;
    if header.has_pyramid() != has_pyramid {
        return Err(Error::corrupt(format!(
            "the header's pyramid flag disagrees with the directory, which holds {} level(s)",
            levels.len()
        )));
    }

    Ok(levels)
}

/// Derive the single uniform level of a v0.1 file.
///
/// v0.1 requires one uniform grid (ADR-0005), so the existing grid
/// derivation is the rule, and the partition is that grid.
fn derive_v0_1_levels(
    header: &Header,
    directory: &Directory,
    dims: Dimensions,
) -> Result<Vec<Level>> {
    let grid = derive_grid(header, directory, dims)?;
    let count = grid.tile_count()? as usize;
    let partition = Partition::uniform(&grid)?;
    Ok(vec![Level {
        index: 0,
        derived: false,
        dimensions: dims,
        start: 0,
        count,
        partition,
    }])
}

/// The smallest rectangle that contains every tile, anchored at the origin.
fn bounding_region(tiles: &[TileRect]) -> Result<Rect> {
    let mut width = 0u32;
    let mut height = 0u32;
    for tile in tiles {
        width = width.max(tile.right()?);
        height = height.max(tile.bottom()?);
    }
    Rect::new(0, 0, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_container::{DirectoryEntry, Header};
    use vpng_core::PixelFormat;

    fn entry(level: u16, x: u32, y: u32, width: u16, height: u16) -> DirectoryEntry {
        DirectoryEntry {
            x,
            y,
            width,
            height,
            codec_id: 0,
            predictor_id: 0,
            flags: level,
            payload_offset: 0,
            compressed_length: 0,
            uncompressed_length: width as u32 * height as u32,
            payload_checksum: 0,
        }
    }

    fn v2_header(levels: u16, tile_count: u32) -> Header {
        Header::new_v2(100, 100, PixelFormat::rgba8(), 0, tile_count, 64, 4096)
            .unwrap()
            .with_feature_flags(if levels > 1 {
                vpng_container::FEATURE_PYRAMID
            } else {
                0
            })
    }

    #[test]
    fn a_single_level_v2_file_is_one_partition() {
        let directory = Directory::new(vec![entry(0, 0, 0, 50, 100), entry(0, 50, 0, 50, 100)]);
        let levels = derive_levels(
            &v2_header(1, 2),
            &directory,
            Dimensions::new(100, 100).unwrap(),
        )
        .unwrap();
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].index(), 0);
        assert!(!levels[0].is_derived());
        assert_eq!(levels[0].tile_count(), 2);
    }

    #[test]
    fn a_two_level_file_is_a_pyramid() {
        let directory = Directory::new(vec![
            entry(0, 0, 0, 50, 100),
            entry(0, 50, 0, 50, 100),
            entry(1, 0, 0, 25, 50),
            entry(1, 25, 0, 25, 50),
        ]);
        let levels = derive_levels(
            &v2_header(2, 4),
            &directory,
            Dimensions::new(100, 100).unwrap(),
        )
        .unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1].index(), 1);
        assert!(levels[1].is_derived());
        assert_eq!(levels[1].dimensions().width, 50);
        assert_eq!(levels[1].dimensions().height, 50);
        assert_eq!(levels[1].first_entry(), 2);
        assert_eq!(levels[1].entry_index(1), Some(3));
    }

    #[test]
    fn out_of_order_entries_are_rejected() {
        let directory = Directory::new(vec![entry(0, 50, 0, 50, 100), entry(0, 0, 0, 50, 100)]);
        assert!(derive_levels(
            &v2_header(1, 2),
            &directory,
            Dimensions::new(100, 100).unwrap()
        )
        .is_err());
    }

    #[test]
    fn a_derived_level_that_is_not_smaller_is_rejected() {
        let directory = Directory::new(vec![entry(0, 0, 0, 100, 100), entry(1, 0, 0, 100, 100)]);
        assert!(derive_levels(
            &v2_header(2, 2),
            &directory,
            Dimensions::new(100, 100).unwrap()
        )
        .is_err());
    }

    #[test]
    fn a_pyramid_flag_without_a_pyramid_is_rejected() {
        let directory = Directory::new(vec![entry(0, 0, 0, 100, 100)]);
        let header = v2_header(1, 1).with_feature_flags(vpng_container::FEATURE_PYRAMID);
        assert!(derive_levels(&header, &directory, Dimensions::new(100, 100).unwrap()).is_err());
    }

    #[test]
    fn a_pyramid_without_the_flag_is_rejected() {
        let directory = Directory::new(vec![entry(0, 0, 0, 100, 100), entry(1, 0, 0, 50, 50)]);
        let header = v2_header(2, 2).with_feature_flags(0);
        assert!(derive_levels(&header, &directory, Dimensions::new(100, 100).unwrap()).is_err());
    }
}
