//! A source of canonical pixels the encoder can read one tile at a time.

use vpng_core::{CanonicalImage, PixelFormat, Result};
use vpng_tiling::TileGeometry;

/// A source of canonical pixels the encoder pulls one tile at a time.
///
/// This is the encoder's counterpart to the decoder's tile source. A source
/// that can produce one tile without holding the whole image is what lets the
/// encoder stream: the encoder asks for tile 0, encodes it, asks for tile 1,
/// and so on, and never allocates the whole picture.
///
/// The trait requires `Sync` because the parallel encoder hands the same
/// source to every worker. A source must therefore produce the same pixels
/// for the same tile every time it is asked: the encoder's output must not
/// depend on which worker asked, or when.
pub trait PixelSource: Sync {
    /// The image's width in pixels.
    fn width(&self) -> u32;

    /// The image's height in pixels.
    fn height(&self) -> u32;

    /// The image's pixel format.
    fn format(&self) -> PixelFormat;

    /// Write the canonical pixels of tile `index` into `out`.
    ///
    /// `geom` is the tile's geometry, already clipped to the image's edge.
    /// The implementation must leave `out` holding exactly
    /// `geom.width * geom.height * bytes_per_pixel` bytes.
    fn tile_pixels(&self, index: usize, geom: &TileGeometry, out: &mut Vec<u8>) -> Result<()>;
}

impl PixelSource for CanonicalImage {
    fn width(&self) -> u32 {
        self.dimensions().width
    }

    fn height(&self) -> u32 {
        self.dimensions().height
    }

    fn format(&self) -> PixelFormat {
        CanonicalImage::format(self)
    }

    fn tile_pixels(&self, _index: usize, geom: &TileGeometry, out: &mut Vec<u8>) -> Result<()> {
        crate::extract_tile(self, geom, out)
    }
}
