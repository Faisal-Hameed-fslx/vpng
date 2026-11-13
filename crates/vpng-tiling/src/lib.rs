//! `vpng-tiling` — tile geometry and region-to-tile mapping.
//!
//! A [`TileGrid`] is fully determined by a nominal tile size and the image
//! dimensions. Edge tiles are clipped to the image. Coverage is exact by
//! construction: every pixel belongs to exactly one tile.

#![forbid(unsafe_code)]

use vpng_core::{Dimensions, Error, Rect, Result};

/// Ceiling division on u32 without overflow, computed in u64.
fn div_ceil(value: u32, divisor: u32) -> u32 {
    debug_assert!(divisor > 0);
    (value as u64).div_ceil(divisor as u64) as u32
}

/// Geometry of one tile: its grid position plus its pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileGeometry {
    pub col: u32,
    pub row: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TileGeometry {
    pub fn rect(&self) -> Rect {
        // x + width <= image width and y + height <= image height by
        // construction, so this cannot fail.
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    /// Linear index in row-major (row, then col) order.
    pub fn index(&self, columns: u32) -> usize {
        (self.row as usize) * (columns as usize) + (self.col as usize)
    }
}

/// A uniform tile partition of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileGrid {
    tile_width: u32,
    tile_height: u32,
    image_width: u32,
    image_height: u32,
    columns: u32,
    rows: u32,
}

impl TileGrid {
    pub fn new(
        tile_width: u32,
        tile_height: u32,
        image_width: u32,
        image_height: u32,
    ) -> Result<Self> {
        if tile_width == 0 || tile_height == 0 {
            return Err(Error::invalid("tile dimensions must be non-zero"));
        }
        let dims = Dimensions::new(image_width, image_height)?;
        let _ = dims;
        let columns = div_ceil(image_width, tile_width);
        let rows = div_ceil(image_height, tile_height);
        Ok(TileGrid {
            tile_width,
            tile_height,
            image_width,
            image_height,
            columns,
            rows,
        })
    }

    pub fn tile_width(&self) -> u32 {
        self.tile_width
    }

    pub fn tile_height(&self) -> u32 {
        self.tile_height
    }

    pub fn image_width(&self) -> u32 {
        self.image_width
    }

    pub fn image_height(&self) -> u32 {
        self.image_height
    }

    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.image_width,
            height: self.image_height,
        }
    }

    pub fn columns(&self) -> u32 {
        self.columns
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn tile_count(&self) -> Result<u32> {
        let count = vpng_core::checked_mul(self.columns as u64, self.rows as u64)?;
        u32::try_from(count).map_err(|_| Error::resource_limit("tile count exceeds u32"))
    }

    /// Grid position containing pixel `(x, y)`.
    pub fn col_row_for_pixel(&self, x: u32, y: u32) -> Result<(u32, u32)> {
        if x >= self.image_width || y >= self.image_height {
            return Err(Error::invalid(format!(
                "pixel ({x}, {y}) is outside {}x{}",
                self.image_width, self.image_height
            )));
        }
        Ok((x / self.tile_width, y / self.tile_height))
    }

    /// Geometry for a grid position, with the edge clipped to the image.
    pub fn geometry(&self, col: u32, row: u32) -> Result<TileGeometry> {
        if col >= self.columns || row >= self.rows {
            return Err(Error::invalid(format!(
                "tile ({col}, {row}) is outside the {}x{} grid",
                self.columns, self.rows
            )));
        }
        let x = col * self.tile_width;
        let y = row * self.tile_height;
        let width = self.tile_width.min(self.image_width - x);
        let height = self.tile_height.min(self.image_height - y);
        Ok(TileGeometry {
            col,
            row,
            x,
            y,
            width,
            height,
        })
    }

    /// Index of a grid position in row-major order.
    pub fn index(&self, col: u32, row: u32) -> Result<usize> {
        if col >= self.columns || row >= self.rows {
            return Err(Error::invalid("tile index out of range"));
        }
        Ok((row as usize) * (self.columns as usize) + (col as usize))
    }

    /// Iterate every tile in row-major order.
    pub fn iter(&self) -> TileGridIter<'_> {
        TileGridIter {
            grid: self,
            col: 0,
            row: 0,
        }
    }

    /// All tiles intersecting `rect`, clipped to the image bounds.
    pub fn geometries_for_rect(&self, rect: Rect) -> Result<Vec<TileGeometry>> {
        if !rect.is_within(self.dimensions()) {
            return Err(Error::invalid("rectangle is outside the image"));
        }
        let (col0, row0) = (rect.x / self.tile_width, rect.y / self.tile_height);
        let (col1, row1) = (
            (rect.right()? - 1) / self.tile_width,
            (rect.bottom()? - 1) / self.tile_height,
        );
        let mut out = Vec::new();
        for row in row0..=row1 {
            for col in col0..=col1 {
                out.push(self.geometry(col, row)?);
            }
        }
        Ok(out)
    }

    /// All tiles whose rectangle intersects `rect`, in row-major order.
    pub fn all_geometries(&self) -> Result<Vec<TileGeometry>> {
        let mut out = Vec::with_capacity(self.columns as usize * self.rows as usize);
        for row in 0..self.rows {
            for col in 0..self.columns {
                out.push(self.geometry(col, row)?);
            }
        }
        Ok(out)
    }
}

