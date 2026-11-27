//! The VPNG Python extension.
//!
//! This module is the **only** place Python touches the format. It is a thin
//! wrapper: every pixel operation is the same Rust code the command line
//! runs, so the two cannot disagree about what a file means.
//!
//! The Python package `vpng` wraps this module and adds the exception
//! hierarchy; see `python/vpng/__init__.py`.
//!
//! # The rules this module keeps
//!
//! * **Lossless, always.** There is no lossy mode and no lossy fallback. A
//!   request for one is refused, not quietly ignored.
//! * **Exact.** `verify` compares the decoded pixels with the original, byte
//!   for byte.
//! * **Deterministic.** The same pixels and the same options produce the
//!   same file, whatever the worker count.
//! * **Never a partial file.** An encode writes to a temporary file and
//!   renames it over the target, so a reader never sees half an image.

#![forbid(unsafe_code)]

mod document;
mod error;
mod image;
mod report;

use std::path::{Path, PathBuf};

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use vpng_core::{CanonicalImage, Error, Limits, PixelFormat, Rect, Result};
use vpng_decoder::Document;
use vpng_editing::DEFAULT_CACHE_BYTES;
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::EncodeOptions;
use vpng_image::FileKind;

use crate::document::Document as PyDocument;
use crate::error::{io_result, register_exception_classes, to_py_result};
use crate::image::{format_name, parse_format};
use crate::report::{CheckResult, DecodeResult, EncodeResult};

/// The version of the VPNG container this extension writes.
const FORMAT_VERSION: &str = "0.4";

/// The default tile edge.
const DEFAULT_TILE_SIZE: u32 = vpng_encoder::DEFAULT_TILE_SIZE;

/// Read a picture from a PNG or a `.vpng` file.
fn read_picture(path: &Path, limits: &Limits) -> Result<CanonicalImage> {
    let text = path.to_string_lossy().into_owned();
    match vpng_image::kind_from_path(&text) {
        Some(FileKind::Png) => vpng_image::read_png(&text, limits),
        Some(FileKind::Vpng) => {
            let bytes = std::fs::read(path)?;
            Document::parse(&bytes, limits)?.decode_all(limits)
        }
        Some(FileKind::Raw) => Err(Error::unsupported(
            "a raw file needs an explicit width, height and format; \
             use the PNG or VPNG form instead",
        )),
        None => Err(Error::invalid(format!(
            "cannot infer the format of `{text}`; use a .png or a .vpng file"
        ))),
    }
}

/// Write a picture to a PNG or a raw file.
fn write_picture(path: &Path, image: &CanonicalImage) -> Result<()> {
    let text = path.to_string_lossy().into_owned();
    match vpng_image::kind_from_path(&text) {
        Some(FileKind::Raw) => vpng_image::write_raw(&text, image),
        _ => vpng_image::write_png(&text, image),
    }
}

/// Register the Python exception classes.
///
/// This is called once, by `python/vpng/__init__.py`.
#[pyfunction]
fn _register_exceptions(mapping: &Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
    register_exception_classes(mapping)
}

/// Encode a picture into a `.vpng` file.
///
/// The file is written through a temporary file in the target's directory and
/// renamed over it, so a failed encode leaves no partial file behind.
///
/// `lossless` must be true: VPNG has no lossy mode, and there is never a
/// lossy fallback.
// Every option is keyword-only on the Python side, so a caller never has to
// remember the order of ten arguments.
#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (
    input,
    output,
    *,
    lossless = true,
    max_size = None,
    adaptive_tiles = false,
    pyramid_levels = 0,
    codec = "auto",
    codec_set = "stable",
    tile_size = DEFAULT_TILE_SIZE,
    workers = 1
))]
fn encode(
    input: PathBuf,
    output: PathBuf,
    lossless: bool,
    max_size: Option<u64>,
    adaptive_tiles: bool,
    pyramid_levels: u32,
    codec: &str,
    codec_set: &str,
    tile_size: u32,
    workers: usize,
) -> PyResult<EncodeResult> {
    let limits = Limits::default();
    let image = to_py_result(read_picture(&input, &limits))?;
    encode_image(
        &image,
        output,
        lossless,
        max_size,
        adaptive_tiles,
        pyramid_levels,
        codec,
        codec_set,
        tile_size,
        workers,
    )
}

