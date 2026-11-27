//! Encoding several layers into one container.
//!
//! A **layer** is one grid of tiles that can be decoded on its own. Layer 0
//! is the base layer — the image itself — and every other layer is a named
//! grid that sits beside it on the same canvas. A layer may carry a **mask**:
//! another layer whose coverage modulates it.
//!
//! The rules the encoder enforces, and the decoder re-checks:
//!
//! * the base layer covers the whole canvas;
//! * every other layer's rectangle lies inside the canvas;
//! * every layer's pixels are in the file's one pixel format;
//! * a mask is a mask layer, and it covers the rectangle of the layer it
//!   masks;
//! * a mask needs a format with an alpha channel, because the coverage is
//!   the alpha channel;
//! * **cross-layer prediction is explicitly out of scope**: every layer is
//!   encoded on its own, and no layer's pixels ever influence another's.
//!
//! A file with layers is a **v0.4** file. A file with no layers is not: the
//! default path still writes the v0.1 bytes the golden fixtures pin.
//!
//! Each layer's tiles form a uniform grid over the layer's own rectangle,
//! in canvas coordinates. The adaptive partition of v0.2 is a property of
//! the base image and is not available inside a layer in v0.4.

use vpng_container::{LayerKind, MAX_LAYERS};
use vpng_core::{Dimensions, Error, PixelFormat, Rect, Result};
use vpng_tiling::{Partition, TileRect};

use crate::source::PixelSource;
use crate::{EncodeOptions, EncodeReport, EncodedLevel, LayerMeta};

/// A pixel source that reads a layer's pixels from a source whose tiles are
/// in **canvas** coordinates.
///
/// A layer's tiles carry canvas coordinates, so a reader can place them
/// without knowing where the layer sits. The pixels themselves live in the
/// layer's own rectangle, so the coordinates are translated on the way in.
///
/// This is the only place the two coordinate systems meet. Everything else
/// works in canvas coordinates.
pub(crate) struct LayerSource<'a> {
    inner: &'a dyn PixelSource,
    origin_x: u32,
    origin_y: u32,
}

impl<'a> LayerSource<'a> {
    /// Read a layer that sits at `origin` on the canvas.
    pub(crate) fn new(inner: &'a dyn PixelSource, origin: Rect) -> Self {
        LayerSource {
            inner,
            origin_x: origin.x,
            origin_y: origin.y,
        }
    }
}

impl PixelSource for LayerSource<'_> {
    fn width(&self) -> u32 {
        self.inner.width()
    }

    fn height(&self) -> u32 {
        self.inner.height()
    }

    fn format(&self) -> PixelFormat {
        self.inner.format()
    }

    fn tile_pixels(&self, index: usize, rect: &TileRect, out: &mut Vec<u8>) -> Result<()> {
        let local = TileRect::new(
            rect.x - self.origin_x,
            rect.y - self.origin_y,
            rect.width,
            rect.height,
        )?;
        self.inner.tile_pixels(index, &local, out)
    }
}

/// One layer of a file to encode.
pub struct LayerInput<'a> {
    /// The layer's name. It may be empty; it is not required to be unique.
    pub name: String,
    /// What the layer holds.
    pub kind: LayerKind,
    /// Where the layer sits on the canvas, in pixels.
    pub rect: Rect,
    /// The layer's opacity, 0 (invisible) to 255 (opaque).
    pub opacity: u8,
    /// The index of the layer that carries this layer's mask, if any.
    pub mask: Option<usize>,
    /// The layer's pixels.
    pub source: &'a dyn PixelSource,
}

impl<'a> LayerInput<'a> {
    /// A plain picture layer.
    pub fn pixels(name: impl Into<String>, rect: Rect, source: &'a dyn PixelSource) -> Self {
        LayerInput {
            name: name.into(),
            kind: LayerKind::Pixels,
            rect,
            opacity: 255,
            mask: None,
            source,
        }
    }

