//! The viewport and the prefetch policy.
//!
//! A viewer knows which rectangle of the image the user is looking at. From
//! that it can work out three things: the tiles that are on screen now, the
//! tiles that are likely to be on screen next, and the order in which they
//! should be decoded.
//!
//! # The rules
//!
//! * **The visible tiles come first, nearest to the centre first.** The
//!   tile in the middle of the viewport is the one the user is most likely
//!   to be looking at directly, so it is decoded first.
//! * **The prefetch is a ring around the viewport.** The tiles just
//!   outside the visible rectangle are the ones that come next if the user
//!   pans by a little. The ring's width is a parameter, not a constant,
//!   because the right width depends on the tile size, the viewport size
//!   and how fast the user pans — and the benchmark measures it.
//! * **The direction of travel is preferred.** When the user is panning to
//!   the right, the tiles to the right are wanted before the tiles to the
//!   left. The preference is folded into the *distance* the scheduler
//!   sorts by, so it is expressed in one number and not in a second
//!   ordering rule that could disagree with the first.
//! * **Nothing outside the image is prefetched.** The ring is clipped to the
//!   partition's rectangle.
//!
//! # Uniform and adaptive partitions
//!
//! The rules work on a [`Partition`], so a file with variable-size tiles is
//! viewed by the same code as one with a uniform grid. For a uniform grid the
//! behaviour is exactly the grid-based behaviour it replaces: the visible
//! tiles are the tiles the rectangle touches, the prefetch ring is the ring
//! of tiles within `radius` nominal tile widths of the rectangle, and the
//! distance of a tile is its Manhattan distance from the viewport's centre
//! measured in nominal tile widths.

use vpng_core::{Error, Rect, Result};
use vpng_tiling::Partition;

use crate::pool::{Pool, PoolOutcome};
use crate::scheduler::{RequestClass, RequestSpec};

/// The direction the viewport is moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// The viewport is not moving.
    #[default]
    None,
    /// The viewport is moving towards smaller column numbers.
    Left,
    /// The viewport is moving towards larger column numbers.
    Right,
    /// The viewport is moving towards smaller row numbers.
    Up,
    /// The viewport is moving towards larger row numbers.
    Down,
}

impl Direction {
    /// The opposite direction.
    pub fn opposite(self) -> Direction {
        match self {
            Direction::None => Direction::None,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
        }
    }
}

/// The parameters of the prefetch policy.
///
/// The defaults are a starting point, not a measurement. The benchmark
/// varies them and records what it finds; the values that ship are the
/// ones the measurement chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefetchPolicy {
    /// How many tiles beyond the visible rectangle the prefetch reaches.
    pub radius: u32,
    /// How many tiles ahead of the visible rectangle the direction
    /// preference applies to.
    pub direction_reach: u32,
    /// How much closer a tile in the direction of travel is treated.
    ///
    /// The value is subtracted from the tile's distance from the centre of
    /// the viewport, and never takes it below zero. A large value makes
    /// the tiles ahead of the travel sort ahead of everything else in the
    /// prefetch ring.
    pub direction_bonus: u32,
}

impl Default for PrefetchPolicy {
    fn default() -> Self {
        PrefetchPolicy {
            radius: 2,
            direction_reach: 2,
            direction_bonus: 4,
        }
    }
}

/// One tile in a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanEntry {
    /// The tile's index in the directory.
    pub tile: usize,
    /// The tile's distance from the centre of the viewport.
    ///
    /// The scheduler sorts by this number, so it is the whole of the
    /// ordering rule: the direction preference is already folded into it.
    pub distance: u32,
}

/// What to decode for one viewport.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewPlan {
    /// The tiles on screen, nearest to the centre first.
    pub visible: Vec<PlanEntry>,
    /// The tiles that come next, nearest first.
    pub prefetch: Vec<PlanEntry>,
}

impl ViewPlan {
    /// The number of tiles in the plan.
    pub fn len(&self) -> usize {
        self.visible.len() + self.prefetch.len()
    }

