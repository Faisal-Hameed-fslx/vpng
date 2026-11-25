//! Pixel formats and regions as Python sees them.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use vpng_core::{CanonicalImage, ColorModel, Error, PixelFormat, Rect, Result};

/// The canonical pixel formats, as the Python side spells them.
pub const FORMAT_NAMES: [&str; 8] = [
    "gray8", "gray16", "graya8", "graya16", "rgb8", "rgb16", "rgba8", "rgba16",
];

/// The name of a pixel format: `rgba8`, `rgb16`, and so on.
pub fn format_name(format: PixelFormat) -> String {
    match (format.color_model, format.bits) {
        (ColorModel::Gray, _) => format!("gray{}", format.bits.bits()),
        (ColorModel::GrayAlpha, _) => format!("graya{}", format.bits.bits()),
        (ColorModel::Rgb, _) => format!("rgb{}", format.bits.bits()),
        (ColorModel::Rgba, _) => format!("rgba{}", format.bits.bits()),
        (ColorModel::Indexed, _) => "indexed".to_string(),
    }
}

/// Parse a pixel-format name such as `rgba8` or `rgb16`.
pub fn parse_format(text: &str) -> Result<PixelFormat> {
    let lowered = text.to_ascii_lowercase();
    let format = match lowered.as_str() {
        "gray8" | "gray" | "g8" | "l8" => PixelFormat::gray8(),
        "gray16" | "g16" | "l16" => PixelFormat::gray16(),
        "graya8" | "gray-alpha8" | "ga8" | "la8" => PixelFormat::gray_alpha8(),
        "graya16" | "gray-alpha16" | "ga16" | "la16" => PixelFormat::gray_alpha16(),
        "rgb8" | "rgb" => PixelFormat::rgb8(),
        "rgb16" => PixelFormat::rgb16(),
        "rgba8" | "rgba" => PixelFormat::rgba8(),
        "rgba16" => PixelFormat::rgba16(),
        other => {
            return Err(Error::invalid(format!(
                "unknown pixel format `{other}` (try rgba8, rgb16, gray8, graya16, ...)"
            )))
        }
    };
    Ok(format)
}

/// A rectangle of canonical pixels.
///
/// The bytes are **canonical**: rows are tightly packed with no padding, and
/// a 16-bit sample is little-endian. That is the same order the format
/// stores, so a region can be handed straight to `write_region`.
#[pyclass(module = "vpng")]
#[derive(Clone)]
pub struct Region {
    /// The canonical pixel bytes.
    data: Vec<u8>,
    /// The region's width in pixels.
    width: u32,
    /// The region's height in pixels.
    height: u32,
    /// The region's left edge in the picture.
    x: u32,
    /// The region's top edge in the picture.
    y: u32,
    /// The picture's pixel format.
    format: PixelFormat,
}

impl Region {
    /// Build a region from a decoded image.
    pub fn from_image(image: &CanonicalImage, rect: Rect) -> Self {
        Region {
            data: image.data().to_vec(),
            width: rect.width,
            height: rect.height,
            x: rect.x,
            y: rect.y,
            format: image.format(),
        }
    }

    /// The canonical pixel bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// The region's width in pixels.
    pub fn rect_width(&self) -> u32 {
        self.width
    }

    /// The region's height in pixels.
    pub fn rect_height(&self) -> u32 {
        self.height
    }

    /// The region's left edge in the picture.
    pub fn rect_x(&self) -> u32 {
        self.x
    }

    /// The region's top edge in the picture.
    pub fn rect_y(&self) -> u32 {
        self.y
    }
}

#[pymethods]
impl Region {
    /// The canonical pixel bytes of the region.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// The region's width in pixels.
    #[getter]
    fn width(&self) -> u32 {
        self.width
    }

    /// The region's height in pixels.
    #[getter]
    fn height(&self) -> u32 {
        self.height
    }

    /// The region's left edge in the picture.
    #[getter]
    fn x(&self) -> u32 {
        self.x
    }

    /// The region's top edge in the picture.
    #[getter]
    fn y(&self) -> u32 {
        self.y
    }

    /// The picture's pixel format, as a string such as `rgba8`.
    #[getter]
    fn format(&self) -> String {
        format_name(self.format)
    }

    /// The number of bytes in the region.
    fn __len__(&self) -> usize {
        self.data.len()
    }

    /// The region's bytes, so `bytes(region)` works.
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!(
            "Region(x={}, y={}, width={}, height={}, format={})",
            self.x,
            self.y,
            self.width,
            self.height,
            format_name(self.format)
        )
    }
}
