//! `vpng-testkit` — deterministic fixtures shared by the conformance tests.
//!
//! Everything here is seeded and reproducible: no `rand`, no clock, no
//! platform-specific behavior. Golden files depend on that.

#![forbid(unsafe_code)]

use vpng_core::{CanonicalImage, Limits, PixelFormat};

/// A tiny deterministic PRNG (xorshift64*). Not cryptographic; used only to
/// build reproducible test images.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn next_u8(&mut self) -> u8 {
        (self.next_u64() >> 33) as u8
    }

    pub fn fill(&mut self, out: &mut [u8]) {
        for byte in out.iter_mut() {
            *byte = self.next_u8();
        }
    }
}

/// A smooth gradient, one distinct value per pixel channel.
pub fn gradient(width: u32, height: u32, format: PixelFormat, seed: u32) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    let mut rng = Rng::new(seed as u64 + 1);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = (y * width as usize + x) * bpp;
            for c in 0..bpp {
                data[base + c] = (((x * 5 + y * 11 + c * 37 + seed as usize) & 0xff) as u8)
                    .wrapping_add(rng.next_u8() % 2);
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// Uniform pseudo-random noise. The worst case for any compressor.
pub fn noise(width: u32, height: u32, format: PixelFormat, seed: u64) -> CanonicalImage {
    let bytes = format.frame_bytes(width, height).unwrap() as usize;
    let mut data = vec![0u8; bytes];
    let mut rng = Rng::new(seed);
    rng.fill(&mut data);
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// A two-color checkerboard.
pub fn checker(width: u32, height: u32, format: PixelFormat, square: u32) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    let square = square.max(1);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = (y * width as usize + x) * bpp;
            let dark = ((x as u32 / square) + (y as u32 / square)) % 2 == 0;
            for c in 0..bpp {
                let v = if dark { 0x10 } else { 0xEF };
                data[base + c] = if c == bpp - 1 && format.color_model.has_alpha() {
                    0xFF
                } else {
                    v
                };
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// Every format enabled in v0.1, plus a few representative sizes.
pub const V0_1_FORMATS: [PixelFormat; 8] = [
    PixelFormat::gray8(),
    PixelFormat::gray16(),
    PixelFormat::gray_alpha8(),
    PixelFormat::gray_alpha16(),
    PixelFormat::rgb8(),
    PixelFormat::rgb16(),
    PixelFormat::rgba8(),
    PixelFormat::rgba16(),
];

/// Sizes that exercise exact fit, single edge tiles, and double edge tiles.
pub const SIZES: [(u32, u32); 6] = [(1, 1), (7, 3), (64, 64), (256, 256), (257, 130), (300, 180)];

pub fn default_limits() -> Limits {
    Limits::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn generators_produce_expected_sizes() {
        for &format in V0_1_FORMATS.iter() {
            for &(w, h) in SIZES.iter() {
                let expected = format.frame_bytes(w, h).unwrap() as usize;
                assert_eq!(gradient(w, h, format, 7).len(), expected);
                assert_eq!(noise(w, h, format, 7).len(), expected);
                assert_eq!(checker(w, h, format, 4).len(), expected);
            }
        }
    }
}