    /// Whether the plan is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Submit the plan to a pool.
    ///
    /// The visible tiles are submitted first and as `Visible`, so they
    /// displace anything less urgent that is already queued. The prefetch
    /// tiles follow as `Prefetch`, and are refused if the queue is full —
    /// a prefetch is a guess, and a guess is not worth blocking the user.
    ///
    /// The generation is the one the plan was made for; a request from an
    /// older generation is refused by the scheduler.
    pub fn submit(&self, pool: &Pool, generation: u64) -> PlanReport {
        let mut report = PlanReport::default();

        for entry in &self.visible {
            let outcome = pool.request(RequestSpec {
                tile: entry.tile,
                class: RequestClass::Visible,
                distance: entry.distance,
                generation,
            });
            report.record(outcome, true);
        }

        for entry in &self.prefetch {
            let outcome = pool.request(RequestSpec {
                tile: entry.tile,
                class: RequestClass::Prefetch,
                distance: entry.distance,
                generation,
            });
            report.record(outcome, false);
        }

        report
    }
}

/// What a plan's submission did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanReport {
    /// The visible tiles that were queued.
    pub visible_queued: u64,
    /// The visible tiles that were already in the cache.
    pub visible_cached: u64,
    /// The visible tiles that were already queued or in flight.
    pub visible_pending: u64,
    /// The prefetch tiles that were queued.
    pub prefetch_queued: u64,
    /// The prefetch tiles that were already in the cache.
    pub prefetch_cached: u64,
    /// The prefetch tiles that were refused because the queue was full.
    pub prefetch_refused: u64,
    /// The prefetch tiles that were already queued or in flight.
    pub prefetch_pending: u64,
    /// The requests refused because they belonged to an older viewport.
    pub stale: u64,
}

impl PlanReport {
    /// Record one outcome.
    fn record(&mut self, outcome: PoolOutcome, visible: bool) {
        match outcome {
            PoolOutcome::Queued => {
                if visible {
                    self.visible_queued += 1;
                } else {
                    self.prefetch_queued += 1;
                }
            }
            PoolOutcome::Promoted => {
                if visible {
                    self.visible_queued += 1;
                } else {
                    self.prefetch_queued += 1;
                }
            }
            PoolOutcome::Cached => {
                if visible {
                    self.visible_cached += 1;
                } else {
                    self.prefetch_cached += 1;
                }
            }
            PoolOutcome::AlreadyPending | PoolOutcome::AlreadyInFlight => {
                if visible {
                    self.visible_pending += 1;
                } else {
                    self.prefetch_pending += 1;
                }
            }
            PoolOutcome::RejectedFull => {
                if visible {
                    self.visible_pending += 1;
                } else {
                    self.prefetch_refused += 1;
                }
            }
            PoolOutcome::RejectedStale => self.stale += 1,
        }
    }
}

/// A rectangle of the image the user is looking at.
#[derive(Debug, Clone)]
pub struct Viewport<'a> {
    /// The rectangle in image pixels.
    rect: Rect,
    /// The partition the image is divided into.
    partition: &'a Partition,
    /// The positions of the visible tiles, in partition order.
    visible: Vec<usize>,
    /// The left edge of the viewport's centre column.
    centre_x: u32,
    /// The top edge of the viewport's centre row.
    centre_y: u32,
    /// The nominal tile width, in pixels.
    nominal_width: u32,
    /// The nominal tile height, in pixels.
    nominal_height: u32,
    /// The left edge of the visible range.
    visible_left: u32,
    /// The right edge of the visible range.
    visible_right: u32,
    /// The top edge of the visible range.
    visible_top: u32,
    /// The bottom edge of the visible range.
    visible_bottom: u32,
}

