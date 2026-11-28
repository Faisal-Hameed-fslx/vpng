//! The fixtures the editor's tests share.
//!
//! Every picture here is **known**: the red channel is the column, the green
//! channel is the row, the blue channel is the sum of the two, and the alpha
//! channel is 255. A test can therefore say exactly what a pixel must be
//! after an edit, without keeping a reference file beside the code.

// Each test binary uses a different part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use vpng_core::{CanonicalImage, ColorModel, PixelFormat};
use vpng_encoder::{encode, EncodeOptions};

/// A picture whose every pixel is known.
pub fn gradient(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
    let channels = format.channels() as usize;
    let bytes = format.bytes_per_channel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * channels * bytes];
    for y in 0..height {
        for x in 0..width {
            let index = (y as usize * width as usize + x as usize) * channels * bytes;
            let samples: Vec<u16> = match format.color_model {
                ColorModel::Gray => vec![(x + y) as u16],
                ColorModel::GrayAlpha => vec![(x + y) as u16, 255],
                ColorModel::Rgb => vec![x as u16, y as u16, (x + y) as u16],
                ColorModel::Rgba => vec![x as u16, y as u16, (x + y) as u16, 255],
                ColorModel::Indexed => unreachable!("the format cannot be built"),
            };
            for (channel, value) in samples.iter().enumerate() {
                let scaled = if bytes == 1 {
                    u16::from(*value as u8)
                } else {
                    value * 257
                };
                let offset = index + channel * bytes;
                if bytes == 1 {
                    data[offset] = scaled as u8;
                } else {
                    data[offset] = scaled as u8;
                    data[offset + 1] = (scaled >> 8) as u8;
                }
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).expect("the fixture is well formed")
}

/// A directory for one test.
pub fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-ui-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).expect("the scratch directory can be created");
    directory
}

/// Write a picture to a file and return the path.
pub fn write_picture(directory: &Path, name: &str, image: &CanonicalImage) -> PathBuf {
    let report = encode(image, &EncodeOptions::default()).expect("the picture encodes");
    let path = directory.join(name);
    std::fs::write(&path, &report.bytes).expect("the file can be written");
    path
}

/// The display colour of the pixel at `(x, y)` in a gradient.
pub fn expected(x: u32, y: u32, format: PixelFormat) -> [u8; 4] {
    match format.color_model {
        ColorModel::Gray => {
            let grey = (x + y) as u8;
            [grey, grey, grey, 255]
        }
        ColorModel::GrayAlpha => {
            let grey = (x + y) as u8;
            [grey, grey, grey, 255]
        }
        ColorModel::Rgb => [x as u8, y as u8, (x + y) as u8, 255],
        ColorModel::Rgba => [x as u8, y as u8, (x + y) as u8, 255],
        ColorModel::Indexed => unreachable!("the format cannot be built"),
    }
}