/// Encode canonical pixel bytes straight into a `.vpng` file.
///
/// The pixels are **canonical**: rows are tightly packed with no padding, and
/// a 16-bit sample is little-endian. This is the same order the format
/// stores, and the same order [`decode_to_bytes`] returns.
///
/// This is the entry point for a caller that already has the pixels in
/// memory; it never writes an intermediate PNG.
#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (
    pixels,
    width,
    height,
    output,
    *,
    format = "rgba8",
    lossless = true,
    max_size = None,
    adaptive_tiles = false,
    pyramid_levels = 0,
    codec = "auto",
    codec_set = "stable",
    tile_size = DEFAULT_TILE_SIZE,
    workers = 1
))]
fn encode_pixels(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    output: PathBuf,
    format: &str,
    lossless: bool,
    max_size: Option<u64>,
    adaptive_tiles: bool,
    pyramid_levels: u32,
    codec: &str,
    codec_set: &str,
    tile_size: u32,
    workers: usize,
) -> PyResult<EncodeResult> {
    let limits = Limits::default();
    let format: PixelFormat = to_py_result(parse_format(format))?;
    let image = to_py_result(CanonicalImage::from_vec_checked(
        width, height, format, pixels, &limits,
    ))?;
    encode_image(
        &image,
        output,
        lossless,
        max_size,
        adaptive_tiles,
        pyramid_levels,
        codec,
        codec_set,
        tile_size,
        workers,
    )
}

/// The shared body of the two encode entry points.
///
/// Both the file form and the in-memory form end here, so they cannot
/// disagree about the options, the version, or the bytes.
#[allow(clippy::too_many_arguments)]
fn encode_image(
    image: &CanonicalImage,
    output: PathBuf,
    lossless: bool,
    max_size: Option<u64>,
    adaptive_tiles: bool,
    pyramid_levels: u32,
    codec: &str,
    codec_set: &str,
    tile_size: u32,
    workers: usize,
) -> PyResult<EncodeResult> {
    if !lossless {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "VPNG is lossless: there is no lossy mode and no lossy fallback, \
             so `lossless=False` is refused rather than ignored",
        ));
    }
    if tile_size == 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "the tile size must be non-zero",
        ));
    }
    if workers == 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "the worker count must be at least one",
        ));
    }

    let limits = Limits::default();

    let codec_id = vpng_codecs::codec_id_by_name(codec).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!(
            "unknown codec `{codec}` (available: auto, raw, palette, rle, predictor, \
             lz77, predictor+dictionary, context-entropy)"
        ))
    })?;
    let set = vpng_codecs::codec_set_by_name(codec_set).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!(
            "unknown codec set `{codec_set}` (available: stable, extended)"
        ))
    })?;

    let mut options = EncodeOptions::default()
        .with_tile_size(tile_size, tile_size)
        .with_codec(codec_id)
        .with_codec_set(set)
        .with_limits(limits);
    if let Some(limit) = max_size {
        options = options.with_max_size(limit);
    }
    if adaptive_tiles {
        options = options.with_adaptive();
    }
    if pyramid_levels > 0 {
        options = options.with_pyramid(pyramid_levels);
    }

    let parallel = ParallelOptions::default().with_workers(workers);
    let report = to_py_result(encode_to_file(image, &options, &parallel, &output))?;

    // The streaming writer decides the version from what the file contains;
    // read the header back so the report says what was actually written.
    let bytes = io_result(std::fs::read(&output))?;
    let header = to_py_result(vpng_container::Header::parse(&bytes))?;
    Ok(EncodeResult::from_stream(&report).with_minor(header.minor))
}