impl<'a> Viewport<'a> {
    /// A viewport over a partition.
    ///
    /// The rectangle must be inside the partition's rectangle, and the tiles
    /// it covers are the visible ones.
    pub fn new(partition: &'a Partition, rect: Rect) -> Result<Self> {
        if !rect.is_within(partition.dimensions()) {
            return Err(Error::invalid("the viewport is outside the image"));
        }
        if rect.width == 0 || rect.height == 0 {
            return Err(Error::invalid("the viewport has no area"));
        }

        let visible = partition.tiles_for(rect)?;
        let columns = partition.column_origins();
        let rows = partition.row_origins();

        // The centre is the middle of the visible range, in the same way a
        // grid-based viewport took the middle of its column and row ranges.
        let (first_col, last_col) = origin_range(&columns, &visible, partition, true)?;
        let (first_row, last_row) = origin_range(&rows, &visible, partition, false)?;

        let centre_x = columns[((first_col + last_col) / 2) as usize];
        let centre_y = rows[((first_row + last_row) / 2) as usize];
        let (nominal_width, nominal_height) = partition.nominal_tile_size();

        let visible_left = visible
            .iter()
            .filter_map(|position| partition.tile(*position))
            .map(|tile| tile.x)
            .min()
            .unwrap_or(0);
        let visible_right = visible
            .iter()
            .filter_map(|position| partition.tile(*position))
            .map(|tile| tile.x)
            .max()
            .unwrap_or(0);
        let visible_top = visible
            .iter()
            .filter_map(|position| partition.tile(*position))
            .map(|tile| tile.y)
            .min()
            .unwrap_or(0);
        let visible_bottom = visible
            .iter()
            .filter_map(|position| partition.tile(*position))
            .map(|tile| tile.y)
            .max()
            .unwrap_or(0);

        Ok(Viewport {
            rect,
            partition,
            visible,
            centre_x,
            centre_y,
            nominal_width: nominal_width.max(1),
            nominal_height: nominal_height.max(1),
            visible_left,
            visible_right,
            visible_top,
            visible_bottom,
        })
    }

