//! `vpng-core` — the shared vocabulary of the VPNG implementation.
//!
//! This crate holds no I/O and no codec logic. It defines what a pixel format
//! is, what a rectangle is, what a resource limit is, how failures are
//! classified, and the canonical image buffer that everything else agrees on.
//!
//! Checked arithmetic helpers live here so that every crate can share exactly
//! one overflow policy.

#![forbid(unsafe_code)]

pub mod error;
pub mod geom;
pub mod image;
pub mod limits;
pub mod pixel;

pub use error::{Error, ErrorKind, Result};
pub use geom::{Dimensions, Rect};
pub use image::CanonicalImage;
pub use limits::Limits;
pub use pixel::{BitsPerChannel, ColorModel, PixelFormat};

/// Checked addition on `u64`, reporting an [`ErrorKind::InvalidArgument`].
pub fn checked_add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| Error::invalid(format!("arithmetic overflow: {a} + {b}")))
}

/// Checked subtraction on `u64`.
pub fn checked_sub(a: u64, b: u64) -> Result<u64> {
    a.checked_sub(b)
        .ok_or_else(|| Error::invalid(format!("arithmetic underflow: {a} - {b}")))
}

/// Checked multiplication on `u64`.
pub fn checked_mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b)
        .ok_or_else(|| Error::invalid(format!("arithmetic overflow: {a} * {b}")))
}

/// Convert a `u64` to `usize`, failing cleanly on 32-bit hosts.
pub fn checked_usize(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::resource_limit(format!("value {value} does not fit in this address space"))
    })
}

/// Verify that `[offset, offset + len)` lies inside `[0, total)`.
pub fn checked_range(offset: u64, len: u64, total: u64) -> Result<()> {
    let end = checked_add(offset, len)?;
    if end > total {
        return Err(Error::corrupt(format!(
            "range {offset}..{end} exceeds available size {total}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_is_reported() {
        assert!(checked_mul(u64::MAX, 2).is_err());
        assert!(checked_add(u64::MAX, 1).is_err());
        assert!(checked_sub(0, 1).is_err());
        assert_eq!(checked_mul(6, 7).unwrap(), 42);
    }

    #[test]
    fn range_validation() {
        assert!(checked_range(0, 10, 10).is_ok());
        assert!(checked_range(5, 5, 10).is_ok());
        assert!(checked_range(5, 6, 10).is_err());
        assert!(checked_range(u64::MAX, 1, 10).is_err());
    }
}