/// Row-major iterator over tile geometry.
pub struct TileGridIter<'a> {
    grid: &'a TileGrid,
    col: u32,
    row: u32,
}

impl Iterator for TileGridIter<'_> {
    type Item = TileGeometry;

    fn next(&mut self) -> Option<TileGeometry> {
        if self.row >= self.grid.rows {
            return None;
        }
        // geometry() cannot fail while col < columns and row < rows.
        let geom = self.grid.geometry(self.col, self.row).ok()?;
        self.col += 1;
        if self.col >= self.grid.columns {
            self.col = 0;
            self.row += 1;
        }
        Some(geom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_partition_covers_image() {
        let grid = TileGrid::new(256, 256, 1000, 700).unwrap();
        assert_eq!(grid.columns(), 4);
        assert_eq!(grid.rows(), 3);
        assert_eq!(grid.tile_count().unwrap(), 12);

        let mut covered = 0u64;
        for geom in grid.iter() {
            assert!(geom.width > 0 && geom.height > 0);
            assert!(geom.x + geom.width <= 1000);
            assert!(geom.y + geom.height <= 700);
            covered += geom.width as u64 * geom.height as u64;
        }
        assert_eq!(covered, 1000 * 700);
    }

    #[test]
    fn edge_tiles_are_clipped() {
        let grid = TileGrid::new(256, 256, 300, 100).unwrap();
        let last = grid.geometry(1, 0).unwrap();
        assert_eq!(last.width, 44);
        assert_eq!(last.height, 100);
    }

    #[test]
    fn region_maps_to_intersecting_tiles_only() {
        let grid = TileGrid::new(64, 64, 512, 512).unwrap();
        // Straddles the tile boundary at 64 on both axes, so it touches the
        // 2x2 block of tiles (0,0), (1,0), (0,1), (1,1).
        let rect = Rect::new(60, 60, 10, 10).unwrap();
        let tiles = grid.geometries_for_rect(rect).unwrap();
        assert_eq!(tiles.len(), 4);
        for t in &tiles {
            assert!(rect.intersection(&t.rect()).is_some());
        }
    }

    #[test]
    fn region_in_one_tile_loads_one_tile() {
        let grid = TileGrid::new(64, 64, 512, 512).unwrap();
        let rect = Rect::new(3, 3, 10, 10).unwrap();
        assert_eq!(grid.geometries_for_rect(rect).unwrap().len(), 1);
    }

    #[test]
    fn index_matches_iteration_order() {
        let grid = TileGrid::new(8, 8, 32, 16).unwrap();
        for (i, geom) in grid.iter().enumerate() {
            assert_eq!(grid.index(geom.col, geom.row).unwrap(), i);
        }
    }

    #[test]
    fn out_of_range_pixels_are_rejected() {
        let grid = TileGrid::new(16, 16, 32, 32).unwrap();
        assert!(grid.col_row_for_pixel(31, 31).is_ok());
        assert!(grid.col_row_for_pixel(32, 0).is_err());
        assert!(grid.geometry(2, 0).is_err());
    }
}
