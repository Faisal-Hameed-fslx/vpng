//! The content-adaptive partition planner (P7).
//!
//! A uniform grid gives every tile the same size, whatever the picture holds.
//! That is simple and it is predictable, but it is also wasteful in both
//! directions: a large flat area is cut into many small tiles that all hold
//! the same few pixels, while a busy area is cut into tiles that are still
//! too large to compress well.
//!
//! The adaptive planner looks at the picture and chooses the tile sizes:
//!
//! * **Calm regions are merged.** A region whose pixels barely change is
//!   covered by one large tile, so a flat background costs one directory
//!   entry instead of a hundred.
//! * **Busy regions are split.** A region whose pixels change quickly is
//!   covered by smaller tiles, so a decoder can fetch a smaller piece of it.
//!
//! # How a region's "busyness" is measured
//!
//! The score of a region is the **mean absolute difference between
//! neighbouring pixels**, per channel, over a sampled lattice of at most
//! 64x64 samples. The alpha channel is left out: it is a mask, not colour.
//!
//! * A flat area scores 0.
//! * A smooth gradient scores a few units.
//! * A photograph scores tens of units.
//!
//! The score is a *mean*, not a sum, so it does not depend on the region's
//! size: a 512x512 photograph and a 64x64 photograph of the same content
//! score the same. That is what makes one threshold meaningful for every
//! tile size.
//!
//! The lattice is a fixed stride, so the score is deterministic: the same
//! picture always produces the same partition, on every machine.
//!
//! # How the partition is built
//!
//! 1. The image is cut into a **coarse** partition whose tiles are at most
//!    `max_edge` pixels on a side. The cuts fall on multiples of `min_edge`,
//!    so the tiles line up with a regular lattice.
//! 2. Every coarse tile is scored.
//! 3. The busiest tile is split, over and over, until either every busy tile
//!    has been split as far as `min_edge` allows or the **tile budget** is
//!    spent. The budget is a multiple of the uniform grid's tile count, so
//!    the adaptive partition can never be much more expensive to index than
//!    the uniform one.
//!
//! Every step is deterministic: the tiles are split in a fixed order (busiest
//! first, then top-left first), so the same picture always yields the same
//! partition and therefore the same file.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use vpng_core::{CanonicalImage, Error, Result};
use vpng_tiling::{Partition, TileRect};

/// The largest edge a tile may have: the directory entry stores it in 16 bits.
const MAX_TILE_EDGE: u32 = u16::MAX as u32;

/// The parameters of the adaptive planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptiveOptions {
    /// The smallest tile edge the planner produces.
    pub min_edge: u32,
    /// The largest tile edge the planner produces.
    pub max_edge: u32,
    /// The score above which a tile is "busy" and worth splitting.
    pub threshold: u32,
    /// The largest number of tiles the planner may produce.
    pub max_tiles: u32,
}

impl AdaptiveOptions {
    /// The default parameters for a given nominal tile edge.
    ///
    /// The smallest tile is half the nominal edge and the largest is twice
    /// it, so the adaptive partition stays in the same order of magnitude as
    /// the uniform one. The tile budget is twice the uniform grid's tile
    /// count, which bounds the extra directory entries an adaptive file can
    /// cost.
    pub fn for_nominal(nominal: u32, uniform_tiles: u32) -> Self {
        let min_edge = (nominal / 2).max(1);
        let max_edge = nominal.saturating_mul(2).clamp(min_edge, MAX_TILE_EDGE);
        AdaptiveOptions {
            min_edge,
            max_edge,
            threshold: DEFAULT_THRESHOLD,
            max_tiles: uniform_tiles.saturating_mul(2).max(1),
        }
    }
}

/// The default busyness threshold: a mean neighbour difference of 8/255.
pub const DEFAULT_THRESHOLD: u32 = 8;

/// One tile of the partition, with its score.
#[derive(Debug, Clone, Copy)]
struct Leaf {
    rect: TileRect,
    score: u32,
    /// Whether the leaf has already been split into two children.
    split: bool,
}