    /// A mask layer.
    pub fn mask(name: impl Into<String>, rect: Rect, source: &'a dyn PixelSource) -> Self {
        LayerInput {
            name: name.into(),
            kind: LayerKind::Mask,
            rect,
            opacity: 255,
            mask: None,
            source,
        }
    }

    /// Set the layer's opacity.
    pub fn with_opacity(mut self, opacity: u8) -> Self {
        self.opacity = opacity;
        self
    }

    /// Set the layer's mask.
    pub fn with_mask(mut self, mask: usize) -> Self {
        self.mask = Some(mask);
        self
    }
}

/// A layer that has been checked and planned: its metadata, its partition and
/// its source.
pub(crate) struct PlannedLayer<'a> {
    pub(crate) name: String,
    pub(crate) kind: LayerKind,
    pub(crate) rect: Rect,
    pub(crate) opacity: u8,
    pub(crate) mask: Option<u16>,
    pub(crate) partition: Partition,
    pub(crate) source: &'a dyn PixelSource,
}

impl PlannedLayer<'_> {
    /// The layer's metadata, for the layer table.
    pub(crate) fn meta(&self) -> LayerMeta {
        LayerMeta {
            name: self.name.clone(),
            kind: self.kind,
            rect: self.rect,
            opacity: self.opacity,
            mask: self.mask,
        }
    }
}

/// Check the layers and plan each one's partition.
///
/// The result is the canvas's dimensions, the file's pixel format and the
/// planned layers, in order.
pub(crate) fn plan_layers<'a>(
    layers: &[LayerInput<'a>],
    options: &EncodeOptions,
) -> Result<(Vec<PlannedLayer<'a>>, Dimensions, PixelFormat)> {
    if layers.is_empty() {
        return Err(Error::invalid("a file needs at least one layer"));
    }
    if layers.len() > MAX_LAYERS as usize {
        return Err(Error::invalid(format!(
            "a container holds at most {MAX_LAYERS} layers, this one has {}",
            layers.len()
        )));
    }
    if options.pyramid {
        return Err(Error::invalid(
            "a file with layers may not also have a preview pyramid in v0.4",
        ));
    }

    let format = layers[0].source.format();
    let canvas = Rect::new(0, 0, layers[0].source.width(), layers[0].source.height())?;
    let canvas_dims = Dimensions {
        width: canvas.width,
        height: canvas.height,
    };
    if layers[0].rect != canvas {
        return Err(Error::invalid(format!(
            "the base layer covers {}x{}+{}+{}, the canvas is {}x{}",
            layers[0].rect.width,
            layers[0].rect.height,
            layers[0].rect.x,
            layers[0].rect.y,
            canvas.width,
            canvas.height
        )));
    }

    let mut planned = Vec::with_capacity(layers.len());
    for (index, layer) in layers.iter().enumerate() {
        if layer.source.format() != format {
            return Err(Error::invalid(format!(
                "layer {index} is in a different pixel format than the base layer"
            )));
        }
        if layer.source.width() != layer.rect.width || layer.source.height() != layer.rect.height {
            return Err(Error::invalid(format!(
                "layer {index} holds {}x{} pixels but its rectangle is {}x{}",
                layer.source.width(),
                layer.source.height(),
                layer.rect.width,
                layer.rect.height
            )));
        }
        if !layer.rect.is_within(canvas_dims) {
            return Err(Error::invalid(format!(
                "layer {index} covers {}x{}+{}+{}, which is outside the {}x{} canvas",
                layer.rect.width,
                layer.rect.height,
                layer.rect.x,
                layer.rect.y,
                canvas.width,
                canvas.height
            )));
        }
        if index == 0 && layer.kind.is_mask() {
            return Err(Error::invalid("the base layer cannot be a mask"));
        }
        if layer.kind.is_mask() && !format.color_model.has_alpha() {
            return Err(Error::unsupported(
                "a mask needs a pixel format with an alpha channel, because the coverage is the alpha channel",
            ));
        }

        let mask = match layer.mask {
            None => None,
            Some(mask_index) => {
                let mask_layer = layers.get(mask_index).ok_or_else(|| {
                    Error::invalid(format!(
                        "layer {index} names layer {mask_index} as its mask, which does not exist"
                    ))
                })?;
                if !mask_layer.kind.is_mask() {
                    return Err(Error::invalid(format!(
                        "layer {index} names layer {mask_index} as its mask, but that layer is not a mask"
                    )));
                }
                if mask_index == index {
                    return Err(Error::invalid(format!(
                        "layer {index} names itself as its own mask"
                    )));
                }
                if mask_layer.rect.x > layer.rect.x
                    || mask_layer.rect.y > layer.rect.y
                    || mask_layer.rect.right()? < layer.rect.right()?
                    || mask_layer.rect.bottom()? < layer.rect.bottom()?
                {
                    return Err(Error::invalid(format!(
                        "the mask of layer {index} does not cover the layer's rectangle"
                    )));
                }
                Some(mask_index as u16)
            }
        };

        let partition = uniform_layer_partition(layer.rect, options)?;
        options.limits.validate_tile_count(partition.len() as u32)?;

        planned.push(PlannedLayer {
            name: layer.name.clone(),
            kind: layer.kind,
            rect: layer.rect,
            opacity: layer.opacity,
            mask,
            partition,
            source: layer.source,
        });
    }

    Ok((planned, canvas_dims, format))
}