    /// The rectangle in image pixels.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The number of visible tiles.
    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }

    /// The distance of a tile from the centre of the viewport.
    ///
    /// The distance is the Manhattan distance measured in nominal tile
    /// widths: a tile one nominal tile to the left and one nominal tile
    /// above the centre is two steps away.
    pub fn distance(&self, tile: &vpng_tiling::TileRect) -> u32 {
        let dx = self.tile_x_distance(tile.x);
        let dy = self.tile_y_distance(tile.y);
        dx.saturating_add(dy)
    }

    /// The distance of a tile, with the direction preference folded in.
    fn biased_distance(
        &self,
        tile: &vpng_tiling::TileRect,
        policy: &PrefetchPolicy,
        direction: Direction,
    ) -> u32 {
        let base = self.distance(tile);
        let ahead = match direction {
            Direction::None => false,
            Direction::Left => self.is_left_of_centre_column(tile.x, policy.direction_reach),
            Direction::Right => self.is_right_of_centre_column(tile.x, policy.direction_reach),
            Direction::Up => self.is_above_centre_row(tile.y, policy.direction_reach),
            Direction::Down => self.is_below_centre_row(tile.y, policy.direction_reach),
        };
        if ahead {
            base.saturating_sub(policy.direction_bonus)
        } else {
            base
        }
    }

    /// The horizontal distance of a left edge from the centre, in tile widths.
    fn tile_x_distance(&self, x: u32) -> u32 {
        let delta = x.abs_diff(self.centre_x);
        delta / self.nominal_width
    }

    /// The vertical distance of a top edge from the centre, in tile heights.
    fn tile_y_distance(&self, y: u32) -> u32 {
        let delta = y.abs_diff(self.centre_y);
        delta / self.nominal_height
    }

    /// Whether the tile is to the left of the visible range, within `reach`.
    fn is_left_of_centre_column(&self, x: u32, reach: u32) -> bool {
        x < self.visible_left && (self.visible_left - x) / self.nominal_width <= reach
    }

    /// Whether the tile is to the right of the visible range, within `reach`.
    fn is_right_of_centre_column(&self, x: u32, reach: u32) -> bool {
        x > self.visible_right && (x - self.visible_right) / self.nominal_width <= reach
    }

    /// Whether the tile is above the visible range, within `reach`.
    fn is_above_centre_row(&self, y: u32, reach: u32) -> bool {
        y < self.visible_top && (self.visible_top - y) / self.nominal_height <= reach
    }

    /// Whether the tile is below the visible range, within `reach`.
    fn is_below_centre_row(&self, y: u32, reach: u32) -> bool {
        y > self.visible_bottom && (y - self.visible_bottom) / self.nominal_height <= reach
    }

    /// The plan for this viewport.
    ///
    /// The visible tiles are listed nearest to the centre first, and the
    /// prefetch tiles after them.
    pub fn plan(&self, policy: &PrefetchPolicy, direction: Direction) -> ViewPlan {
        let mut visible = Vec::with_capacity(self.visible.len());
        for position in &self.visible {
            let tile = self
                .partition
                .tile(*position)
                .expect("a visible position is inside the partition");
            visible.push(PlanEntry {
                tile: *position,
                distance: self.distance(tile),
            });
        }
        visible.sort_by_key(|entry| entry.distance);

        // The prefetch ring, clipped to the partition and with the visible
        // tiles left out.
        let mut prefetch = Vec::new();
        if policy.radius > 0 {
            if let Some(ring) = self.ring_rect(policy.radius) {
                // The ring is inside the partition's rectangle by
                // construction, so the lookup cannot fail.
                let ring_tiles = self.partition.tiles_for(ring).unwrap_or_default();
                for position in ring_tiles {
                    if self.visible.binary_search(&position).is_ok() {
                        continue;
                    }
                    let tile = self
                        .partition
                        .tile(position)
                        .expect("a ring position is inside the partition");
                    prefetch.push(PlanEntry {
                        tile: position,
                        distance: self.biased_distance(tile, policy, direction),
                    });
                }
            }
        }
        prefetch.sort_by_key(|entry| entry.distance);

        ViewPlan { visible, prefetch }
    }

    /// The rectangle the prefetch ring covers.
    ///
    /// The ring extends `radius` nominal tile widths beyond the viewport on
    /// every side, clipped to the partition's rectangle.
    fn ring_rect(&self, radius: u32) -> Option<Rect> {
        let region = self.partition.region();
        let left = self
            .rect
            .x
            .saturating_sub(radius.saturating_mul(self.nominal_width));
        let top = self
            .rect
            .y
            .saturating_sub(radius.saturating_mul(self.nominal_height));
        let right = self
            .rect
            .right()
            .ok()?
            .saturating_add(radius.saturating_mul(self.nominal_width))
            .min(region.right().ok()?);
        let bottom = self
            .rect
            .bottom()
            .ok()?
            .saturating_add(radius.saturating_mul(self.nominal_height))
            .min(region.bottom().ok()?);
        if right <= left || bottom <= top {
            return None;
        }
        Rect::new(left, top, right - left, bottom - top).ok()
    }
}

/// The first and last origin indices the visible tiles touch.
///
/// The origins are the partition's distinct left edges (or top edges). The
/// range is taken over the *visible* tiles, so the centre of a viewport over
/// an adaptive partition is the middle of the range it actually covers.
fn origin_range(
    origins: &[u32],
    visible: &[usize],
    partition: &Partition,
    horizontal: bool,
) -> Result<(u32, u32)> {
    let mut first = u32::MAX;
    let mut last = 0u32;
    for position in visible {
        let tile = partition
            .tile(*position)
            .ok_or_else(|| Error::internal("a visible position is outside the partition"))?;
        let origin = if horizontal { tile.x } else { tile.y };
        let index = origins
            .binary_search(&origin)
            .map_err(|_| Error::internal("a tile origin is not in the partition's origin list"))?
            as u32;
        first = first.min(index);
        last = last.max(index);
    }
    if first == u32::MAX {
        return Err(Error::invalid("the viewport covers no tile"));
    }
    Ok((first, last))
}
