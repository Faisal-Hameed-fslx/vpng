//! The layers of a container: the named grids of a v0.4 file.
//!
//! A **layer** is one grid of tiles that can be decoded on its own. Every
//! container has at least one: layer 0 is the **base layer**, the image
//! itself. A v0.4 container may hold more, each with a name, a rectangle on
//! the canvas, an opacity and — optionally — a mask.
//!
//! The layers are derived from the directory and the layer table by
//! [`derive_layers`], the same way a v0.1 reader derives the tile grid from
//! the directory. It is one implementation, shared by the in-memory decoder
//! and the file-backed reader, so the two cannot disagree about what a layer
//! is.
//!
//! # The rules
//!
//! * The directory entries are in canonical `(layer, level, y, x)` order, and
//!   each layer's entries are contiguous.
//! * A layered container has **no** preview pyramid: every entry is at level
//!   0. The pyramid arrived in v0.2 and a v0.4 file may not have both.
//! * Each layer's tiles are an **exact partition** of the layer's rectangle.
//! * Layer 0's rectangle is the whole canvas, and every other layer's
//!   rectangle lies inside it.
//! * A layer's byte accounting matches the directory exactly: its
//!   uncompressed bytes are its level-0 pixels, and its compressed bytes are
//!   all of its payloads.
//!
//! A container with no layer table still has exactly one layer, so a caller
//! never has to ask "is this a layered file?" before asking "what is its
//! base layer?".

use vpng_container::{Directory, Header, LayerKind, LayerTable, MINOR_V0_4};
use vpng_core::{Dimensions, Error, Rect, Result};
use vpng_tiling::{Partition, TileRect};

use crate::levels::{derive_levels, Level};

/// One layer of a container.
#[derive(Debug, Clone)]
pub struct Layer {
    index: u16,
    name: String,
    kind: LayerKind,
    rect: Rect,
    opacity: u8,
    mask: Option<u16>,
    levels: Vec<Level>,
    uncompressed_bytes: u64,
    compressed_bytes: u64,
}

impl Layer {
    /// The layer's index. Layer 0 is the base layer.
    pub fn index(&self) -> u16 {
        self.index
    }

    /// The layer's name. It may be empty; it is not required to be unique.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the layer holds.
    pub fn kind(&self) -> LayerKind {
        self.kind
    }

    /// Whether the layer is a mask rather than picture pixels.
    pub fn is_mask(&self) -> bool {
        self.kind.is_mask()
    }

    /// The layer's rectangle on the canvas, in pixels.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The layer's opacity, 0 (invisible) to 255 (opaque).
    pub fn opacity(&self) -> u8 {
        self.opacity
    }

    /// The layer that carries this layer's mask, if any.
    pub fn mask(&self) -> Option<u16> {
        self.mask
    }

    /// The layer's levels.
    ///
    /// A layered container has exactly one level per layer. A container with
    /// no layer table has the pyramid, and it is all in layer 0.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The number of levels in the layer.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The layer's level 0: its original pixels.
    pub fn base_level(&self) -> &Level {
        &self.levels[0]
    }

    /// The layer's exact partition.
    pub fn partition(&self) -> &Partition {
        self.base_level().partition()
    }

    /// The layer's uncompressed pixel bytes: level 0's pixels.
    pub fn uncompressed_bytes(&self) -> u64 {
        self.uncompressed_bytes
    }

    /// The layer's compressed payload bytes: every payload it owns.
    pub fn compressed_bytes(&self) -> u64 {
        self.compressed_bytes
    }

    /// The layer's compression ratio: uncompressed bytes over compressed
    /// bytes.
    ///
    /// A ratio above 1 means the layer got smaller; a ratio of 1 means it did
    /// not; a ratio below 1 means it grew. The container never hides any of
    /// the three.
    pub fn ratio(&self) -> f64 {
        if self.compressed_bytes == 0 {
            return 1.0;
        }
        self.uncompressed_bytes as f64 / self.compressed_bytes as f64
    }
}

/// Derive a container's layers from its header, directory and layer table.
///
/// `table` is `None` for every container written before v0.4. Such a
/// container has exactly one layer — the base layer — and the pyramid, if it
/// has one, belongs to that layer.
pub fn derive_layers(
    header: &Header,
    directory: &Directory,
    table: Option<&LayerTable>,
    dims: Dimensions,
) -> Result<Vec<Layer>> {
    match table {
        None => derive_single_layer(header, directory, dims),
        Some(table) => derive_named_layers(header, directory, table, dims),
    }
}

/// The one layer of a container that has no layer table.
fn derive_single_layer(
    header: &Header,
    directory: &Directory,
    dims: Dimensions,
) -> Result<Vec<Layer>> {
    if header.has_layers() {
        return Err(Error::corrupt(
            "the header declares a layer table but none was supplied",
        ));
    }
    let levels = derive_levels(header, directory, dims)?;
    let base = &levels[0];
    let (uncompressed, compressed) = accounting(directory, base);
    Ok(vec![Layer {
        index: 0,
        name: String::new(),
        kind: LayerKind::Pixels,
        rect: Rect::new(0, 0, dims.width, dims.height)?,
        opacity: 255,
        mask: None,
        levels,
        uncompressed_bytes: uncompressed,
        compressed_bytes: compressed,
    }])
}