/// Encode several layers into a `.vpng` file.
///
/// The file is a **v0.4** file: it carries a layer table, and its directory
/// entries name the layer each tile belongs to. The layers are encoded
/// independently of one another, and each layer's bytes are accounted for on
/// its own.
///
/// Each layer is a mapping with these keys:
///
/// * `pixels` — the layer's canonical pixel bytes (**required**);
/// * `width` and `height` — the shape of those pixels (**required**);
/// * `name` — the layer's name (default: empty);
/// * `kind` — `"pixels"` (the default) or `"mask"`;
/// * `x` and `y` — where the layer sits on the canvas (default: 0, 0);
/// * `opacity` — 0 to 255 (default: 255);
/// * `mask` — the index of the layer that masks this one (default: none).
///
/// The base layer is layer 0: it must cover the whole canvas.
#[pyfunction]
#[pyo3(signature = (
    output,
    layers,
    *,
    format = "rgba8",
    tile_size = DEFAULT_TILE_SIZE,
    workers = 1
))]
fn encode_layers(
    output: PathBuf,
    layers: Vec<Bound<'_, PyAny>>,
    format: &str,
    tile_size: u32,
    workers: usize,
) -> PyResult<EncodeResult> {
    if tile_size == 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "the tile size must be non-zero",
        ));
    }
    if workers == 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "the worker count must be at least one",
        ));
    }
    if layers.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "a file needs at least one layer",
        ));
    }

    let limits = Limits::default();
    let pixel_format: PixelFormat = to_py_result(parse_format(format))?;

    let mut images: Vec<CanonicalImage> = Vec::with_capacity(layers.len());
    let mut specs: Vec<LayerSpec> = Vec::with_capacity(layers.len());

    for (index, entry) in layers.iter().enumerate() {
        let dict = entry.cast::<pyo3::types::PyDict>().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(format!("layer {index} must be a mapping"))
        })?;
        let pixels = match dict.get_item("pixels")? {
            Some(value) => value.extract::<Vec<u8>>().map_err(|_| {
                pyo3::exceptions::PyTypeError::new_err(format!(
                    "layer {index}'s `pixels` must be a bytes-like object"
                ))
            })?,
            None => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "layer {index} needs `pixels`"
                )))
            }
        };
        let width = required_u32(dict, "width", index)?;
        let height = required_u32(dict, "height", index)?;
        let image = to_py_result(CanonicalImage::from_vec_checked(
            width,
            height,
            pixel_format,
            pixels,
            &limits,
        ))?;

        let name = optional_string(dict, "name")?.unwrap_or_default();
        let kind = optional_string(dict, "kind")?.unwrap_or_else(|| "pixels".to_string());
        let x = optional_u32(dict, "x")?.unwrap_or(0);
        let y = optional_u32(dict, "y")?.unwrap_or(0);
        let opacity = optional_u32(dict, "opacity")?.unwrap_or(255).min(255) as u8;
        let rect = to_py_result(Rect::new(x, y, width, height))?;

        match kind.as_str() {
            "pixels" | "image" | "mask" => {}
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "layer {index} has the kind `{other}`; it must be `pixels` or `mask`"
                )))
            }
        }

        images.push(image);
        specs.push(LayerSpec {
            name,
            is_mask: kind == "mask",
            rect,
            opacity,
            mask: optional_u32(dict, "mask")?.map(|mask| mask as usize),
        });
    }

    // The layers borrow their images, so they are built after every image is
    // in place.
    let mut inputs: Vec<vpng_encoder::LayerInput<'_>> = Vec::with_capacity(specs.len());
    for (spec, image) in specs.iter().zip(images.iter()) {
        let mut layer = if spec.is_mask {
            vpng_encoder::LayerInput::mask(spec.name.clone(), spec.rect, image)
        } else {
            vpng_encoder::LayerInput::pixels(spec.name.clone(), spec.rect, image)
        };
        layer.opacity = spec.opacity;
        if let Some(mask) = spec.mask {
            layer = layer.with_mask(mask);
        }
        inputs.push(layer);
    }

    let options = EncodeOptions::default()
        .with_tile_size(tile_size, tile_size)
        .with_limits(limits);
    let parallel = ParallelOptions::default().with_workers(workers);
    let report = to_py_result(vpng_encoder::stream::encode_layers_to_file(
        &inputs, &options, &parallel, &output,
    ))?;

    let bytes = io_result(std::fs::read(&output))?;
    let header = to_py_result(vpng_container::Header::parse(&bytes))?;
    Ok(EncodeResult::from_stream(&report).with_minor(header.minor))
}

/// What a layer's description says, apart from its pixels.
struct LayerSpec {
    /// The layer's name.
    name: String,
    /// Whether the layer is a mask rather than picture pixels.
    is_mask: bool,
    /// Where the layer sits on the canvas.
    rect: Rect,
    /// The layer's opacity, 0 to 255.
    opacity: u8,
    /// The index of the layer that masks this one.
    mask: Option<usize>,
}

/// A required integer in a layer's description.
fn required_u32(entry: &Bound<'_, pyo3::types::PyDict>, key: &str, index: usize) -> PyResult<u32> {
    match entry.get_item(key)? {
        Some(value) => value.extract::<u32>().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(format!(
                "layer {index}'s `{key}` must be an integer"
            ))
        }),
        None => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "layer {index} needs `{key}`"
        ))),
    }
}