/// Plan an adaptive partition of `image`.
///
/// The partition's tiles are in canonical `(y, x)` order, so a tile's
/// position in the partition is the index of its directory entry.
pub fn plan_adaptive(image: &CanonicalImage, options: &AdaptiveOptions) -> Result<Partition> {
    if options.min_edge == 0 || options.max_edge < options.min_edge {
        return Err(Error::invalid(
            "the adaptive planner needs a non-zero minimum edge and a maximum edge \
             that is not smaller than it",
        ));
    }

    let region = image.bounds();
    let root = TileRect::new(0, 0, region.width, region.height)?;

    // Step one: the coarse partition. Every tile is at most `max_edge` on a
    // side, and the cuts fall on multiples of `min_edge`.
    let mut leaves: Vec<Leaf> = Vec::new();
    coarse(root, options, &mut leaves)?;

    // Step two: score every coarse tile.
    for leaf in leaves.iter_mut() {
        leaf.score = region_score(image, &leaf.rect);
    }

    // Step three: split the busiest tiles until the budget is spent.
    //
    // The heap's key is (score, then the top-left tile first), so the order
    // the tiles are split in is fixed: the busiest tile first, and among
    // equally busy tiles the top-left one first.
    let mut heap: BinaryHeap<(u32, Reverse<u32>, Reverse<u32>, usize)> = BinaryHeap::new();
    for index in 0..leaves.len() {
        push_candidate(&mut heap, &leaves, index, options);
    }

    let mut splits = 0usize;
    let initial = leaves.len();
    while let Some((score, _, _, index)) = heap.pop() {
        // The budget is the number of *final* tiles: every split turns one
        // tile into two, so the final count is the initial count plus the
        // number of splits.
        if initial + splits >= options.max_tiles as usize {
            break;
        }
        let leaf = leaves[index];
        if leaf.split || leaf.score != score {
            continue;
        }
        let Some((first, second)) = split_leaf(leaf.rect, options.min_edge) else {
            continue;
        };

        leaves[index].split = true;
        let child_a = TileRect::new(first.x, first.y, first.width, first.height)?;
        let child_b = TileRect::new(second.x, second.y, second.width, second.height)?;
        let index_a = leaves.len();
        leaves.push(Leaf {
            rect: child_a,
            score: region_score(image, &child_a),
            split: false,
        });
        let index_b = leaves.len();
        leaves.push(Leaf {
            rect: child_b,
            score: region_score(image, &child_b),
            split: false,
        });
        splits += 1;

        push_candidate(&mut heap, &leaves, index_a, options);
        push_candidate(&mut heap, &leaves, index_b, options);
    }

    // Step four: the final tiles, in canonical order.
    let mut tiles: Vec<TileRect> = leaves
        .iter()
        .filter(|leaf| !leaf.split)
        .map(|leaf| leaf.rect)
        .collect();
    tiles.sort_by_key(|tile| (tile.y, tile.x));

    Partition::from_tiles(region, tiles)
}

/// Put a leaf on the heap when it is busy enough and large enough to split.
fn push_candidate(
    heap: &mut BinaryHeap<(u32, Reverse<u32>, Reverse<u32>, usize)>,
    leaves: &[Leaf],
    index: usize,
    options: &AdaptiveOptions,
) {
    let leaf = leaves[index];
    if leaf.split || leaf.score <= options.threshold {
        return;
    }
    if split_leaf(leaf.rect, options.min_edge).is_none() {
        return;
    }
    heap.push((
        leaf.score,
        Reverse(leaf.rect.y),
        Reverse(leaf.rect.x),
        index,
    ));
}