/// The layers of a v0.4 container.
fn derive_named_layers(
    header: &Header,
    directory: &Directory,
    table: &LayerTable,
    dims: Dimensions,
) -> Result<Vec<Layer>> {
    if header.minor < MINOR_V0_4 {
        return Err(Error::corrupt(format!(
            "a layer table in a v0.{} file is not defined",
            header.minor
        )));
    }
    if !header.has_layers() {
        return Err(Error::corrupt(
            "the header does not declare the layer table that was supplied",
        ));
    }
    table.validate()?;

    let entries = &directory.entries;

    // The entries are in canonical `(layer, level, y, x)` order. A file whose
    // entries are out of order is corrupt: a reader must be able to find a
    // layer's tiles without scanning the whole directory.
    for index in 1..entries.len() {
        let previous = (
            entries[index - 1].layer(),
            entries[index - 1].level(),
            entries[index - 1].y,
            entries[index - 1].x,
        );
        let current = (
            entries[index].layer(),
            entries[index].level(),
            entries[index].y,
            entries[index].x,
        );
        if current <= previous {
            return Err(Error::corrupt(format!(
                "tile entries are not in canonical (layer, level, y, x) order: \
                 entry {index} ({}, {}, {}) does not follow ({}, {}, {})",
                current.0, current.1, current.2, previous.0, previous.1, previous.2
            )));
        }
    }

    let mut layers = Vec::with_capacity(table.len());
    let mut cursor = 0usize;

    for (index, descriptor) in table.layers.iter().enumerate() {
        let layer_index = index as u16;
        let start = cursor;
        while cursor < entries.len() && entries[cursor].layer() == layer_index {
            cursor += 1;
        }
        let end = cursor;

        if start == end {
            return Err(Error::corrupt(format!("layer {index} has no tiles")));
        }

        // A layered container has no preview pyramid: every entry is at
        // level 0.
        for entry in &entries[start..end] {
            if entry.level() != 0 {
                return Err(Error::corrupt(format!(
                    "layer {index} holds a tile at pyramid level {}, which a layered container does not have",
                    entry.level()
                )));
            }
        }

        if !descriptor.rect.is_within(dims) {
            return Err(Error::corrupt(format!(
                "layer {index} covers {}x{}+{}+{}, which is outside the {}x{} canvas",
                descriptor.rect.width,
                descriptor.rect.height,
                descriptor.rect.x,
                descriptor.rect.y,
                dims.width,
                dims.height
            )));
        }
        if index == 0 && (descriptor.rect.x != 0 || descriptor.rect.y != 0) {
            return Err(Error::corrupt(
                "the base layer must start at the canvas origin",
            ));
        }

        let tiles: Vec<TileRect> = entries[start..end]
            .iter()
            .map(|entry| TileRect {
                x: entry.x,
                y: entry.y,
                width: entry.width as u32,
                height: entry.height as u32,
            })
            .collect();

        let region = bounding_region(&tiles)?;
        if region != descriptor.rect {
            return Err(Error::corrupt(format!(
                "layer {index} covers {}x{}+{}+{}, the table says {}x{}+{}+{}",
                region.width,
                region.height,
                region.x,
                region.y,
                descriptor.rect.width,
                descriptor.rect.height,
                descriptor.rect.x,
                descriptor.rect.y
            )));
        }
        let partition = Partition::from_tiles(region, tiles)?;

        let level = Level::new(
            0,
            false,
            region_dimensions(region),
            start,
            end - start,
            partition,
        );
        let (uncompressed, compressed) = accounting(directory, &level);
        if uncompressed != descriptor.uncompressed_bytes
            || compressed != descriptor.compressed_bytes
        {
            return Err(Error::corrupt(format!(
                "layer {index} accounts for {} uncompressed and {} compressed bytes, the directory says {uncompressed} and {compressed}",
                descriptor.uncompressed_bytes, descriptor.compressed_bytes
            )));
        }

        layers.push(Layer {
            index: layer_index,
            name: descriptor.name.clone(),
            kind: descriptor.kind,
            rect: descriptor.rect,
            opacity: descriptor.opacity,
            mask: descriptor.mask,
            levels: vec![level],
            uncompressed_bytes: uncompressed,
            compressed_bytes: compressed,
        });
    }

    if cursor != entries.len() {
        return Err(Error::corrupt(
            "the directory holds tiles that belong to no layer",
        ));
    }

    Ok(layers)
}

/// The layer's uncompressed and compressed byte counts, from the directory.
fn accounting(directory: &Directory, level: &Level) -> (u64, u64) {
    let mut uncompressed = 0u64;
    let mut compressed = 0u64;
    for position in 0..level.tile_count() {
        if let Some(entry) = level
            .entry_index(position)
            .and_then(|i| directory.entries.get(i))
        {
            uncompressed += entry.uncompressed_length as u64;
            compressed += entry.compressed_length;
        }
    }
    (uncompressed, compressed)
}

/// The smallest rectangle that contains every tile.
///
/// Unlike a pyramid level — which is anchored at the origin — a layer may sit
/// anywhere on the canvas, so the bounding box is the real one.
fn bounding_region(tiles: &[TileRect]) -> Result<Rect> {
    let mut left = u32::MAX;
    let mut top = u32::MAX;
    let mut right = 0u32;
    let mut bottom = 0u32;
    for tile in tiles {
        left = left.min(tile.x);
        top = top.min(tile.y);
        right = right.max(tile.right()?);
        bottom = bottom.max(tile.bottom()?);
    }
    if tiles.is_empty() {
        return Rect::new(0, 0, 0, 0);
    }
    Rect::new(left, top, right - left, bottom - top)
}

/// The dimensions of a rectangle.
fn region_dimensions(region: Rect) -> Dimensions {
    Dimensions {
        width: region.width,
        height: region.height,
    }
}