/// An optional integer in a layer's description.
fn optional_u32(entry: &Bound<'_, pyo3::types::PyDict>, key: &str) -> PyResult<Option<u32>> {
    match entry.get_item(key)? {
        None => Ok(None),
        Some(value) => {
            if value.is_none() {
                Ok(None)
            } else {
                value.extract::<u32>().map(Some).map_err(|_| {
                    pyo3::exceptions::PyTypeError::new_err(format!("`{key}` must be an integer"))
                })
            }
        }
    }
}

/// An optional string in a layer's description.
fn optional_string(entry: &Bound<'_, pyo3::types::PyDict>, key: &str) -> PyResult<Option<String>> {
    match entry.get_item(key)? {
        None => Ok(None),
        Some(value) => {
            if value.is_none() {
                Ok(None)
            } else {
                value.extract::<String>().map(Some).map_err(|_| {
                    pyo3::exceptions::PyTypeError::new_err(format!("`{key}` must be a string"))
                })
            }
        }
    }
}

/// Decode a `.vpng` file into a picture.
///
/// `level` picks a preview level; zero (the default) is the original, which
/// is always exact. The output is a PNG, or a raw file when the path ends in
/// `.raw`.
#[pyfunction]
#[pyo3(signature = (source, output, *, level = 0))]
fn decode(source: PathBuf, output: PathBuf, level: u32) -> PyResult<DecodeResult> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let image = to_py_result(document.decode_level(level as usize, &limits))?;
    to_py_result(write_picture(&output, &image))?;
    let written_bytes = io_result(std::fs::metadata(&output).map(|m| m.len()))?;
    Ok(DecodeResult::new(
        image.width(),
        image.height(),
        format_name(image.format()),
        bytes.len() as u64,
        level,
        written_bytes,
    ))
}

/// Decode a `.vpng` file into canonical pixel bytes.
///
/// The bytes are **canonical**: rows are tightly packed with no padding, and
/// a 16-bit sample is little-endian. This is the same order the format
/// stores.
#[pyfunction]
#[pyo3(signature = (source, *, level = 0))]
fn decode_to_bytes<'py>(
    source: PathBuf,
    level: u32,
    py: Python<'py>,
) -> PyResult<Bound<'py, PyBytes>> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let image = to_py_result(document.decode_level(level as usize, &limits))?;
    Ok(PyBytes::new(py, image.data()))
}

/// Decode a region of a `.vpng` file into a picture.
#[pyfunction]
#[pyo3(signature = (source, output, x, y, width, height))]
fn decode_region(
    source: PathBuf,
    output: PathBuf,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> PyResult<DecodeResult> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let rect = to_py_result(Rect::new(x, y, width, height))?;
    let image = to_py_result(document.decode_region(rect, &limits))?;
    to_py_result(write_picture(&output, &image))?;
    let written_bytes = io_result(std::fs::metadata(&output).map(|m| m.len()))?;
    Ok(DecodeResult::new(
        image.width(),
        image.height(),
        format_name(image.format()),
        bytes.len() as u64,
        0,
        written_bytes,
    ))
}

/// List the layers of a `.vpng` file.
///
/// A file written before format v0.4 has exactly one layer: the base layer,
/// which is the picture itself. A v0.4 file may hold more, each with a name,
/// a rectangle on the canvas, an opacity and — optionally — a mask.
///
/// Every layer is reported with its own byte accounting: the pixels it holds
/// and the bytes its payloads take.
#[pyfunction]
fn layers<'py>(source: PathBuf, py: Python<'py>) -> PyResult<Vec<Bound<'py, pyo3::types::PyDict>>> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;

    let mut out = Vec::with_capacity(document.layer_count());
    for layer in document.layers() {
        let rect = layer.rect();
        let info = pyo3::types::PyDict::new(py);
        info.set_item("index", layer.index())?;
        info.set_item("name", layer.name())?;
        info.set_item("kind", if layer.is_mask() { "mask" } else { "pixels" })?;
        info.set_item("is_mask", layer.is_mask())?;
        info.set_item("x", rect.x)?;
        info.set_item("y", rect.y)?;
        info.set_item("width", rect.width)?;
        info.set_item("height", rect.height)?;
        info.set_item("opacity", layer.opacity())?;
        info.set_item("mask", layer.mask())?;
        info.set_item("tile_count", layer.base_level().tile_count())?;
        info.set_item("uncompressed_bytes", layer.uncompressed_bytes())?;
        info.set_item("compressed_bytes", layer.compressed_bytes())?;
        info.set_item("ratio", layer.ratio())?;
        out.push(info);
    }
    Ok(out)
}