/// Cut a rectangle into the coarse partition: every tile is at most
/// `max_edge` on a side.
fn coarse(rect: TileRect, options: &AdaptiveOptions, out: &mut Vec<Leaf>) -> Result<()> {
    if rect.width <= options.max_edge && rect.height <= options.max_edge {
        out.push(Leaf {
            rect,
            score: 0,
            split: false,
        });
        return Ok(());
    }

    // Split along the longer axis, so the tiles stay as square as possible.
    let horizontal = rect.width >= rect.height;
    let size = if horizontal { rect.width } else { rect.height };
    let Some((first, second)) = split_size(size, options.min_edge) else {
        out.push(Leaf {
            rect,
            score: 0,
            split: false,
        });
        return Ok(());
    };

    let (a, b) = if horizontal {
        (
            TileRect::new(rect.x, rect.y, first, rect.height)?,
            TileRect::new(rect.x + first, rect.y, second, rect.height)?,
        )
    } else {
        (
            TileRect::new(rect.x, rect.y, rect.width, first)?,
            TileRect::new(rect.x, rect.y + first, rect.width, second)?,
        )
    };
    coarse(a, options, out)?;
    coarse(b, options, out)?;
    Ok(())
}

/// Split a leaf into two, along its longer axis.
fn split_leaf(rect: TileRect, min_edge: u32) -> Option<(TileRect, TileRect)> {
    let horizontal = rect.width >= rect.height;
    let size = if horizontal { rect.width } else { rect.height };
    let (first, second) = split_size(size, min_edge)?;

    if horizontal {
        Some((
            TileRect::new(rect.x, rect.y, first, rect.height).ok()?,
            TileRect::new(rect.x + first, rect.y, second, rect.height).ok()?,
        ))
    } else {
        Some((
            TileRect::new(rect.x, rect.y, rect.width, first).ok()?,
            TileRect::new(rect.x, rect.y + first, rect.width, second).ok()?,
        ))
    }
}

/// Split a length into two parts, the first a multiple of `unit`.
///
/// The first part is at most half the length and at least `unit`, so the
/// split always makes progress and the parts stay on the lattice.
fn split_size(size: u32, unit: u32) -> Option<(u32, u32)> {
    if unit == 0 || size < unit.saturating_mul(2) {
        return None;
    }
    let mut first = (size / 2 / unit) * unit;
    if first == 0 {
        first = unit;
    }
    if first >= size {
        return None;
    }
    Some((first, size - first))
}

/// The mean absolute difference between neighbouring pixels of a region.
///
/// The pixels are sampled on a lattice of at most 64x64 samples, so the cost
/// of a score does not depend on the region's size. The result is per
/// channel: the alpha channel is left out.
fn region_score(image: &CanonicalImage, rect: &TileRect) -> u32 {
    let format = image.format();
    let channels = format.channels() as usize;
    if channels == 0 {
        return 0;
    }
    let sample_bytes = format.bytes_per_channel() as usize;
    let bpp = format.bytes_per_pixel() as usize;
    let Ok(row) = image.row_bytes() else {
        return 0;
    };
    let row = row as usize;
    let data = image.data();

    let Ok(right) = rect.right() else {
        return 0;
    };
    let Ok(bottom) = rect.bottom() else {
        return 0;
    };

    let step_x = (rect.width / 64).max(1);
    let step_y = (rect.height / 64).max(1);

    // The colour channels are the first `channels - alpha` channels.
    let colour_channels = if format.color_model.has_alpha() {
        channels - 1
    } else {
        channels
    };
    if colour_channels == 0 {
        return 0;
    }

    let mut sum: u64 = 0;
    let mut pairs: u64 = 0;

    let mut y = rect.y;
    while y < bottom {
        let mut x = rect.x;
        while x < right {
            let here = (y as usize) * row + (x as usize) * bpp;
            if x + 1 < right {
                let neighbour = (y as usize) * row + ((x + 1) as usize) * bpp;
                sum += channel_delta(data, here, neighbour, sample_bytes, colour_channels);
                pairs += 1;
            }
            if y + 1 < bottom {
                let neighbour = ((y + 1) as usize) * row + (x as usize) * bpp;
                sum += channel_delta(data, here, neighbour, sample_bytes, colour_channels);
                pairs += 1;
            }
            x += step_x;
        }
        y += step_y;
    }

    if pairs == 0 {
        return 0;
    }

    // The mean difference over the colour channels, rounded down.
    let per_pair = sum / pairs;
    (per_pair / colour_channels as u64) as u32
}

