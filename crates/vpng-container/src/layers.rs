//! The layer table: the named grids of a v0.4 container.
//!
//! A **layer** is one grid of tiles that can be decoded on its own. Layer 0
//! is the base layer — the image itself — and every other layer is a named
//! grid that sits beside it on the same canvas. A layer may carry a **mask**:
//! another layer whose coverage modulates it.
//!
//! The table sits between the tile directory and the tile payloads, and the
//! header's last eight bytes say where it is. The layout is:
//!
//! ```text
//! offset  size  field
//!      0     4  layer count
//!      4     4  descriptor array length
//!      8     4  CRC32C over the descriptor array
//!     12     4  reserved (must be zero)
//!     16   ...  descriptor array
//! ```
//!
//! and one descriptor is:
//!
//! ```text
//! offset  size  field
//!      0     1  name length in bytes
//!      1     1  kind flags (bit 0: this layer is a mask)
//!      2     2  reserved (must be zero)
//!      4     4  x
//!      8     4  y
//!     12     4  width
//!     16     4  height
//!     20     1  opacity (0..255)
//!     21     1  reserved (must be zero)
//!     22     2  mask layer index (0xFFFF: no mask)
//!     24     8  uncompressed pixel bytes
//!     32     8  compressed payload bytes
//!     40   ...  name (UTF-8)
//! ```
//!
//! The byte counts are the layer's **own**: its level-0 pixels and the
//! payloads that hold them. A v0.4 container has no preview pyramid — the
//! pyramid arrived in v0.2 and a file may not have both — so a layer's
//! accounting is exact and needs no second rule.

use vpng_core::{Error, Rect, Result};

use crate::checksum;
use crate::leb;

/// The fixed part of the layer table: count, descriptor length, checksum and
/// one reserved word.
pub const LAYER_TABLE_HEADER_SIZE: usize = 16;

/// The fixed part of one layer descriptor. The name follows it.
pub const LAYER_DESCRIPTOR_SIZE: usize = 40;

/// The mask index that means "this layer has no mask".
pub const NO_MASK: u16 = u16::MAX;

/// The longest layer name the table can hold.
pub const MAX_NAME_LENGTH: usize = u8::MAX as usize;

/// What a layer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerKind {
    /// Ordinary picture pixels.
    Pixels,
    /// A coverage mask: its alpha channel is the coverage.
    Mask,
}

impl LayerKind {
    /// The kind's flag byte.
    pub fn flags(self) -> u8 {
        match self {
            LayerKind::Pixels => 0,
            LayerKind::Mask => 1,
        }
    }

    /// Read a kind out of its flag byte.
    pub fn from_flags(value: u8) -> Result<Self> {
        match value {
            0 => Ok(LayerKind::Pixels),
            1 => Ok(LayerKind::Mask),
            other => Err(Error::corrupt(format!(
                "unknown layer kind flag {other:#04x}"
            ))),
        }
    }

    /// Whether the layer is a mask.
    pub fn is_mask(self) -> bool {
        matches!(self, LayerKind::Mask)
    }
}

/// One layer's description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerDescriptor {
    /// The layer's name. It may be empty; it is not required to be unique.
    pub name: String,
    /// What the layer holds.
    pub kind: LayerKind,
    /// Where the layer sits on the canvas, in pixels.
    pub rect: Rect,
    /// The layer's opacity, 0 (invisible) to 255 (opaque).
    pub opacity: u8,
    /// The layer that carries this layer's mask, if any.
    pub mask: Option<u16>,
    /// The layer's uncompressed pixel bytes.
    pub uncompressed_bytes: u64,
    /// The layer's compressed payload bytes.
    pub compressed_bytes: u64,
}

impl LayerDescriptor {
    /// A plain picture layer covering `rect`.
    pub fn pixels(name: impl Into<String>, rect: Rect) -> Self {
        LayerDescriptor {
            name: name.into(),
            kind: LayerKind::Pixels,
            rect,
            opacity: 255,
            mask: None,
            uncompressed_bytes: 0,
            compressed_bytes: 0,
        }
    }

    /// A mask layer covering `rect`.
    pub fn mask(name: impl Into<String>, rect: Rect) -> Self {
        LayerDescriptor {
            name: name.into(),
            kind: LayerKind::Mask,
            rect,
            opacity: 255,
            mask: None,
            uncompressed_bytes: 0,
            compressed_bytes: 0,
        }
    }

    /// The layer's compression ratio: uncompressed bytes over compressed bytes.
    ///
    /// A ratio above 1 means the layer got smaller; a ratio of 1 means it did
    /// not; a ratio below 1 means the layer grew, which the codecs are allowed
    /// to do and the encoder never hides.
    pub fn ratio(&self) -> f64 {
        if self.compressed_bytes == 0 {
            return 1.0;
        }
        self.uncompressed_bytes as f64 / self.compressed_bytes as f64
    }

