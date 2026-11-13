//! Dimensions and rectangles with overflow-checked arithmetic.

use crate::error::{Error, Result};

/// A non-zero image size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::invalid("image dimensions must be non-zero"));
        }
        Ok(Dimensions { width, height })
    }

    pub fn pixel_count(&self) -> Result<u64> {
        crate::checked_mul(self.width as u64, self.height as u64)
    }
}

/// An axis-aligned rectangle in image pixel coordinates.
///
/// `x + width` and `y + height` are guaranteed not to overflow, because the
/// constructor and mutators reject values that would wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::invalid("rectangle dimensions must be non-zero"));
        }
        let rect = Rect {
            x,
            y,
            width,
            height,
        };
        rect.right()?;
        rect.bottom()?;
        Ok(rect)
    }

    /// Exclusive right edge (`x + width`).
    pub fn right(&self) -> Result<u32> {
        self.x
            .checked_add(self.width)
            .ok_or_else(|| Error::invalid("rectangle x + width overflows u32"))
    }

    /// Exclusive bottom edge (`y + height`).
    pub fn bottom(&self) -> Result<u32> {
        self.y
            .checked_add(self.height)
            .ok_or_else(|| Error::invalid("rectangle y + height overflows u32"))
    }

    pub fn area(&self) -> Result<u64> {
        crate::checked_mul(self.width as u64, self.height as u64)
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        let (x, y) = (x as u64, y as u64);
        x >= self.x as u64
            && y >= self.y as u64
            && x < self.x as u64 + self.width as u64
            && y < self.y as u64 + self.height as u64
    }

    pub fn is_within(&self, dims: Dimensions) -> bool {
        match (self.right(), self.bottom()) {
            (Ok(r), Ok(b)) => r <= dims.width && b <= dims.height,
            _ => false,
        }
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().ok()?.min(other.right().ok()?);
        let y1 = self.bottom().ok()?.min(other.bottom().ok()?);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_dimensions() {
        assert!(Dimensions::new(0, 10).is_err());
        assert!(Rect::new(0, 0, 0, 5).is_err());
    }

    #[test]
    fn rejects_overflowing_rect() {
        assert!(Rect::new(u32::MAX, 0, 2, 1).is_err());
        assert!(Rect::new(0, u32::MAX, 1, 2).is_err());
    }

    #[test]
    fn contains_and_clip() {
        let r = Rect::new(10, 10, 100, 50).unwrap();
        assert!(r.contains(10, 10));
        assert!(r.contains(109, 59));
        assert!(!r.contains(110, 59));
        assert!(!r.contains(9, 10));

        let dims = Dimensions::new(64, 64).unwrap();
        assert!(!r.is_within(dims));
        let small = Rect::new(0, 0, 64, 64).unwrap();
        assert!(small.is_within(dims));
    }

    #[test]
    fn intersection_edges() {
        let a = Rect::new(0, 0, 10, 10).unwrap();
        let b = Rect::new(5, 5, 10, 10).unwrap();
        assert_eq!(a.intersection(&b), Some(Rect::new(5, 5, 5, 5).unwrap()));
        let c = Rect::new(10, 0, 5, 5).unwrap();
        assert_eq!(a.intersection(&c), None);
    }
}