/// The sum of the absolute differences of the colour channels of two pixels.
fn channel_delta(data: &[u8], a: usize, b: usize, sample_bytes: usize, channels: usize) -> u64 {
    let mut total = 0u64;
    for channel in 0..channels {
        let left = read_sample(data, a + channel * sample_bytes, sample_bytes);
        let right = read_sample(data, b + channel * sample_bytes, sample_bytes);
        total += left.abs_diff(right) as u64;
    }
    total
}

/// Read one channel sample as an integer.
fn read_sample(data: &[u8], offset: usize, sample_bytes: usize) -> u32 {
    match sample_bytes {
        1 => *data.get(offset).unwrap_or(&0) as u32,
        2 => {
            let low = *data.get(offset).unwrap_or(&0) as u32;
            let high = *data.get(offset + 1).unwrap_or(&0) as u32;
            (high << 8) | low
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{CanonicalImage, PixelFormat};

    fn flat(width: u32, height: u32) -> CanonicalImage {
        CanonicalImage::from_vec(
            width,
            height,
            PixelFormat::rgba8(),
            vec![0x40; (width * height * 4) as usize],
        )
        .unwrap()
    }

    fn noise(width: u32, height: u32) -> CanonicalImage {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut data = vec![0u8; (width * height * 4) as usize];
        for byte in data.iter_mut() {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
        }
        CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
    }

    fn options(nominal: u32) -> AdaptiveOptions {
        let uniform = (1024u32 / nominal) * (1024u32 / nominal);
        AdaptiveOptions::for_nominal(nominal, uniform)
    }

    #[test]
    fn a_flat_image_is_covered_by_large_tiles() {
        let image = flat(1024, 1024);
        let partition = plan_adaptive(&image, &options(256)).unwrap();
        // The flat image needs no small tiles at all: four 512x512 tiles
        // cover it, where the uniform grid would have used sixteen.
        assert_eq!(partition.len(), 4, "a flat image needs few tiles");
        assert!(partition.max_tile_width() <= 512);
        assert!(partition.tiles().iter().all(|tile| tile.width >= 512));
    }

    #[test]
    fn a_noisy_image_is_split_further() {
        let image = noise(1024, 1024);
        let partition = plan_adaptive(&image, &options(256)).unwrap();
        assert!(
            partition.len() > 16,
            "a noisy image needs more tiles: {}",
            partition.len()
        );
    }

    #[test]
    fn the_partition_covers_the_image_exactly() {
        let image = noise(300, 200);
        let partition = plan_adaptive(&image, &options(64)).unwrap();
        assert_eq!(partition.dimensions().width, 300);
        assert_eq!(partition.dimensions().height, 200);
        let mut area = 0u64;
        for tile in partition.tiles() {
            area += tile.width as u64 * tile.height as u64;
        }
        assert_eq!(area, 300 * 200);
    }

    #[test]
    fn the_planner_is_deterministic() {
        let image = noise(512, 512);
        let a = plan_adaptive(&image, &options(128)).unwrap();
        let b = plan_adaptive(&image, &options(128)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn the_tile_budget_is_respected() {
        let image = noise(1024, 1024);
        let mut opts = options(256);
        opts.max_tiles = 20;
        let partition = plan_adaptive(&image, &opts).unwrap();
        assert!(partition.len() <= 20, "got {} tiles", partition.len());
    }

    #[test]
    fn the_tiles_are_in_canonical_order() {
        let image = noise(512, 512);
        let partition = plan_adaptive(&image, &options(128)).unwrap();
        let mut previous = (0u32, 0u32);
        for (index, tile) in partition.tiles().iter().enumerate() {
            let key = (tile.y, tile.x);
            if index > 0 {
                assert!(key > previous, "the tiles are not in (y, x) order");
            }
            previous = key;
        }
    }
}