    /// Serialize the descriptor, name included.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let name = self.name.as_bytes();
        if name.len() > MAX_NAME_LENGTH {
            return Err(Error::invalid(format!(
                "the layer name is {} bytes, the format allows {MAX_NAME_LENGTH}",
                name.len()
            )));
        }
        if !self.name.is_char_boundary(name.len()) {
            return Err(Error::invalid("the layer name is not valid UTF-8"));
        }

        let mut out = Vec::with_capacity(LAYER_DESCRIPTOR_SIZE + name.len());
        out.push(name.len() as u8);
        out.push(self.kind.flags());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&self.rect.x.to_le_bytes());
        out.extend_from_slice(&self.rect.y.to_le_bytes());
        out.extend_from_slice(&self.rect.width.to_le_bytes());
        out.extend_from_slice(&self.rect.height.to_le_bytes());
        out.push(self.opacity);
        out.push(0); // reserved
        out.extend_from_slice(&self.mask.unwrap_or(NO_MASK).to_le_bytes());
        out.extend_from_slice(&self.uncompressed_bytes.to_le_bytes());
        out.extend_from_slice(&self.compressed_bytes.to_le_bytes());
        out.extend_from_slice(name);
        Ok(out)
    }

    /// Parse one descriptor from the front of `bytes`.
    ///
    /// Returns the descriptor and how many bytes it took.
    pub fn decode(bytes: &[u8]) -> Result<(LayerDescriptor, usize)> {
        if bytes.len() < LAYER_DESCRIPTOR_SIZE {
            return Err(Error::corrupt("layer descriptor is truncated"));
        }
        let name_length = bytes[0] as usize;
        let kind = LayerKind::from_flags(bytes[1])?;
        if leb::read_u16(bytes, 2) != 0 {
            return Err(Error::corrupt(
                "layer descriptor reserved bytes must be zero",
            ));
        }
        if bytes[21] != 0 {
            return Err(Error::corrupt(
                "layer descriptor reserved bytes must be zero",
            ));
        }
        let total = LAYER_DESCRIPTOR_SIZE + name_length;
        if bytes.len() < total {
            return Err(Error::corrupt("layer descriptor name is truncated"));
        }
        let name = std::str::from_utf8(&bytes[LAYER_DESCRIPTOR_SIZE..total])
            .map_err(|_| Error::corrupt("layer name is not valid UTF-8"))?
            .to_string();

        let mask_index = leb::read_u16(bytes, 22);
        let mask = if mask_index == NO_MASK {
            None
        } else {
            Some(mask_index)
        };

        Ok((
            LayerDescriptor {
                name,
                kind,
                rect: Rect::new(
                    leb::read_u32(bytes, 4),
                    leb::read_u32(bytes, 8),
                    leb::read_u32(bytes, 12),
                    leb::read_u32(bytes, 16),
                )?,
                opacity: bytes[20],
                mask,
                uncompressed_bytes: leb::read_u64(bytes, 24),
                compressed_bytes: leb::read_u64(bytes, 32),
            },
            total,
        ))
    }
}

/// The whole layer table: the container's list of named grids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayerTable {
    /// The layers, in the order the container lists them.
    pub layers: Vec<LayerDescriptor>,
}

impl LayerTable {
    /// Build a table from its layers.
    pub fn new(layers: Vec<LayerDescriptor>) -> Self {
        LayerTable { layers }
    }

    /// The number of layers.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// One layer by position.
    pub fn get(&self, index: usize) -> Option<&LayerDescriptor> {
        self.layers.get(index)
    }