/// Decode one layer's own pixels into a picture.
///
/// The result is sized to the **layer's own rectangle**, not to the canvas,
/// and it holds the layer's pixels exactly. A mask layer decodes to its
/// coverage image: the same pixels, of which only the alpha channel carries
/// meaning.
#[pyfunction]
fn decode_layer(source: PathBuf, output: PathBuf, layer: u32) -> PyResult<DecodeResult> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let image = to_py_result(document.decode_layer(layer as usize, &limits))?;
    to_py_result(write_picture(&output, &image))?;
    let written_bytes = io_result(std::fs::metadata(&output).map(|m| m.len()))?;
    Ok(DecodeResult::new(
        image.width(),
        image.height(),
        format_name(image.format()),
        bytes.len() as u64,
        layer,
        written_bytes,
    ))
}

/// Decode one layer's own pixels into canonical pixel bytes.
///
/// The bytes are **canonical**: rows are tightly packed with no padding, and
/// a 16-bit sample is little-endian.
#[pyfunction]
fn decode_layer_to_bytes<'py>(
    source: PathBuf,
    layer: u32,
    py: Python<'py>,
) -> PyResult<Bound<'py, PyBytes>> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let image = to_py_result(document.decode_layer(layer as usize, &limits))?;
    Ok(PyBytes::new(py, image.data()))
}

/// Composite every layer into one picture.
///
/// The base layer is the canvas, exactly. Each layer above it is blended over
/// the result with its own opacity and — when it has one — its mask. A file
/// with one layer composites to exactly that layer's pixels.
#[pyfunction]
fn composite(source: PathBuf, output: PathBuf) -> PyResult<DecodeResult> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let image = to_py_result(document.composite(&limits))?;
    to_py_result(write_picture(&output, &image))?;
    let written_bytes = io_result(std::fs::metadata(&output).map(|m| m.len()))?;
    Ok(DecodeResult::new(
        image.width(),
        image.height(),
        format_name(image.format()),
        bytes.len() as u64,
        0,
        written_bytes,
    ))
}

/// Composite every layer into canonical pixel bytes.
#[pyfunction]
fn composite_to_bytes<'py>(source: PathBuf, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let image = to_py_result(document.composite(&limits))?;
    Ok(PyBytes::new(py, image.data()))
}

/// Compare a `.vpng` file with the picture it was made from.
///
/// Returns `True` when the decoded pixels are **exactly** the original's, and
/// `False` when they are not. A file that cannot be read or decoded raises
/// instead: that is a different failure from "the pixels differ".
#[pyfunction]
fn verify(input: PathBuf, source: PathBuf) -> PyResult<bool> {
    let limits = Limits::default();
    let original = to_py_result(read_picture(&input, &limits))?;
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let decoded = to_py_result(document.decode_all(&limits))?;
    Ok(decoded == original)
}

/// Check a `.vpng` file's internal integrity.
///
/// Every tile payload's CRC32C is verified. No pixels are decoded and no
/// pixel buffer is allocated.
#[pyfunction]
fn check(source: PathBuf) -> PyResult<CheckResult> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let report = to_py_result(document.check_integrity())?;
    Ok(CheckResult::new(report.tiles_checked, report.payload_bytes))
}

/// Read a `.vpng` file's header and directory.
///
/// This is O(1) in the tile count, and it cannot detect a corrupted payload:
/// use `check()` for that.
#[pyfunction]
fn info<'py>(source: PathBuf, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let limits = Limits::default();
    let bytes = io_result(std::fs::read(&source))?;
    let document = to_py_result(Document::parse(&bytes, &limits))?;
    let header = *document.header();
    let (nominal_width, nominal_height) = document.partition().nominal_tile_size();
    let (typical_width, typical_height) = document.partition().typical_tile_size();
    let info = pyo3::types::PyDict::new(py);
    info.set_item("file", source.to_string_lossy().into_owned())?;
    info.set_item("file_size", bytes.len())?;
    info.set_item("major", header.major)?;
    info.set_item("minor", header.minor)?;
    info.set_item("width", header.width)?;
    info.set_item("height", header.height)?;
    info.set_item("color_model", header.color_model.name())?;
    info.set_item("bits_per_channel", header.bits.bits())?;
    info.set_item("channels", header.channels())?;
    info.set_item("default_codec", header.default_codec)?;
    info.set_item("tile_count", header.tile_count)?;
    info.set_item("nominal_tile_width", nominal_width)?;
    info.set_item("nominal_tile_height", nominal_height)?;
    info.set_item("typical_tile_width", typical_width)?;
    info.set_item("typical_tile_height", typical_height)?;
    info.set_item("uniform_tiles", document.partition().is_uniform_size())?;
    info.set_item("level_count", document.level_count())?;
    info.set_item("directory_offset", header.directory_offset)?;
    info.set_item("declared_file_size", header.declared_file_size)?;
    info.set_item("feature_flags", header.feature_flags)?;
    info.set_item("payloads_verified", false)?;
    Ok(info)
}