/// The uniform partition of a layer's rectangle, in canvas coordinates.
fn uniform_layer_partition(rect: Rect, options: &EncodeOptions) -> Result<Partition> {
    let mut tiles = Vec::new();
    let mut y = rect.y;
    while y < rect.bottom()? {
        let height = options.tile_height.min(rect.bottom()? - y);
        let mut x = rect.x;
        while x < rect.right()? {
            let width = options.tile_width.min(rect.right()? - x);
            tiles.push(TileRect::new(x, y, width, height)?);
            x += width;
        }
        y += height;
    }
    Partition::from_tiles(rect, tiles)
}

/// Encode several layers into a complete `.vpng` byte stream.
///
/// The file is a **v0.4** file: it carries a layer table, and its directory
/// entries name the layer each tile belongs to. The layers are encoded
/// independently of one another — cross-layer prediction is out of scope —
/// and each layer's bytes are accounted for on its own.
///
/// The result is byte-identical to
/// [`crate::stream::encode_layers_to_file`] and to
/// [`encode_layers_parallel`], whatever the worker count is.
pub fn encode_layers(layers: &[LayerInput<'_>], options: &EncodeOptions) -> Result<EncodeReport> {
    encode_layers_parallel(
        layers,
        options,
        &crate::parallel::ParallelOptions::default(),
    )
}

/// Encode several layers, with the parallel encoder.
///
/// The worker count changes the speed and not the bytes: the layers are
/// encoded one after the other, and the tiles within a layer are consumed in
/// tile order, so one worker and N workers write the same file.
pub fn encode_layers_parallel(
    layers: &[LayerInput<'_>],
    options: &EncodeOptions,
    parallel: &crate::parallel::ParallelOptions,
) -> Result<EncodeReport> {
    let (planned, dims, format) = plan_layers(layers, options)?;

    let mut levels: Vec<EncodedLevel> = Vec::with_capacity(planned.len());
    let sources: Vec<LayerSource<'_>> = planned
        .iter()
        .map(|layer| LayerSource::new(layer.source, layer.rect))
        .collect();
    for (index, layer) in planned.iter().enumerate() {
        let payloads = crate::parallel::encode_all_tiles(
            &sources[index] as &dyn PixelSource,
            &layer.partition,
            options,
            parallel,
        )?;
        levels.push(EncodedLevel {
            index: 0,
            layer: index as u16,
            partition: layer.partition.clone(),
            payloads,
        });
    }

    let meta: Vec<LayerMeta> = planned.iter().map(|layer| layer.meta()).collect();

    crate::assemble(dims, format, options, &levels, Some(&meta))
}