    /// Serialize the table.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.layers.len() > crate::directory::MAX_LAYERS as usize {
            return Err(Error::invalid(format!(
                "a container holds at most {} layers, this one has {}",
                crate::directory::MAX_LAYERS,
                self.layers.len()
            )));
        }

        let mut descriptors = Vec::new();
        for layer in &self.layers {
            descriptors.extend_from_slice(&layer.encode()?);
        }

        let mut out = Vec::with_capacity(LAYER_TABLE_HEADER_SIZE + descriptors.len());
        out.extend_from_slice(&(self.layers.len() as u32).to_le_bytes());
        out.extend_from_slice(&(descriptors.len() as u32).to_le_bytes());
        out.extend_from_slice(&checksum::crc32c(&descriptors).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved
        out.extend_from_slice(&descriptors);
        Ok(out)
    }

    /// Parse and checksum-verify a layer table.
    pub fn decode(bytes: &[u8]) -> Result<LayerTable> {
        if bytes.len() < LAYER_TABLE_HEADER_SIZE {
            return Err(Error::corrupt("layer table is truncated"));
        }
        let count = leb::read_u32(bytes, 0);
        let descriptor_bytes = leb::read_u32(bytes, 4);
        let stored = leb::read_u32(bytes, 8);
        if leb::read_u32(bytes, 12) != 0 {
            return Err(Error::corrupt("layer table reserved bytes must be zero"));
        }

        let body = &bytes[LAYER_TABLE_HEADER_SIZE..];
        if body.len() != descriptor_bytes as usize {
            return Err(Error::corrupt(format!(
                "layer table holds {} descriptor bytes, the header says {descriptor_bytes}",
                body.len()
            )));
        }
        let computed = checksum::crc32c(body);
        if stored != computed {
            return Err(Error::corrupt(format!(
                "layer table checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            )));
        }
        if count == 0 {
            return Err(Error::corrupt("a layer table must hold at least one layer"));
        }
        if count > crate::directory::MAX_LAYERS as u32 {
            return Err(Error::corrupt(format!(
                "a layer table holds at most {} layers, this one claims {count}",
                crate::directory::MAX_LAYERS
            )));
        }

        let mut layers = Vec::with_capacity(count as usize);
        let mut cursor = 0usize;
        for _ in 0..count {
            let (layer, used) = LayerDescriptor::decode(&body[cursor..])?;
            cursor += used;
            layers.push(layer);
        }
        if cursor != body.len() {
            return Err(Error::corrupt(
                "the layer table has bytes left over after its last descriptor",
            ));
        }

        Ok(LayerTable { layers })
    }

    /// Check the table's internal references.
    ///
    /// The rules are:
    ///
    /// * layer 0 is the base layer, and it is never a mask;
    /// * a mask is named by a mask layer, and a mask layer is never named by
    ///   anything;
    /// * a mask layer's rectangle contains the rectangle of the layer it
    ///   masks.
    pub fn validate(&self) -> Result<()> {
        for (index, layer) in self.layers.iter().enumerate() {
            if index == 0 && layer.kind.is_mask() {
                return Err(Error::corrupt("the base layer cannot be a mask"));
            }
            if let Some(mask_index) = layer.mask {
                let mask = self.layers.get(mask_index as usize).ok_or_else(|| {
                    Error::corrupt(format!(
                        "layer {index} names mask layer {mask_index}, which does not exist"
                    ))
                })?;
                if !mask.kind.is_mask() {
                    return Err(Error::corrupt(format!(
                        "layer {index} names layer {mask_index} as its mask, but that layer is not a mask"
                    )));
                }
                if usize::from(mask_index) == index {
                    return Err(Error::corrupt(format!(
                        "layer {index} names itself as its own mask"
                    )));
                }
                if mask.rect.x > layer.rect.x
                    || mask.rect.y > layer.rect.y
                    || mask.rect.right()? < layer.rect.right()?
                    || mask.rect.bottom()? < layer.rect.bottom()?
                {
                    return Err(Error::corrupt(format!(
                        "the mask of layer {index} does not cover the layer's rectangle"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> LayerDescriptor {
        LayerDescriptor::pixels("base", Rect::new(0, 0, 100, 50).unwrap())
    }

    #[test]
    fn a_table_round_trips() {
        let table = LayerTable::new(vec![descriptor()]);
        let bytes = table.encode().unwrap();
        assert_eq!(
            bytes.len(),
            LAYER_TABLE_HEADER_SIZE + LAYER_DESCRIPTOR_SIZE + 4
        );
        assert_eq!(LayerTable::decode(&bytes).unwrap(), table);
    }

    #[test]
    fn a_name_travels_with_its_descriptor() {
        let mut layer = descriptor();
        layer.name = "the sky".to_string();
        let table = LayerTable::new(vec![layer]);
        let bytes = table.encode().unwrap();
        let decoded = LayerTable::decode(&bytes).unwrap();
        assert_eq!(decoded.layers[0].name, "the sky");
    }

    #[test]
    fn a_corrupt_table_is_refused() {
        let table = LayerTable::new(vec![descriptor()]);
        let mut bytes = table.encode().unwrap();
        bytes[LAYER_TABLE_HEADER_SIZE] ^= 0xff;
        assert!(LayerTable::decode(&bytes).is_err());
    }

    #[test]
    fn a_truncated_table_is_refused() {
        let table = LayerTable::new(vec![descriptor()]);
        let bytes = table.encode().unwrap();
        assert!(LayerTable::decode(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn a_mask_must_be_a_mask() {
        let mut mask = LayerDescriptor::mask("m", Rect::new(0, 0, 100, 50).unwrap());
        mask.kind = LayerKind::Pixels;
        let mut layer = descriptor();
        layer.mask = Some(1);
        let table = LayerTable::new(vec![layer, mask]);
        assert!(table.validate().is_err());
    }

    #[test]
    fn a_mask_must_cover_its_layer() {
        let mask = LayerDescriptor::mask("m", Rect::new(0, 0, 10, 10).unwrap());
        let mut layer = descriptor();
        layer.mask = Some(1);
        let table = LayerTable::new(vec![layer, mask]);
        assert!(table.validate().is_err());
    }

    #[test]
    fn the_base_layer_is_not_a_mask() {
        let table = LayerTable::new(vec![LayerDescriptor::mask(
            "m",
            Rect::new(0, 0, 10, 10).unwrap(),
        )]);
        assert!(table.validate().is_err());
    }

    #[test]
    fn a_layer_reports_its_own_ratio() {
        let mut layer = descriptor();
        layer.uncompressed_bytes = 1000;
        layer.compressed_bytes = 250;
        assert_eq!(layer.ratio(), 4.0);
    }
}
