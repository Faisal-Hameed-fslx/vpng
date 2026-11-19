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
//! * **Nothing outside the image is prefetched.** The ring is clipped to
//!   the grid.

use vpng_core::{Error, Rect, Result};
use vpng_tiling::TileGrid;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// The rectangle in image pixels.
    rect: Rect,
    /// The width of a tile, in pixels.
    tile_width: u32,
    /// The height of a tile, in pixels.
    tile_height: u32,
    /// The number of tile columns in the image.
    columns: u32,
    /// The number of tile rows in the image.
    rows: u32,
    /// The first visible column.
    first_col: u32,
    /// The last visible column.
    last_col: u32,
    /// The first visible row.
    first_row: u32,
    /// The last visible row.
    last_row: u32,
}

impl Viewport {
    /// A viewport over a grid.
    ///
    /// The rectangle is clipped to the image by the grid, and the tiles it
    /// covers are the visible ones.
    pub fn new(grid: &TileGrid, rect: Rect) -> Result<Self> {
        if !rect.is_within(grid.dimensions()) {
            return Err(Error::invalid("the viewport is outside the image"));
        }
        if rect.width == 0 || rect.height == 0 {
            return Err(Error::invalid("the viewport has no area"));
        }

        let geometries = grid.geometries_for_rect(rect)?;
        let first_col = geometries.iter().map(|g| g.col).min().unwrap_or(0);
        let last_col = geometries.iter().map(|g| g.col).max().unwrap_or(0);
        let first_row = geometries.iter().map(|g| g.row).min().unwrap_or(0);
        let last_row = geometries.iter().map(|g| g.row).max().unwrap_or(0);

        Ok(Viewport {
            rect,
            tile_width: grid.tile_width(),
            tile_height: grid.tile_height(),
            columns: grid.columns(),
            rows: grid.rows(),
            first_col,
            last_col,
            first_row,
            last_row,
        })
    }

    /// The rectangle in image pixels.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The number of visible tiles.
    pub fn visible_count(&self) -> usize {
        ((self.last_col - self.first_col + 1) * (self.last_row - self.first_row + 1)) as usize
    }

    /// The centre of the viewport, in tile coordinates.
    fn centre(&self) -> (i64, i64) {
        let col = (self.first_col as i64 + self.last_col as i64) / 2;
        let row = (self.first_row as i64 + self.last_row as i64) / 2;
        (col, row)
    }

    /// The distance of a tile from the centre of the viewport.
    ///
    /// The distance is the Manhattan distance in tiles: a tile one column
    /// and one row away is two steps away.
    pub fn distance(&self, col: u32, row: u32) -> u32 {
        let (centre_col, centre_row) = self.centre();
        let dc = (col as i64 - centre_col).abs();
        let dr = (row as i64 - centre_row).abs();
        (dc + dr) as u32
    }

    /// The distance of a tile, with the direction preference folded in.
    fn biased_distance(
        &self,
        col: u32,
        row: u32,
        policy: &PrefetchPolicy,
        direction: Direction,
    ) -> u32 {
        let base = self.distance(col, row);
        let ahead = match direction {
            Direction::None => false,
            Direction::Left => {
                col < self.first_col && self.first_col - col <= policy.direction_reach
            }
            Direction::Right => {
                col > self.last_col && col - self.last_col <= policy.direction_reach
            }
            Direction::Up => row < self.first_row && self.first_row - row <= policy.direction_reach,
            Direction::Down => row > self.last_row && row - self.last_row <= policy.direction_reach,
        };
        if ahead {
            base.saturating_sub(policy.direction_bonus)
        } else {
            base
        }
    }

    /// The plan for this viewport.
    ///
    /// The visible tiles are listed nearest to the centre first, and the
    /// prefetch tiles after them.
    pub fn plan(&self, policy: &PrefetchPolicy, direction: Direction) -> ViewPlan {
        let mut visible = Vec::with_capacity(self.visible_count());
        for row in self.first_row..=self.last_row {
            for col in self.first_col..=self.last_col {
                visible.push(PlanEntry {
                    tile: self.index(col, row),
                    distance: self.distance(col, row),
                });
            }
        }
        visible.sort_by_key(|entry| entry.distance);

        // The prefetch ring, clipped to the image and with the visible
        // tiles left out.
        let mut prefetch = Vec::new();
        let first_col = self.first_col.saturating_sub(policy.radius);
        let last_col = (self.last_col + policy.radius).min(self.columns.saturating_sub(1));
        let first_row = self.first_row.saturating_sub(policy.radius);
        let last_row = (self.last_row + policy.radius).min(self.rows.saturating_sub(1));

        for row in first_row..=last_row {
            for col in first_col..=last_col {
                if col >= self.first_col
                    && col <= self.last_col
                    && row >= self.first_row
                    && row <= self.last_row
                {
                    continue;
                }
                prefetch.push(PlanEntry {
                    tile: self.index(col, row),
                    distance: self.biased_distance(col, row, policy, direction),
                });
            }
        }
        prefetch.sort_by_key(|entry| entry.distance);

        ViewPlan { visible, prefetch }
    }

    /// The index of a tile in the directory.
    fn index(&self, col: u32, row: u32) -> usize {
        (row as usize) * (self.columns as usize) + (col as usize)
    }
}