/// Open a `.vpng` file for editing.
///
/// The document is a context manager. `cache_bytes` bounds the decoded-tile
/// cache; `history_bytes` bounds the undo history.
#[pyfunction]
#[pyo3(signature = (source, *, cache_bytes = DEFAULT_CACHE_BYTES, history_bytes = vpng_editing::DEFAULT_HISTORY_BYTES))]
fn open_document(source: PathBuf, cache_bytes: u64, history_bytes: u64) -> PyResult<PyDocument> {
    let limits = Limits::default();
    let document = to_py_result(PyDocument::open(
        &source,
        &limits,
        cache_bytes,
        history_bytes,
    ))?;
    Ok(document)
}

/// The pixel format a name refers to.
#[pyfunction]
fn pixel_format(name: &str) -> PyResult<String> {
    let format: PixelFormat = to_py_result(parse_format(name))?;
    Ok(format_name(format))
}

/// The Python extension module.
///
/// The module is named `vpng._vpng`; import `vpng`, not this module.
#[pymodule]
fn _vpng(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("FORMAT_VERSION", FORMAT_VERSION)?;
    module.add("DEFAULT_TILE_SIZE", DEFAULT_TILE_SIZE)?;
    module.add("DEFAULT_CACHE_BYTES", DEFAULT_CACHE_BYTES)?;
    module.add(
        "FORMATS",
        pyo3::types::PyTuple::new(module.py(), image::FORMAT_NAMES)?,
    )?;
    module.add(
        "CODECS",
        pyo3::types::PyTuple::new(
            module.py(),
            vpng_codecs::IMPLEMENTED_CODEC_IDS.iter().copied(),
        )?,
    )?;
    module.add(
        "CODEC_SETS",
        pyo3::types::PyTuple::new(module.py(), ["stable", "extended"])?,
    )?;

    module.add_function(wrap_pyfunction!(encode, module)?)?;
    module.add_function(wrap_pyfunction!(encode_pixels, module)?)?;
    module.add_function(wrap_pyfunction!(encode_layers, module)?)?;
    module.add_function(wrap_pyfunction!(decode, module)?)?;
    module.add_function(wrap_pyfunction!(decode_to_bytes, module)?)?;
    module.add_function(wrap_pyfunction!(decode_region, module)?)?;
    module.add_function(wrap_pyfunction!(layers, module)?)?;
    module.add_function(wrap_pyfunction!(decode_layer, module)?)?;
    module.add_function(wrap_pyfunction!(decode_layer_to_bytes, module)?)?;
    module.add_function(wrap_pyfunction!(composite, module)?)?;
    module.add_function(wrap_pyfunction!(composite_to_bytes, module)?)?;
    module.add_function(wrap_pyfunction!(verify, module)?)?;
    module.add_function(wrap_pyfunction!(check, module)?)?;
    module.add_function(wrap_pyfunction!(info, module)?)?;
    module.add_function(wrap_pyfunction!(open_document, module)?)?;
    module.add_function(wrap_pyfunction!(pixel_format, module)?)?;
    module.add_function(wrap_pyfunction!(_register_exceptions, module)?)?;

    module.add("VpngError", module.py().get_type::<error::VpngError>())?;
    module.add_class::<image::Region>()?;
    module.add_class::<report::EncodeResult>()?;
    module.add_class::<report::DecodeResult>()?;
    module.add_class::<report::CheckResult>()?;
    module.add_class::<report::CommitResult>()?;
    module.add_class::<document::Document>()?;
    module.add_class::<document::Transaction>()?;

    Ok(())
}
